//! The collector's only way to open files – for reading, never for anything else.
//!
//! On Windows every handle is opened with read access and full sharing
//! (`FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE`), so the scan never
//! blocks another program from writing, renaming or deleting a file while it
//! is being read. Windows may still update a file's last-access time when it
//! is read, depending on the volume's policy – like any other reader, e.g. a
//! virus scanner (see CLAUDE.md).

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read};
use std::path::Path;

use vbs_core::module::ReadError;
use vbs_core::views::{DirEntryInfo, FileView, ViewError};

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
    match error.kind() {
        io::ErrorKind::PermissionDenied => ViewError::AccessDenied,
        io::ErrorKind::NotFound => ViewError::NotFound,
        _ => ViewError::Failed(error.to_string()),
    }
}

/// [`FileView`] over the real file system, read-only.
#[derive(Debug, Default, Clone, Copy)]
pub struct ReadOnlyFiles;

impl FileView for ReadOnlyFiles {
    fn list_dir(&self, path: &Path) -> Result<Vec<DirEntryInfo>, ViewError> {
        let mut entries = Vec::new();
        for entry in fs::read_dir(path).map_err(|e| view_error(&e))? {
            let entry = entry.map_err(|e| view_error(&e))?;
            let file_type = entry.file_type().map_err(|e| view_error(&e))?;
            if file_type.is_symlink() {
                continue; // links and junctions are never followed
            }
            let size = if file_type.is_file() { entry.metadata().map(|m| m.len()).unwrap_or(0) } else { 0 };
            entries.push(DirEntryInfo {
                name: entry.file_name().to_string_lossy().into_owned(),
                is_dir: file_type.is_dir(),
                size,
            });
        }
        Ok(entries)
    }

    fn read(&self, path: &Path, limit: u64) -> Result<Vec<u8>, ViewError> {
        read_limited(path, limit).map_err(|error| match error {
            ReadError::AccessDenied => ViewError::AccessDenied,
            ReadError::NotFound => ViewError::NotFound,
            other => ViewError::Failed(other.to_string()),
        })
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
        assert_eq!(listed, [DirEntryInfo { name: "a.vbs".into(), is_dir: false, size: 8 }]);
        assert_eq!(
            ReadOnlyFiles.read(&path, 2),
            Err(ViewError::Failed(ReadError::TooLarge { size: 3, limit: 2 }.to_string()))
        );
    }
}
