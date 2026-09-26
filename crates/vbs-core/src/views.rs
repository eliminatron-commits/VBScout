//! Read-only views of the system that modules examine.
//!
//! Modules never call operating-system APIs themselves. They read through these
//! traits, which expose **reading operations only** – there is deliberately no
//! way to create, change or delete anything through them. The collector
//! implements them per platform (`crates/vbs-collector/src/platform/`); tests
//! use the in-memory implementations below, so every module can be tested on
//! any machine.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use thiserror::Error;
use time::OffsetDateTime;

/// Why a view could not deliver.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ViewError {
    #[error("not found")]
    NotFound,
    #[error("access denied")]
    AccessDenied,
    /// The source does not exist on this system or platform (e.g. Sysmon not installed).
    #[error("not available on this system")]
    Unavailable,
    #[error("{0}")]
    Failed(String),
}

impl ViewError {
    /// Machine-readable reason for coverage entries.
    pub fn reason_code(&self) -> &'static str {
        match self {
            ViewError::NotFound => "notFound",
            ViewError::AccessDenied => "accessDenied",
            ViewError::Unavailable => "unavailable",
            ViewError::Failed(_) => "failed",
        }
    }
}

/// Facts about the environment the collector runs in.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SystemEnvironment {
    /// Running with administrator rights (elevated token).
    pub elevated: bool,
    /// Windows directory, e.g. `C:\Windows`.
    pub system_root: Option<PathBuf>,
    /// e.g. `C:\ProgramData`.
    pub program_data: Option<PathBuf>,
}

/// Everything a system module may look at.
pub struct SystemView<'a> {
    pub env: &'a SystemEnvironment,
    pub registry: &'a dyn RegistryView,
    pub files: &'a dyn FileView,
    pub event_logs: &'a dyn EventLogView,
    pub wmi: &'a dyn WmiView,
}

// --- registry -------------------------------------------------------------------

/// Registry root keys a module may read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Hive {
    LocalMachine,
    CurrentUser,
    /// All loaded user hives (`HKEY_USERS`); unloaded profiles are never loaded.
    Users,
}

impl Hive {
    /// Short name used in finding locations, e.g. `HKLM`.
    pub const fn prefix(self) -> &'static str {
        match self {
            Hive::LocalMachine => "HKLM",
            Hive::CurrentUser => "HKCU",
            Hive::Users => "HKU",
        }
    }
}

/// Which registry view to read on 64-bit Windows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Bitness {
    /// The native (64-bit) view.
    Native,
    /// The 32-bit view (`WOW6432Node`).
    Wow32,
}

/// A registry value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegValue {
    Text(String),
    /// `REG_EXPAND_SZ`, not expanded.
    ExpandText(String),
    MultiText(Vec<String>),
    Dword(u32),
    Qword(u64),
    Binary(Vec<u8>),
    /// Any other type (raw type number).
    Other(u32),
}

impl RegValue {
    /// The value as text for string types.
    pub fn as_text(&self) -> Option<&str> {
        match self {
            RegValue::Text(text) | RegValue::ExpandText(text) => Some(text),
            _ => None,
        }
    }

    pub fn as_u32(&self) -> Option<u32> {
        match self {
            RegValue::Dword(value) => Some(*value),
            _ => None,
        }
    }
}

/// Read-only registry access.
pub trait RegistryView: Send + Sync {
    /// Names of the direct subkeys of `path`.
    fn subkeys(&self, hive: Hive, path: &str, bitness: Bitness) -> Result<Vec<String>, ViewError>;

    /// All values of `path` (`""` is the default value).
    fn values(&self, hive: Hive, path: &str, bitness: Bitness) -> Result<Vec<(String, RegValue)>, ViewError>;

    /// One value of `path`; `Ok(None)` if the key exists but the value does not.
    fn value(&self, hive: Hive, path: &str, name: &str, bitness: Bitness) -> Result<Option<RegValue>, ViewError> {
        Ok(self.values(hive, path, bitness)?.into_iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v))
    }
}

// --- files ----------------------------------------------------------------------

/// A directory entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirEntryInfo {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
}

/// Read-only access to known locations (task definitions, policy scripts, startup folders, …).
pub trait FileView: Send + Sync {
    fn list_dir(&self, path: &Path) -> Result<Vec<DirEntryInfo>, ViewError>;

    /// Reads a whole file; files larger than `limit` bytes fail with `Failed`.
    fn read(&self, path: &Path, limit: u64) -> Result<Vec<u8>, ViewError>;
}

// --- event logs -----------------------------------------------------------------

/// What a log channel holds right now (reported as log coverage).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ChannelInfo {
    pub records: Option<u64>,
    pub oldest: Option<OffsetDateTime>,
    pub newest: Option<OffsetDateTime>,
}

/// One event, with its named data fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventRecord {
    pub record_id: u64,
    pub event_id: u32,
    pub provider: String,
    pub time: Option<OffsetDateTime>,
    pub data: BTreeMap<String, String>,
}

/// Read-only access to existing event logs; nothing is subscribed, exported or cleared.
pub trait EventLogView: Send + Sync {
    fn channel(&self, channel: &str) -> Result<ChannelInfo, ViewError>;

    /// Runs a structured XPath query once and returns at most `max` records.
    fn query(&self, channel: &str, xpath: &str, max: usize) -> Result<Vec<EventRecord>, ViewError>;
}

