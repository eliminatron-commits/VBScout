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

use crate::module::ReadSeek;

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
    /// A network path or network drive: system modules never read across the network.
    #[error("network location (not read)")]
    NetworkPath,
    /// An online-only cloud file or folder: reading it would download it, so it is left alone.
    #[error("online-only cloud file (not downloaded)")]
    CloudPlaceholder,
    #[error("larger than the limit")]
    TooLarge,
    /// Opened exclusively by another program (sharing or lock violation).
    #[error("in use by another program")]
    Locked,
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
            ViewError::NetworkPath => "networkPath",
            ViewError::CloudPlaceholder => "cloudPlaceholder",
            ViewError::TooLarge => "tooLarge",
            ViewError::Locked => "locked",
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
    /// Machine-wide environment variables (`SystemRoot`, `ProgramFiles`, …) for expanding
    /// `%NAME%` in commands; names in upper case. User-specific variables are not included.
    pub variables: BTreeMap<String, String>,
}

impl SystemEnvironment {
    /// Expands `%NAME%` references (case-insensitive) with the machine-wide variables;
    /// unknown names stay as written.
    pub fn expand(&self, text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        let mut rest = text;
        while let Some(start) = rest.find('%') {
            out.push_str(&rest[..start]);
            let after = &rest[start + 1..];
            match after.find('%') {
                Some(end) if end > 0 && !after[..end].contains(char::is_whitespace) => {
                    let name = &after[..end];
                    match self.variables.get(&name.to_ascii_uppercase()) {
                        Some(value) => out.push_str(value),
                        None => {
                            out.push('%');
                            out.push_str(name);
                            out.push('%');
                        }
                    }
                    rest = &after[end + 1..];
                }
                _ => {
                    out.push('%');
                    rest = after;
                }
            }
        }
        out.push_str(rest);
        out
    }
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
    pub modified: Option<OffsetDateTime>,
}

/// Read-only access to known local locations (task definitions, policy scripts, startup folders,
/// installer packages, …). Network paths and network drives are refused with
/// [`ViewError::NetworkPath`]: system modules never cause network access.
pub trait FileView: Send + Sync {
    /// Lists a directory; links and junctions are left out (never followed).
    fn list_dir(&self, path: &Path) -> Result<Vec<DirEntryInfo>, ViewError>;

    /// Reads a whole file; files larger than `limit` bytes fail with [`ViewError::TooLarge`].
    fn read(&self, path: &Path, limit: u64) -> Result<Vec<u8>, ViewError>;

    /// Opens a file for streaming reads (large containers such as installer packages).
    fn open(&self, path: &Path) -> Result<Box<dyn ReadSeek + '_>, ViewError>;
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

    /// Runs an XPath query once and returns at most `max` records, newest first.
    /// Event data without names is keyed by position (`#1`, `#2`, …).
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

    fn open(&self, _: &Path) -> Result<Box<dyn ReadSeek + '_>, ViewError> {
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

/// In-memory registry for tests: key paths match case-insensitively and keep the case they
/// were created with (like the real registry).
#[derive(Debug, Default, Clone)]
pub struct MemoryRegistry {
    /// (hive, view, lower-case path) → key
    keys: BTreeMap<(Hive, Bitness, String), MemoryKey>,
}

/// Key name as created and its values.
type MemoryKey = (String, Vec<(String, RegValue)>);

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
        let mut parent = String::new();
        for segment in path.trim_matches('\\').split('\\') {
            if !parent.is_empty() {
                parent.push('\\');
            }
            parent.push_str(segment);
            self.keys
                .entry((hive, bitness, Self::normalize(&parent)))
                .or_insert_with(|| (segment.to_owned(), Vec::new()));
        }
        let entry = self.keys.entry((hive, bitness, Self::normalize(path))).or_default();
        entry.1.extend(values.iter().map(|(name, value)| ((*name).to_owned(), value.clone())));
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
            .iter()
            .filter(|((h, b, key), _)| *h == hive && *b == bitness && key.starts_with(&prefix))
            .filter_map(|((_, _, key), (name, _))| {
                let rest = &key[prefix.len()..];
                (!rest.is_empty() && !rest.contains('\\')).then(|| name.clone())
            })
            .collect())
    }

    fn values(&self, hive: Hive, path: &str, bitness: Bitness) -> Result<Vec<(String, RegValue)>, ViewError> {
        self.keys
            .get(&(hive, bitness, Self::normalize(path)))
            .map(|(_, values)| values.clone())
            .ok_or(ViewError::NotFound)
    }
}

