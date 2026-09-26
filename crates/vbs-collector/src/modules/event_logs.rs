//! Recorded use of VBScript in existing event logs, read once (no subscription):
//!
//! * Windows' VBScript deprecation alerts – event 4096 of `VBScriptDeprecationAlert` in the
//!   Application log, naming the process and its process tree;
//! * Sysmon, where installed: process starts that run VBScript (event 1) and programs that
//!   load `vbscript.dll` (event 7).
//!
//! Events are grouped per program (and script), with their number and first/last time. The
//! time span of the examined records is the log coverage – no completeness promise: older
//! use may have rolled out of the log.

use std::collections::BTreeMap;

use time::OffsetDateTime;
use vbs_core::model::{Activation, Location, LocationKind, SourceStatus};
use vbs_core::module::{CREDENTIAL_RULE, Module, ModuleInfo, Report};
use vbs_core::views::{ChannelInfo, EventRecord, SystemView, ViewError};

use crate::analysis::command::{self, Usage};

/// Newest records read per query (large logs are capped; the coverage shows the span read).
const MAX_EVENTS: usize = 50_000;

const APPLICATION: &str = "Application";
const ALERT_PROVIDER: &str = "VBScriptDeprecationAlert";
const ALERT_EVENT: u32 = 4096;
const SYSMON: &str = "Microsoft-Windows-Sysmon/Operational";
const SYSMON_PROVIDER: &str = "Microsoft-Windows-Sysmon";

pub struct DeprecationAlerts;

static ALERTS: ModuleInfo = ModuleInfo {
    id: "event-log-deprecation-alert",
    system_source: Some("eventLog.vbscriptDeprecation"),
    rules: &["VBS-501"],
    fallback_rule: "VBS-500",
    needs_admin: false,
};

impl Module for DeprecationAlerts {
    fn info(&self) -> &'static ModuleInfo {
        &ALERTS
    }

    fn scan_system(&self, system: &SystemView<'_>, report: &mut Report) {
        let Some(info) = open_channel(system, report, APPLICATION) else { return };
        let xpath = format!("*[System[Provider[@Name='{ALERT_PROVIDER}'] and (EventID={ALERT_EVENT})]]");
        let records = match system.event_logs.query(APPLICATION, &xpath, MAX_EVENTS) {
            Ok(records) => records,
            Err(error) => return failed(report, &error),
        };
        cover(report, &info, &[&records]);
        let mut groups: BTreeMap<String, Group> = BTreeMap::new();
        for record in &records {
            if record.event_id != ALERT_EVENT || !record.provider.eq_ignore_ascii_case(ALERT_PROVIDER) {
                continue;
            }
            report.count_inspected(1);
            let (process, tree) = alert_process(record);
            let key = format!(
                "{}|{}",
                process.to_ascii_lowercase(),
                tree.as_deref().unwrap_or_default().to_ascii_lowercase()
            );
            groups.entry(key).or_insert_with(|| Group::new(process.clone(), tree.clone())).add(record);
        }
        for group in groups.values() {
            let location = Location {
                kind: LocationKind::EventLog,
                path: APPLICATION.into(),
                item: Some(format!("{ALERT_PROVIDER} {ALERT_EVENT}")),
            };
            let mut builder = group.details(report.finding("VBS-501", location)).target(group.program.clone());
            if let Some(tree) = &group.extra {
                builder = builder.detail("processTree", tree.clone()).evidence(None, &format!("ProcessTree: {tree}"));
            }
            builder.emit();
        }
    }
}

pub struct Sysmon;

static SYSMON_INFO: ModuleInfo = ModuleInfo {
    id: "event-log-sysmon",
    system_source: Some("eventLog.sysmon"),
    rules: &["VBS-502", "VBS-503", CREDENTIAL_RULE],
    fallback_rule: "VBS-500",
    needs_admin: true,
};

