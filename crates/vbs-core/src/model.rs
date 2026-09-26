//! The result model: what one collector run on one machine found (`result.json`).
//!
//! Field names are camelCase in JSON. The schema is versioned (`schemaVersion`,
//! see `docs/result-format.md` and `docs/result.schema.json`):
//!
//! * Additive changes – new optional fields, new values of the *open* enumerations
//!   below (finding kinds, activations, reasons, source ids) – keep the version.
//!   Readers keep unknown values verbatim (`Unknown(…)`) and still show them.
//! * Every other change increments [`SCHEMA_VERSION`] and ships a migration.
//!   Readers refuse files with a newer schema version instead of guessing.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

/// Current schema version of `result.json`.
pub const SCHEMA_VERSION: u32 = 1;

/// Defines a string enumeration that tolerates values from newer versions:
/// unknown strings deserialize to `Unknown(value)` and serialize back unchanged.
macro_rules! open_enum {
    ($(#[$meta:meta])* pub enum $name:ident { $($(#[$vmeta:meta])* $variant:ident = $text:literal,)+ }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        pub enum $name {
            $($(#[$vmeta])* #[serde(rename = $text)] $variant,)+
            /// A value written by a newer version, kept verbatim.
            #[serde(untagged)]
            Unknown(String),
        }

        impl $name {
            /// All values this version knows, in declaration order.
            pub const KNOWN: &'static [$name] = &[$($name::$variant),+];

            /// The JSON value, e.g. for building translation keys.
            pub fn as_str(&self) -> &str {
                match self {
                    $($name::$variant => $text,)+
                    $name::Unknown(value) => value,
                }
            }

            pub fn is_known(&self) -> bool {
                !matches!(self, $name::Unknown(_))
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }
    };
}

/// One collector run on one machine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanResult {
    /// Media type of the container (identical to the `mimetype` entry).
    pub format: String,
    pub schema_version: u32,
    /// Unique per run; identical IDs in two files mean the same run was imported twice.
    pub scan_id: Uuid,
    pub generator: Generator,
    #[serde(with = "time::serde::rfc3339")]
    pub started_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub finished_at: OffsetDateTime,
    pub machine: Machine,
    /// What the run was asked to scan.
    pub scope: Scope,
    /// What the run could actually check – never a completeness promise.
    pub coverage: Coverage,
    #[serde(default)]
    pub findings: Vec<Finding>,
}

/// The program that wrote the file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Generator {
    /// Product name at scan time (informational; readers never depend on it).
    pub product: String,
    /// Always `collector` for files written by the collector.
    pub component: String,
    pub version: String,
    /// Date of the rule catalog the collector used (`asOf` in `rules/catalog.json`).
    pub rules_as_of: String,
    /// Operating system family the collector ran on (`windows`; other values only in development builds).
    pub platform: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Machine {
    /// Computer name (NetBIOS name on Windows).
    pub hostname: String,
    /// Fully qualified DNS name, if configured.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fqdn: Option<String>,
    /// DNS domain from the local configuration (no directory query).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    /// Pseudonymous, stable machine identifier (SHA-256 of the Windows MachineGuid).
    /// Lets the evaluation recognise repeated scans of the same machine.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine_id: Option<String>,
    pub os: OperatingSystem,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OperatingSystem {
    /// `windows` (other values only in development builds).
    pub family: String,
    /// Product name, e.g. "Windows 11 Pro", "Windows Server 2022 Standard".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Full version, e.g. "10.0.26100.4652".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build: Option<u32>,
    /// Feature update, e.g. "24H2".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_version: Option<String>,
    /// Edition ID, e.g. "Professional", "ServerStandard".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edition: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub product_type: Option<ProductType>,
    /// Native processor architecture: "x64", "arm64" or "x86".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub architecture: Option<String>,
}

open_enum! {
    /// Role of the machine.
    pub enum ProductType {
        Workstation = "workstation",
        Server = "server",
        DomainController = "domainController",
    }
}

/// What the run was asked to scan (from the command line).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Scope {
    /// All fixed local drives (the default); `false` when `--path` restricted the file scan.
    pub local_drives: bool,
    /// Local folders given with `--path`.
    #[serde(default)]
    pub paths: Vec<String>,
    /// Network paths given explicitly with `--include-unc`, read with the rights of the running account.
    #[serde(default)]
    pub network_paths: Vec<String>,
    /// Registry, scheduled tasks, services, event logs, … – `false` with `--files-only`.
    pub system_sources: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Coverage {
    /// `limited` whenever something could not be checked as intended (e.g. no administrator rights).
    pub mode: CoverageMode,
    /// The collector ran with administrator rights.
    pub elevated: bool,
    #[serde(default)]
    pub limitations: Vec<Limitation>,
    /// One entry per examined source (file roots, registry areas, logs, …).
    #[serde(default)]
    pub sources: Vec<SourceCoverage>,
}

open_enum! {
    pub enum CoverageMode {
        Full = "full",
        Limited = "limited",
    }
}

/// Why the coverage is limited.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Limitation {
    pub code: LimitationCode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

open_enum! {
    pub enum LimitationCode {
        /// Not run as administrator: protected locations (other profiles, task definitions, …) were not readable.
        NotElevated = "notElevated",
        /// `--files-only`: registry, tasks, services, WMI and logs were not examined.
        SystemSourcesSkipped = "systemSourcesSkipped",
        /// `--path`: only the given folders were scanned instead of all local drives.
        FileScopeRestricted = "fileScopeRestricted",
        /// A source failed or was only partly readable (see the source entries).
        SourceIncomplete = "sourceIncomplete",
        /// The collector runs on a platform other than Windows (development builds only).
        UnsupportedPlatform = "unsupportedPlatform",
    }
}

/// Coverage of one source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceCoverage {
    /// Source identifier, e.g. `files.localDrives`, `files.networkPaths`, `registry.autostart`, `eventLog.application`.
    pub source: String,
    pub status: SourceStatus,
    /// Machine-readable reason for any status other than `complete`, e.g. `accessDenied`, `notElevated`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// File roots (file sources) or channels/keys examined.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roots: Vec<String>,
    /// Items enumerated: directory entries, registry values, tasks, log records, …
    #[serde(default)]
    pub entries: u64,
    /// Items inspected in detail by a module (candidate files, matching entries).
    #[serde(default)]
    pub inspected: u64,
    /// Items that could not be read (access denied, locked, …).
    #[serde(default)]
    pub errors: u64,
    /// Items deliberately not examined: links and junctions (never followed),
    /// online-only cloud folders (listing them would download them).
    #[serde(default)]
    pub skipped: u64,
    /// The first few unreadable items, as examples for the report.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub error_samples: Vec<String>,
    #[serde(default)]
    pub duration_ms: u64,
    /// Event logs: time span of the records that were available (log coverage, no completeness promise).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_range: Option<TimeRange>,
}

open_enum! {
    pub enum SourceStatus {
        /// Examined completely.
        Complete = "complete",
        /// Examined, but parts were not readable (see `errors`).
        Partial = "partial",
        /// Deliberately not examined (scope options).
        Skipped = "skipped",
        /// Does not exist on this machine (e.g. Sysmon not installed).
        Unavailable = "unavailable",
        /// Could not be examined at all.
        Failed = "failed",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimeRange {
    #[serde(with = "time::serde::rfc3339")]
    pub from: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub to: OffsetDateTime,
}

/// One dependency on VBScript (or one item that could not be checked).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Finding {
    /// Unique within the file (`f1`, `f2`, …).
    pub id: String,
    /// Rule from the rule catalog, e.g. `VBS-101`.
    pub rule: String,
    pub kind: FindingKind,
    /// Copied from the rule catalog at scan time, so older readers can show findings of newer rules.
    pub classification: Classification,
    pub status: FindingStatus,
    /// Why the item could not be checked (`status = notCheckable` only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<NotCheckableReason>,
    /// How the dependency gets executed – the basis of the risk assessment.
    pub activation: Activation,
    pub location: Location,
    /// Script or program the finding executes or references, as written (e.g. a task running a .vbs file).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    /// Facts about the file behind a file location (de-duplication across machines).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<FileFacts>,
    /// Short excerpt of the affected lines; secrets are always masked.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<Evidence>,
    /// Module-specific attributes (small scalar values only).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub details: BTreeMap<String, Detail>,
}

open_enum! {
    /// Finding types – one per collector module family.
    pub enum FindingKind {
        /// .vbs/.vbe/.wsf/.wsc/.hta file containing VBScript.
        ScriptFile = "scriptFile",
        /// Another script (batch, PowerShell, …) that runs wscript/cscript/mshta with VBScript.
        ScriptInvocation = "scriptInvocation",
        /// Shortcut (.lnk) that starts VBScript.
        Shortcut = "shortcut",
        ScheduledTask = "scheduledTask",
        /// Run/RunOnce registry values and startup folders.
        Autostart = "autostart",
        Service = "service",
        WmiSubscription = "wmiSubscription",
        /// Logon/logoff/startup/shutdown scripts (local policy, registry, SYSVOL/NETLOGON).
        LogonScript = "logonScript",
        /// VBScript custom action in an installed MSI package.
        MsiCustomAction = "msiCustomAction",
        /// VBScript use recorded in an event log (deprecation alerts, Sysmon).
        EventLogUsage = "eventLogUsage",
        /// VBA project using VBScript (RegExp, ScriptControl, .vbs calls, WSH objects, …).
        OfficeMacro = "officeMacro",
        /// Security finding: a password or connection string in a scanned script or macro (value never stored).
        HardcodedCredential = "hardcodedCredential",
    }
}

/// Assessment of a rule – "review" whenever the impact is uncertain.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Classification {
    /// Stops working when VBScript is disabled or removed.
    #[serde(rename = "breaks")]
    Breaks,
    /// May be affected; needs a human check.
    #[serde(rename = "review")]
    Review,
    /// A value written by a newer version; treated like `review`.
    #[serde(untagged)]
    Unknown(String),
}

impl Classification {
    pub fn as_str(&self) -> &str {
        match self {
            Classification::Breaks => "breaks",
            Classification::Review => "review",
            Classification::Unknown(value) => value,
        }
    }

    /// Unknown classifications are handled as `review` – never as harmless.
    pub fn effective(&self) -> Classification {
        match self {
            Classification::Breaks => Classification::Breaks,
            _ => Classification::Review,
        }
    }
}

open_enum! {
    pub enum FindingStatus {
        Detected = "detected",
        /// The item may depend on VBScript but could not be analysed – never silently skipped.
        NotCheckable = "notCheckable",
    }
}

open_enum! {
    pub enum NotCheckableReason {
        /// VBA project or document protected by a password.
        PasswordProtected = "passwordProtected",
        AccessDenied = "accessDenied",
        /// Opened by another program without read sharing.
        Locked = "locked",
        /// Damaged or not in the expected format.
        Corrupt = "corrupt",
        /// Format recognised but not supported (e.g. some Access databases).
        UnsupportedFormat = "unsupportedFormat",
        /// Larger than the collector's safety limit.
        TooLarge = "tooLarge",
        /// Online-only cloud file: reading it would download it, so it was left alone.
        CloudPlaceholder = "cloudPlaceholder",
        /// Encrypted (e.g. EFS or an encrypted Office file).
        Encrypted = "encrypted",
        /// A script on a network path that system sources refer to: never read without
        /// `--include-unc` (no network access).
        NetworkLocation = "networkLocation",
        /// The analysis failed unexpectedly (bug report welcome).
        InternalError = "internalError",
    }
}

open_enum! {
    /// How a dependency is executed. The evaluation ranks the risk:
    /// automatic > logged > macro > dormant (installer and manual in between).
    pub enum Activation {
        /// Runs without user action: scheduled task, service, autostart, WMI subscription, logon/startup script.
        Automatic = "automatic",
        /// Evidence of actual execution in an event log.
        Logged = "logged",
        /// Office macro, runs when a document is used.
        Macro = "macro",
        /// MSI custom action, runs during install, repair or uninstall.
        Installer = "installer",
        /// Started by users, e.g. through a shortcut.
        Manual = "manual",
        /// A file with no known trigger.
        Dormant = "dormant",
    }
}

/// Where a finding lives.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Location {
    #[serde(rename = "type")]
    pub kind: LocationKind,
    /// Primary locator: file path, registry key, task path, service name, WMI object, log channel or product code.
    pub path: String,
    /// Sub-item: registry value, task action, VBA module, custom action, WMI consumer, …
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item: Option<String>,
}

open_enum! {
    pub enum LocationKind {
        File = "file",
        Registry = "registry",
        ScheduledTask = "scheduledTask",
        Service = "service",
        Wmi = "wmi",
        EventLog = "eventLog",
        MsiPackage = "msiPackage",
    }
}

/// Facts about a file-backed finding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileFacts {
    pub size: u64,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "time::serde::rfc3339::option")]
    pub modified_at: Option<OffsetDateTime>,
    /// Lower-case hex SHA-256 of the content, when the whole file was read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    /// The file lives on a network share – the same file seen from several machines is de-duplicated.
    #[serde(default)]
    pub network: bool,
}

