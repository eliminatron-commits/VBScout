//! Writes the reports for result files or for the synthetic sample organization – for development,
//! review and CI artifacts (the app is where users create reports):
//!
//! ```text
//! cargo run -p vbs-evaluation --example report -- --out <folder> [--lang de] [--sample 30]
//!     [--edition free|organization:<name>|msp:<company>] [--customer <name>] [--logo <png|jpeg>]
//!     [result files or folders …]
//! ```
//!
//! Writes `report-<lang>.pdf` (licensed editions only) and `findings-<lang>.xlsx` into `--out`;
//! `--list` prints the items of the organization (not the Windows components) in priority order.

use std::path::PathBuf;

use time::OffsetDateTime;
use time::macros::{date, datetime};
use vbs_evaluation::assessment::Assessment;
use vbs_evaluation::edition::Edition;
use vbs_evaluation::import::{ImportedFile, LoadedState, import};
use vbs_evaluation::report::{Branding, Logo, ReportContext, pdf, xlsx};
use vbs_i18n::Lang;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let mut out: Option<PathBuf> = None;
    let mut lang = Lang::En;
    let mut edition = Edition::Organization { name: "Example Organization".into() };
    let mut branding = Branding::default();
    let mut sample = 0usize;
    let mut list = false;
    let mut inputs: Vec<PathBuf> = Vec::new();
    while let Some(arg) = args.next() {
        let mut value = || args.next().ok_or_else(|| format!("{arg} needs a value"));
        match arg.as_str() {
            "--out" => out = Some(value()?.into()),
            "--lang" => lang = Lang::from_tag(&value()?).ok_or("unknown language")?,
            "--sample" => sample = value()?.parse()?,
            "--customer" => branding.customer = Some(value()?),
            "--list" => list = true,
            "--logo" => branding.logo = Some(Logo::from_bytes(std::fs::read(value()?)?)?),
            "--edition" => {
                let value = value()?;
                edition = match value.split_once(':') {
                    Some(("organization", name)) => Edition::Organization { name: name.into() },
                    Some(("msp", company)) => Edition::Msp { company: company.into(), expires: date!(2099 - 12 - 31) },
                    _ if value == "free" => Edition::free(),
                    _ => return Err(format!("unknown edition {value}").into()),
                };
            }
            _ => inputs.push(arg.into()),
        }
    }
    let out = out.ok_or("--out <folder> is required")?;
    std::fs::create_dir_all(&out)?;

    let mut files: Vec<ImportedFile> = vbs_evaluation::sample::organization(sample, datetime!(2026-09-25 07:30 UTC))
        .into_iter()
        .enumerate()
        .map(|(index, result)| ImportedFile {
            path: format!("sample-{index}.vbscout").into(),
            result,
            unknown_values: 0,
        })
        .collect();
    if !inputs.is_empty() {
        let batch = import(&inputs, &LoadedState::of(&files), edition.machine_limit());
        for error in &batch.errors {
            eprintln!("not read: {} ({})", error.path.display(), error.problem.code());
        }
        files.extend(batch.files);
    }
    let assessment = Assessment::build(&files, edition.machine_limit());
    let context = ReportContext {
        lang,
        edition: &edition,
        branding: &branding,
        created: OffsetDateTime::now_utc(),
        version: env!("CARGO_PKG_VERSION"),
    };
    println!(
        "{} machines: {} items of the organization (high {}, medium {}, low {}, not checkable {}), {} Windows components ({} locations)",
        assessment.machines.len(),
        assessment.summary.items,
        assessment.summary.high,
        assessment.summary.medium,
        assessment.summary.low,
        assessment.summary.not_checkable,
        assessment.summary.windows_items,
        assessment.summary.windows_occurrences,
    );
    if list {
        for item in assessment.own_items() {
            let first = item.first();
            println!(
                "#{} {} {} {} ×{} {}",
                item.number,
                item.risk.as_str(),
                item.rule,
                item.activation,
                item.machines,
                first.location.path
            );
        }
    }
    let tag = lang.tag();
    let excel = out.join(format!("findings-{tag}.xlsx"));
    std::fs::write(&excel, xlsx::write(&assessment, &context)?)?;
    println!("{} ({} items, {} machines)", excel.display(), assessment.items.len(), assessment.machines.len());
    if edition.allows_pdf() {
        let path = out.join(format!("report-{tag}.pdf"));
        std::fs::write(&path, pdf::write(&assessment, &context)?)?;
        println!("{}", path.display());
    }
    Ok(())
}
