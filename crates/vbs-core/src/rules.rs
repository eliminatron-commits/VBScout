//! The rule catalog (`rules/catalog.json`).
//!
//! Every detection rule has an ID (`VBS-nnn`), the finding kind it produces,
//! a classification – `breaks` (stops working once VBScript is disabled) or
//! `review` (impact uncertain; the default whenever in doubt) – and at least
//! one source with the date it was checked. Title and rationale are
//! translatable texts in `i18n/<lang>.json` under `rule.<stem>.title` and
//! `rule.<stem>.rationale`, where `<stem>` is the ID in lower case without
//! the hyphen (`VBS-101` → `vbs101`). Rule IDs are stable: a retired rule is
//! never reused for something else.
//!
//! ID ranges: 1xx script files, 2xx scripts and shortcuts that start VBScript,
//! 3xx scheduled tasks, autostart, services, WMI and logon scripts,
//! 4xx MSI custom actions, 5xx event logs, 6xx Office macros, 9xx security.
//! `x00` of each range is the "could not be checked" rule of that range.

use std::collections::HashSet;
use std::sync::LazyLock;

use serde::Deserialize;
use time::Date;

use crate::model::{Classification, FindingKind};
use crate::validate::is_rule_id;

time::serde::format_description!(iso_date, Date, "[year]-[month]-[day]");

/// Raw contents of `rules/catalog.json`, embedded at compile time.
pub const CATALOG_JSON: &str = include_str!("../../../rules/catalog.json");

/// Version of the catalog file format (not of its content – that is `asOf`).
pub const CATALOG_FORMAT_VERSION: u32 = 1;

static CATALOG: LazyLock<RuleCatalog> = LazyLock::new(|| {
    RuleCatalog::parse(CATALOG_JSON).unwrap_or_else(|problem| panic!("rules/catalog.json is invalid: {problem}"))
});

/// Returns the embedded rule catalog.
pub fn catalog() -> &'static RuleCatalog {
    &CATALOG
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuleCatalog {
    pub format_version: u32,
    /// Date the catalog content was last checked against its sources ("Stand").
    #[serde(with = "iso_date")]
    pub as_of: Date,
    pub sources: Vec<Source>,
    pub rules: Vec<Rule>,
}

/// A document a rule relies on – current vendor documentation wherever possible.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Source {
    pub id: String,
    pub publisher: String,
    pub title: String,
    pub url: String,
    /// Publication date of the document, if it states one.
    #[serde(default, with = "iso_date::option")]
    pub published: Option<Date>,
    /// When the statement the rules rely on was last checked.
    #[serde(with = "iso_date")]
    pub checked: Date,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Rule {
    pub id: String,
    pub kind: FindingKind,
    pub classification: Classification,
    /// IDs of the sources backing the classification.
    pub sources: Vec<String>,
    /// Rule-of-thumb effort per finding, used by the evaluation.
    pub effort: Effort,
    /// Migration hint: the text `hint.<id>` in `i18n/<lang>.json`, shared by rules that are
    /// migrated the same way.
    pub hint: String,
}

/// Rough effort estimate in hours – always presented as a rule of thumb, never as a quote.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Effort {
    pub min_hours: f64,
    pub max_hours: f64,
    #[serde(default)]
    pub basis: EffortBasis,
}

/// What the effort range refers to.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EffortBasis {
    /// The range as given (checking or changing one item).
    #[default]
    Fixed,
    /// Code to rewrite: the range grows with the size of the script file.
    ScriptSize,
    /// An entry that starts a script (task, autostart value, shortcut, …): changing the entry.
    /// The script itself is counted where the scan found it – or with the effort of a typical
    /// script when the scan did not find it.
    Entry,
}

impl Rule {
    /// `VBS-101` → `vbs101`, the stem of the rule's translation keys.
    pub fn key_stem(&self) -> String {
        key_stem(&self.id)
    }

    pub fn title_key(&self) -> String {
        format!("rule.{}.title", self.key_stem())
    }

    pub fn rationale_key(&self) -> String {
        format!("rule.{}.rationale", self.key_stem())
    }

    pub fn hint_key(&self) -> String {
        format!("hint.{}", self.hint)
    }
}

/// Translation key stem of a rule ID (`VBS-101` → `vbs101`).
pub fn key_stem(rule_id: &str) -> String {
    rule_id.to_ascii_lowercase().replace('-', "")
}

impl RuleCatalog {
    /// Parses and validates a catalog.
    pub fn parse(json: &str) -> Result<Self, String> {
        let catalog: RuleCatalog = serde_json::from_str(json).map_err(|e| e.to_string())?;
        catalog.validate()?;
        Ok(catalog)
    }

    pub fn rule(&self, id: &str) -> Option<&Rule> {
        self.rules.iter().find(|rule| rule.id == id)
    }

    pub fn source(&self, id: &str) -> Option<&Source> {
        self.sources.iter().find(|source| source.id == id)
    }

    /// The catalog date as `YYYY-MM-DD` (written into every result file).
    pub fn as_of_text(&self) -> String {
        format!("{:04}-{:02}-{:02}", self.as_of.year(), u8::from(self.as_of.month()), self.as_of.day())
    }