/// One affected line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Evidence {
    /// 1-based line number in the source; absent for sources without lines (registry values, task actions).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    /// The line, trimmed and shortened; secrets replaced by a mask.
    pub text: String,
    /// A secret was masked in this line.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub masked: bool,
}

/// A small scalar attribute of a finding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Detail {
    Flag(bool),
    Number(i64),
    Text(String),
}

impl From<bool> for Detail {
    fn from(value: bool) -> Self {
        Detail::Flag(value)
    }
}

impl From<i64> for Detail {
    fn from(value: i64) -> Self {
        Detail::Number(value)
    }
}

impl From<&str> for Detail {
    fn from(value: &str) -> Self {
        Detail::Text(value.to_owned())
    }
}

impl From<String> for Detail {
    fn from(value: String) -> Self {
        Detail::Text(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_enums_keep_unknown_values() {
        let kind: FindingKind = serde_json::from_str("\"scriptFile\"").unwrap();
        assert_eq!(kind, FindingKind::ScriptFile);
        let future: FindingKind = serde_json::from_str("\"powerShellV1Script\"").unwrap();
        assert_eq!(future, FindingKind::Unknown("powerShellV1Script".into()));
        assert!(!future.is_known());
        assert_eq!(serde_json::to_string(&future).unwrap(), "\"powerShellV1Script\"");
        assert_eq!(FindingKind::OfficeMacro.as_str(), "officeMacro");
        assert!(FindingKind::KNOWN.iter().all(FindingKind::is_known));
    }

    #[test]
    fn unknown_classification_counts_as_review() {
        let value: Classification = serde_json::from_str("\"maybe\"").unwrap();
        assert_eq!(value.effective(), Classification::Review);
        assert_eq!(Classification::Breaks.effective(), Classification::Breaks);
        assert_eq!(serde_json::to_string(&Classification::Review).unwrap(), "\"review\"");
    }

    #[test]
    fn every_known_value_has_a_translation() {
        let mut keys: Vec<String> = Vec::new();
        keys.extend(FindingKind::KNOWN.iter().map(|v| format!("kind.{v}")));
        keys.extend(Activation::KNOWN.iter().map(|v| format!("activation.{v}")));
        keys.extend(NotCheckableReason::KNOWN.iter().map(|v| format!("reason.{v}")));
        keys.extend(LimitationCode::KNOWN.iter().map(|v| format!("limitation.{v}")));
        keys.extend(CoverageMode::KNOWN.iter().map(|v| format!("coverage.{v}")));
        keys.extend(["classification.breaks", "classification.review", "findingStatus.notCheckable"].map(String::from));
        for key in keys {
            assert!(vbs_i18n::has_key(&key), "i18n/en.json lacks {key}");
        }
    }

    #[test]
    fn details_are_scalars() {
        let details: BTreeMap<String, Detail> =
            serde_json::from_str(r#"{"engine":"VBScript","events":17,"hidden":true}"#).unwrap();
        assert_eq!(details["engine"], Detail::Text("VBScript".into()));
        assert_eq!(details["events"], Detail::Number(17));
        assert_eq!(details["hidden"], Detail::Flag(true));
    }
}
