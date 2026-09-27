//! Serializable views of the assessment for the frontend. The edition's limits are applied
//! here: in the free edition the views carry no migration hints and no effort values at all, so
//! the UI cannot show what the edition does not include.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use time::format_description::well_known::Rfc3339;
use vbs_core::model::Detail;
use vbs_evaluation::assessment::{Assessment, Item, Origin};
use vbs_evaluation::coverage::{DEPRECATION_LOG, LogSpan, SYSMON_LOG};
use vbs_evaluation::edition::Edition;
use vbs_evaluation::effort::Effort;
use vbs_evaluation::report::text::hint_key;

/// Occurrences listed in the detail view (the Excel list has all of them).
const MAX_OCCURRENCES: usize = 200;
/// Largest page of the finding list.
const MAX_PAGE: usize = 500;

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Range {
    pub min: f64,
    pub max: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KindRow {
    pub kind: String,
    pub items: usize,
    pub high: usize,
    pub medium: usize,
    pub low: usize,
    pub not_checkable: usize,
    pub machines: usize,
    pub effort: Option<Range>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogSpanView {
    pub read: usize,
    pub reported: usize,
    pub min_days: Option<f64>,
    pub max_days: Option<f64>,
    pub earliest: Option<String>,
}

impl From<LogSpan> for LogSpanView {
    fn from(span: LogSpan) -> Self {
        Self {
            read: span.read,
            reported: span.reported,
            min_days: span.min_days.map(f64::round),
            max_days: span.max_days.map(f64::round),
            earliest: span.earliest.and_then(|value| value.format(&Rfc3339).ok()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CoverageView {
    pub full: usize,
    pub limited: usize,
    /// (limitation code, machines)
    pub limitations: Vec<(String, usize)>,
    pub deprecation: LogSpanView,
    pub sysmon: LogSpanView,
    pub file_entries: u64,
    pub file_errors: u64,
    pub file_skipped: u64,
    /// (reason code, own items)
    pub not_checkable: Vec<(String, usize)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetAsideView {
    pub file: String,
    pub hostname: String,
    pub scanned_at: String,
    /// `setAside.<code>` translation keys.
    pub why: &'static str,
}

/// Key figures, risks by type and coverage.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Overview {
    pub machines: usize,
    pub items: usize,
    pub high: usize,
    pub medium: usize,
    pub low: usize,
    pub not_checkable: usize,
    pub credentials: usize,
    pub windows_items: usize,
    pub windows_occurrences: usize,
    /// Rule-of-thumb total – `None` in the free edition.
    pub effort: Option<Range>,
    pub by_kind: Vec<KindRow>,
    pub coverage: CoverageView,
    pub set_aside: Vec<SetAsideView>,
}

impl Overview {
    pub fn new(assessment: &Assessment, edition: &Edition) -> Self {
        let summary = &assessment.summary;
        let coverage = &assessment.coverage;
        let effort = edition.allows_effort();
        Self {
            machines: summary.machines,
            items: summary.items,
            high: summary.high,
            medium: summary.medium,
            low: summary.low,
            not_checkable: summary.not_checkable,
            credentials: summary.credentials,
            windows_items: summary.windows_items,
            windows_occurrences: summary.windows_occurrences,
            effort: effort.then_some(Range { min: summary.effort_min, max: summary.effort_max }),
            by_kind: summary
                .by_kind
                .iter()
                .map(|tally| KindRow {
                    kind: tally.kind.as_str().to_owned(),
                    items: tally.items,
                    high: tally.high,
                    medium: tally.medium,
                    low: tally.low,
                    not_checkable: tally.not_checkable,
                    machines: tally.machines,
                    effort: effort.then_some(Range { min: tally.effort_min, max: tally.effort_max }),
                })
                .collect(),
            coverage: CoverageView {
                full: coverage.full,
                limited: coverage.limited,
                limitations: coverage.limitations.clone(),
                deprecation: coverage.log_span(DEPRECATION_LOG).into(),
                sysmon: coverage.log_span(SYSMON_LOG).into(),
                file_entries: coverage.files.entries,
                file_errors: coverage.files.errors,
                file_skipped: coverage.files.skipped,
                not_checkable: coverage.not_checkable.clone(),
            },
            set_aside: assessment
                .set_aside
                .iter()
                .map(|entry| SetAsideView {
                    file: entry.file.display().to_string(),
                    hostname: entry.hostname.clone(),
                    scanned_at: entry.started_at.format(&Rfc3339).unwrap_or_default(),
                    why: entry.why.as_str(),
                })
                .collect(),
        }
    }
}

/// Filters of the finding list; empty values match everything.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FindingQuery {
    /// `high`, `medium`, `low` or `info`.
    pub risk: Option<String>,
    /// Finding type (`scriptFile`, …).
    pub kind: Option<String>,
    /// `own` (default), `windows` or `all`.
    pub origin: Option<String>,
    /// Text in the rule, location, target or machine names (case-insensitive).
    pub search: Option<String>,
    pub offset: usize,
    pub limit: usize,
}

impl FindingQuery {
    fn matches(&self, assessment: &Assessment, item: &Item, search: Option<&str>) -> bool {
        let origin_ok = match self.origin.as_deref() {
            Some("all") => true,
            Some("windows") => item.origin == Origin::Windows,
            _ => item.origin == Origin::Own,
        };
        let risk_ok = self.risk.as_deref().is_none_or(|risk| risk.is_empty() || risk == item.risk.as_str());
        let kind_ok = self.kind.as_deref().is_none_or(|kind| kind.is_empty() || kind == item.kind.as_str());
        origin_ok && risk_ok && kind_ok && search.is_none_or(|search| contains(assessment, item, search))
    }
}

fn contains(assessment: &Assessment, item: &Item, search: &str) -> bool {
    item.rule.to_lowercase().contains(search)
        || item.occurrences.iter().any(|occurrence| {
            occurrence.location.path.to_lowercase().contains(search)
                || occurrence.location.item.as_deref().is_some_and(|value| value.to_lowercase().contains(search))
                || occurrence.target.as_deref().is_some_and(|value| value.to_lowercase().contains(search))
                || assessment.machines[occurrence.machine].machine.hostname.to_lowercase().contains(search)
        })
}

/// One row of the finding list.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FindingRow {
    pub number: usize,
    pub risk: &'static str,
    pub classification: String,
    pub status: String,
    pub reason: Option<String>,
    pub rule: String,
    pub kind: String,
    pub activation: String,
    pub origin: &'static str,
    pub machines: usize,
    /// Host name of the first machine.
    pub machine: String,
    pub location: String,
    pub item: Option<String>,
    pub target: Option<String>,
    /// Counted rule-of-thumb range – `None` in the free edition or when not counted.
    pub effort: Option<Range>,
}

impl FindingRow {
    fn new(assessment: &Assessment, item: &Item, edition: &Edition) -> Self {
        let first = item.first();
        Self {
            number: item.number,
            risk: item.risk.as_str(),
            classification: item.classification.as_str().to_owned(),
            status: item.status.as_str().to_owned(),
            reason: item.reason.as_ref().map(|reason| reason.as_str().to_owned()),
            rule: item.rule.clone(),
            kind: item.kind.as_str().to_owned(),
            activation: item.activation.as_str().to_owned(),
            origin: item.origin.as_str(),
            machines: item.machines,
            machine: assessment.machines[first.machine].machine.hostname.clone(),
            location: first.location.path.clone(),
            item: first.location.item.clone(),
            target: first.target.clone(),
            effort: edition.allows_effort().then(|| item.effort.hours().map(|(min, max)| Range { min, max })).flatten(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FindingsPage {
    pub total: usize,
    pub offset: usize,
    pub rows: Vec<FindingRow>,
}

pub fn findings_page(assessment: &Assessment, edition: &Edition, query: &FindingQuery) -> FindingsPage {
    let search = query.search.as_deref().map(str::trim).filter(|search| !search.is_empty()).map(str::to_lowercase);
    let matching: Vec<&Item> =
        assessment.items.iter().filter(|item| query.matches(assessment, item, search.as_deref())).collect();
    let limit = if query.limit == 0 { 100 } else { query.limit.min(MAX_PAGE) };
    let offset = query.offset.min(matching.len());
    FindingsPage {
        total: matching.len(),
        offset,
        rows: matching.iter().skip(offset).take(limit).map(|item| FindingRow::new(assessment, item, edition)).collect(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceView {
    pub line: Option<u32>,
    pub text: String,
    pub masked: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OccurrenceView {
    pub machine: String,
    pub location_kind: String,
    pub path: String,
    pub item: Option<String>,
    pub target: Option<String>,
    pub activation: String,
}

/// What the effort value includes (translated by the frontend with `effort.*`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EffortNote {
    pub same_as: Option<usize>,
    pub windows: bool,
    pub size_factor: u32,
    pub typical_script: bool,
    pub counted_once: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceView {
    pub publisher: String,
    pub title: String,
    pub url: String,
    pub checked: String,
}

/// Everything about one item.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FindingDetail {
    #[serde(flatten)]
    pub row: FindingRow,
    pub reported_activation: String,
    pub evidence: Vec<EvidenceView>,
    pub occurrences: Vec<OccurrenceView>,
    pub more_occurrences: usize,
    pub started_by: Vec<usize>,
    pub starts: Vec<usize>,
    pub same_content_as: Option<usize>,
    pub file_size: Option<u64>,
    pub sha256: Option<String>,
    pub details: BTreeMap<String, String>,
    /// Translation key of the migration hint – `None` in the free edition.
    pub hint: Option<String>,
    /// `None` in the free edition.
    pub effort_note: Option<EffortNote>,
    pub sources: Vec<SourceView>,
}

pub fn finding_detail(assessment: &Assessment, edition: &Edition, number: usize) -> Option<FindingDetail> {
    let item = assessment.item(number)?;
    let first = item.first();
    let catalog = vbs_core::rules::catalog();
    let sources = catalog
        .rule(&item.rule)
        .map(|rule| {
            rule.sources
                .iter()
                .filter_map(|id| catalog.source(id))
                .map(|source| SourceView {
                    publisher: source.publisher.clone(),
                    title: source.title.clone(),
                    url: source.url.clone(),
                    checked: format!(
                        "{:04}-{:02}-{:02}",
                        source.checked.year(),
                        u8::from(source.checked.month()),
                        source.checked.day()
                    ),
                })
                .collect()
        })
        .unwrap_or_default();
    let note = match item.effort {
        Effort::SameAs(number) => EffortNote {
            same_as: Some(number),
            windows: false,
            size_factor: 1,
            typical_script: false,
            counted_once: false,
        },
        Effort::Windows => {
            EffortNote { same_as: None, windows: true, size_factor: 1, typical_script: false, counted_once: false }
        }
        Effort::Hours(estimate) => EffortNote {
            same_as: None,
            windows: false,
            size_factor: estimate.size_factor,
            typical_script: estimate.typical_script,
            counted_once: item.machines > 1,
        },
    };
    let effort_note = edition.allows_effort().then_some(note);
    Some(FindingDetail {
        row: FindingRow::new(assessment, item, edition),
        reported_activation: item.reported_activation.as_str().to_owned(),
        evidence: item
            .evidence
            .iter()
            .map(|line| EvidenceView { line: line.line, text: line.text.clone(), masked: line.masked })
            .collect(),
        occurrences: item
            .occurrences
            .iter()
            .take(MAX_OCCURRENCES)
            .map(|occurrence| OccurrenceView {
                machine: assessment.machines[occurrence.machine].machine.hostname.clone(),
                location_kind: occurrence.location.kind.as_str().to_owned(),
                path: occurrence.location.path.clone(),
                item: occurrence.location.item.clone(),
                target: occurrence.target.clone(),
                activation: occurrence.activation.as_str().to_owned(),
            })
            .collect(),
        more_occurrences: item.occurrences.len().saturating_sub(MAX_OCCURRENCES),
        started_by: item.started_by.clone(),
        starts: item.starts.clone(),
        same_content_as: item.same_content_as,
        file_size: first.file.as_ref().map(|file| file.size),
        sha256: first.file.as_ref().and_then(|file| file.sha256.clone()),
        details: item
            .details
            .iter()
            .map(|(key, value)| {
                let value = match value {
                    Detail::Flag(flag) => flag.to_string(),
                    Detail::Number(number) => number.to_string(),
                    Detail::Text(text) => text.clone(),
                };
                (key.clone(), value)
            })
            .collect(),
        hint: edition.allows_hints().then(|| hint_key(item)),
        effort_note,
        sources,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::{date, datetime};
    use vbs_evaluation::import::ImportedFile;

    fn assessment() -> Assessment {
        let files: Vec<ImportedFile> = vbs_evaluation::sample::organization(12, datetime!(2026-09-25 08:00 UTC))
            .into_iter()
            .enumerate()
            .map(|(index, result)| ImportedFile { path: format!("m{index}.vbscout").into(), result, unknown_values: 0 })
            .collect();
        Assessment::build(&files, None)
    }

    #[test]
    fn the_free_edition_views_carry_no_hints_and_no_effort() {
        let assessment = assessment();
        let free = Edition::free();
        let overview = Overview::new(&assessment, &free);
        assert!(overview.effort.is_none() && overview.by_kind.iter().all(|row| row.effort.is_none()));
        let page = findings_page(&assessment, &free, &FindingQuery::default());
        assert!(page.rows.iter().all(|row| row.effort.is_none()));
        let detail = finding_detail(&assessment, &free, 1).unwrap();
        assert!(detail.hint.is_none() && detail.effort_note.is_none());
        let json = serde_json::to_string(&detail).unwrap();
        assert!(!json.contains("hint.") && !json.contains("\"min\""), "{json}");

        let licensed = Edition::Msp { company: "IT".into(), expires: date!(2099 - 01 - 01) };
        let detail = finding_detail(&assessment, &licensed, 1).unwrap();
        assert!(detail.hint.as_deref().is_some_and(|key| key.starts_with("hint.")));
        assert!(detail.row.effort.is_some() && Overview::new(&assessment, &licensed).effort.is_some());
    }

    #[test]
    fn finding_list_filters_and_pages() {
        let assessment = assessment();
        let free = Edition::free();
        let all_own = findings_page(&assessment, &free, &FindingQuery::default());
        assert_eq!(all_own.total, assessment.summary.items);
        let windows = findings_page(
            &assessment,
            &free,
            &FindingQuery { origin: Some("windows".into()), ..FindingQuery::default() },
        );
        assert_eq!(windows.total, assessment.summary.windows_items);
        let high =
            findings_page(&assessment, &free, &FindingQuery { risk: Some("high".into()), ..FindingQuery::default() });
        assert_eq!(high.total, assessment.summary.high);
        let search = findings_page(
            &assessment,
            &free,
            &FindingQuery { search: Some("NETLOGON".into()), ..FindingQuery::default() },
        );
        assert!(
            search.total >= 2
                && search.rows.iter().all(|row| row.location.contains("NETLOGON") || row.target.is_some())
        );
        let paged = findings_page(&assessment, &free, &FindingQuery { offset: 2, limit: 3, ..FindingQuery::default() });
        assert_eq!((paged.offset, paged.rows.len(), paged.rows[0].number), (2, 3, 3));
        assert!(
            finding_detail(&assessment, &free, 0).is_none() && finding_detail(&assessment, &free, 10_000).is_none()
        );
    }
}
