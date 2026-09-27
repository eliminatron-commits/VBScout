//! What the scans could see: coverage per machine and source, the time span the event logs
//! covered, and what could not be checked. Reports show it as it is – absence of a finding is
//! never presented as proof that nothing depends on VBScript.

use std::collections::BTreeMap;

use time::OffsetDateTime;
use vbs_core::model::{CoverageMode, FindingStatus, SourceStatus};

use crate::assessment::{Item, MachineInfo};

/// Source IDs of the event logs (`eventLog.<name>`).
pub const LOG_PREFIX: &str = "eventLog.";
/// Event log of the VBScript deprecation alerts (event 4096).
pub const DEPRECATION_LOG: &str = "eventLog.vbscriptDeprecation";
/// Sysmon's operational log.
pub const SYSMON_LOG: &str = "eventLog.sysmon";

#[derive(Debug, Clone, Default, PartialEq)]
pub struct CoverageSummary {
    pub machines: usize,
    pub full: usize,
    pub limited: usize,
    /// Machines per limitation code.
    pub limitations: Vec<(String, usize)>,
    /// Status of every source over all machines.
    pub sources: Vec<SourceTally>,
    /// The event logs of every machine.
    pub logs: Vec<LogWindow>,
    /// The file walk over all machines.
    pub files: FileTally,
    /// Own items that could not be checked, per reason.
    pub not_checkable: Vec<(String, usize)>,
}

/// How many machines had a source complete, partial, skipped, unavailable or failed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SourceTally {
    pub source: String,
    pub complete: usize,
    pub partial: usize,
    pub skipped: usize,
    pub unavailable: usize,
    pub failed: usize,
    /// Status values from newer versions.
    pub other: usize,
}

/// One event log of one machine.
#[derive(Debug, Clone, PartialEq)]
pub struct LogWindow {
    /// Index into the assessment's machines.
    pub machine: usize,
    pub source: String,
    pub status: SourceStatus,
    pub reason: Option<String>,
    /// Records in the log.
    pub records: u64,
    /// Oldest and newest record available when the machine was scanned.
    pub from: Option<OffsetDateTime>,
    pub to: Option<OffsetDateTime>,
}

impl LogWindow {
    /// Whether the log could be read.
    pub fn read(&self) -> bool {
        matches!(self.status, SourceStatus::Complete | SourceStatus::Partial)
    }

    /// Days between the oldest and the newest record.
    pub fn days(&self) -> Option<f64> {
        let (from, to) = (self.from?, self.to?);
        Some(((to - from).whole_seconds().max(0) as f64) / 86_400.0)
    }
}

/// Totals of the file walk.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FileTally {
    pub entries: u64,
    pub inspected: u64,
    pub errors: u64,
    pub skipped: u64,
}

/// A log over all machines: how many could read it and which span it covered.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct LogSpan {
    /// Machines that could read the log.
    pub read: usize,
    /// Machines that reported the log at all.
    pub reported: usize,
    /// Shortest and longest span in days among the machines that read it.
    pub min_days: Option<f64>,
    pub max_days: Option<f64>,
    pub earliest: Option<OffsetDateTime>,
    pub latest: Option<OffsetDateTime>,
}

impl CoverageSummary {
    /// Span of one event log over all machines.
    pub fn log_span(&self, source: &str) -> LogSpan {
        let mut span = LogSpan::default();
        for log in self.logs.iter().filter(|log| log.source == source) {
            span.reported += 1;
            if !log.read() {
                continue;
            }
            span.read += 1;
            if let Some(days) = log.days() {
                span.min_days = Some(span.min_days.map_or(days, |min: f64| min.min(days)));
                span.max_days = Some(span.max_days.map_or(days, |max: f64| max.max(days)));
            }
            if let Some(from) = log.from {
                span.earliest = Some(span.earliest.map_or(from, |earliest| earliest.min(from)));
            }
            if let Some(to) = log.to {
                span.latest = Some(span.latest.map_or(to, |latest| latest.max(to)));
            }
        }
        span
    }
}

