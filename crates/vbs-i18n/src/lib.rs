//! Translations shared by the Rust crates (collector, reports) and the Svelte frontend.
//!
//! Taken over from Stepwright's `sw-i18n`. The catalogs live in `i18n/<tag>.json`
//! at the repository root: flat `key → message` maps, English is the source
//! language. They are embedded at compile time. Messages use `{name}`
//! placeholders; `{product}` is always filled from the central product
//! configuration, so no catalog contains the product name. Plural forms use the
//! key suffixes `_one`, `_few`, `_many` and `_other` (CLDR categories for
//! integers, see [`plural_category`]).
//!
//! `i18n/languages.json` marks which languages have been reviewed by a native
//! speaker (English and German); the others show a hint that corrections are
//! welcome.
//!
//! Key completeness is enforced by the tests below and by
//! `scripts/check-i18n.mjs`, which additionally checks every key used in code
//! and every rule of the rule catalog.

use std::collections::HashMap;
use std::fmt;
use std::sync::LazyLock;

use serde::Deserialize;

/// A supported language (UI, console output, reports and migration hints).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Lang {
    En,
    De,
    Fr,
    Es,
    It,
    Nl,
    Pl,
    PtBr,
}

impl Lang {
    /// All supported languages; English first (source language).
    pub const ALL: [Lang; 8] = [Lang::En, Lang::De, Lang::Fr, Lang::Es, Lang::It, Lang::Nl, Lang::Pl, Lang::PtBr];

    /// BCP 47 tag, identical to the catalog file name.
    pub const fn tag(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::De => "de",
            Lang::Fr => "fr",
            Lang::Es => "es",
            Lang::It => "it",
            Lang::Nl => "nl",
            Lang::Pl => "pl",
            Lang::PtBr => "pt-BR",
        }
    }

    /// Parses a supported tag exactly (case-insensitive, `_` accepted for `-`).
    pub fn from_tag(tag: &str) -> Option<Lang> {
        let normalized = tag.trim().replace('_', "-");
        Lang::ALL.into_iter().find(|lang| lang.tag().eq_ignore_ascii_case(&normalized))
    }

    /// Best supported language for an arbitrary locale such as `de-AT`,
    /// `pt_PT.UTF-8` or `fr-CA`. Falls back to English.
    pub fn negotiate(locale: &str) -> Lang {
        let locale = locale.split(['.', '@']).next().unwrap_or_default();
        if let Some(lang) = Lang::from_tag(locale) {
            return lang;
        }
        let primary = locale.split(['-', '_']).next().unwrap_or_default().to_ascii_lowercase();
        match primary.as_str() {
            "de" => Lang::De,
            "fr" => Lang::Fr,
            "es" => Lang::Es,
            "it" => Lang::It,
            "nl" => Lang::Nl,
            "pl" => Lang::Pl,
            "pt" => Lang::PtBr,
            _ => Lang::En,
        }
    }

    /// Whether a native speaker has reviewed this translation (`i18n/languages.json`).
    /// Unreviewed languages show a hint that corrections are welcome.
    pub fn is_reviewed(self) -> bool {
        LANGUAGES.languages.get(self.tag()).is_some_and(|meta| meta.reviewed)
    }
}

impl fmt::Display for Lang {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.tag())
    }
}

/// Raw catalog sources, embedded at compile time.
const SOURCES: [(Lang, &str); 8] = [
    (Lang::En, include_str!("../../../i18n/en.json")),
    (Lang::De, include_str!("../../../i18n/de.json")),
    (Lang::Fr, include_str!("../../../i18n/fr.json")),
    (Lang::Es, include_str!("../../../i18n/es.json")),
    (Lang::It, include_str!("../../../i18n/it.json")),
    (Lang::Nl, include_str!("../../../i18n/nl.json")),
    (Lang::Pl, include_str!("../../../i18n/pl.json")),
    (Lang::PtBr, include_str!("../../../i18n/pt-BR.json")),
];

