//! Parallel, read-only walk over file roots.
//!
//! Rules that keep the walk harmless:
//! * links, junctions and mount points are never followed (no loops, no
//!   escaping the scope, no surprise network targets);
//! * online-only cloud files and offline files are never opened – reading them
//!   would download or recall them; candidates among them are reported as
//!   "not checkable";
//! * directories whose listing lives in the cloud are not entered;
//! * files are opened read-only with full sharing (see [`crate::read_only`]).
//!
//! Every candidate file (an extension some module wants) is inspected by the
//! interested modules in the worker thread that found it. A panic inside a
//! module is caught and turns into a "not checkable" finding for that file.

use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs::{self, DirEntry};
use std::io::BufReader;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, OnceLock, PoisonError};

use sha2::{Digest, Sha256};
use time::OffsetDateTime;
use vbs_core::model::{Finding, NotCheckableReason};
use vbs_core::module::{CandidateFile, Counters, Module, ReadError, ReadSeek, Report};

use crate::{platform, read_only};

/// Largest file a module may read completely; bigger containers are streamed through `open`.
pub const MAX_CONTENT_BYTES: u64 = 64 * 1024 * 1024;

/// What one walk found.
#[derive(Debug, Default)]
pub struct WalkOutcome {
    pub findings: Vec<Finding>,
    pub counters: Counters,
    /// Links, junctions and mount points that were not followed.
    pub links_skipped: u64,
    /// Online-only cloud directories that were not entered.
    pub cloud_dirs_skipped: u64,
}

impl WalkOutcome {
    fn merge(&mut self, other: WalkOutcome) {
        self.findings.extend(other.findings);
        self.counters.merge(other.counters);
        self.links_skipped += other.links_skipped;
        self.cloud_dirs_skipped += other.cloud_dirs_skipped;
    }
}

/// Directory queue shared by the workers; the walk ends when it is empty and nobody is busy.
struct Queue {
    state: Mutex<(Vec<PathBuf>, usize)>,
    changed: Condvar,
}

impl Queue {
    fn pop(&self) -> Option<PathBuf> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        loop {
            if let Some(dir) = state.0.pop() {
                state.1 += 1;
                return Some(dir);
            }
            if state.1 == 0 {
                self.changed.notify_all();
                return None;
            }
            state = self.changed.wait(state).unwrap_or_else(PoisonError::into_inner);
        }
    }

    fn done(&self, subdirs: Vec<PathBuf>) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.1 -= 1;
        state.0.extend(subdirs);
        self.changed.notify_all();
    }
}

/// Walks `roots` with `threads` workers and lets `modules` inspect the candidate files.
pub fn walk(roots: &[PathBuf], network: bool, modules: &[&dyn Module], threads: usize) -> WalkOutcome {
    let mut dispatch: HashMap<&str, Vec<usize>> = HashMap::new();
    for (index, module) in modules.iter().enumerate() {
        for extension in module.extensions() {
            dispatch.entry(extension).or_default().push(index);
        }
    }
    let queue = Queue { state: Mutex::new((roots.to_vec(), 0)), changed: Condvar::new() };
    let context = Context { dispatch: &dispatch, modules, network };

    let mut outcome = WalkOutcome::default();
    std::thread::scope(|scope| {
        let workers: Vec<_> = (0..threads.max(1))
            .map(|_| {
                scope.spawn(|| {
                    let mut local = WalkOutcome::default();
                    while let Some(dir) = queue.pop() {
                        let subdirs = context.visit_dir(&dir, &mut local);
                        queue.done(subdirs);
                    }
                    local
                })
            })
            .collect();
        for worker in workers {
            // Workers catch module panics themselves; a panic here would be a walker bug.
            if let Ok(local) = worker.join() {
                outcome.merge(local);
            }
        }
    });
    outcome
}

struct Context<'a> {
    dispatch: &'a HashMap<&'a str, Vec<usize>>,
    modules: &'a [&'a dyn Module],
    network: bool,
}

