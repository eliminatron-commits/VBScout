//! Texts of the reports in the report language: translated values, numbers, dates and hours.
//!
//! Number and date formats are part of the catalogs, so translators can adjust them:
//! `format.number` is the number 1234.5 written in the language (`1,234.5`, `1.234,5`,
//! `1 234,5`), `format.date` a pattern with `{year}`, `{month}` and `{day}`.

use time::OffsetDateTime;
use vbs_core::model::{Activation, Classification, FindingKind, FindingStatus, NotCheckableReason, SourceStatus};
use vbs_core::rules;
use vbs_i18n::{Lang, has_key, key, t, t_args, t_count};

use crate::assessment::{Item, Origin, Risk};

/// Formatting in one language.
#[derive(Debug, Clone)]
pub struct Text {
    pub lang: Lang,
    group: char,
    decimal: char,
    date: String,
}

impl Text {
    pub fn new(lang: Lang) -> Self {
        let sample: Vec<char> = t(lang, "format.number").chars().collect();
        let (group, decimal) = match sample.as_slice() {
            ['1', group, '2', '3', '4', decimal, '5'] => (*group, *decimal),
            _ => (',', '.'),
        };
        Self { lang, group, decimal, date: t(lang, "format.date") }
    }

    /// A message without arguments.
    pub fn t(&self, key: &str) -> String {
        t(self.lang, key)
    }

    pub fn args(&self, key: &str, args: &[(&str, &dyn std::fmt::Display)]) -> String {
        t_args(self.lang, key, args)
    }

    pub fn count(&self, base: &str, count: usize, args: &[(&str, &dyn std::fmt::Display)]) -> String {
        t_count(self.lang, base, count as u64, args)
    }

    /// A message whose key is built from a value (`kind.scriptFile`); unknown values (from newer
    /// versions) are shown as they are.
    pub fn value(&self, prefix: &str, value: &str) -> String {
        let key = format!("{prefix}.{value}");
        if has_key(&key) { t(self.lang, &key) } else { value.to_owned() }
    }

    pub fn integer(&self, value: u64) -> String {
        let digits = value.to_string();
        let mut out = String::with_capacity(digits.len() + digits.len() / 3);
        for (index, digit) in digits.chars().enumerate() {
            if index > 0 && (digits.len() - index).is_multiple_of(3) {
                out.push(self.group);
            }
            out.push(digit);
        }
        out
    }

    pub fn count_of(&self, value: usize) -> String {
        self.integer(value as u64)
    }

    /// A number with at most `digits` decimals, trailing zeros removed.
    pub fn decimal(&self, value: f64, digits: usize) -> String {
        let text = format!("{value:.digits$}");
        let text = if text.contains('.') { text.trim_end_matches('0').trim_end_matches('.').to_owned() } else { text };
        let (whole, fraction) = text.split_once('.').unwrap_or((&text, ""));
        let negative = whole.starts_with('-');
        let whole = self.integer(whole.trim_start_matches('-').parse().unwrap_or(0));
        let sign = if negative { "-" } else { "" };
        if fraction.is_empty() { format!("{sign}{whole}") } else { format!("{sign}{whole}{}{fraction}", self.decimal) }
    }

    /// Hours without false precision: two decimals below 1 h, one below 10 h, whole hours above.
    pub fn hours(&self, value: f64) -> String {
        if value >= 10.0 {
            self.decimal(value.round(), 0)
        } else if value >= 1.0 {
            self.decimal(value, 1)
        } else {
            self.decimal(value, 2)
        }
    }

    /// "0.5–2 h".
    pub fn hour_range(&self, min: f64, max: f64) -> String {
        if (max - min).abs() < f64::EPSILON {
            return self.args("report.hours", &[("value", &self.hours(min))]);
        }
        self.args("report.hourRange", &[("min", &self.hours(min)), ("max", &self.hours(max))])
    }

    /// Person-days of 8 hours.
    pub fn days(&self, hours: f64) -> String {
        let days = hours / 8.0;
        if days >= 10.0 { self.decimal(days.round(), 0) } else { self.decimal(days, 1) }
    }

    pub fn date(&self, value: OffsetDateTime) -> String {
        let value = value.to_offset(time::UtcOffset::UTC);
        self.date
            .replace("{year}", &format!("{:04}", value.year()))
            .replace("{month}", &format!("{:02}", u8::from(value.month())))
            .replace("{day}", &format!("{:02}", value.day()))
    }

    pub fn date_time(&self, value: OffsetDateTime) -> String {
        let utc = value.to_offset(time::UtcOffset::UTC);
        format!("{} {:02}:{:02} UTC", self.date(value), utc.hour(), utc.minute())
    }

    pub fn kind(&self, kind: &FindingKind) -> String {
        self.value("kind", kind.as_str())
    }

