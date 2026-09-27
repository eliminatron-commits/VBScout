//! Importing result files.
//!
//! Users pick files or whole folders (MSPs collect hundreds of `.vbscout`
//! files on a share). Folders are searched recursively for the result file
//! extension; all files are read in parallel. A file that cannot be read is
//! reported with its reason – it never disappears silently – and a run that
//! is already loaded (same scan ID) is counted as a duplicate. The edition's
//! machine limit is enforced here: files of further machines are listed as
//! not loaded (another scan of a machine that is already loaded is fine).

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use uuid::Uuid;
use vbs_core::{FormatError, ScanResult};

use crate::assessment::MachineKey;

/// A successfully read result file.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportedFile {
    pub path: PathBuf,
    pub result: ScanResult,
    /// Values from a newer format revision shown generically (see `vbs_core::Loaded`).
    pub unknown_values: usize,
}

/// Why a file could not be imported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportProblem {
    NotAResultFile,
    WrongFormat(String),
    NewerSchema { found: u32, supported: u32 },
    Invalid(String),
    TooLarge(String),
    Io(String),
}

impl ImportProblem {
    /// Stable code for the frontend (`import.error.<code>` translation keys).
    pub fn code(&self) -> &'static str {
        match self {
            ImportProblem::NotAResultFile => "notResultFile",
            ImportProblem::WrongFormat(_) => "wrongFormat",
            ImportProblem::NewerSchema { .. } => "newerSchema",
            ImportProblem::Invalid(_) => "invalid",
            ImportProblem::TooLarge(_) => "tooLarge",
            ImportProblem::Io(_) => "io",
        }
    }
}