impl Module for Sysmon {
    fn info(&self) -> &'static ModuleInfo {
        &SYSMON_INFO
    }

    fn scan_system(&self, system: &SystemView<'_>, report: &mut Report) {
        let info = match system.event_logs.channel(SYSMON) {
            Ok(info) => info,
            Err(ViewError::NotFound | ViewError::Unavailable) => {
                report.set_source_status(SourceStatus::Unavailable, "notInstalled");
                return;
            }
            Err(error) => return failed(report, &error),
        };
        report.add_root(SYSMON);
        report.count_entries(info.records.unwrap_or(0));
        let starts = match system.event_logs.query(SYSMON, "*[System[(EventID=1)]]", MAX_EVENTS) {
            Ok(records) => records,
            Err(error) => return failed(report, &error),
        };
        let loads = match system.event_logs.query(SYSMON, "*[System[(EventID=7)]]", MAX_EVENTS) {
            Ok(records) => records,
            Err(error) => return failed(report, &error),
        };
        cover(report, &info, &[&starts, &loads]);

        let mut started: BTreeMap<String, Group> = BTreeMap::new();
        for record in starts.iter().filter(|r| r.event_id == 1 && r.provider.eq_ignore_ascii_case(SYSMON_PROVIDER)) {
            let command_line = field(record, "CommandLine").unwrap_or_default();
            let references = command::analyze(&command_line);
            let Some(reference) = references.iter().find(|r| r.usage == Usage::VbScript) else { continue };
            report.count_inspected(1);
            let image = field(record, "Image").unwrap_or_default();
            let script = reference.script.clone();
            let key = format!(
                "{}|{}",
                image.to_ascii_lowercase(),
                script.as_deref().unwrap_or_default().to_ascii_lowercase()
            );
            let group = started.entry(key).or_insert_with(|| {
                let mut group = Group::new(image.clone(), field(record, "ParentImage"));
                group.script = script.clone();
                group.sample = Some(command_line.clone());
                group
            });
            group.add(record);
        }
        for group in started.values() {
            let location =
                Location { kind: LocationKind::EventLog, path: SYSMON.into(), item: Some("Sysmon 1".into()) };
            let mut builder = group
                .details(report.finding("VBS-502", location.clone()))
                .detail("image", group.program.clone())
                .target(group.script.clone().unwrap_or_else(|| group.program.clone()));
            if let Some(parent) = &group.extra {
                builder = builder.detail("parentImage", parent.clone());
            }
            if let Some(sample) = &group.sample {
                builder = builder.evidence(None, sample);
            }
            builder.emit();
            if let Some(sample) = &group.sample {
                report.credentials(location, None, Activation::Logged, [(None, sample.as_str())]);
            }
        }

        let mut loaded: BTreeMap<String, Group> = BTreeMap::new();
        for record in loads.iter().filter(|r| r.event_id == 7 && r.provider.eq_ignore_ascii_case(SYSMON_PROVIDER)) {
            let library = field(record, "ImageLoaded").unwrap_or_default();
            if !library.to_ascii_lowercase().replace('/', "\\").ends_with("\\vbscript.dll") {
                continue;
            }
            report.count_inspected(1);
            let image = field(record, "Image").unwrap_or_default();
            loaded
                .entry(image.to_ascii_lowercase())
                .or_insert_with(|| {
                    let mut group = Group::new(image.clone(), None);
                    group.sample = Some(format!("ImageLoaded: {library}"));
                    group
                })
                .add(record);
        }
        for group in loaded.values() {
            let location =
                Location { kind: LocationKind::EventLog, path: SYSMON.into(), item: Some("Sysmon 7".into()) };
            let mut builder = group
                .details(report.finding("VBS-503", location))
                .detail("image", group.program.clone())
                .target(group.program.clone());
            if let Some(sample) = &group.sample {
                builder = builder.evidence(None, sample);
            }
            builder.emit();
        }
    }
}

/// Events of one program (and script), counted.
struct Group {
    program: String,
    /// Process tree (alerts) or parent image (Sysmon).
    extra: Option<String>,
    script: Option<String>,
    sample: Option<String>,
    events: i64,
    first: Option<OffsetDateTime>,
    last: Option<OffsetDateTime>,
}

impl Group {
    fn new(program: String, extra: Option<String>) -> Self {
        Self { program, extra, script: None, sample: None, events: 0, first: None, last: None }
    }

    fn add(&mut self, record: &EventRecord) {
        self.events += 1;
        if let Some(time) = record.time {
            self.first = Some(self.first.map_or(time, |first| first.min(time)));
            self.last = Some(self.last.map_or(time, |last| last.max(time)));
        }
    }

    fn details<'r>(&self, builder: vbs_core::module::FindingBuilder<'r>) -> vbs_core::module::FindingBuilder<'r> {
        let mut builder = builder.activation(Activation::Logged).detail("events", self.events);
        for (key, time) in [("firstSeen", self.first), ("lastSeen", self.last)] {
            if let Some(text) = time.and_then(|t| t.format(&time::format_description::well_known::Rfc3339).ok()) {
                builder = builder.detail(key, text);
            }
        }
        builder
    }
}

