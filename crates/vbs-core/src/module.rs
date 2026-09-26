//! The module interface: every finding type is a module with the same shape.
//!
//! A module is either *file-driven* (it names the file extensions it wants and
//! gets each matching file the walk finds) or *system-driven* (it examines
//! registry keys, scheduled tasks, services, WMI, event logs … through the
//! read-only [`SystemView`]) – or both. It never calls operating-system APIs
//! itself and never writes anything.
//!
//! Modules report through [`Report`]. The report fills in kind and
//! classification from the rule catalog, shortens and masks evidence as it
//! arrives, and offers [`Report::not_checkable`] so that an item a module
//! cannot analyse is reported instead of skipped silently.
//!
//! To add a finding type: implement [`Module`] in
//! `crates/vbs-collector/src/modules/`, register it in `modules::all()`, add
//! its rules to `rules/catalog.json` (with sources) and their texts to
//! `i18n/`, and add positive and negative cases to `tests/corpus/`.

use std::io::{Read, Seek};
use std::path::Path;
use std::sync::Arc;

use thiserror::Error;
use time::OffsetDateTime;

use crate::model::{
    Activation, Classification, Detail, Evidence, FileFacts, Finding, FindingKind, FindingStatus, Location,
    LocationKind, NotCheckableReason, SourceStatus, TimeRange,
};
use crate::rules;
use crate::secrets::{self, SecretKind};
use crate::validate::{self, limits};
use crate::views::SystemView;

/// Rule of the security finding "hard-coded credentials" (value never stored).
pub const CREDENTIAL_RULE: &str = "VBS-901";

/// Static description of a module.
#[derive(Debug)]
pub struct ModuleInfo {
    /// Stable identifier, e.g. `script-file`.
    pub id: &'static str,
    /// Coverage source of the module's system part, e.g. `registry.autostart`;
    /// `None` for purely file-driven modules (they count under the file sources of the walk).
    pub system_source: Option<&'static str>,
    /// Rules the module may report (tests check them against the catalog and the corpus).
    pub rules: &'static [&'static str],
    /// "Could not be checked" rule of the module's range (`VBS-x00`), reported
    /// for items the module cannot analyse – including an unexpected failure.
    pub fallback_rule: &'static str,
    /// A complete result needs administrator rights (otherwise the coverage is "limited").
    pub needs_admin: bool,
}

/// A finding type.
pub trait Module: Send + Sync {
    fn info(&self) -> &'static ModuleInfo;

    /// Lower-case file extensions without the dot (e.g. `vbs`) this module inspects.
    fn extensions(&self) -> &'static [&'static str] {
        &[]
    }

    /// Lower-case file names (e.g. `scripts.ini`) this module inspects in addition to its extensions.
    fn file_names(&self) -> &'static [&'static str] {
        &[]
    }

    /// Inspects one file of a wanted extension. Called from several threads at once.
    fn inspect_file(&self, _file: &dyn CandidateFile, _report: &mut Report) {}

    /// Examines system locations. Runs once per scan for modules with a
    /// `system_source`, unless `--files-only` is given.
    fn scan_system(&self, _system: &SystemView<'_>, _report: &mut Report) {}
}

/// Readable and seekable – the handle type for streaming access.
pub trait ReadSeek: Read + Seek {}

impl<T: Read + Seek> ReadSeek for T {}

/// Why a file could not be read.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ReadError {
    #[error("access denied")]
    AccessDenied,
    #[error("the file is locked by another program")]
    Locked,
    #[error("the file no longer exists")]
    NotFound,
    #[error("the file is larger than the limit ({size} > {limit} bytes)")]
    TooLarge { size: u64, limit: u64 },
    /// Online-only cloud file: reading would trigger a download, so it is not opened.
    #[error("online-only cloud file (not downloaded)")]
    CloudPlaceholder,
    #[error("the file is encrypted")]
    Encrypted,
    #[error("{0}")]
    Io(String),
}

impl ReadError {
    /// The not-checkable reason reported for this error. A file that vanished
    /// between listing and reading is not a finding and returns `None`.
    pub fn reason(&self) -> Option<NotCheckableReason> {
        Some(match self {
            ReadError::AccessDenied => NotCheckableReason::AccessDenied,
            ReadError::Locked => NotCheckableReason::Locked,
            ReadError::NotFound => return None,
            ReadError::TooLarge { .. } => NotCheckableReason::TooLarge,
            ReadError::CloudPlaceholder => NotCheckableReason::CloudPlaceholder,
            ReadError::Encrypted => NotCheckableReason::Encrypted,
            ReadError::Io(_) => NotCheckableReason::Corrupt,
        })
    }
}