pub fn summarize(machines: &[MachineInfo], items: &[Item]) -> CoverageSummary {
    let mut summary = CoverageSummary { machines: machines.len(), ..CoverageSummary::default() };
    let mut limitations: BTreeMap<String, usize> = BTreeMap::new();
    let mut sources: BTreeMap<String, SourceTally> = BTreeMap::new();
    for (index, machine) in machines.iter().enumerate() {
        let coverage = &machine.coverage;
        if coverage.mode == CoverageMode::Full {
            summary.full += 1;
        } else {
            summary.limited += 1;
        }
        let mut codes: Vec<&str> = coverage.limitations.iter().map(|limitation| limitation.code.as_str()).collect();
        codes.sort_unstable();
        codes.dedup();
        for code in codes {
            *limitations.entry(code.to_owned()).or_default() += 1;
        }
        for source in &coverage.sources {
            let tally = sources
                .entry(source.source.clone())
                .or_insert_with(|| SourceTally { source: source.source.clone(), ..SourceTally::default() });
            match source.status {
                SourceStatus::Complete => tally.complete += 1,
                SourceStatus::Partial => tally.partial += 1,
                SourceStatus::Skipped => tally.skipped += 1,
                SourceStatus::Unavailable => tally.unavailable += 1,
                SourceStatus::Failed => tally.failed += 1,
                SourceStatus::Unknown(_) => tally.other += 1,
            }
            if source.source.starts_with("files.") {
                summary.files.entries += source.entries;
                summary.files.inspected += source.inspected;
                summary.files.errors += source.errors;
                summary.files.skipped += source.skipped;
            }
            if source.source.starts_with(LOG_PREFIX) {
                summary.logs.push(LogWindow {
                    machine: index,
                    source: source.source.clone(),
                    status: source.status.clone(),
                    reason: source.reason.clone(),
                    records: source.entries,
                    from: source.time_range.map(|range| range.from),
                    to: source.time_range.map(|range| range.to),
                });
            }
        }
    }
    summary.limitations = limitations.into_iter().collect();
    summary.sources = sources.into_values().collect();
    let mut reasons: BTreeMap<String, usize> = BTreeMap::new();
    for item in items.iter().filter(|item| item.is_own() && item.status == FindingStatus::NotCheckable) {
        let reason = item.reason.as_ref().map_or("unknown", |reason| reason.as_str());
        *reasons.entry(reason.to_owned()).or_default() += 1;
    }
    let mut not_checkable: Vec<(String, usize)> = reasons.into_iter().collect();
    not_checkable.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    summary.not_checkable = not_checkable;
    summary
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assessment::Assessment;
    use crate::assessment::tests::scan;
    use time::macros::datetime;
    use vbs_core::model::{Limitation, LimitationCode, SourceCoverage, TimeRange};

    fn source(id: &str, status: SourceStatus, range: Option<(OffsetDateTime, OffsetDateTime)>) -> SourceCoverage {
        SourceCoverage {
            source: id.into(),
            status,
            reason: None,
            roots: vec![],
            entries: 10,
            inspected: 2,
            errors: 1,
            skipped: 3,
            error_samples: vec![],
            duration_ms: 5,
            time_range: range.map(|(from, to)| TimeRange { from, to }),
        }
    }

    #[test]
    fn coverage_over_machines() {
        let t = datetime!(2026-09-26 10:00 UTC);
        let mut first = scan("PC-1", t, vec![]);
        first.result.coverage.sources = vec![
            source("files.localDrives", SourceStatus::Partial, None),
            source(DEPRECATION_LOG, SourceStatus::Complete, Some((datetime!(2026-09-06 10:00 UTC), t))),
            source(SYSMON_LOG, SourceStatus::Unavailable, None),
        ];
        let mut second = scan("PC-2", t, vec![]);
        second.result.coverage.mode = CoverageMode::Limited;
        second.result.coverage.limitations = vec![Limitation { code: LimitationCode::NotElevated, detail: None }];
        second.result.coverage.sources = vec![
            source("files.localDrives", SourceStatus::Complete, None),
            source(DEPRECATION_LOG, SourceStatus::Complete, Some((datetime!(2026-09-24 10:00 UTC), t))),
            source(SYSMON_LOG, SourceStatus::Complete, Some((datetime!(2026-09-25 10:00 UTC), t))),
        ];
        let assessment = Assessment::build(&[first, second], None);
        let coverage = &assessment.coverage;
        assert_eq!((coverage.full, coverage.limited), (1, 1));
        assert_eq!(coverage.limitations, vec![("notElevated".to_owned(), 1)]);
        assert_eq!(coverage.files, FileTally { entries: 20, inspected: 4, errors: 2, skipped: 6 });
        let files = coverage.sources.iter().find(|s| s.source == "files.localDrives").unwrap();
        assert_eq!((files.complete, files.partial), (1, 1));
        let deprecation = coverage.log_span(DEPRECATION_LOG);
        assert_eq!((deprecation.read, deprecation.reported), (2, 2));
        assert_eq!((deprecation.min_days, deprecation.max_days), (Some(2.0), Some(20.0)));
        let sysmon = coverage.log_span(SYSMON_LOG);
        assert_eq!((sysmon.read, sysmon.reported), (1, 2));
    }
}