fn open_channel(system: &SystemView<'_>, report: &mut Report, channel: &str) -> Option<ChannelInfo> {
    match system.event_logs.channel(channel) {
        Ok(info) => {
            report.add_root(channel);
            report.count_entries(info.records.unwrap_or(0));
            Some(info)
        }
        Err(ViewError::NotFound | ViewError::Unavailable) => {
            report.set_source_status(SourceStatus::Unavailable, "notFound");
            None
        }
        Err(error) => {
            failed(report, &error);
            None
        }
    }
}

fn failed(report: &mut Report, error: &ViewError) {
    let reason = if *error == ViewError::AccessDenied { "accessDenied" } else { error.reason_code() };
    report.set_source_status(SourceStatus::Failed, reason);
}

/// The time span in which every query saw all records: the whole log, or – where a query
/// hit [`MAX_EVENTS`] – from its oldest record read.
fn cover(report: &mut Report, info: &ChannelInfo, queries: &[&Vec<EventRecord>]) {
    let (Some(mut from), Some(to)) = (info.oldest, info.newest) else { return };
    for records in queries {
        if records.len() >= MAX_EVENTS
            && let Some(oldest_read) = records.iter().filter_map(|r| r.time).min()
        {
            from = from.max(oldest_read);
        }
    }
    report.cover_time(from, to.max(from));
}

fn field(record: &EventRecord, name: &str) -> Option<String> {
    record
        .data
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.trim().to_owned())
        .filter(|v| !v.is_empty())
}

/// Process name and process tree of a deprecation alert: named fields where the event has
/// them, otherwise the positional values (`#1`, `#2`, …) or the labelled message text.
fn alert_process(record: &EventRecord) -> (String, Option<String>) {
    let mut process = field(record, "ProcessName");
    let mut tree = field(record, "ProcessTree");
    for value in record.data.values() {
        for line in value.lines() {
            let line = line.trim();
            if let Some(rest) = strip_label(line, "ProcessName") {
                process.get_or_insert_with(|| rest.to_owned());
            } else if let Some(rest) = strip_label(line, "ProcessTree") {
                tree.get_or_insert_with(|| rest.to_owned());
            }
        }
    }
    if process.is_none() || tree.is_none() {
        for value in record.data.values().map(|v| v.trim()) {
            let lower = value.to_ascii_lowercase();
            if tree.is_none() && value.contains(';') && lower.contains(".exe") && !value.contains('\n') {
                tree = Some(value.to_owned());
            } else if process.is_none() && lower.ends_with(".exe") && !value.contains(char::is_whitespace) {
                process = Some(value.to_owned());
            }
        }
    }
    let process = process
        .or_else(|| tree.as_ref().and_then(|t| t.split(';').next().map(str::to_owned)))
        .unwrap_or_else(|| "unknown".into());
    (process, tree)
}