/// A file found by the walk. Implemented by the collector with read-only access.
pub trait CandidateFile {
    /// Path as found; network paths keep their UNC form.
    fn path(&self) -> &Path;
    /// Lower-case extension without the dot.
    fn extension(&self) -> &str;
    fn size(&self) -> u64;
    fn modified(&self) -> Option<OffsetDateTime>;
    /// The file lives on a network share.
    fn network(&self) -> bool;
    /// The whole content (at most `limit` bytes, otherwise `TooLarge`). Read once, shared by all modules.
    fn contents(&self, limit: u64) -> Result<Arc<[u8]>, ReadError>;
    /// A fresh read-only handle for streaming access to large containers.
    fn open(&self) -> Result<Box<dyn ReadSeek + '_>, ReadError>;
    /// Hex SHA-256 of the content, if it has been read completely.
    fn sha256(&self) -> Option<String>;

    /// Location of this file in findings.
    fn location(&self) -> Location {
        Location { kind: LocationKind::File, path: self.path().display().to_string(), item: None }
    }

    /// File facts for findings (size, time, hash, network flag).
    fn facts(&self) -> FileFacts {
        FileFacts { size: self.size(), modified_at: self.modified(), sha256: self.sha256(), network: self.network() }
    }
}

/// Counters for the coverage entry a module contributes to.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Counters {
    pub entries: u64,
    pub inspected: u64,
    pub errors: u64,
    pub error_samples: Vec<String>,
}

impl Counters {
    pub fn merge(&mut self, other: Counters) {
        self.entries += other.entries;
        self.inspected += other.inspected;
        self.errors += other.errors;
        let room = limits::MAX_ERROR_SAMPLES.saturating_sub(self.error_samples.len());
        self.error_samples.extend(other.error_samples.into_iter().take(room));
    }
}

/// Collects what a module reports.
#[derive(Debug, Default)]
pub struct Report {
    findings: Vec<Finding>,
    counters: Counters,
    status: Option<(SourceStatus, String)>,
    roots: Vec<String>,
    skipped: u64,
    time_range: Option<TimeRange>,
}

impl Report {
    pub fn new() -> Self {
        Self::default()
    }

