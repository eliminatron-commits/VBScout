//! Structural checks for results read from disk and the invariants enforced on
//! every result written.
//!
//! Result files travel from customer machines to the evaluation (USB sticks,
//! shares, RMM uploads), so every file is untrusted input: sizes and counts are
//! bounded, IDs must be unique, and evidence must look like what the collector
//! writes. The writer side ([`sanitize`]) shortens excerpts, strips control
//! characters and masks secrets again, so a module bug can never leak a
//! password into a result file.

use std::collections::HashSet;

use crate::error::FormatError;
use crate::model::{Evidence, FindingStatus, ScanResult};
use crate::secrets;

/// Upper bounds for reading (corrupt or malicious files, zip bombs) and writing.
pub mod limits {
    /// Maximum number of entries in the ZIP container.
    pub const MAX_ENTRIES: usize = 16;
    pub const MAX_MIMETYPE_BYTES: u64 = 256;
    /// Maximum uncompressed size of `result.json`.
    pub const MAX_RESULT_BYTES: u64 = 256 * 1024 * 1024;
    pub const MAX_FINDINGS: usize = 500_000;
    pub const MAX_SOURCES: usize = 10_000;
    /// Evidence lines per finding written by the collector …
    pub const MAX_EVIDENCE_LINES: usize = 5;
    /// … and characters per evidence line (longer lines are shortened with "…").
    pub const MAX_EVIDENCE_CHARS: usize = 240;
    /// Readers accept a little more, so a slightly different writer stays readable.
    pub const MAX_EVIDENCE_LINES_READ: usize = 50;
    pub const MAX_EVIDENCE_CHARS_READ: usize = 4_096;
    /// Paths, items, targets and detail values.
    pub const MAX_TEXT_CHARS: usize = 32_768;
    pub const MAX_DETAILS: usize = 64;
    pub const MAX_ERROR_SAMPLES: usize = 20;
}

/// Checks a result read from disk. Violations reject the whole file.
pub fn check_structure(result: &ScanResult) -> Result<(), FormatError> {
    let invalid = |message: String| Err(FormatError::Invalid(message));
    if result.findings.len() > limits::MAX_FINDINGS {
        return Err(FormatError::LimitExceeded(format!("more than {} findings", limits::MAX_FINDINGS)));
    }
    if result.coverage.sources.len() > limits::MAX_SOURCES {
        return Err(FormatError::LimitExceeded(format!("more than {} coverage sources", limits::MAX_SOURCES)));
    }
    if result.machine.hostname.trim().is_empty() {
        return invalid("machine.hostname is empty".into());
    }
    if result.finished_at < result.started_at {
        return invalid("finishedAt lies before startedAt".into());
    }

    let mut ids = HashSet::with_capacity(result.findings.len());
    for finding in &result.findings {
        if finding.id.is_empty() || finding.id.len() > 32 || !ids.insert(finding.id.as_str()) {
            return invalid(format!("finding id {:?} is empty, too long or not unique", finding.id));
        }
        if !is_rule_id(&finding.rule) {
            return invalid(format!("finding {}: rule id {:?} is malformed", finding.id, finding.rule));
        }
        let not_checkable = finding.status == FindingStatus::NotCheckable;
        if not_checkable != finding.reason.is_some() {
            return invalid(format!("finding {}: a reason belongs to exactly the not-checkable findings", finding.id));
        }
        let detail_texts = finding.details.values().filter_map(|detail| match detail {
            crate::model::Detail::Text(text) => Some(text),
            _ => None,
        });
        let texts = std::iter::once(&finding.location.path)
            .chain(&finding.location.item)
            .chain(&finding.target)
            .chain(finding.details.keys())
            .chain(detail_texts);
        if texts.into_iter().any(|text| text.chars().count() > limits::MAX_TEXT_CHARS) {
            return Err(FormatError::LimitExceeded(format!("finding {}: text too long", finding.id)));
        }
        if finding.details.len() > limits::MAX_DETAILS {
            return Err(FormatError::LimitExceeded(format!("finding {}: too many details", finding.id)));
        }
        if finding.evidence.len() > limits::MAX_EVIDENCE_LINES_READ
            || finding.evidence.iter().any(|e| e.text.chars().count() > limits::MAX_EVIDENCE_CHARS_READ)
        {
            return Err(FormatError::LimitExceeded(format!("finding {}: evidence too long", finding.id)));
        }
    }
    Ok(())
}

/// `VBS-` followed by three digits, e.g. `VBS-101`.
pub fn is_rule_id(id: &str) -> bool {
    id.len() == 7 && id.starts_with("VBS-") && id[4..].chars().all(|c| c.is_ascii_digit())
}

/// Enforces the writer invariants on a result that is about to be written.
/// Returns how many evidence lines had a secret masked at this late stage
/// (should be zero – modules mask earlier through the finding builder).
pub fn sanitize(result: &mut ScanResult) -> usize {
    let mut late_masks = 0;
    for finding in &mut result.findings {
        finding.evidence.truncate(limits::MAX_EVIDENCE_LINES);
        for evidence in &mut finding.evidence {
            late_masks += usize::from(sanitize_evidence(evidence));
        }
    }
    for source in &mut result.coverage.sources {
        source.error_samples.truncate(limits::MAX_ERROR_SAMPLES);
    }
    late_masks
}

/// Cleans one evidence line in place; returns `true` if a secret had to be masked.
pub fn sanitize_evidence(evidence: &mut Evidence) -> bool {
    let cleaned: String =
        evidence.text.trim().chars().map(|c| if c.is_control() && c != '\t' { ' ' } else { c }).collect();
    let masked = secrets::mask_line(&cleaned);
    let newly_masked = masked.masked();
    evidence.masked |= newly_masked;
    evidence.text = shorten(&masked.text, limits::MAX_EVIDENCE_CHARS);
    newly_masked
}

/// Shortens `text` to at most `max` characters, ending with "…" when cut.
pub fn shorten(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let mut short: String = text.chars().take(max.saturating_sub(1)).collect();
    short.push('…');
    short
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rule_ids() {
        assert!(is_rule_id("VBS-101"));
        assert!(!is_rule_id("VBS-1010"));
        assert!(!is_rule_id("vbs-101"));
        assert!(!is_rule_id("VBS-10a"));
    }

    #[test]
    fn evidence_is_cleaned_shortened_and_masked() {
        let mut evidence = Evidence {
            line: Some(3),
            text: format!("  pwd = \"hunter2\"\u{1b}[31m {}", "x".repeat(400)),
            masked: false,
        };
        assert!(sanitize_evidence(&mut evidence));
        assert!(evidence.masked);
        assert!(!evidence.text.contains("hunter2"));
        assert!(!evidence.text.contains('\u{1b}'));
        assert_eq!(evidence.text.chars().count(), limits::MAX_EVIDENCE_CHARS);
        assert!(evidence.text.ends_with('…'));
        // Idempotent: a second pass finds nothing new.
        assert!(!sanitize_evidence(&mut evidence));
    }
}
