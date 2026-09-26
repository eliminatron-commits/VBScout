//! Helpers shared by the collector's integration tests.
#![allow(dead_code, clippy::disallowed_methods)] // each test binary uses a subset; tests run the collector binary

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use sha2::{Digest, Sha256};

/// Repository root (plain absolute path – no `..`, no Windows verbatim prefix).
pub fn repo_root() -> PathBuf {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest.parent().and_then(Path::parent).expect("crates/<name> lies two levels below the root").to_path_buf()
}

/// `tests/corpus` in the repository.
pub fn corpus() -> PathBuf {
    repo_root().join("tests").join("corpus")
}

/// Copies a directory tree (test fixture setup).
pub fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target).unwrap();
        }
    }
}

/// Everything about a file that the collector must never change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileState {
    pub kind: &'static str,
    pub len: u64,
    pub modified: Option<std::time::SystemTime>,
    pub created: Option<std::time::SystemTime>,
    pub readonly: bool,
    #[cfg(unix)]
    pub mode: u32,
    #[cfg(windows)]
    pub attributes: u32,
    pub sha256: Option<String>,
    pub link_target: Option<PathBuf>,
}

/// Snapshot of a tree: relative path → state (links are recorded, not followed).
pub fn snapshot(root: &Path) -> BTreeMap<PathBuf, FileState> {
    let mut states = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            let metadata = fs::symlink_metadata(&path).unwrap();
            let file_type = metadata.file_type();
            let kind = if file_type.is_symlink() {
                "link"
            } else if file_type.is_dir() {
                "dir"
            } else if file_type.is_file() {
                "file"
            } else {
                "other"
            };
            if kind == "dir" {
                stack.push(path.clone());
            }
            let sha256 = (kind == "file").then(|| {
                let digest = Sha256::digest(fs::read(&path).unwrap());
                digest.iter().map(|byte| format!("{byte:02x}")).collect::<String>()
            });
            states.insert(
                path.strip_prefix(root).unwrap().to_path_buf(),
                FileState {
                    kind,
                    len: if kind == "dir" { 0 } else { metadata.len() },
                    modified: metadata.modified().ok(),
                    created: metadata.created().ok(),
                    readonly: metadata.permissions().readonly(),
                    #[cfg(unix)]
                    mode: std::os::unix::fs::PermissionsExt::mode(&metadata.permissions()),
                    #[cfg(windows)]
                    attributes: std::os::windows::fs::MetadataExt::file_attributes(&metadata),
                    sha256,
                    link_target: if kind == "link" { fs::read_link(&path).ok() } else { None },
                },
            );
        }
    }
    states
}

/// Names of the entries directly in `dir`.
pub fn entries(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> =
        fs::read_dir(dir).unwrap().map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    names
}

/// Runs the collector binary in `cwd` with a private temp folder.
pub fn run_collector(args: &[&std::ffi::OsStr], cwd: &Path, temp: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_vbs-collector"))
        .args(args)
        .current_dir(cwd)
        .env("TEMP", temp)
        .env("TMP", temp)
        .env("TMPDIR", temp)
        .output()
        .expect("collector starts")
}