/// In-memory event logs for tests. The XPath is not evaluated – modules filter the records
/// they get themselves (they must anyway: a log may hold events of other providers with the
/// same ID).
#[derive(Debug, Default, Clone)]
pub struct MemoryEventLogs {
    channels: BTreeMap<String, Vec<EventRecord>>,
}

impl MemoryEventLogs {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_channel(mut self, channel: &str, records: Vec<EventRecord>) -> Self {
        self.channels.entry(channel.to_ascii_lowercase()).or_default().extend(records);
        self
    }
}

impl EventLogView for MemoryEventLogs {
    fn channel(&self, channel: &str) -> Result<ChannelInfo, ViewError> {
        let records = self.channels.get(&channel.to_ascii_lowercase()).ok_or(ViewError::NotFound)?;
        let times = records.iter().filter_map(|record| record.time);
        Ok(ChannelInfo { records: Some(records.len() as u64), oldest: times.clone().min(), newest: times.max() })
    }

    fn query(&self, channel: &str, _xpath: &str, max: usize) -> Result<Vec<EventRecord>, ViewError> {
        let mut records = self.channels.get(&channel.to_ascii_lowercase()).ok_or(ViewError::NotFound)?.clone();
        records.sort_by_key(|record| std::cmp::Reverse((record.time, record.record_id)));
        records.truncate(max);
        Ok(records)
    }
}

/// In-memory WMI for tests: instances per namespace and class (case-insensitive).
#[derive(Debug, Default, Clone)]
pub struct MemoryWmi {
    namespaces: BTreeMap<String, BTreeMap<String, Vec<WmiObject>>>,
}

impl MemoryWmi {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_instances(mut self, namespace: &str, class: &str, objects: Vec<WmiObject>) -> Self {
        let classes = self.namespaces.entry(namespace.to_ascii_lowercase()).or_default();
        classes.entry(class.to_ascii_lowercase()).or_default().extend(objects);
        self
    }
}

impl WmiView for MemoryWmi {
    fn instances(&self, namespace: &str, class: &str) -> Result<Vec<WmiObject>, ViewError> {
        let classes = self.namespaces.get(&namespace.to_ascii_lowercase()).ok_or(ViewError::NotFound)?;
        Ok(classes.get(&class.to_ascii_lowercase()).cloned().unwrap_or_default())
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
            ["CurrentVersion"]
        );
        assert_eq!(registry.subkeys(Hive::LocalMachine, "SOFTWARE\\Only64", Bitness::Wow32), Err(ViewError::NotFound));
        assert_eq!(registry.values(Hive::CurrentUser, run, Bitness::Native), Err(ViewError::NotFound));
        assert_eq!(Unavailable.subkeys(Hive::Users, "", Bitness::Native), Err(ViewError::Unavailable));
    }

    #[test]
    fn expands_machine_variables_only() {
        let env = SystemEnvironment {
            variables: BTreeMap::from([("SYSTEMROOT".into(), r"C:\Windows".into())]),
            ..SystemEnvironment::default()
        };
        assert_eq!(env.expand(r"%SystemRoot%\System32\wscript.exe"), r"C:\Windows\System32\wscript.exe");
        assert_eq!(env.expand(r"%APPDATA%\x.vbs 100%"), r"%APPDATA%\x.vbs 100%");
        assert_eq!(env.expand("50% of %% and %a b%"), "50% of %% and %a b%");
    }

    #[test]
    fn memory_logs_return_the_newest_records_first() {
        let record = |id: u64, minute: u8| EventRecord {
            record_id: id,
            event_id: 1,
            provider: "p".into(),
            time: Some(time::macros::datetime!(2026-01-01 00:00 UTC).replace_minute(minute).unwrap()),
            data: BTreeMap::new(),
        };
        let logs = MemoryEventLogs::new().with_channel("App", vec![record(1, 1), record(2, 3), record(3, 2)]);
        let ids: Vec<u64> = logs.query("app", "*", 2).unwrap().iter().map(|r| r.record_id).collect();
        assert_eq!(ids, [2, 3]);
        let info = logs.channel("APP").unwrap();
        assert_eq!(info.records, Some(3));
        assert!(info.oldest < info.newest);
        assert_eq!(logs.channel("Security"), Err(ViewError::NotFound));
        let wmi = MemoryWmi::new().with_instances("root\\subscription", "X", Vec::new());
        assert_eq!(wmi.instances("ROOT\\Subscription", "Y"), Ok(Vec::new()));
        assert_eq!(wmi.instances("root\\default", "X"), Err(ViewError::NotFound));
    }
}
