//! `--smoke-test[=<seconds>]`: starts the app normally, waits until the
//! frontend reports ready, runs an offline self-check (a result file is
//! written to a temporary folder, imported through the same path as the UI
//! uses and checked; the assessment is built and both reports are rendered in
//! memory in every language; all catalogs are exercised) and exits with code 0.
//!
//! CI uses it to prove that the app starts, and as the workload of the network
//! block test (`scripts/nettest/`). Exit codes: 2 = frontend not ready in time,
//! 3 = self-check failed.

use std::time::Duration;

use tauri::AppHandle;
use time::OffsetDateTime;
use uuid::Uuid;
use vbs_core::model::{
    Activation, Coverage, CoverageMode, Evidence, FileFacts, Finding, FindingKind, FindingStatus, Generator, Location,
    LocationKind, Machine, OperatingSystem, SCHEMA_VERSION, ScanResult, Scope,
};
use vbs_core::rules;
use vbs_i18n::{Lang, t, t_count};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SmokeTest {
    enabled: bool,
    timeout: Duration,
}

impl SmokeTest {
    pub fn from_args(args: impl Iterator<Item = String>) -> Self {
        let mut test = Self { enabled: false, timeout: Duration::from_secs(60) };
        for arg in args {
            if arg == "--smoke-test" {
                test.enabled = true;
            } else if let Some(seconds) = arg.strip_prefix("--smoke-test=") {
                test.enabled = true;
                if let Ok(seconds) = seconds.parse() {
                    test.timeout = Duration::from_secs(seconds);
                }
            }
        }
        test
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// Fails the smoke test if the frontend never reports ready.
    pub fn arm_watchdog(&self) {
        if !self.enabled {
            return;
        }
        let timeout = self.timeout;
        std::thread::spawn(move || {
            std::thread::sleep(timeout);
            eprintln!("SMOKE FAIL: frontend not ready after {} s", timeout.as_secs());
            std::process::exit(2);
        });
    }
}

pub fn run(app: &AppHandle) {
    match self_check() {
        Ok(findings) => {
            eprintln!("SMOKE OK: result file written, imported and checked ({findings} findings)");
            app.exit(0);
        }
        Err(problem) => {
            eprintln!("SMOKE FAIL: {problem}");
            app.exit(3);
        }
    }
}

/// A small but complete result, as the collector would write it.
fn sample() -> ScanResult {
    let now = OffsetDateTime::now_utc();
    let finding = Finding {
        id: "f1".into(),
        rule: "VBS-101".into(),
        kind: FindingKind::ScriptFile,
        classification: rules::catalog()
            .rule("VBS-101")
            .map(|r| r.classification.clone())
            .unwrap_or_else(|| vbs_core::model::Classification::Review),
        status: FindingStatus::Detected,
        reason: None,
        activation: Activation::Dormant,
        location: Location { kind: LocationKind::File, path: r"C:\Scripts\backup.vbs".into(), item: None },
        target: None,
        file: Some(FileFacts { size: 42, modified_at: Some(now), sha256: None, network: false }),
        evidence: vec![Evidence { line: Some(3), text: r#"strPwd = "not stored""#.into(), masked: false }],
        details: Default::default(),
    };
    ScanResult {
        format: vbs_core::media_type().into(),
        schema_version: SCHEMA_VERSION,
        scan_id: Uuid::new_v4(),
        generator: Generator {
            product: vbs_config::product().name.clone(),
            component: "collector".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            rules_as_of: rules::catalog().as_of_text(),
            platform: "windows".into(),
        },
        started_at: now,
        finished_at: now,
        machine: Machine {
            hostname: "SMOKE-PC".into(),
            fqdn: None,
            domain: None,
            machine_id: None,
            os: OperatingSystem { family: "windows".into(), ..OperatingSystem::default() },
        },
        scope: Scope { local_drives: true, paths: Vec::new(), network_paths: Vec::new(), system_sources: true },
        coverage: Coverage { mode: CoverageMode::Full, elevated: true, limitations: Vec::new(), sources: Vec::new() },
        findings: vec![finding],
    }
}

fn self_check() -> Result<usize, String> {
    let dir = std::env::temp_dir().join(format!("vbs-smoke-{}", Uuid::new_v4().simple()));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(format!("smoke.{}", vbs_core::file_extension()));
    let written = vbs_core::to_bytes(&sample()).and_then(|bytes| Ok(std::fs::write(&path, bytes)?));
    let batch = written.map(|()| {
        vbs_evaluation::import::import(
            std::slice::from_ref(&dir),
            &vbs_evaluation::import::LoadedState::default(),
            None,
        )
    });
    let _ = std::fs::remove_dir_all(&dir);
    let batch = batch.map_err(|e| e.to_string())?;

    let [file] = batch.files.as_slice() else {
        return Err(format!("expected one imported file, got {} (errors: {:?})", batch.files.len(), batch.errors));
    };
    let result = &file.result;
    if result.machine.hostname != "SMOKE-PC" || result.findings.len() != 1 {
        return Err("imported result differs from the written one".into());
    }
    if result.findings[0].evidence.iter().any(|e| e.text.contains("not stored") || !e.masked) {
        return Err("a secret in the evidence was not masked when writing".into());
    }
    for lang in Lang::ALL {
        let texts = [t(lang, "app.tagline"), t_count(lang, "machine.count", 3, &[]), t(lang, "rule.vbs101.title")];
        if texts.iter().any(|text| text.trim().is_empty() || text.contains("app.tagline")) {
            return Err(format!("catalog {lang} is incomplete"));
        }
    }
    reports(&batch.files)?;
    Ok(result.findings.len())
}

/// Builds the assessment and renders both reports in memory, in every language: the Excel list
/// as the free edition gets it, the PDF as a licensed edition would (the free edition is refused).
fn reports(files: &[vbs_evaluation::import::ImportedFile]) -> Result<(), String> {
    use vbs_evaluation::assessment::Assessment;
    use vbs_evaluation::edition::Edition;
    use vbs_evaluation::report::{Branding, ReportContext, ReportError, pdf, xlsx};

    let assessment = Assessment::build(files, None);
    if assessment.items.len() != 1 {
        return Err(format!("expected one item in the assessment, got {}", assessment.items.len()));
    }
    let free = Edition::free();
    let licensed = Edition::Organization { name: "Smoke test".into() };
    let branding = Branding::default();
    for lang in Lang::ALL {
        let context = |edition| ReportContext {
            lang,
            edition,
            branding: &branding,
            created: OffsetDateTime::now_utc(),
            version: env!("CARGO_PKG_VERSION"),
        };
        let excel = xlsx::write(&assessment, &context(&free)).map_err(|e| e.to_string())?;
        let pdf = pdf::write(&assessment, &context(&licensed)).map_err(|e| e.to_string())?;
        if !excel.starts_with(b"PK") || !pdf.starts_with(b"%PDF-") {
            return Err(format!("{lang}: a report is not an .xlsx or PDF file"));
        }
        if !matches!(pdf::write(&assessment, &context(&free)), Err(ReportError::NotLicensed)) {
            return Err("the free edition must not create the PDF report".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> SmokeTest {
        SmokeTest::from_args(args.iter().map(|arg| arg.to_string()))
    }

    #[test]
    fn parses_the_smoke_test_flag() {
        assert!(!parse(&[]).enabled());
        assert!(!parse(&["--smoke"]).enabled());
        assert_eq!(parse(&["--smoke-test"]), SmokeTest { enabled: true, timeout: Duration::from_secs(60) });
        assert_eq!(parse(&["--smoke-test=90"]), SmokeTest { enabled: true, timeout: Duration::from_secs(90) });
        assert_eq!(parse(&["--smoke-test=x"]).timeout, Duration::from_secs(60));
    }

    #[test]
    fn self_check_passes() {
        assert_eq!(self_check(), Ok(1));
    }
}
