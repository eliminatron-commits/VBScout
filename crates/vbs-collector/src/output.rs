//! The **only** place where the collector writes to disk.
//!
//! One run writes exactly one file – its result – and never touches anything
//! else: the file is created with `create_new`, so an existing file (even an
//! earlier result) is never replaced. `scripts/check-readonly.mjs` fails the
//! build if any other collector source writes, renames or deletes files.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use time::OffsetDateTime;

#[derive(Debug)]
pub enum OutputError {
    /// The target exists – nothing is overwritten.
    Exists(PathBuf),
    /// The target folder does not exist – the collector never creates folders.
    DirMissing(PathBuf),
    /// No `--out` and the current folder is a Windows system folder (e.g. `C:\Windows\System32`
    /// when started as SYSTEM through GPO): the collector does not put its file there.
    SystemFolder(PathBuf),
    Io(PathBuf, io::Error),
}

/// Decides where the result goes: into the `--out` folder, to the `--out`
/// file (which must carry the result file extension), or into the current
/// folder unless that is inside `system_root`. Fails early – before scanning –
/// if the folder is missing or the file exists already.
pub fn resolve(
    out: Option<&Path>,
    hostname: &str,
    at: OffsetDateTime,
    system_root: Option<&Path>,
) -> Result<PathBuf, OutputError> {
    let name = default_file_name(hostname, at);
    let path = match out {
        None => {
            let cwd = std::env::current_dir().map_err(|error| OutputError::Io(PathBuf::from("."), error))?;
            if system_root.is_some_and(|root| is_within(&cwd, root)) {
                return Err(OutputError::SystemFolder(cwd));
            }
            cwd.join(name)
        }
        Some(out) if out.is_dir() => out.join(name),
        // A file name carries the result extension; anything else names a folder, which must exist.
        Some(out) if has_result_extension(out) => out.to_path_buf(),
        Some(out) => return Err(OutputError::DirMissing(out.to_path_buf())),
    };
    let parent = path.parent().filter(|parent| !parent.as_os_str().is_empty()).unwrap_or(Path::new("."));
    if !parent.is_dir() {
        return Err(OutputError::DirMissing(parent.to_path_buf()));
    }
    if fs::symlink_metadata(&path).is_ok() {
        return Err(OutputError::Exists(path));
    }
    Ok(path)
}

fn has_result_extension(path: &Path) -> bool {
    path.extension().is_some_and(|ext| ext.to_string_lossy().eq_ignore_ascii_case(vbs_core::file_extension()))
}

/// `path` is `root` or lies below it (case-insensitive, as on Windows).
fn is_within(path: &Path, root: &Path) -> bool {
    let normalize = |p: &Path| p.to_string_lossy().replace('/', "\\").trim_end_matches('\\').to_lowercase();
    let (path, root) = (normalize(path), normalize(root));
    !root.is_empty() && (path == root || path.starts_with(&format!("{root}\\")))
}

/// `<computer>_<yyyymmdd-hhmmss>Z.<extension>` in UTC, e.g. `PC-042_20260926-141503Z.vbscout`.
pub fn default_file_name(hostname: &str, at: OffsetDateTime) -> String {
    let safe: String =
        hostname.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    let host = if safe.is_empty() { "computer".to_owned() } else { safe };
    format!(
        "{host}_{:04}{:02}{:02}-{:02}{:02}{:02}Z.{}",
        at.year(),
        u8::from(at.month()),
        at.day(),
        at.hour(),
        at.minute(),
        at.second(),
        vbs_core::file_extension()
    )
}

/// Creates the result file (never overwriting anything) and writes `bytes`.
pub fn write_result(path: &Path, bytes: &[u8]) -> Result<(), OutputError> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path).map_err(|error| {
        if error.kind() == io::ErrorKind::AlreadyExists {
            OutputError::Exists(path.to_path_buf())
        } else {
            OutputError::Io(path.to_path_buf(), error)
        }
    })?;
    if let Err(error) = file.write_all(bytes).and_then(|()| file.sync_all()) {
        drop(file);
        // `create_new` guarantees this run created the file: remove the incomplete result
        // so no half-written file is mistaken for a scan.
        let _ = fs::remove_file(path);
        return Err(OutputError::Io(path.to_path_buf(), error));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    #[test]
    fn default_names_are_safe_and_sortable() {
        let name = default_file_name("PC 042/ä", datetime!(2026-09-26 14:15:03 UTC));
        assert_eq!(name, format!("PC_042___20260926-141503Z.{}", vbs_core::file_extension()));
    }

    #[test]
    fn never_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let at = datetime!(2026-09-26 14:15:03 UTC);
        let path = resolve(Some(dir.path()), "PC1", at, None).unwrap();
        assert_eq!(path.parent(), Some(dir.path()));
        write_result(&path, b"first").unwrap();
        assert!(matches!(resolve(Some(dir.path()), "PC1", at, None), Err(OutputError::Exists(_))));
        assert!(matches!(resolve(Some(&path), "PC1", at, None), Err(OutputError::Exists(_))));
        assert!(matches!(write_result(&path, b"second"), Err(OutputError::Exists(_))));
        assert_eq!(fs::read(&path).unwrap(), b"first");
        let extension = vbs_core::file_extension();
        let missing = dir.path().join("no-such-folder").join(format!("x.{extension}"));
        assert!(matches!(resolve(Some(&missing), "PC1", at, None), Err(OutputError::DirMissing(_))));
        // Without the result extension, --out names a folder – and folders are never created.
        let folder = dir.path().join("results");
        assert!(matches!(resolve(Some(&folder), "PC1", at, None), Err(OutputError::DirMissing(p)) if p == folder));
        assert!(!folder.exists());
        let named = dir.path().join(format!("scan.{}", extension.to_uppercase()));
        assert_eq!(resolve(Some(&named), "PC1", at, None).unwrap(), named);
    }

    #[test]
    fn never_writes_into_system_folders_by_default() {
        let at = datetime!(2026-09-26 14:15:03 UTC);
        let cwd = std::env::current_dir().unwrap();
        assert!(matches!(resolve(None, "PC1", at, Some(&cwd)), Err(OutputError::SystemFolder(_))));
        if let Some(parent) = cwd.parent() {
            assert!(matches!(resolve(None, "PC1", at, Some(parent)), Err(OutputError::SystemFolder(_))));
        }
        let elsewhere = resolve(None, "PC1", at, Some(Path::new("/definitely/not/the/cwd"))).unwrap();
        assert_eq!(elsewhere.parent(), Some(cwd.as_path()));
        assert!(is_within(Path::new(r"C:\WINDOWS\system32"), Path::new(r"C:\Windows")));
        assert!(!is_within(Path::new(r"C:\WindowsApps"), Path::new(r"C:\Windows")));
    }
}
