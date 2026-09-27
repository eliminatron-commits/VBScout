//! Checks of both reports: every language complete (no raw keys, no open placeholders), the
//! edition limits, the rule-of-thumb labels and the logo.

use std::io::Read;

use time::macros::{date, datetime};
use vbs_i18n::{Lang, t};

use super::tests::PNG;
use super::{Branding, Logo, ReportContext, ReportError, pdf, xlsx};
use crate::assessment::Assessment;
use crate::edition::Edition;
use crate::import::ImportedFile;

fn assessment() -> Assessment {
    let files: Vec<ImportedFile> = crate::sample::organization(24, datetime!(2026-09-25 08:00 UTC))
        .into_iter()
        .enumerate()
        .map(|(index, result)| ImportedFile { path: format!("m{index}.vbscout").into(), result, unknown_values: 0 })
        .collect();
    Assessment::build(&files, None)
}

fn msp() -> Edition {
    Edition::Msp { company: "IT-Service Nord GmbH".into(), expires: date!(2099 - 12 - 31) }
}

fn context<'a>(lang: Lang, edition: &'a Edition, branding: &'a Branding) -> ReportContext<'a> {
    ReportContext { lang, edition, branding, created: datetime!(2026-09-27 09:00 UTC), version: "0.1.0" }
}

/// The shared strings of an `.xlsx` file (every text cell) and the workbook part (sheet names).
fn excel_texts(bytes: &[u8]) -> (String, String) {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).expect("xlsx is a zip file");
    let mut read = |name: &str| {
        let mut text = String::new();
        archive.by_name(name).expect(name).read_to_string(&mut text).expect("utf-8");
        text
    };
    (read("xl/sharedStrings.xml"), read("xl/workbook.xml"))
}

/// Text without whitespace – lines break anywhere in a PDF.
fn squeeze(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

fn open_placeholder(text: &str) -> Option<&str> {
    let start = text.find('{')?;
    let end = text[start..].find('}')? + start;
    let name = &text[start + 1..end];
    (!name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric())).then_some(name)
}

#[test]
fn every_language_is_complete_in_both_reports() {
    let assessment = assessment();
    let edition = msp();
    let branding = Branding { customer: Some("Muster AG".into()), logo: Some(Logo::from_bytes(PNG.to_vec()).unwrap()) };
    for lang in Lang::ALL {
        let context = context(lang, &edition, &branding);
        let (pages, bytes) = pdf::compose(&assessment, &context).unwrap();
        assert!(bytes.starts_with(b"%PDF-"));
        let text = pages.join("\n");
        let flat = squeeze(&text);
        for section in [
            "report.section.summary",
            "report.section.risks",
            "report.section.priorities",
            "report.section.recommendations",
            "report.section.effort",
            "report.section.coverage",
            "report.section.windows",
            "report.section.method",
            "report.section.powershell",
        ] {
            assert!(flat.contains(&squeeze(&t(lang, section))), "{lang}: PDF lacks {section}");
        }
        if let Some(key) = vbs_i18n::source_keys().find(|key| text.contains(key)) {
            panic!("{lang}: PDF shows the raw key {key}");
        }
        assert_eq!(open_placeholder(&text), None, "{lang}: PDF has an open placeholder");

        let (strings, workbook) = excel_texts(&xlsx::write(&assessment, &context).unwrap());
        assert!(workbook.contains(&t(lang, "report.sheet.findings").replace('&', "&amp;")), "{lang}: sheet names");
        assert!(strings.contains(&t(lang, "report.column.hint")), "{lang}: hint column");
        if let Some(key) = vbs_i18n::source_keys().find(|key| strings.contains(&format!(">{key}<"))) {
            panic!("{lang}: Excel shows the raw key {key}");
        }
        assert_eq!(open_placeholder(&strings), None, "{lang}: Excel has an open placeholder");
    }
}

#[test]
fn sheet_names_are_valid_in_every_language() {
    for lang in Lang::ALL {
        for key in ["summary", "findings", "locations", "machines", "coverage", "rules", "setAside"] {
            let name = t(lang, &format!("report.sheet.{key}"));
            assert!(!name.is_empty() && name.chars().count() <= 31, "{lang}: {name}");
            assert!(!name.contains(['[', ']', ':', '*', '?', '/', '\\']), "{lang}: {name}");
        }
    }
}

#[test]
fn the_free_edition_gets_the_finding_list_only() {
    let assessment = assessment();
    let free = Edition::free();
    let branding = Branding { customer: None, logo: Some(Logo::from_bytes(PNG.to_vec()).unwrap()) };
    let context = context(Lang::En, &free, &branding);
    assert!(matches!(pdf::write(&assessment, &context), Err(ReportError::NotLicensed)));
    let (strings, _) = excel_texts(&xlsx::write(&assessment, &context).unwrap());
    assert!(strings.contains(&t(Lang::En, "report.column.finding")));
    for missing in ["report.column.hint", "report.column.effortMin", "report.column.effortRule", "hint.rewriteScript"] {
        assert!(!strings.contains(&t(Lang::En, missing)), "free edition must not contain {missing}");
    }
    assert!(strings.contains(&t(Lang::En, "report.freeNotice").replace("{max}", "25")));
}

#[test]
fn effort_is_labelled_as_a_rule_of_thumb() {
    let assessment = assessment();
    let edition = Edition::Organization { name: "ACME GmbH".into() };
    let branding = Branding::default();
    let context = context(Lang::De, &edition, &branding);
    let (pages, _) = pdf::compose(&assessment, &context).unwrap();
    let text = pages.join("\n");
    assert!(text.contains("Aufwand (Faustwert)"));
    assert!(squeeze(&text).contains(&squeeze(&t(Lang::De, "report.effort.intro"))));
    assert!(text.contains("ACME GmbH"), "the organization's name is in the report");
    let (strings, _) = excel_texts(&xlsx::write(&assessment, &context).unwrap());
    assert!(strings.contains("Aufwand von (h, Faustwert)") && strings.contains("Aufwand bis (h, Faustwert)"));
}

#[test]
fn only_the_msp_edition_shows_a_logo() {
    let assessment = assessment();
    let branding = Branding { customer: Some("Kunde".into()), logo: Some(Logo::from_bytes(PNG.to_vec()).unwrap()) };
    let has_image = |edition: &Edition| {
        let bytes = pdf::write(&assessment, &context(Lang::En, edition, &branding)).unwrap();
        bytes.windows(b"/Subtype/Image".len()).any(|window| window == b"/Subtype/Image")
    };
    assert!(has_image(&msp()));
    assert!(!has_image(&Edition::Organization { name: "ACME".into() }));
}