fn strip_label<'a>(line: &'a str, label: &str) -> Option<&'a str> {
    let rest = line.strip_prefix(label)?.trim_start();
    let rest = rest.strip_prefix(':').or_else(|| rest.strip_prefix('='))?;
    Some(rest.trim()).filter(|r| !r.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::testing::scan_system;
    use std::collections::BTreeMap;
    use time::macros::datetime;
    use vbs_core::model::Detail;
    use vbs_core::views::{MemoryEventLogs, MemoryRegistry, MemoryWmi, SystemEnvironment};

    fn event(id: u64, event_id: u32, provider: &str, time: OffsetDateTime, data: &[(&str, &str)]) -> EventRecord {
        EventRecord {
            record_id: id,
            event_id,
            provider: provider.into(),
            time: Some(time),
            data: data.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect::<BTreeMap<_, _>>(),
        }
    }

    #[test]
    fn deprecation_alerts_grouped_per_process() {
        let logs = MemoryEventLogs::new().with_channel(
            APPLICATION,
            vec![
                event(
                    1,
                    4096,
                    ALERT_PROVIDER,
                    datetime!(2026-09-01 08:00 UTC),
                    &[("ProcessName", "cscript.exe"), ("ProcessTree", "cscript.exe;cmd.exe;userinit.exe;winlogon.exe")],
                ),
                event(
                    2,
                    4096,
                    ALERT_PROVIDER,
                    datetime!(2026-09-02 08:00 UTC),
                    &[("ProcessName", "cscript.exe"), ("ProcessTree", "cscript.exe;cmd.exe;userinit.exe;winlogon.exe")],
                ),
                event(
                    3,
                    4096,
                    ALERT_PROVIDER,
                    datetime!(2026-09-03 09:00 UTC),
                    &[("#1", "EXCEL.EXE"), ("#2", "EXCEL.EXE;explorer.exe"), ("#3", "vbscript.dll+0x1234")],
                ),
                event(4, 4096, "SomeOtherApp", datetime!(2026-09-04 09:00 UTC), &[("#1", "other.exe")]),
                event(5, 1000, "Application Error", datetime!(2026-08-01 00:00 UTC), &[]),
            ],
        );
        let report = scan_system(
            &DeprecationAlerts,
            &SystemEnvironment::default(),
            &MemoryRegistry::new(),
            &logs,
            &MemoryWmi::new(),
        );
        let summary: Vec<(Option<&str>, &Detail)> =
            report.findings().iter().map(|f| (f.target.as_deref(), &f.details["events"])).collect();
        assert_eq!(summary, [(Some("cscript.exe"), &Detail::Number(2)), (Some("EXCEL.EXE"), &Detail::Number(1))]);
        assert_eq!(report.findings()[0].activation, Activation::Logged);
        assert_eq!(report.findings()[0].details["firstSeen"], Detail::Text("2026-09-01T08:00:00Z".into()));
        let range = report.time_range().unwrap();
        assert_eq!((range.from, range.to), (datetime!(2026-08-01 00:00 UTC), datetime!(2026-09-04 09:00 UTC)));
    }

    #[test]
    fn sysmon_starts_and_loads() {
        let logs = MemoryEventLogs::new().with_channel(
            SYSMON,
            vec![
                event(
                    1,
                    1,
                    SYSMON_PROVIDER,
                    datetime!(2026-09-01 08:00 UTC),
                    &[
                        ("Image", r"C:\Windows\System32\cscript.exe"),
                        ("CommandLine", r#"cscript.exe //nologo C:\Ops\report.vbs /pwd:"S3cret""#),
                        ("ParentImage", r"C:\Windows\System32\cmd.exe"),
                    ],
                ),
                event(
                    2,
                    1,
                    SYSMON_PROVIDER,
                    datetime!(2026-09-01 09:00 UTC),
                    &[("Image", r"C:\Windows\System32\notepad.exe"), ("CommandLine", r"notepad.exe C:\Ops\report.vbs")],
                ),
                event(
                    3,
                    7,
                    SYSMON_PROVIDER,
                    datetime!(2026-09-01 10:00 UTC),
                    &[("Image", r"C:\Apps\legacy.exe"), ("ImageLoaded", r"C:\Windows\SysWOW64\vbscript.dll")],
                ),
                event(
                    4,
                    7,
                    SYSMON_PROVIDER,
                    datetime!(2026-09-01 11:00 UTC),
                    &[("Image", r"C:\Apps\legacy.exe"), ("ImageLoaded", r"C:\Windows\System32\jscript.dll")],
                ),
            ],
        );
        let report =
            scan_system(&Sysmon, &SystemEnvironment::default(), &MemoryRegistry::new(), &logs, &MemoryWmi::new());
        let rules: Vec<&str> = report.findings().iter().map(|f| f.rule.as_str()).collect();
        assert_eq!(rules, ["VBS-502", CREDENTIAL_RULE, "VBS-503"]);
        assert_eq!(report.findings()[0].target.as_deref(), Some(r"C:\Ops\report.vbs"));
        assert_eq!(report.findings()[2].target.as_deref(), Some(r"C:\Apps\legacy.exe"));
        let missing = scan_system(
            &Sysmon,
            &SystemEnvironment::default(),
            &MemoryRegistry::new(),
            &MemoryEventLogs::new(),
            &MemoryWmi::new(),
        );
        assert_eq!(missing.source_status(), Some(&(SourceStatus::Unavailable, "notInstalled".to_owned())));
    }

    #[test]
    fn reads_labelled_alert_messages() {
        let record = event(
            1,
            4096,
            ALERT_PROVIDER,
            datetime!(2026-09-01 08:00 UTC),
            &[(
                "#1",
                "The following process has been detected as using VBScript.\nProcessName: wscript.exe\nProcessTree: wscript.exe;explorer.exe\nCallStack: vbscript.dll+0x1",
            )],
        );
        assert_eq!(alert_process(&record), ("wscript.exe".to_owned(), Some("wscript.exe;explorer.exe".to_owned())));
    }
}