impl Context<'_> {
    /// Lists one directory; returns the subdirectories to visit.
    fn visit_dir(&self, dir: &Path, out: &mut WalkOutcome) -> Vec<PathBuf> {
        let mut subdirs = Vec::new();
        let entries = match fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(error) => {
                record_error(out, dir, &error.to_string());
                return subdirs;
            }
        };
        for entry in entries {
            match entry {
                Ok(entry) => self.visit_entry(&entry, &mut subdirs, out),
                Err(error) => record_error(out, dir, &error.to_string()),
            }
        }
        subdirs
    }

    fn visit_entry(&self, entry: &DirEntry, subdirs: &mut Vec<PathBuf>, out: &mut WalkOutcome) {
        out.counters.entries += 1;
        // The type comes with the directory listing; metadata (attributes, size, times) is only
        // fetched for directories and candidate files – most entries need nothing else.
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(error) => {
                record_error(out, &entry.path(), &error.to_string());
                return;
            }
        };
        if file_type.is_symlink() {
            out.links_skipped += 1;
            return;
        }
        if file_type.is_dir() {
            let path = entry.path();
            match entry.metadata() {
                Ok(metadata) if platform::entry_state(&metadata).recall_on_open => out.cloud_dirs_skipped += 1,
                Ok(_) if platform::skip_dir(&path) => {}
                Ok(_) => subdirs.push(path),
                Err(error) => record_error(out, &path, &error.to_string()),
            }
            return;
        }
        if !file_type.is_file() {
            return;
        }
        let extension = extension_of(&entry.file_name());
        let Some(interested) = self.dispatch.get(extension.as_str()) else { return };
        let path = entry.path();
        let metadata = match entry.metadata() {
            Ok(metadata) => metadata,
            Err(error) => {
                record_error(out, &path, &error.to_string());
                return;
            }
        };
        let state = platform::entry_state(&metadata);

        out.counters.inspected += 1;
        let file = DiskFile {
            path,
            extension,
            size: metadata.len(),
            modified: metadata.modified().ok().map(OffsetDateTime::from),
            network: self.network,
            placeholder: state.placeholder,
            encrypted: state.encrypted,
            contents: OnceLock::new(),
            sha256: OnceLock::new(),
        };
        for &index in interested {
            let module = self.modules[index];
            let mut report = Report::new();
            let inspected = panic::catch_unwind(AssertUnwindSafe(|| module.inspect_file(&file, &mut report)));
            if inspected.is_err() {
                // Discard whatever the failed module reported and say so instead.
                report = Report::new();
                report
                    .not_checkable(module.info().fallback_rule, file.location(), NotCheckableReason::InternalError)
                    .file(file.facts())
                    .emit();
            }
            let (findings, counters) = report.into_parts();
            out.findings.extend(findings);
            out.counters.merge(counters);
        }
    }
}

fn record_error(out: &mut WalkOutcome, path: &Path, message: &str) {
    out.counters.errors += 1;
    if out.counters.error_samples.len() < vbs_core::validate::limits::MAX_ERROR_SAMPLES {
        out.counters.error_samples.push(format!("{} ({message})", path.display()));
    }
}

/// Lower-case extension without the dot (empty if none).
fn extension_of(name: &OsStr) -> String {
    Path::new(name).extension().map(|ext| ext.to_string_lossy().to_lowercase()).unwrap_or_default()
}

/// A candidate file on disk; content is read at most once and shared by all modules.
struct DiskFile {
    path: PathBuf,
    extension: String,
    size: u64,
    modified: Option<OffsetDateTime>,
    network: bool,
    placeholder: bool,
    encrypted: bool,
    contents: OnceLock<Result<Arc<[u8]>, ReadError>>,
    sha256: OnceLock<String>,
}

impl DiskFile {
    fn refine(&self, error: ReadError) -> ReadError {
        match error {
            ReadError::AccessDenied if self.encrypted => ReadError::Encrypted,
            other => other,
        }
    }
}

impl CandidateFile for DiskFile {
    fn path(&self) -> &Path {
        &self.path
    }

    fn extension(&self) -> &str {
        &self.extension
    }

    fn size(&self) -> u64 {
        self.size
    }

    fn modified(&self) -> Option<OffsetDateTime> {
        self.modified
    }

    fn network(&self) -> bool {
        self.network
    }