    /// Starts a finding of `rule` (must exist in the catalog and in the module's `rules`).
    pub fn finding(&mut self, rule: &str, location: Location) -> FindingBuilder<'_> {
        let (kind, classification) = match rules::catalog().rule(rule) {
            Some(entry) => (entry.kind.clone(), entry.classification.clone()),
            // Programming error, caught by the module tests; never report it as harmless.
            None => (FindingKind::Unknown(String::new()), Classification::Review),
        };
        debug_assert!(kind.is_known(), "rule {rule} is not in rules/catalog.json");
        FindingBuilder {
            report: self,
            finding: Finding {
                id: String::new(),
                rule: rule.to_owned(),
                kind,
                classification,
                status: FindingStatus::Detected,
                reason: None,
                activation: Activation::Dormant,
                location,
                target: None,
                file: None,
                evidence: Vec::new(),
                details: Default::default(),
            },
            omitted_evidence: 0,
        }
    }

    /// Starts a finding for a candidate file (location and file facts filled in).
    pub fn file_finding(&mut self, rule: &str, file: &dyn CandidateFile) -> FindingBuilder<'_> {
        self.finding(rule, file.location()).file(file.facts())
    }

    /// Reports an item that may depend on VBScript but could not be analysed.
    /// `rule` is the "could not be checked" rule of the module's range (`VBS-x00`).
    pub fn not_checkable(&mut self, rule: &str, location: Location, reason: NotCheckableReason) -> FindingBuilder<'_> {
        let mut builder = self.finding(rule, location);
        builder.finding.status = FindingStatus::NotCheckable;
        builder.finding.reason = Some(reason);
        builder
    }

    /// Counts enumerated items (directory entries, registry values, tasks, log records, …).
    pub fn count_entries(&mut self, count: u64) {
        self.counters.entries += count;
    }

    /// Counts items inspected in detail.
    pub fn count_inspected(&mut self, count: u64) {
        self.counters.inspected += count;
    }

    /// Counts items deliberately not examined (e.g. user hives that are not loaded).
    pub fn count_skipped(&mut self, count: u64) {
        self.skipped += count;
    }

    /// Names a location the module examined (a folder, key or log channel) for the coverage entry.
    pub fn add_root(&mut self, root: impl Into<String>) {
        let root = root.into();
        if !self.roots.contains(&root) && self.roots.len() < limits::MAX_ERROR_SAMPLES {
            self.roots.push(root);
        }
    }

    /// Widens the time span of the log records that were available (log coverage).
    pub fn cover_time(&mut self, from: OffsetDateTime, to: OffsetDateTime) {
        let (from, to) = if from <= to { (from, to) } else { (to, from) };
        self.time_range = Some(match self.time_range {
            Some(range) => TimeRange { from: range.from.min(from), to: range.to.max(to) },
            None => TimeRange { from, to },
        });
    }

    /// Reports the security finding "hard-coded credentials" (`VBS-901`) if `lines` contain
    /// passwords, connection-string passwords or URL credentials. Only masked lines are kept
    /// as evidence and the values are never stored. Returns the number of secrets found.
    pub fn credentials<'l>(
        &mut self,
        location: Location,
        file: Option<FileFacts>,
        activation: Activation,
        lines: impl IntoIterator<Item = (Option<u32>, &'l str)>,
    ) -> usize {
        let mut kinds: Vec<SecretKind> = Vec::new();
        let mut evidence: Vec<(Option<u32>, &str)> = Vec::new();
        let mut count = 0;
        for (line, text) in lines {
            let masked = secrets::mask_line(text);
            if !masked.masked() {
                continue;
            }
            count += masked.secrets.len();
            for kind in &masked.secrets {
                if !kinds.contains(kind) {
                    kinds.push(*kind);
                }
            }
            // The builder masks the line again on arrival (and marks it as masked).
            evidence.push((line, text));
        }
        if count == 0 {
            return 0;
        }
        kinds.sort();
        let kind_names: Vec<&str> = kinds.iter().map(|kind| kind.as_str()).collect();
        let mut builder = self
            .finding(CREDENTIAL_RULE, location)
            .activation(activation)
            .detail("secrets", i64::try_from(count).unwrap_or(i64::MAX))
            .detail("secretKinds", kind_names.join(","));
        if let Some(file) = file {
            builder = builder.file(file);
        }
        for (line, text) in &evidence {
            builder = builder.evidence(*line, text);
        }
        builder.emit();
        count
    }

    /// Records an unreadable item; the first few are kept as examples.
    pub fn record_error(&mut self, sample: impl Into<String>) {
        self.counters.errors += 1;
        if self.counters.error_samples.len() < limits::MAX_ERROR_SAMPLES {
            self.counters.error_samples.push(validate::shorten(&sample.into(), 400));
        }
    }

    /// Declares that the module's source is not complete, e.g.
    /// `(SourceStatus::Unavailable, "notInstalled")` when Sysmon is missing.
    /// Without this call the source counts as complete (or partial if errors were recorded).
    pub fn set_source_status(&mut self, status: SourceStatus, reason: impl Into<String>) {
        self.status = Some((status, reason.into()));
    }

    pub fn findings(&self) -> &[Finding] {
        &self.findings
    }

    pub fn source_status(&self) -> Option<&(SourceStatus, String)> {
        self.status.as_ref()
    }

    pub fn roots(&self) -> &[String] {
        &self.roots
    }

    pub fn skipped(&self) -> u64 {
        self.skipped
    }

    pub fn time_range(&self) -> Option<TimeRange> {
        self.time_range
    }

    pub fn into_parts(self) -> (Vec<Finding>, Counters) {
        (self.findings, self.counters)
    }
}

/// Builds one finding; nothing is reported until [`FindingBuilder::emit`].
#[must_use = "a finding is only reported by calling emit()"]
pub struct FindingBuilder<'r> {
    report: &'r mut Report,
    finding: Finding,
    omitted_evidence: u32,
}

