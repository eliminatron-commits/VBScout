//! Machine facts and read-only system views per platform.
//!
//! Windows is the only supported platform. Other platforms compile so the
//! cross-platform parts (file walk, modules, result format) can be developed
//! and tested anywhere; their scans are marked as limited development runs.
//! All operating-system calls of the collector live below this module
//! (see CLAUDE.md: platform APIs outside `platform/` are forbidden).

use std::fs::Metadata;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use vbs_core::model::{Machine, OperatingSystem, ProductType};
use vbs_core::views::{Bitness, EventLogView, FileView, Hive, RegValue, RegistryView, SystemEnvironment, WmiView};

#[cfg(not(windows))]
mod other;
#[cfg(windows)]
mod windows;

#[cfg(not(windows))]
use other as imp;
#[cfg(windows)]
use windows as imp;

/// Everything the engine needs to know about and read from the machine.
pub struct Host {
    pub machine: Machine,
    pub env: SystemEnvironment,
    /// Fixed local drives (Windows) – the default file scope.
    pub local_roots: Vec<PathBuf>,
    pub registry: Box<dyn RegistryView>,
    pub files: Box<dyn FileView>,
    pub event_logs: Box<dyn EventLogView>,
    pub wmi: Box<dyn WmiView>,
    /// `false` on development platforms other than Windows.
    pub supported: bool,
}

/// Collects the machine facts and opens the read-only views.
pub fn host() -> Host {
    imp::host()
}

/// Per-entry facts from the directory listing that decide whether the walk may touch it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EntryState {
    /// Online-only cloud file or offline (HSM) file: reading would download or recall it.
    pub placeholder: bool,
    /// Directory whose listing lives in the cloud: listing it would fetch it.
    pub recall_on_open: bool,
    /// Encrypted with EFS.
    pub encrypted: bool,
}

/// Reads [`EntryState`] from metadata that came with the directory listing (no extra I/O on Windows).
pub fn entry_state(metadata: &Metadata) -> EntryState {
    imp::entry_state(metadata)
}

/// Directories the walk never enters (pseudo file systems of development platforms).
pub fn skip_dir(path: &Path) -> bool {
    imp::skip_dir(path)
}

