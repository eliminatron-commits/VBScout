//! Definition of Done #5 (second half): 1,000 result files are merged in under a minute.
//!
//! Run in release mode (CI does):
//! `cargo test --release -p vbs-evaluation --test merge_performance -- --ignored --nocapture`
//!
//! 1,000 synthetic machines (100 servers) with about 215 findings each – Windows' own scripts in the
//! component store, scripts deployed to many machines, scripts of their own, tasks, services, logon
//! scripts, macros – are written as result files first (not timed). Timed: reading all files,
//! the merge (newest scan per machine, de-duplication, Windows components, links, risk, effort,
//! coverage) and, separately, both reports.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use time::macros::{date, datetime};
use vbs_evaluation::assessment::Assessment;
use vbs_evaluation::edition::Edition;
use vbs_evaluation::import::{LoadedState, import};
use vbs_evaluation::report::{Branding, ReportContext, pdf, xlsx};
use vbs_i18n::Lang;

const MACHINES: usize = 1_000;
const EXTRA_FINDINGS: usize = 200;
const LIMIT: Duration = Duration::from_secs(60);

#[test]
#[ignore = "performance test – run with --release --ignored"]
fn thousand_result_files_merge_in_under_a_minute() {
    let dir = tempfile::tempdir().unwrap();
    let extension = vbs_core::file_extension();
    let first = datetime!(2026-09-25 07:00 UTC);
    let mut findings = 0;
    for index in 0..MACHINES {
        let result =
            vbs_evaluation::sample::busy_machine(index, first + time::Duration::minutes(index as i64), EXTRA_FINDINGS);
        findings += result.findings.len();
        let path: PathBuf = dir.path().join(format!("{}.{extension}", result.machine.hostname));
        std::fs::write(path, vbs_core::to_bytes(&result).unwrap()).unwrap();
    }

    let start = Instant::now();
    let batch = import(&[dir.path().to_path_buf()], &LoadedState::default(), None);
    let read = start.elapsed();
    assert!(batch.errors.is_empty(), "{:?}", batch.errors);
    assert_eq!(batch.files.len(), MACHINES);
    let assessment = Assessment::build(&batch.files, None);
    let merged = start.elapsed();

    let edition = Edition::Msp { company: "Performance test".into(), expires: date!(2099 - 12 - 31) };
    let branding = Branding::default();
    let context = ReportContext {
        lang: Lang::En,
        edition: &edition,
        branding: &branding,
        created: time::OffsetDateTime::now_utc(),
        version: env!("CARGO_PKG_VERSION"),
    };
    let reports = Instant::now();
    let excel = xlsx::write(&assessment, &context).unwrap();
    let pdf = pdf::write(&assessment, &context).unwrap();
    let reports = reports.elapsed();

    println!(
        "{MACHINES} result files, {findings} findings → {} items ({} Windows components) on {} machines",
        assessment.items.len(),
        assessment.summary.windows_items,
        assessment.machines.len()
    );
    println!("read: {read:.2?}, read + merged: {merged:.2?} (limit {LIMIT:?})");
    println!("reports: {reports:.2?} (Excel {} KiB, PDF {} KiB)", excel.len() / 1024, pdf.len() / 1024);
    assert_eq!(assessment.machines.len(), MACHINES);
    assert!(assessment.items.len() < findings, "merging must reduce the list");
    assert!(merged < LIMIT, "merging took {merged:?}");
    assert!(reports < LIMIT, "the reports took {reports:?}");
}