// --- WMI ------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WmiValue {
    Null,
    Bool(bool),
    Int(i64),
    Text(String),
    TextList(Vec<String>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WmiObject {
    /// Relative object path, e.g. `ActiveScriptEventConsumer.Name="x"`.
    pub path: String,
    pub properties: BTreeMap<String, WmiValue>,
}

/// Read-only WMI access: instance enumeration only, no methods, no writes.
pub trait WmiView: Send + Sync {
    fn instances(&self, namespace: &str, class: &str) -> Result<Vec<WmiObject>, ViewError>;
}

// --- implementations for tests and unsupported platforms ------------------------

/// A view that has nothing: every call reports `Unavailable`.
#[derive(Debug, Default, Clone, Copy)]
pub struct Unavailable;

impl RegistryView for Unavailable {
    fn subkeys(&self, _: Hive, _: &str, _: Bitness) -> Result<Vec<String>, ViewError> {
        Err(ViewError::Unavailable)
    }

    fn values(&self, _: Hive, _: &str, _: Bitness) -> Result<Vec<(String, RegValue)>, ViewError> {
        Err(ViewError::Unavailable)
    }
}

impl FileView for Unavailable {
    fn list_dir(&self, _: &Path) -> Result<Vec<DirEntryInfo>, ViewError> {
        Err(ViewError::Unavailable)
    }

    fn read(&self, _: &Path, _: u64) -> Result<Vec<u8>, ViewError> {
        Err(ViewError::Unavailable)
    }
}

impl EventLogView for Unavailable {
    fn channel(&self, _: &str) -> Result<ChannelInfo, ViewError> {
        Err(ViewError::Unavailable)
    }

    fn query(&self, _: &str, _: &str, _: usize) -> Result<Vec<EventRecord>, ViewError> {
        Err(ViewError::Unavailable)
    }
}

impl WmiView for Unavailable {
    fn instances(&self, _: &str, _: &str) -> Result<Vec<WmiObject>, ViewError> {
        Err(ViewError::Unavailable)
    }
}

/// In-memory registry for tests: keys are case-insensitive paths.
#[derive(Debug, Default, Clone)]
pub struct MemoryRegistry {
    keys: BTreeMap<(Hive, Bitness, String), Vec<(String, RegValue)>>,
}

impl MemoryRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    fn normalize(path: &str) -> String {
        path.trim_matches('\\').to_ascii_lowercase()
    }

    /// Creates `path` (and its parents) with the given values, in both views unless `bitness` says otherwise.
    pub fn with_key(mut self, hive: Hive, path: &str, values: &[(&str, RegValue)]) -> Self {
        for bitness in [Bitness::Native, Bitness::Wow32] {
            self = self.with_key_in(hive, bitness, path, values);
        }
        self
    }

    pub fn with_key_in(mut self, hive: Hive, bitness: Bitness, path: &str, values: &[(&str, RegValue)]) -> Self {
        let path = Self::normalize(path);
        let mut parent = String::new();
        for segment in path.split('\\') {
            if !parent.is_empty() {
                parent.push('\\');
            }
            parent.push_str(segment);
            self.keys.entry((hive, bitness, parent.clone())).or_default();
        }
        let entry = self.keys.entry((hive, bitness, path)).or_default();
        entry.extend(values.iter().map(|(name, value)| ((*name).to_owned(), value.clone())));
        self
    }
}

impl RegistryView for MemoryRegistry {
    fn subkeys(&self, hive: Hive, path: &str, bitness: Bitness) -> Result<Vec<String>, ViewError> {
        let path = Self::normalize(path);
        if !path.is_empty() && !self.keys.contains_key(&(hive, bitness, path.clone())) {
            return Err(ViewError::NotFound);
        }
        let prefix = if path.is_empty() { String::new() } else { format!("{path}\\") };
        Ok(self
            .keys
            .keys()
            .filter(|(h, b, key)| *h == hive && *b == bitness && key.starts_with(&prefix))
            .filter_map(|(_, _, key)| {
                let rest = &key[prefix.len()..];
                (!rest.is_empty() && !rest.contains('\\')).then(|| rest.to_owned())
            })
            .collect())
    }

    fn values(&self, hive: Hive, path: &str, bitness: Bitness) -> Result<Vec<(String, RegValue)>, ViewError> {
        self.keys.get(&(hive, bitness, Self::normalize(path))).cloned().ok_or(ViewError::NotFound)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_registry_behaves_like_a_registry() {
        let registry = MemoryRegistry::new()
            .with_key(
                Hive::LocalMachine,
                r"SOFTWARE\Microsoft\Windows\CurrentVersion\Run",
                &[("Backup", RegValue::Text(r"wscript.exe C:\Scripts\backup.vbs".into()))],
            )
            .with_key_in(Hive::LocalMachine, Bitness::Native, r"SOFTWARE\Only64", &[]);
        let run = r"software\microsoft\windows\currentversion\run";
        let value = registry.value(Hive::LocalMachine, run, "backup", Bitness::Native).unwrap();
        assert_eq!(value.as_ref().and_then(RegValue::as_text), Some(r"wscript.exe C:\Scripts\backup.vbs"));
        assert_eq!(
            registry.subkeys(Hive::LocalMachine, r"SOFTWARE\Microsoft\Windows", Bitness::Wow32).unwrap(),
            ["currentversion"]
        );
        assert_eq!(registry.subkeys(Hive::LocalMachine, "SOFTWARE\\Only64", Bitness::Wow32), Err(ViewError::NotFound));
        assert_eq!(registry.values(Hive::CurrentUser, run, Bitness::Native), Err(ViewError::NotFound));
        assert_eq!(Unavailable.subkeys(Hive::Users, "", Bitness::Native), Err(ViewError::Unavailable));
    }
}
