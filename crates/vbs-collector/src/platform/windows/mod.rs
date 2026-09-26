//! Windows: machine facts and read-only system views.
//!
//! Every Win32 function used here only reads. The list of allowed and
//! forbidden APIs is enforced by `scripts/check-readonly.mjs`; handles are
//! always opened with query/read access (`KEY_READ`, `TOKEN_QUERY`).
#![allow(unsafe_code)] // FFI to read-only Win32 APIs; every unsafe block states its invariants.

mod eventlog;
mod registry;
mod system;
mod wmi;

use std::fs::Metadata;
use std::os::windows::fs::MetadataExt;
use std::path::Path;

use vbs_core::model::Machine;
use vbs_core::views::SystemEnvironment;
use windows_sys::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_ENCRYPTED, FILE_ATTRIBUTE_OFFLINE, FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS,
    FILE_ATTRIBUTE_RECALL_ON_OPEN,
};

use super::{EntryState, Host};
use crate::read_only::ReadOnlyFiles;

pub fn host() -> Host {
    let registry = registry::WinRegistry;
    let os = super::windows_os(&registry, system::native_architecture());
    let machine = Machine {
        hostname: system::computer_name(system::NameKind::NetBios).unwrap_or_else(|| "UNKNOWN".into()),
        fqdn: system::computer_name(system::NameKind::FullyQualified),
        domain: system::computer_name(system::NameKind::Domain),
        machine_id: super::machine_guid(&registry).map(|guid| super::machine_id(&guid)),
        os,
    };
    let env = SystemEnvironment {
        elevated: system::is_elevated(),
        system_root: std::env::var_os("SystemRoot").map(Into::into),
        program_data: std::env::var_os("ProgramData").map(Into::into),
        variables: system::machine_variables(),
    };
    Host {
        machine,
        env,
        local_roots: system::fixed_drives(),
        registry: Box::new(registry),
        files: Box::new(ReadOnlyFiles),
        event_logs: Box::new(eventlog::WinEventLogs),
        wmi: Box::new(wmi::WinWmi),
        supported: true,
    }
}

pub fn entry_state(metadata: &Metadata) -> EntryState {
    let attributes = metadata.file_attributes();
    EntryState {
        placeholder: attributes & (FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS | FILE_ATTRIBUTE_OFFLINE) != 0,
        recall_on_open: attributes & FILE_ATTRIBUTE_RECALL_ON_OPEN != 0,
        encrypted: attributes & FILE_ATTRIBUTE_ENCRYPTED != 0,
    }
}

pub fn skip_dir(_path: &Path) -> bool {
    false
}

/// A path on a mapped network drive (`GetDriveTypeW` = `DRIVE_REMOTE`).
pub fn is_remote_drive(path: &Path) -> bool {
    system::drive_letter(path).is_some_and(system::is_remote_drive)
}
