//! Development platforms (Linux, macOS): enough to run the file walk and the
//! modules; system views are unavailable and the result is marked as a
//! limited development run.

use std::fs::{self, Metadata};
use std::path::{Path, PathBuf};

use vbs_core::model::{Machine, OperatingSystem};
use vbs_core::views::{SystemEnvironment, Unavailable};

use super::{EntryState, Host};
use crate::read_only::ReadOnlyFiles;

pub fn host() -> Host {
    let hostname = fs::read_to_string("/proc/sys/kernel/hostname")
        .ok()
        .or_else(|| std::env::var("HOSTNAME").ok())
        .map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "localhost".into());
    let os_release = fs::read_to_string("/etc/os-release").unwrap_or_default();
    let field = |key: &str| {
        os_release
            .lines()
            .find_map(|line| line.strip_prefix(key)?.strip_prefix('=').map(|v| v.trim_matches('"').to_owned()))
    };
    let machine = Machine {
        hostname,
        fqdn: None,
        domain: None,
        machine_id: fs::read_to_string("/etc/machine-id").ok().map(|id| super::machine_id(&id)),
        os: OperatingSystem {
            family: std::env::consts::OS.into(),
            name: field("PRETTY_NAME"),
            version: field("VERSION_ID"),
            architecture: Some(std::env::consts::ARCH.into()),
            ..OperatingSystem::default()
        },
    };
    Host {
        machine,
        env: SystemEnvironment { elevated: is_root(), ..SystemEnvironment::default() },
        local_roots: vec![PathBuf::from("/")],
        registry: Box::new(Unavailable),
        files: Box::new(ReadOnlyFiles),
        event_logs: Box::new(Unavailable),
        wmi: Box::new(Unavailable),
        supported: false,
    }
}

/// Effective user ID 0, read from `/proc/self/status`.
fn is_root() -> bool {
    fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|status| {
            let uids = status.lines().find_map(|line| line.strip_prefix("Uid:"))?.split_whitespace().nth(1)?.to_owned();
            Some(uids == "0")
        })
        .unwrap_or(false)
}

pub fn entry_state(_metadata: &Metadata) -> EntryState {
    EntryState::default()
}

/// Development platforms have no drive letters.
pub fn is_remote_drive(_path: &Path) -> bool {
    false
}

pub fn skip_dir(path: &Path) -> bool {
    ["/proc", "/sys", "/dev", "/run"].iter().any(|pseudo| path == Path::new(pseudo))
}