/// Raw `i18n/languages.json` (source language and review status per language).
pub const LANGUAGES_JSON: &str = include_str!("../../../i18n/languages.json");

#[derive(Debug, Deserialize)]
struct LanguageFile {
    languages: HashMap<String, LanguageMeta>,
}

#[derive(Debug, Deserialize)]
struct LanguageMeta {
    reviewed: bool,
}

static LANGUAGES: LazyLock<LanguageFile> = LazyLock::new(|| {
    serde_json::from_str(LANGUAGES_JSON).unwrap_or_else(|e| panic!("i18n/languages.json is invalid: {e}"))
});

type Catalog = HashMap<String, String>;

static CATALOGS: LazyLock<HashMap<Lang, Catalog>> = LazyLock::new(|| {
    SOURCES
        .iter()
        .map(|&(lang, source)| {
            let catalog: Catalog = serde_json::from_str(source)
                .unwrap_or_else(|e| panic!("i18n/{}.json is not a flat string map: {e}", lang.tag()));
            (lang, catalog)
        })
        .collect()
});

fn catalog(lang: Lang) -> &'static Catalog {
    &CATALOGS[&lang]
}

/// Marks a string literal as a message key where it is not passed to
/// [`t`] directly, so `scripts/check-i18n.mjs` can verify it exists.
pub const fn key(key: &'static str) -> &'static str {
    key
}

/// All message keys of the source (English) catalog.
pub fn source_keys() -> impl Iterator<Item = &'static str> {
    catalog(Lang::En).keys().map(String::as_str)
}

/// Whether the source catalog defines `key`.
pub fn has_key(key: &str) -> bool {
    catalog(Lang::En).contains_key(key)
}

/// Translates `key` without arguments (`{product}` is still filled in).
pub fn t(lang: Lang, key: &str) -> String {
    t_args(lang, key, &[])
}

/// Translates `key`, substituting `{placeholders}` from `args`.
///
/// Missing keys fall back to English and finally to the key itself, so a gap
/// never breaks the UI (and is caught by the checks before release).
pub fn t_args(lang: Lang, key: &str, args: &[(&str, &dyn fmt::Display)]) -> String {
    let template = catalog(lang).get(key).or_else(|| catalog(Lang::En).get(key)).map_or(key, String::as_str);
    format_message(template, args)
}

/// Plural-aware translation of `<base>_<category>`; `{count}` is available as
/// placeholder in addition to `args`.
pub fn t_count(lang: Lang, base: &str, count: u64, args: &[(&str, &dyn fmt::Display)]) -> String {
    let lookup = |lang: Lang, category: PluralCategory| catalog(lang).get(&format!("{base}_{}", category.suffix()));
    let template = lookup(lang, plural_category(lang, count))
        .or_else(|| lookup(lang, PluralCategory::Other))
        .or_else(|| lookup(Lang::En, plural_category(Lang::En, count)))
        .map_or(base, String::as_str);
    let mut all_args: Vec<(&str, &dyn fmt::Display)> = vec![("count", &count)];
    all_args.extend_from_slice(args);
    format_message(template, &all_args)
}