/// `\\server\share\…` (or `//server/share/…`), including the `\\?\UNC\` form.
pub fn is_network_path(path: &Path) -> bool {
    let text = path.to_string_lossy();
    let unc = text.strip_prefix(r"\\?\UNC\").map(|rest| format!(r"\\{rest}"));
    let text = unc.as_deref().unwrap_or(&text);
    let mut chars = text.chars();
    let (first, second) = (chars.next(), chars.next());
    matches!((first, second), (Some('\\'), Some('\\')) | (Some('/'), Some('/')))
        && !text.starts_with(r"\\?\")
        && !text.starts_with(r"\\.\")
        && text[2..].split(['\\', '/']).filter(|segment| !segment.is_empty()).count() >= 2
}

/// Pseudonymous machine ID: SHA-256 over the Windows MachineGuid with a fixed,
/// product-independent prefix (so a rename does not change IDs).
pub fn machine_id(machine_guid: &str) -> String {
    let digest = Sha256::digest(format!("vbs-machine-id-v1:{}", machine_guid.trim().to_ascii_lowercase()));
    hex(&digest)
}

/// Lower-case hex encoding.
pub fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        text.push(DIGITS[usize::from(byte >> 4)] as char);
        text.push(DIGITS[usize::from(byte & 0x0f)] as char);
    }
    text
}

const CURRENT_VERSION: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion";
const PRODUCT_OPTIONS: &str = r"SYSTEM\CurrentControlSet\Control\ProductOptions";
const CRYPTOGRAPHY: &str = r"SOFTWARE\Microsoft\Cryptography";

/// Windows version facts from the registry (works on any `RegistryView`, so it is testable anywhere).
pub fn windows_os(registry: &dyn RegistryView, architecture: Option<String>) -> OperatingSystem {
    let value = |path: &str, name: &str| registry.value(Hive::LocalMachine, path, name, Bitness::Native).ok().flatten();
    let text = |path: &str, name: &str| {
        value(path, name).and_then(|v| v.as_text().map(str::trim).filter(|t| !t.is_empty()).map(str::to_owned))
    };
    let number = |name: &str| value(CURRENT_VERSION, name).as_ref().and_then(RegValue::as_u32);

    let build = text(CURRENT_VERSION, "CurrentBuildNumber")
        .or_else(|| text(CURRENT_VERSION, "CurrentBuild"))
        .and_then(|b| b.parse::<u32>().ok());
    let product_type = text(PRODUCT_OPTIONS, "ProductType").map(|t| match t.to_ascii_lowercase().as_str() {
        "winnt" => ProductType::Workstation,
        "servernt" => ProductType::Server,
        "lanmannt" => ProductType::DomainController,
        _ => ProductType::Unknown(t),
    });
    // Windows 11 still reports "Windows 10 …" as ProductName; the build number tells them apart.
    let name = text(CURRENT_VERSION, "ProductName").map(|name| {
        let windows_11 = build.is_some_and(|b| b >= 22_000) && product_type == Some(ProductType::Workstation);
        match name.strip_prefix("Windows 10") {
            Some(rest) if windows_11 => format!("Windows 11{rest}"),
            _ => name,
        }
    });
    let version = match (number("CurrentMajorVersionNumber"), number("CurrentMinorVersionNumber"), build) {
        (Some(major), Some(minor), Some(build)) => Some(match number("UBR") {
            Some(ubr) => format!("{major}.{minor}.{build}.{ubr}"),
            None => format!("{major}.{minor}.{build}"),
        }),
        _ => text(CURRENT_VERSION, "CurrentVersion"),
    };
    OperatingSystem {
        family: "windows".into(),
        name,
        version,
        build,
        display_version: text(CURRENT_VERSION, "DisplayVersion").or_else(|| text(CURRENT_VERSION, "ReleaseId")),
        edition: text(CURRENT_VERSION, "EditionID"),
        product_type,
        architecture,
    }
}

/// The Windows MachineGuid, if readable.
pub fn machine_guid(registry: &dyn RegistryView) -> Option<String> {
    registry
        .value(Hive::LocalMachine, CRYPTOGRAPHY, "MachineGuid", Bitness::Native)
        .ok()
        .flatten()
        .and_then(|value| value.as_text().map(str::to_owned))
        .filter(|guid| !guid.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use vbs_core::views::MemoryRegistry;

    fn registry(product_name: &str, build: &str, product_type: &str) -> MemoryRegistry {
        MemoryRegistry::new()
            .with_key(
                Hive::LocalMachine,
                CURRENT_VERSION,
                &[
                    ("ProductName", RegValue::Text(product_name.into())),
                    ("CurrentBuildNumber", RegValue::Text(build.into())),
                    ("CurrentMajorVersionNumber", RegValue::Dword(10)),
                    ("CurrentMinorVersionNumber", RegValue::Dword(0)),
                    ("UBR", RegValue::Dword(4652)),
                    ("DisplayVersion", RegValue::Text("24H2".into())),
                    ("EditionID", RegValue::Text("Professional".into())),
                ],
            )
            .with_key(Hive::LocalMachine, PRODUCT_OPTIONS, &[("ProductType", RegValue::Text(product_type.into()))])
            .with_key(
                Hive::LocalMachine,
                CRYPTOGRAPHY,
                &[("MachineGuid", RegValue::Text("6A8E0F3C-2B1D-4E5F-9A7B-1C2D3E4F5A6B".into()))],
            )
    }

    #[test]
    fn windows_11_is_named_correctly() {
        let os = windows_os(&registry("Windows 10 Pro", "26100", "WinNT"), Some("x64".into()));
        assert_eq!(os.name.as_deref(), Some("Windows 11 Pro"));
        assert_eq!(os.version.as_deref(), Some("10.0.26100.4652"));
        assert_eq!(os.build, Some(26100));
        assert_eq!(os.display_version.as_deref(), Some("24H2"));
        assert_eq!(os.product_type, Some(ProductType::Workstation));
        assert_eq!(os.architecture.as_deref(), Some("x64"));
    }

    #[test]
    fn servers_and_older_windows_keep_their_names() {
        let server = windows_os(&registry("Windows Server 2025 Standard", "26100", "ServerNT"), None);
        assert_eq!(server.name.as_deref(), Some("Windows Server 2025 Standard"));
        assert_eq!(server.product_type, Some(ProductType::Server));
        let dc = windows_os(&registry("Windows Server 2016 Datacenter", "14393", "LanmanNT"), None);
        assert_eq!(dc.product_type, Some(ProductType::DomainController));
        let windows_10 = windows_os(&registry("Windows 10 Enterprise", "19045", "WinNT"), None);
        assert_eq!(windows_10.name.as_deref(), Some("Windows 10 Enterprise"));
    }

    #[test]
    fn machine_ids_are_stable_pseudonyms() {
        let guid = machine_guid(&registry("x", "1", "WinNT")).unwrap();
        let id = machine_id(&guid);
        assert_eq!(id.len(), 64);
        assert_eq!(id, machine_id(&guid.to_lowercase()));
        assert!(!id.contains(&guid.to_lowercase().replace('-', "")[..8]));
    }

    #[test]
    fn network_paths() {
        assert!(is_network_path(Path::new(r"\\srv\share")));
        assert!(is_network_path(Path::new(r"\\srv\share\scripts")));
        assert!(is_network_path(Path::new("//srv/share/x")));
        assert!(is_network_path(Path::new(r"\\?\UNC\srv\share\x")));
        assert!(!is_network_path(Path::new(r"\\srv")));
        assert!(!is_network_path(Path::new(r"\\?\C:\x")));
        assert!(!is_network_path(Path::new(r"C:\Scripts")));
        assert!(!is_network_path(Path::new("/home/user")));
    }
}