impl FindingBuilder<'_> {
    /// States the kind of an item that could not be checked. A "could not be checked" rule
    /// (`VBS-x00`) covers its whole range, e.g. `VBS-200` scripts *and* shortcuts.
    pub fn kind(mut self, kind: FindingKind) -> Self {
        debug_assert!(self.finding.rule.ends_with("00"), "only VBS-x00 rules may state their kind");
        self.finding.kind = kind;
        self
    }

    pub fn activation(mut self, activation: Activation) -> Self {
        self.finding.activation = activation;
        self
    }

    pub fn item(mut self, item: impl Into<String>) -> Self {
        self.finding.location.item = Some(item.into());
        self
    }

    /// Script or program the finding executes, as written.
    pub fn target(mut self, target: impl Into<String>) -> Self {
        self.finding.target = Some(target.into());
        self
    }

    pub fn file(mut self, facts: FileFacts) -> Self {
        self.finding.file = Some(facts);
        self
    }

    /// Adds an affected line. It is cleaned, shortened and masked right away;
    /// lines beyond the limit are only counted.
    pub fn evidence(mut self, line: Option<u32>, text: &str) -> Self {
        if self.finding.evidence.len() >= limits::MAX_EVIDENCE_LINES {
            self.omitted_evidence += 1;
            return self;
        }
        let mut evidence = Evidence { line, text: text.to_owned(), masked: false };
        validate::sanitize_evidence(&mut evidence);
        self.finding.evidence.push(evidence);
        self
    }

    pub fn detail(mut self, key: &str, value: impl Into<Detail>) -> Self {
        self.finding.details.insert(key.to_owned(), value.into());
        self
    }

    /// Reports the finding.
    pub fn emit(mut self) {
        if self.omitted_evidence > 0 {
            self.finding.details.insert("evidenceOmitted".into(), Detail::Number(self.omitted_evidence.into()));
        }
        self.report.findings.push(self.finding);
    }
}