/// Replaces `{name}` placeholders. Unknown placeholders are left untouched.
fn format_message(template: &str, args: &[(&str, &dyn fmt::Display)]) -> String {
    let mut out = String::with_capacity(template.len() + 16);
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let name_len = after.find('}').filter(|&len| len > 0 && is_placeholder_name(&after[..len]));
        match name_len {
            Some(len) => {
                let name = &after[..len];
                match args.iter().find(|(arg, _)| *arg == name) {
                    Some((_, value)) => out.push_str(&value.to_string()),
                    None if name == "product" => out.push_str(&vbs_config::product().name),
                    None => {
                        out.push('{');
                        out.push_str(name);
                        out.push('}');
                    }
                }
                rest = &after[len + 1..];
            }
            None => {
                out.push('{');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

fn is_placeholder_name(name: &str) -> bool {
    name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Names of the `{placeholders}` used in a message.
pub fn placeholders(message: &str) -> Vec<&str> {
    let mut names = Vec::new();
    let mut rest = message;
    while let Some(open) = rest.find('{') {
        let after = &rest[open + 1..];
        match after.find('}') {
            Some(len) if len > 0 && is_placeholder_name(&after[..len]) => {
                names.push(&after[..len]);
                rest = &after[len + 1..];
            }
            _ => rest = after,
        }
    }
    names.sort_unstable();
    names.dedup();
    names
}

/// CLDR plural category (integer counts only).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PluralCategory {
    One,
    Few,
    Many,
    Other,
}

impl PluralCategory {
    pub const fn suffix(self) -> &'static str {
        match self {
            PluralCategory::One => "one",
            PluralCategory::Few => "few",
            PluralCategory::Many => "many",
            PluralCategory::Other => "other",
        }
    }
}

/// Plural category of an integer `n` in `lang` (CLDR rules, integer subset).
pub fn plural_category(lang: Lang, n: u64) -> PluralCategory {
    match lang {
        Lang::En | Lang::De | Lang::Nl | Lang::It | Lang::Es => {
            if n == 1 {
                PluralCategory::One
            } else {
                PluralCategory::Other
            }
        }
        Lang::Fr | Lang::PtBr => {
            if n <= 1 {
                PluralCategory::One
            } else {
                PluralCategory::Other
            }
        }
        Lang::Pl => {
            let (m10, m100) = (n % 10, n % 100);
            if n == 1 {
                PluralCategory::One
            } else if (2..=4).contains(&m10) && !(12..=14).contains(&m100) {
                PluralCategory::Few
            } else {
                PluralCategory::Many
            }
        }
    }
}

/// Plural keys every catalog of `lang` must define for each plural message.
/// `other` is always required as universal fallback.
pub fn required_plural_categories(lang: Lang) -> &'static [PluralCategory] {
    match lang {
        Lang::Pl => &[PluralCategory::One, PluralCategory::Few, PluralCategory::Many, PluralCategory::Other],
        _ => &[PluralCategory::One, PluralCategory::Other],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, BTreeSet};

    /// Splits `machine.count_one` into (`machine.count`, `one`).
    fn split_plural(key: &str) -> Option<(&str, &str)> {
        let (base, suffix) = key.rsplit_once('_')?;
        matches!(suffix, "one" | "few" | "many" | "other").then_some((base, suffix))
    }

    /// Plural groups (`base → categories`) and plain keys of a catalog.
    fn shape(lang: Lang) -> (BTreeSet<&'static str>, BTreeMap<&'static str, BTreeSet<&'static str>>) {
        let mut plain = BTreeSet::new();
        let mut plural: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
        for key in catalog(lang).keys() {
            match split_plural(key) {
                Some((base, suffix)) => {
                    plural.entry(base).or_default().insert(suffix);
                }
                None => {
                    plain.insert(key.as_str());
                }
            }
        }
        (plain, plural)
    }

    #[test]
    fn all_catalogs_have_the_source_keys() {
        let (en_plain, en_plural) = shape(Lang::En);
        for lang in Lang::ALL {
            let (plain, plural) = shape(lang);
            let missing: Vec<_> = en_plain.difference(&plain).collect();
            let extra: Vec<_> = plain.difference(&en_plain).collect();
            assert!(missing.is_empty() && extra.is_empty(), "{lang}: missing {missing:?}, extra {extra:?}");
            let bases: BTreeSet<_> = plural.keys().collect();
            let en_bases: BTreeSet<_> = en_plural.keys().collect();
            assert_eq!(bases, en_bases, "{lang}: plural groups differ");
            let required: BTreeSet<&str> = required_plural_categories(lang).iter().map(|c| c.suffix()).collect();
            for (base, categories) in &plural {
                assert_eq!(categories, &required, "{lang}: plural forms of {base}");
            }
        }
    }

    #[test]
    fn placeholders_match_the_source() {
        for lang in Lang::ALL {
            for (key, message) in catalog(lang) {
                let source_key = match split_plural(key) {
                    Some((base, _)) => format!("{base}_other"),
                    None => key.clone(),
                };
                let mut expected = placeholders(&catalog(Lang::En)[&source_key]);
                let mut actual = placeholders(message);
                if split_plural(key).is_some() {
                    // A plural form may spell out the number ("one machine").
                    expected.retain(|p| *p != "count");
                    actual.retain(|p| *p != "count");
                }
                assert_eq!(actual, expected, "{lang}: placeholders of {key}");
            }
        }
    }

    #[test]
    fn messages_are_clean() {
        let product = &vbs_config::product().name;
        for lang in Lang::ALL {
            for (key, message) in catalog(lang) {
                assert!(!message.trim().is_empty(), "{lang}: {key} is empty");
                assert_eq!(message.trim(), message, "{lang}: {key} has surrounding whitespace");
                assert!(
                    !message.contains(product.as_str()),
                    "{lang}: {key} hard-codes the product name, use {{product}}"
                );
            }
        }
    }

    #[test]
    fn language_metadata_matches_the_catalogs() {
        let raw: serde_json::Value = serde_json::from_str(LANGUAGES_JSON).unwrap();
        assert_eq!(raw["source"], Lang::En.tag());
        let listed: BTreeSet<&str> = LANGUAGES.languages.keys().map(String::as_str).collect();
        let supported: BTreeSet<&str> = Lang::ALL.iter().map(|lang| lang.tag()).collect();
        assert_eq!(listed, supported);
        // English and German are the reviewed languages (see CLAUDE.md).
        let reviewed: Vec<Lang> = Lang::ALL.into_iter().filter(|lang| lang.is_reviewed()).collect();
        assert_eq!(reviewed, [Lang::En, Lang::De]);
    }

    #[test]
    fn negotiates_locales() {
        assert_eq!(Lang::negotiate("de-AT"), Lang::De);
        assert_eq!(Lang::negotiate("de_DE.UTF-8"), Lang::De);
        assert_eq!(Lang::negotiate("pt-PT"), Lang::PtBr);
        assert_eq!(Lang::negotiate("pt_BR"), Lang::PtBr);
        assert_eq!(Lang::negotiate("fr-CA"), Lang::Fr);
        assert_eq!(Lang::negotiate("ja-JP"), Lang::En);
        assert_eq!(Lang::negotiate(""), Lang::En);
        assert_eq!(Lang::from_tag("PT-br"), Some(Lang::PtBr));
    }

    #[test]
    fn plural_rules() {
        use PluralCategory::*;
        let pl: Vec<_> = [1, 2, 4, 5, 12, 14, 21, 22, 25, 112, 122].map(|n| plural_category(Lang::Pl, n)).into();
        assert_eq!(pl, [One, Few, Few, Many, Many, Many, Many, Few, Many, Many, Few]);
        assert_eq!(plural_category(Lang::Fr, 0), One);
        assert_eq!(plural_category(Lang::En, 0), Other);
        assert_eq!(plural_category(Lang::De, 1), One);
    }

    #[test]
    fn formats_messages() {
        assert_eq!(t_count(Lang::En, "machine.count", 1, &[]), "1 machine");
        assert_eq!(t_count(Lang::De, "machine.count", 3, &[]), "3 Rechner");
        assert_eq!(t_count(Lang::Pl, "machine.count", 22, &[]), "22 komputery");
        assert_eq!(t_count(Lang::Pl, "machine.count", 25, &[]), "25 komputerów");
        let exists = t_args(Lang::En, "collector.error.outputExists", &[("path", &"C:\\out.x")]);
        assert!(exists.contains(&vbs_config::product().name) && exists.contains("C:\\out.x"));
        assert_eq!(format_message("{a} {b} {} {x-y}", &[("a", &1)]), "1 {b} {} {x-y}");
    }

    #[test]
    fn missing_keys_fall_back() {
        let missing = "does.not.exist";
        assert_eq!(t(Lang::De, missing), missing);
        assert!(!has_key(missing));
    }
}