    /// Checks the invariants the collector, the evaluation and the reports rely on.
    pub fn validate(&self) -> Result<(), String> {
        if self.format_version != CATALOG_FORMAT_VERSION {
            return Err(format!("formatVersion must be {CATALOG_FORMAT_VERSION}"));
        }
        let mut source_ids = HashSet::new();
        for source in &self.sources {
            let valid_id = !source.id.is_empty()
                && source.id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
            if !valid_id || !source_ids.insert(source.id.as_str()) {
                return Err(format!("source id {:?} must be unique kebab-case", source.id));
            }
            if !source.url.starts_with("https://") {
                return Err(format!("source {}: url must be https://", source.id));
            }
            if source.title.trim().is_empty() || source.publisher.trim().is_empty() {
                return Err(format!("source {}: title and publisher are required", source.id));
            }
            if source.checked > self.as_of {
                return Err(format!("source {}: checked after the catalog date", source.id));
            }
            if source.published.is_some_and(|published| published > source.checked) {
                return Err(format!("source {}: published after it was checked", source.id));
            }
        }

        let mut rule_ids = HashSet::new();
        let mut referenced = HashSet::new();
        for rule in &self.rules {
            if !is_rule_id(&rule.id) || !rule_ids.insert(rule.id.as_str()) {
                return Err(format!("rule id {:?} must be unique and look like VBS-nnn", rule.id));
            }
            if !rule.kind.is_known() {
                return Err(format!("rule {}: unknown kind {}", rule.id, rule.kind));
            }
            if !matches!(rule.classification, Classification::Breaks | Classification::Review) {
                return Err(format!("rule {}: classification must be breaks or review", rule.id));
            }
            if rule.sources.is_empty() {
                return Err(format!("rule {}: at least one source is required", rule.id));
            }
            for source in &rule.sources {
                if !source_ids.contains(source.as_str()) {
                    return Err(format!("rule {}: unknown source {source:?}", rule.id));
                }
                referenced.insert(source.as_str());
            }
            let effort = rule.effort;
            if !(effort.min_hours > 0.0 && effort.min_hours <= effort.max_hours && effort.max_hours <= 100.0) {
                return Err(format!("rule {}: effort must satisfy 0 < minHours ≤ maxHours ≤ 100", rule.id));
            }
            if rule.hint.is_empty() || !rule.hint.chars().all(|c| c.is_ascii_alphanumeric()) {
                return Err(format!("rule {}: hint must be a camelCase identifier", rule.id));
            }
            if rule.id.ends_with("00") && rule.classification != Classification::Review {
                return Err(format!("rule {}: \"could not be checked\" rules are always review", rule.id));
            }
        }
        if let Some(unused) = self.sources.iter().find(|source| !referenced.contains(source.id.as_str())) {
            return Err(format!("source {} is not used by any rule", unused.id));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid() -> serde_json::Value {
        serde_json::from_str(CATALOG_JSON).expect("catalog is JSON")
    }

    #[test]
    fn embedded_catalog_is_valid() {
        let catalog = catalog();
        assert!(!catalog.rules.is_empty());
        assert_eq!(catalog.as_of_text().len(), 10);
        for rule in &catalog.rules {
            for source in &rule.sources {
                assert!(catalog.source(source).is_some());
            }
        }
    }

    #[test]
    fn every_rule_has_translated_texts() {
        for rule in &catalog().rules {
            for key in [rule.title_key(), rule.rationale_key(), rule.hint_key()] {
                assert!(vbs_i18n::has_key(&key), "i18n/en.json lacks {key}");
            }
        }
    }

    #[test]
    fn rejects_invalid_catalogs() {
        type Mutation = (&'static str, fn(&mut serde_json::Value));
        let mutations: [Mutation; 11] = [
            ("classification", |c| c["rules"][0]["classification"] = "harmless".into()),
            ("effort order", |c| c["rules"][0]["effort"]["minHours"] = 9.0.into()),
            ("effort basis", |c| c["rules"][0]["effort"]["basis"] = "perMachine".into()),
            ("missing effort", |c| drop(c["rules"][0].as_object_mut().map(|rule| rule.remove("effort")))),
            ("hint", |c| c["rules"][0]["hint"] = "not a key".into()),
            ("missing sources", |c| c["rules"][0]["sources"] = serde_json::json!([])),
            ("unknown source", |c| c["rules"][0]["sources"] = serde_json::json!(["nope"])),
            ("rule id", |c| c["rules"][0]["id"] = "R1".into()),
            ("http source", |c| c["sources"][0]["url"] = "http://example.org".into()),
            ("checked in the future", |c| c["sources"][0]["checked"] = "2999-01-01".into()),
            ("unknown field", |c| c["rules"][0]["severity"] = "high".into()),
        ];
        for (label, mutate) in mutations {
            let mut catalog = valid();
            mutate(&mut catalog);
            assert!(RuleCatalog::parse(&catalog.to_string()).is_err(), "{label} should be rejected");
        }
    }

    #[test]
    fn key_stems() {
        assert_eq!(key_stem("VBS-101"), "vbs101");
    }
}