    fn contents(&self, limit: u64) -> Result<Arc<[u8]>, ReadError> {
        let limit = limit.min(MAX_CONTENT_BYTES);
        if self.placeholder {
            return Err(ReadError::CloudPlaceholder);
        }
        if self.size > limit {
            return Err(ReadError::TooLarge { size: self.size, limit });
        }
        let contents = self.contents.get_or_init(|| {
            let bytes = read_only::read_limited(&self.path, MAX_CONTENT_BYTES).map_err(|e| self.refine(e))?;
            let _ = self.sha256.set(platform::hex(&Sha256::digest(&bytes)));
            Ok(bytes.into())
        });
        match contents {
            Ok(bytes) if bytes.len() as u64 > limit => Err(ReadError::TooLarge { size: bytes.len() as u64, limit }),
            other => other.clone(),
        }
    }

    fn open(&self) -> Result<Box<dyn ReadSeek + '_>, ReadError> {
        if self.placeholder {
            return Err(ReadError::CloudPlaceholder);
        }
        let file = read_only::open(&self.path).map_err(|error| self.refine(read_only::classify(&error)))?;
        Ok(Box::new(BufReader::new(file)))
    }

    fn sha256(&self) -> Option<String> {
        self.sha256.get().cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vbs_core::model::FindingStatus;
    use vbs_core::module::ModuleInfo;

    static INFO: ModuleInfo = ModuleInfo {
        id: "test",
        system_source: None,
        rules: &["VBS-101"],
        fallback_rule: "VBS-100",
        needs_admin: false,
    };

    /// Reports every `.vbs` file whose content mentions "boom" by panicking.
    struct Probe;

    impl Module for Probe {
        fn info(&self) -> &'static ModuleInfo {
            &INFO
        }

        fn extensions(&self) -> &'static [&'static str] {
            &["vbs"]
        }

        fn inspect_file(&self, file: &dyn CandidateFile, report: &mut Report) {
            let bytes = file.contents(1024).expect("readable");
            assert!(!bytes.starts_with(b"boom"), "simulated module bug");
            report.file_finding("VBS-101", file).evidence(Some(1), &String::from_utf8_lossy(&bytes)).emit();
        }
    }

    #[test]
    fn walks_in_parallel_and_survives_module_panics() {
        let dir = tempfile::tempdir().unwrap();
        for n in 0..40 {
            let sub = dir.path().join(format!("d{}", n % 7)).join(format!("e{}", n % 3));
            fs::create_dir_all(&sub).unwrap();
            fs::write(sub.join(format!("s{n}.VBS")), format!("WScript.Echo {n}")).unwrap();
            fs::write(sub.join(format!("n{n}.txt")), "no").unwrap();
        }
        fs::write(dir.path().join("broken.vbs"), "boom").unwrap();

        let outcome = walk(&[dir.path().to_path_buf()], false, &[&Probe], 4);
        let detected = outcome.findings.iter().filter(|f| f.status == FindingStatus::Detected).count();
        assert_eq!(detected, 40);
        let failed: Vec<_> = outcome.findings.iter().filter(|f| f.status == FindingStatus::NotCheckable).collect();
        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0].rule, "VBS-100");
        assert_eq!(failed[0].reason, Some(NotCheckableReason::InternalError));
        assert!(failed[0].location.path.ends_with("broken.vbs"));
        assert_eq!(outcome.counters.inspected, 41);
        assert!(outcome.counters.entries >= 81);
        // Every file was read completely (the failing module read it before panicking), so all carry a hash.
        let with_hash = outcome.findings.iter().filter(|f| f.file.as_ref().is_some_and(|x| x.sha256.is_some())).count();
        assert_eq!(with_hash, 41);
    }

    #[cfg(unix)]
    #[test]
    fn never_follows_links() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("outside.vbs"), "x").unwrap();
        std::os::unix::fs::symlink(outside.path(), dir.path().join("link")).unwrap();
        std::os::unix::fs::symlink(outside.path().join("outside.vbs"), dir.path().join("file-link.vbs")).unwrap();
        let outcome = walk(&[dir.path().to_path_buf()], false, &[&Probe], 2);
        assert!(outcome.findings.is_empty());
        assert_eq!(outcome.links_skipped, 2);
    }

    #[test]
    fn unreadable_roots_are_counted() {
        let outcome = walk(&[PathBuf::from("/definitely/not/here")], false, &[&Probe], 1);
        assert_eq!(outcome.counters.errors, 1);
        assert_eq!(outcome.counters.error_samples.len(), 1);
    }
}
