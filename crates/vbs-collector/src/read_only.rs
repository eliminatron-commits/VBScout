//! The collector's only way to open files – for reading, never for anything else.
//!
//! On Windows every handle is opened with read access and full sharing
//! (`FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE`), so the scan never
//! blocks another program from writing, renaming or deleting a file while it
//! is being read. Windows may still update a file's last-access time when it
//! is read, depending on the volume's policy – like any other reader, e.g. a
//! virus scanner (see CLAUDE.md).

use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, Read};
use std::path::Path;

use time::OffsetDateTime;
use vbs_core::module::{ReadError, ReadSeek};
use vbs_core::views::{DirEntryInfo, FileView, ViewError};

use crate::platform;

/// Opens `path` for reading only.
pub fn open(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::{
            FILE_FLAG_SEQUENTIAL_SCAN, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
        };
        options.share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE);
        options.custom_flags(FILE_FLAG_SEQUENTIAL_SCAN);
    }
    options.open(path)
}

/// Reads at most `limit` bytes; a longer file is `TooLarge` (it may have grown since it was listed).
pub fn read_limited(path: &Path, limit: u64) -> Result<Vec<u8>, ReadError> {
    let file = open(path).map_err(|error| classify(&error))?;
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes).map_err(|error| classify(&error))?;
    let size = bytes.len() as u64;
    if size > limit {
        return Err(ReadError::TooLarge { size, limit });
    }
    Ok(bytes)
}

/// Maps an I/O error to the reason a file could not be read.
pub fn classify(error: &io::Error) -> ReadError {
    // ERROR_SHARING_VIOLATION (32) and ERROR_LOCK_VIOLATION (33).
    #[cfg(windows)]
    if matches!(error.raw_os_error(), Some(32 | 33)) {
        return ReadError::Locked;
    }
    match error.kind() {
        io::ErrorKind::PermissionDenied => ReadError::AccessDenied,
        io::ErrorKind::NotFound => ReadError::NotFound,
        _ => ReadError::Io(error.to_string()),
    }
}

fn view_error(error: &io::Error) -> ViewError {
    match classify(error) {
        ReadError::AccessDenied => ViewError::AccessDenied,
        ReadError::NotFound => ViewError::NotFound,
        ReadError::Locked => ViewError::Locked,
        _ => ViewError::Failed(error.to_string()),
    }
}

/// [`FileView`] over the real local file system, read-only. Network paths and network drives
/// are refused: system modules follow paths from the registry, task definitions or policies,
/// and none of them may cause network access (the walk reads network paths only when they are
/// given explicitly with `--include-unc`).
#[derive(Debug, Default, Clone, Copy)]
pub struct ReadOnlyFiles;

impl ReadOnlyFiles {
    /// Refuses everything that is not a plain local path: network paths and drives, device
    /// paths (`\\.\pipe\…` would block), relative paths and paths through a link that
    /// points to the network.
    fn local(path: &Path) -> Result<(), ViewError> {
        if platform::is_network_location(path) {
            return Err(ViewError::NetworkPath);
        }
        let text = path.to_string_lossy();
        let verbatim_drive = text.strip_prefix(r"\\?\").is_some_and(|rest| rest.as_bytes().get(1) == Some(&b':'));
        if (text.starts_with(r"\\") || text.starts_with("//")) && !verbatim_drive {
            return Err(ViewError::Failed("device path (not read)".into()));
        }
        if !path.is_absolute() {
            return Err(ViewError::Failed("relative path (not read)".into()));
        }
        for ancestor in path.ancestors().skip(1) {
            let is_link = fs::symlink_metadata(ancestor).is_ok_and(|m| m.file_type().is_symlink());
            if is_link && fs::read_link(ancestor).is_ok_and(|target| platform::is_network_location(&target)) {
                return Err(ViewError::NetworkPath);
            }
        }
        Ok(())
    }

    /// A regular local file that can be read without side effects (no link, no device, no cloud placeholder).
    fn plain_file(path: &Path) -> Result<(), ViewError> {
        Self::local(path)?;
        let metadata = fs::symlink_metadata(path).map_err(|e| view_error(&e))?;
        if metadata.file_type().is_symlink() {
            return Err(ViewError::Failed("link (not followed)".into()));
        }
        if !metadata.is_file() {
            return Err(ViewError::Failed("not a regular file".into()));
        }
        if platform::entry_state(&metadata).placeholder {
            return Err(ViewError::CloudPlaceholder);
        }
        Ok(())
    }
}

impl FileView for ReadOnlyFiles {
    fn list_dir(&self, path: &Path) -> Result<Vec<DirEntryInfo>, ViewError> {
        Self::local(path)?;
        let metadata = fs::symlink_metadata(path).map_err(|e| view_error(&e))?;
        if metadata.file_type().is_symlink() {
            return Err(ViewError::Failed("link (not followed)".into()));
        }
        if platform::entry_state(&metadata).recall_on_open {
            return Err(ViewError::CloudPlaceholder);
        }
        let mut entries = Vec::new();
        for entry in fs::read_dir(path).map_err(|e| view_error(&e))? {
            let entry = entry.map_err(|e| view_error(&e))?;
            let file_type = entry.file_type().map_err(|e| view_error(&e))?;
            if file_type.is_symlink() {
                continue; // links and junctions are never followed
            }
            let metadata = entry.metadata().ok();
            let size = if file_type.is_file() { metadata.as_ref().map_or(0, fs::Metadata::len) } else { 0 };
            entries.push(DirEntryInfo {
                name: entry.file_name().to_string_lossy().into_owned(),
                is_dir: file_type.is_dir(),
                size,
                modified: metadata.and_then(|m| m.modified().ok()).map(OffsetDateTime::from),
            });
        }
        Ok(entries)
    }