impl From<FormatError> for ImportProblem {
    fn from(error: FormatError) -> Self {
        match error {
            FormatError::NotAResultFile => ImportProblem::NotAResultFile,
            FormatError::WrongFormat(format) => ImportProblem::WrongFormat(format),
            FormatError::NewerSchema { found, supported } => ImportProblem::NewerSchema { found, supported },
            FormatError::UnsupportedSchema(version) => ImportProblem::Invalid(format!("schema version {version}")),
            FormatError::Zip(message) | FormatError::Invalid(message) => ImportProblem::Invalid(message),
            FormatError::LimitExceeded(message) => ImportProblem::TooLarge(message),
            FormatError::Io(error) => ImportProblem::Io(error.to_string()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportError {
    pub path: PathBuf,
    pub problem: ImportProblem,
}

/// Outcome of one import action.
#[derive(Debug, Default)]
pub struct ImportBatch {
    pub files: Vec<ImportedFile>,
    pub errors: Vec<ImportError>,
    /// Files whose scan is already loaded (or appears twice in this batch).
    pub duplicates: Vec<PathBuf>,
    /// Files of further machines that the edition's machine limit does not allow.
    pub over_limit: Vec<PathBuf>,
}

/// What is loaded already: scan IDs (duplicates) and machines (the machine limit).
#[derive(Debug, Clone, Default)]
pub struct LoadedState {
    pub scan_ids: HashSet<Uuid>,
    pub machines: HashSet<MachineKey>,
}

impl LoadedState {
    pub fn of(files: &[ImportedFile]) -> Self {
        Self {
            scan_ids: files.iter().map(|file| file.result.scan_id).collect(),
            machines: files.iter().map(|file| MachineKey::of(&file.result)).collect(),
        }
    }
}

/// Expands `paths` – files as given, folders recursively (result files only) – in a stable order.
pub fn collect_paths(paths: &[PathBuf]) -> (Vec<PathBuf>, Vec<ImportError>) {
    let extension = vbs_core::file_extension();
    let mut files = Vec::new();
    let mut errors = Vec::new();
    let mut stack: Vec<PathBuf> = paths.to_vec();
    while let Some(path) = stack.pop() {
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.is_dir() => match fs::read_dir(&path) {
                Ok(entries) => {
                    for entry in entries.flatten() {
                        let child = entry.path();
                        let is_dir = entry.file_type().is_ok_and(|t| t.is_dir());
                        let wanted =
                            child.extension().is_some_and(|ext| ext.to_string_lossy().eq_ignore_ascii_case(extension));
                        if is_dir {
                            stack.push(child);
                        } else if wanted {
                            files.push(child);
                        }
                    }
                }
                Err(error) => errors.push(ImportError { path, problem: ImportProblem::Io(error.to_string()) }),
            },
            Ok(_) => files.push(path),
            Err(error) => errors.push(ImportError { path, problem: ImportProblem::Io(error.to_string()) }),
        }
    }
    files.sort();
    files.dedup();
    (files, errors)
}

/// Reads all result files under `paths` in parallel. Scans that are already
/// `loaded` (or appear twice) are reported as duplicates; files of more distinct
/// machines than `machine_limit` allows are reported as over the limit.
pub fn import(paths: &[PathBuf], loaded: &LoadedState, machine_limit: Option<usize>) -> ImportBatch {
    let (files, mut errors) = collect_paths(paths);
    let results = read_parallel(&files);

    let mut batch = ImportBatch::default();
    let mut seen = loaded.scan_ids.clone();
    let mut machines = loaded.machines.clone();
    for (path, outcome) in files.into_iter().zip(results) {
        match outcome {
            Ok(file) if seen.contains(&file.result.scan_id) => batch.duplicates.push(path),
            Ok(file) => {
                let machine = MachineKey::of(&file.result);
                if !machines.contains(&machine) && machine_limit.is_some_and(|limit| machines.len() >= limit) {
                    batch.over_limit.push(path);
                    continue;
                }
                seen.insert(file.result.scan_id);
                machines.insert(machine);
                batch.files.push(file);
            }
            Err(problem) => errors.push(ImportError { path, problem }),
        }
    }
    batch.errors = errors;
    batch
}

fn read_one(path: &Path) -> Result<ImportedFile, ImportProblem> {
    let loaded = vbs_core::read_file(path)?;
    Ok(ImportedFile { path: path.to_path_buf(), result: loaded.result, unknown_values: loaded.unknown_values })
}

/// Reads `files` with a few threads; results keep the order of `files`.
fn read_parallel(files: &[PathBuf]) -> Vec<Result<ImportedFile, ImportProblem>> {
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).clamp(1, 8).min(files.len().max(1));
    let next = Mutex::new(0usize);
    let slots: Vec<Mutex<Option<Result<ImportedFile, ImportProblem>>>> =
        files.iter().map(|_| Mutex::new(None)).collect();
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| {
                loop {
                    let index = {
                        let mut next = next.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                        let index = *next;
                        *next += 1;
                        index
                    };
                    let Some(path) = files.get(index) else { break };
                    let outcome = read_one(path);
                    *slots[index].lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(outcome);
                }
            });
        }
    });
    slots
        .into_iter()
        .map(|slot| {
            slot.into_inner()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .unwrap_or_else(|| Err(ImportProblem::Io("not read".into())))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;
    use vbs_core::model::*;

    fn sample(hostname: &str) -> ScanResult {
        ScanResult {
            format: vbs_core::media_type().into(),
            schema_version: SCHEMA_VERSION,
            scan_id: Uuid::new_v4(),
            generator: Generator {
                product: "test".into(),
                component: "collector".into(),
                version: "0.1.0".into(),
                rules_as_of: "2026-09-26".into(),
                platform: "windows".into(),
            },
            started_at: datetime!(2026-09-26 10:00 UTC),
            finished_at: datetime!(2026-09-26 10:05 UTC),
            machine: Machine {
                hostname: hostname.into(),
                fqdn: None,
                domain: None,
                machine_id: None,
                os: OperatingSystem { family: "windows".into(), ..OperatingSystem::default() },
            },
            scope: Scope { local_drives: true, paths: vec![], network_paths: vec![], system_sources: true },
            coverage: Coverage { mode: CoverageMode::Full, elevated: true, limitations: vec![], sources: vec![] },
            findings: vec![],
        }
    }

    fn write(path: &Path, result: &ScanResult) {
        fs::write(path, vbs_core::to_bytes(result).unwrap()).unwrap();
    }

    #[test]
    fn imports_files_and_folders_and_reports_problems() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("customer-a").join("site-1");
        fs::create_dir_all(&nested).unwrap();
        let extension = vbs_core::file_extension();
        let first = sample("PC-1");
        write(&dir.path().join(format!("pc1.{extension}")), &first);
        write(&nested.join(format!("pc2.{}", extension.to_uppercase())), &sample("PC-2"));
        write(&nested.join(format!("pc1-copy.{extension}")), &first);
        fs::write(nested.join(format!("broken.{extension}")), b"not a zip").unwrap();
        fs::write(nested.join("notes.txt"), b"ignored").unwrap();

        let batch = import(&[dir.path().to_path_buf()], &LoadedState::default(), None);
        let mut hosts: Vec<_> = batch.files.iter().map(|f| f.result.machine.hostname.as_str()).collect();
        hosts.sort_unstable();
        assert_eq!(hosts, ["PC-1", "PC-2"]);
        assert_eq!(batch.duplicates.len(), 1);
        assert_eq!(batch.errors.len(), 1);
        assert_eq!(batch.errors[0].problem, ImportProblem::NotAResultFile);

        // Importing again: everything is a duplicate now.
        let loaded = LoadedState::of(&batch.files);
        let again = import(&[dir.path().to_path_buf()], &loaded, None);
        assert!(again.files.is_empty());
        assert_eq!(again.duplicates.len(), 3);

        let missing = import(&[dir.path().join("missing")], &LoadedState::default(), None);
        assert_eq!(missing.errors[0].problem.code(), "io");
    }

    #[test]
    fn the_machine_limit_is_enforced_on_import() {
        let dir = tempfile::tempdir().unwrap();
        let extension = vbs_core::file_extension();
        for host in ["PC-1", "PC-2", "PC-3"] {
            write(&dir.path().join(format!("{host}.{extension}")), &sample(host));
        }
        let batch = import(&[dir.path().to_path_buf()], &LoadedState::default(), Some(2));
        assert_eq!(batch.files.len(), 2);
        assert_eq!(batch.over_limit.len(), 1);
        assert!(batch.over_limit[0].ends_with(format!("PC-3.{extension}")), "stable order: by path");

        // A newer scan of a loaded machine is still accepted at the limit.
        let loaded = LoadedState::of(&batch.files);
        write(&dir.path().join(format!("PC-1-again.{extension}")), &sample("PC-1"));
        let again = import(&[dir.path().join(format!("PC-1-again.{extension}"))], &loaded, Some(2));
        assert_eq!((again.files.len(), again.over_limit.len()), (1, 0));
    }

    #[test]
    fn newer_schemas_are_refused_with_a_clear_reason() {
        let problem: ImportProblem = FormatError::NewerSchema { found: 9, supported: 1 }.into();
        assert_eq!(problem.code(), "newerSchema");
    }
}