/// Sorts findings into a stable order (kind, location, item, first line, rule)
/// and numbers them `f1`, `f2`, … – the same scan always gives the same file.
pub fn number_findings(findings: &mut [Finding]) {
    findings.sort_by(|a, b| {
        let first_line = |f: &Finding| f.evidence.first().and_then(|e| e.line);
        (a.kind.as_str(), &a.location.path, &a.location.item, first_line(a), &a.rule, &a.target).cmp(&(
            b.kind.as_str(),
            &b.location.path,
            &b.location.item,
            first_line(b),
            &b.rule,
            &b.target,
        ))
    });
    for (index, finding) in findings.iter_mut().enumerate() {
        finding.id = format!("f{}", index + 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::path::PathBuf;

    struct MemoryFile {
        path: PathBuf,
        bytes: Vec<u8>,
    }

    impl CandidateFile for MemoryFile {
        fn path(&self) -> &Path {
            &self.path
        }
        fn extension(&self) -> &str {
            "vbs"
        }
        fn size(&self) -> u64 {
            self.bytes.len() as u64
        }
        fn modified(&self) -> Option<OffsetDateTime> {
            None
        }
        fn network(&self) -> bool {
            false
        }
        fn contents(&self, _limit: u64) -> Result<Arc<[u8]>, ReadError> {
            Ok(self.bytes.clone().into())
        }
        fn open(&self) -> Result<Box<dyn ReadSeek + '_>, ReadError> {
            Ok(Box::new(Cursor::new(&self.bytes)))
        }
        fn sha256(&self) -> Option<String> {
            None
        }
    }

    #[test]
    fn findings_take_kind_and_classification_from_the_catalog() {
        let file = MemoryFile { path: PathBuf::from("C:/Scripts/a.vbs"), bytes: b"MsgBox 1".to_vec() };
        let mut report = Report::new();
        report.file_finding("VBS-101", &file).evidence(Some(1), "MsgBox 1").emit();
        report.not_checkable("VBS-100", file.location(), NotCheckableReason::AccessDenied).emit();
        let (findings, _) = report.into_parts();
        assert_eq!(findings[0].kind, FindingKind::ScriptFile);
        assert_eq!(findings[0].classification, Classification::Breaks);
        assert_eq!(findings[0].file.as_ref().map(|f| f.size), Some(8));
        assert_eq!(findings[1].status, FindingStatus::NotCheckable);
        assert_eq!(findings[1].classification, Classification::Review);
        assert_eq!(findings[1].reason, Some(NotCheckableReason::AccessDenied));
    }

    #[test]
    fn evidence_is_masked_and_limited_on_arrival() {
        let location = Location { kind: LocationKind::File, path: "x.vbs".into(), item: None };
        let mut report = Report::new();
        let mut builder = report.finding("VBS-101", location);
        for line in 1..=8 {
            builder = builder.evidence(Some(line), &format!("strPwd{line} = \"secret{line}\""));
        }
        builder.emit();
        let finding = &report.findings()[0];
        assert_eq!(finding.evidence.len(), limits::MAX_EVIDENCE_LINES);
        assert!(finding.evidence.iter().all(|e| e.masked && !e.text.contains("secret")));
        assert_eq!(finding.details["evidenceOmitted"], Detail::Number(3));
    }

    #[test]
    fn credentials_are_reported_without_their_values() {
        let location = Location { kind: LocationKind::File, path: "logon.vbs".into(), item: None };
        let mut report = Report::new();
        let lines = [
            (Some(1), "Set shell = CreateObject(\"WScript.Shell\")"),
            (Some(2), "strPwd = \"Sommer2024!\""),
            (Some(3), "conn.Open \"DSN=x;UID=sa;PWD=hunter2\""),
        ];
        assert_eq!(report.credentials(location.clone(), None, Activation::Automatic, lines), 2);
        assert_eq!(report.credentials(location, None, Activation::Dormant, [(Some(1), "MsgBox 1")]), 0);
        let findings = report.findings();
        assert_eq!(findings.len(), 1);
        let finding = &findings[0];
        assert_eq!((finding.rule.as_str(), &finding.kind), (CREDENTIAL_RULE, &FindingKind::HardcodedCredential));
        assert_eq!(finding.activation, Activation::Automatic);
        assert_eq!(finding.details["secrets"], Detail::Number(2));
        assert_eq!(finding.details["secretKinds"], Detail::Text("password,connectionString".into()));
        assert_eq!(finding.evidence.iter().map(|e| e.line).collect::<Vec<_>>(), [Some(2), Some(3)]);
        let text = serde_json::to_string(finding).unwrap();
        assert!(!text.contains("Sommer2024!") && !text.contains("hunter2"), "{text}");
    }

    #[test]
    fn coverage_extras_accumulate() {
        let mut report = Report::new();
        report.add_root("Application");
        report.add_root("Application");
        report.count_skipped(3);
        let (a, b) = (OffsetDateTime::UNIX_EPOCH, OffsetDateTime::UNIX_EPOCH + time::Duration::days(2));
        report.cover_time(b, a + time::Duration::days(1));
        report.cover_time(a, a);
        assert_eq!(report.roots(), ["Application"]);
        assert_eq!(report.skipped(), 3);
        assert_eq!(report.time_range(), Some(TimeRange { from: a, to: b }));
        let not_checkable = report
            .not_checkable(
                "VBS-100",
                Location { kind: LocationKind::File, path: "x.lnk".into(), item: None },
                NotCheckableReason::Corrupt,
            )
            .kind(FindingKind::Shortcut);
        not_checkable.emit();
        assert_eq!(report.findings()[0].kind, FindingKind::Shortcut);
    }

    #[test]
    fn numbering_is_deterministic() {
        let mut report = Report::new();
        for path in ["b.vbs", "a.vbs", "c.vbs"] {
            report.finding("VBS-101", Location { kind: LocationKind::File, path: path.into(), item: None }).emit();
        }
        let (mut findings, _) = report.into_parts();
        number_findings(&mut findings);
        let order: Vec<_> = findings.iter().map(|f| (f.id.as_str(), f.location.path.as_str())).collect();
        assert_eq!(order, [("f1", "a.vbs"), ("f2", "b.vbs"), ("f3", "c.vbs")]);
    }

    #[test]
    fn counters_keep_a_few_samples() {
        let mut report = Report::new();
        for n in 0..30 {
            report.record_error(format!("C:/denied/{n}"));
        }
        let (_, counters) = report.into_parts();
        assert_eq!(counters.errors, 30);
        assert_eq!(counters.error_samples.len(), limits::MAX_ERROR_SAMPLES);
        assert_eq!(ReadError::NotFound.reason(), None);
        assert_eq!(ReadError::CloudPlaceholder.reason(), Some(NotCheckableReason::CloudPlaceholder));
    }
}