    fn read(&self, path: &Path, limit: u64) -> Result<Vec<u8>, ViewError> {
        Self::plain_file(path)?;
        read_limited(path, limit).map_err(|error| match error {
            ReadError::AccessDenied => ViewError::AccessDenied,
            ReadError::NotFound => ViewError::NotFound,
            ReadError::TooLarge { .. } => ViewError::TooLarge,
            ReadError::Locked => ViewError::Locked,
            other => ViewError::Failed(other.to_string()),
        })
    }

    fn open(&self, path: &Path) -> Result<Box<dyn ReadSeek + '_>, ViewError> {
        Self::plain_file(path)?;
        let file = open(path).map_err(|error| view_error(&error))?;
        Ok(Box::new(BufReader::new(file)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_within_the_limit_only() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.vbs");
        std::fs::write(&path, b"MsgBox 1").unwrap();
        assert_eq!(read_limited(&path, 100).unwrap(), b"MsgBox 1");
        assert_eq!(read_limited(&path, 3), Err(ReadError::TooLarge { size: 4, limit: 3 }));
        assert_eq!(read_limited(&dir.path().join("missing"), 10), Err(ReadError::NotFound));
        let listed = ReadOnlyFiles.list_dir(dir.path()).unwrap();
        assert_eq!((listed[0].name.as_str(), listed[0].is_dir, listed[0].size), ("a.vbs", false, 8));
        assert!(listed[0].modified.is_some());
        assert_eq!(ReadOnlyFiles.read(&path, 2), Err(ViewError::TooLarge));
        let mut content = String::new();
        ReadOnlyFiles.open(&path).unwrap().read_to_string(&mut content).unwrap();
        assert_eq!(content, "MsgBox 1");
    }

    #[cfg(unix)]
    #[test]
    fn never_reads_devices_links_or_relative_paths() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("real.vbs"), "x").unwrap();
        std::os::unix::fs::symlink(dir.path().join("real.vbs"), dir.path().join("link.vbs")).unwrap();
        assert!(matches!(ReadOnlyFiles.read(&dir.path().join("link.vbs"), 10), Err(ViewError::Failed(_))));
        assert!(matches!(ReadOnlyFiles.read(Path::new("relative.vbs"), 10), Err(ViewError::Failed(_))));
        assert!(matches!(ReadOnlyFiles.read(Path::new("/dev/null"), 10), Err(ViewError::Failed(_))));
        assert_eq!(ReadOnlyFiles.read(&dir.path().join("missing.vbs"), 10), Err(ViewError::NotFound));
    }

    #[test]
    fn never_reads_network_paths() {
        for path in [r"\\server\share\logon.vbs", "//server/share/x.bat", r"\\?\UNC\srv\share\x"] {
            let path = Path::new(path);
            assert_eq!(ReadOnlyFiles.read(path, 10), Err(ViewError::NetworkPath));
            assert_eq!(ReadOnlyFiles.list_dir(path), Err(ViewError::NetworkPath));
            assert!(matches!(ReadOnlyFiles.open(path), Err(ViewError::NetworkPath)));
        }
    }
}