    pub fn activation(&self, activation: &Activation) -> String {
        self.value("activation", activation.as_str())
    }

    pub fn risk(&self, risk: Risk) -> String {
        self.t(&format!("risk.{}", risk.as_str()))
    }

    pub fn origin(&self, origin: Origin) -> String {
        self.t(&format!("origin.{}", origin.as_str()))
    }

    pub fn classification(&self, classification: &Classification) -> String {
        self.value("classification", classification.effective().as_str())
    }

    pub fn status(&self, status: &FindingStatus, reason: Option<&NotCheckableReason>) -> String {
        match status {
            FindingStatus::NotCheckable => match reason {
                Some(reason) => {
                    format!("{} – {}", self.t("findingStatus.notCheckable"), self.value("reason", reason.as_str()))
                }
                None => self.t("findingStatus.notCheckable"),
            },
            _ => self.t("findingStatus.detected"),
        }
    }

    pub fn reason(&self, reason: &NotCheckableReason) -> String {
        self.value("reason", reason.as_str())
    }

    pub fn source(&self, id: &str) -> String {
        self.value("source", id)
    }

    pub fn source_status(&self, status: &SourceStatus) -> String {
        self.value("sourceStatus", status.as_str())
    }

    pub fn source_reason(&self, code: &str) -> String {
        self.value("sourceReason", code)
    }

    pub fn limitation(&self, code: &str) -> String {
        self.value("limitation", code)
    }

    /// Title of a rule; rules from newer versions show their ID.
    pub fn rule_title(&self, rule: &str) -> String {
        let key = format!("rule.{}.title", rules::key_stem(rule));
        if has_key(&key) { t(self.lang, &key) } else { rule.to_owned() }
    }

    pub fn rule_rationale(&self, rule: &str) -> Option<String> {
        let key = format!("rule.{}.rationale", rules::key_stem(rule));
        has_key(&key).then(|| t(self.lang, &key))
    }

    /// The migration hint of an item in this language (see [`hint_key`]).
    pub fn hint(&self, item: &Item) -> String {
        t(self.lang, &hint_key(item))
    }

    pub fn yes_no(&self, value: bool) -> String {
        self.t(if value { key("report.yes") } else { key("report.no") })
    }
}

/// The translation key of an item's migration hint: Windows components and items that could not
/// be checked have their own; everything else the hint of its rule.
pub fn hint_key(item: &Item) -> String {
    if item.origin == Origin::Windows {
        return key("hint.windowsComponent").to_owned();
    }
    if let Some(reason) = &item.reason {
        let reason_key = format!("hint.reason.{}", reason.as_str());
        if has_key(&reason_key) {
            return reason_key;
        }
    }
    match rules::catalog().rule(&item.rule) {
        Some(rule) => rule.hint_key(),
        None => key("hint.notCheckable").to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    #[test]
    fn numbers_dates_and_hours_per_language() {
        let en = Text::new(Lang::En);
        let de = Text::new(Lang::De);
        assert_eq!(en.integer(1_234_567), "1,234,567");
        assert_eq!(de.integer(1_234_567), "1.234.567");
        assert_eq!(en.integer(999), "999");
        assert_eq!(en.hours(0.25), "0.25");
        assert_eq!(de.hours(0.25), "0,25");
        assert_eq!(de.hours(1.5), "1,5");
        assert_eq!(en.hours(2.0), "2");
        assert_eq!(en.hours(37.35), "37");
        assert_eq!(en.hours(1234.4), "1,234");
        assert_eq!(en.hour_range(0.5, 2.0), "0.5–2 h");
        assert_eq!(de.hour_range(4.0, 4.0), "4 h");
        assert_eq!(de.days(12.0), "1,5");
        let when = datetime!(2026-09-07 08:05 UTC);
        assert_eq!(en.date(when), "2026-09-07");
        assert_eq!(de.date(when), "07.09.2026");
        assert_eq!(de.date_time(when), "07.09.2026 08:05 UTC");
        for lang in Lang::ALL {
            let text = Text::new(lang);
            assert_ne!(text.group, text.decimal, "{lang}: format.number");
            assert!(text.date(when).contains("2026") && text.date(when).contains("07"), "{lang}: format.date");
        }
    }

    #[test]
    fn values_from_newer_versions_are_shown_as_they_are() {
        let en = Text::new(Lang::En);
        assert_eq!(en.kind(&FindingKind::Unknown("powerShellV1Script".into())), "powerShellV1Script");
        assert_eq!(en.rule_title("VBS-777"), "VBS-777");
        assert_eq!(en.source("files.localDrives"), t(Lang::En, "source.files.localDrives"));
        assert_eq!(en.source_reason("somethingNew"), "somethingNew");
    }
}
