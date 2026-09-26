//! Runs a scan: the system parts of the modules, then the file walk per root
//! group, and assembles the result with its coverage and limitations.

use std::panic::{self, AssertUnwindSafe};
use std::path::PathBuf;
use std::time::Instant;

use time::OffsetDateTime;
use uuid::Uuid;
use vbs_core::model::{
    Coverage, CoverageMode, Finding, Generator, Limitation, LimitationCode, SCHEMA_VERSION, ScanResult, Scope,
    SourceCoverage, SourceStatus,
};
use vbs_core::module::{Counters, Module, Report, number_findings};
use vbs_core::rules;
use vbs_core::views::SystemView;

use crate::platform::Host;
use crate::walk;

/// Coverage source IDs of the file walk.
pub const SOURCE_LOCAL_DRIVES: &str = "files.localDrives";
pub const SOURCE_PATHS: &str = "files.paths";
pub const SOURCE_NETWORK_PATHS: &str = "files.networkPaths";

/// What to scan (from the command line).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    /// Local folders given with `--path`; empty = all fixed local drives.
    pub paths: Vec<PathBuf>,
    /// Network paths given explicitly with `--include-unc`.
    pub network_paths: Vec<PathBuf>,
    /// Registry, tasks, services, WMI and logs (`false` with `--files-only`).
    pub system_sources: bool,
    pub threads: usize,
}

impl Plan {
    /// The local file roots: the `--path` folders or all fixed drives.
    pub fn local_roots(&self, host: &Host) -> Vec<PathBuf> {
        if self.paths.is_empty() { host.local_roots.clone() } else { self.paths.clone() }
    }
}

/// Default number of walker threads: enough to overlap I/O, gentle on servers.
pub fn default_threads() -> usize {
    std::thread::available_parallelism().map_or(4, |n| n.get()).clamp(2, 8)
}

/// Scans the machine according to `plan` with `modules` and returns the complete result.
pub fn scan(plan: &Plan, host: &Host, modules: &[Box<dyn Module>]) -> ScanResult {
    let started_at = now();
    let mut findings = Vec::new();
    let mut sources = Vec::new();

    if plan.system_sources {
        let system = SystemView {
            env: &host.env,
            registry: host.registry.as_ref(),
            files: host.files.as_ref(),
            event_logs: host.event_logs.as_ref(),
            wmi: host.wmi.as_ref(),
        };
        for module in modules {
            if let Some(source) = module.info().system_source {
                sources.push(run_system_part(module.as_ref(), source, &system, host, &mut findings));
            }
        }
    }

    let file_modules: Vec<&dyn Module> =
        modules.iter().map(AsRef::as_ref).filter(|module| !module.extensions().is_empty()).collect();
    let local_source = if plan.paths.is_empty() { SOURCE_LOCAL_DRIVES } else { SOURCE_PATHS };
    sources.push(walk_source(local_source, &plan.local_roots(host), false, &file_modules, plan, &mut findings));
    if !plan.network_paths.is_empty() {
        sources.push(walk_source(SOURCE_NETWORK_PATHS, &plan.network_paths, true, &file_modules, plan, &mut findings));
    }

    number_findings(&mut findings);
    let limitations = limitations(plan, host, &sources);
    let mode = if limitations.is_empty() { CoverageMode::Full } else { CoverageMode::Limited };
    let catalog = rules::catalog();
    ScanResult {
        format: vbs_core::media_type().to_owned(),
        schema_version: SCHEMA_VERSION,
        scan_id: Uuid::new_v4(),
        generator: Generator {
            product: vbs_config::product().name.clone(),
            component: "collector".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            rules_as_of: catalog.as_of_text(),
            platform: std::env::consts::OS.into(),
        },
        started_at,
        finished_at: now().max(started_at),
        machine: host.machine.clone(),
        scope: Scope {
            local_drives: plan.paths.is_empty(),
            paths: plan.paths.iter().map(|p| p.display().to_string()).collect(),
            network_paths: plan.network_paths.iter().map(|p| p.display().to_string()).collect(),
            system_sources: plan.system_sources,
        },
        coverage: Coverage { mode, elevated: host.env.elevated, limitations, sources },
        findings,
    }
}

fn run_system_part(
    module: &dyn Module,
    source: &str,
    system: &SystemView<'_>,
    host: &Host,
    findings: &mut Vec<Finding>,
) -> SourceCoverage {
    let clock = Instant::now();
    let mut report = Report::new();
    let completed = panic::catch_unwind(AssertUnwindSafe(|| module.scan_system(system, &mut report))).is_ok();
    let declared = report.source_status().cloned();
    let (module_findings, counters) = report.into_parts();
    let (status, reason) = if !completed {
        (SourceStatus::Failed, Some("internalError".to_owned()))
    } else if let Some((status, reason)) = declared {
        (status, Some(reason))
    } else if counters.errors > 0 {
        (SourceStatus::Partial, Some("unreadableItems".to_owned()))
    } else if module.info().needs_admin && !host.env.elevated {
        (SourceStatus::Partial, Some("notElevated".to_owned()))
    } else {
        (SourceStatus::Complete, None)
    };
    if completed {
        findings.extend(module_findings);
    }
    coverage(source, status, reason, Vec::new(), counters, 0, clock)
}

fn walk_source(
    source: &str,
    roots: &[PathBuf],
    network: bool,
    modules: &[&dyn Module],
    plan: &Plan,
    findings: &mut Vec<Finding>,
) -> SourceCoverage {
    let clock = Instant::now();
    let outcome = walk::walk(roots, network, modules, plan.threads);
    findings.extend(outcome.findings);
    let (status, reason) = if roots.is_empty() {
        (SourceStatus::Failed, Some("noRoots".to_owned()))
    } else if outcome.counters.errors > 0 {
        (SourceStatus::Partial, Some("unreadableItems".to_owned()))
    } else {
        (SourceStatus::Complete, None)
    };
    let roots = roots.iter().map(|root| root.display().to_string()).collect();
    let skipped = outcome.links_skipped + outcome.cloud_dirs_skipped;
    coverage(source, status, reason, roots, outcome.counters, skipped, clock)
}

fn coverage(
    source: &str,
    status: SourceStatus,
    reason: Option<String>,
    roots: Vec<String>,
    counters: Counters,
    skipped: u64,
    clock: Instant,
) -> SourceCoverage {
    SourceCoverage {
        source: source.to_owned(),
        status,
        reason,
        roots,
        entries: counters.entries,
        inspected: counters.inspected,
        errors: counters.errors,
        skipped,
        error_samples: counters.error_samples,
        duration_ms: u64::try_from(clock.elapsed().as_millis()).unwrap_or(u64::MAX),
        time_range: None,
    }
}

/// Why this scan does not show the whole machine – recorded in every result.
fn limitations(plan: &Plan, host: &Host, sources: &[SourceCoverage]) -> Vec<Limitation> {
    let mut limitations = Vec::new();
    let mut add = |code: LimitationCode| limitations.push(Limitation { code, detail: None });
    if !host.supported {
        add(LimitationCode::UnsupportedPlatform);
    } else if !host.env.elevated {
        add(LimitationCode::NotElevated);
    }
    if !plan.system_sources {
        add(LimitationCode::SystemSourcesSkipped);
    }
    if !plan.paths.is_empty() {
        add(LimitationCode::FileScopeRestricted);
    }
    // A few unreadable folders are normal on every machine and stay visible in the
    // file sources; a failed source or an incomplete system source limits the scan.
    let incomplete = sources.iter().any(|source| {
        source.status == SourceStatus::Failed
            || (source.status == SourceStatus::Partial && !source.source.starts_with("files."))
    });
    if incomplete {
        add(LimitationCode::SourceIncomplete);
    }
    limitations
}

/// Current UTC time with millisecond precision.
fn now() -> OffsetDateTime {
    let now = OffsetDateTime::now_utc();
    now.replace_millisecond(now.millisecond()).unwrap_or(now)
}

#[cfg(test)]
mod tests {
    use super::*;
    use vbs_core::model::{Machine, OperatingSystem};
    use vbs_core::views::{SystemEnvironment, Unavailable};

    use crate::read_only::ReadOnlyFiles;

    fn host(elevated: bool, roots: Vec<PathBuf>) -> Host {
        Host {
            machine: Machine {
                hostname: "TEST-PC".into(),
                fqdn: None,
                domain: None,
                machine_id: None,
                os: OperatingSystem { family: "windows".into(), ..OperatingSystem::default() },
            },
            env: SystemEnvironment { elevated, ..SystemEnvironment::default() },
            local_roots: roots,
            registry: Box::new(Unavailable),
            files: Box::new(ReadOnlyFiles),
            event_logs: Box::new(Unavailable),
            wmi: Box::new(Unavailable),
            supported: true,
        }
    }

    #[test]
    fn empty_scan_is_complete_and_valid() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("readme.txt"), "nothing to see").unwrap();
        let plan = Plan { paths: Vec::new(), network_paths: Vec::new(), system_sources: true, threads: 2 };
        let result = scan(&plan, &host(true, vec![dir.path().to_path_buf()]), &[]);
        assert_eq!(result.coverage.mode, CoverageMode::Full);
        assert!(result.coverage.limitations.is_empty());
        assert!(result.findings.is_empty());
        assert_eq!(result.coverage.sources.len(), 1);
        let files = &result.coverage.sources[0];
        assert_eq!(
            (files.source.as_str(), &files.status, files.entries),
            (SOURCE_LOCAL_DRIVES, &SourceStatus::Complete, 1)
        );
        let bytes = vbs_core::to_bytes(&result).unwrap();
        let loaded = vbs_core::read(std::io::Cursor::new(bytes)).unwrap();
        assert_eq!(loaded.result, result);
    }

    #[test]
    fn restricted_and_unelevated_scans_are_limited() {
        let dir = tempfile::tempdir().unwrap();
        let plan = Plan {
            paths: vec![dir.path().to_path_buf()],
            network_paths: Vec::new(),
            system_sources: false,
            threads: 1,
        };
        let result = scan(&plan, &host(false, Vec::new()), &[]);
        assert_eq!(result.coverage.mode, CoverageMode::Limited);
        let codes: Vec<_> = result.coverage.limitations.iter().map(|l| l.code.clone()).collect();
        assert_eq!(
            codes,
            [LimitationCode::NotElevated, LimitationCode::SystemSourcesSkipped, LimitationCode::FileScopeRestricted]
        );
        assert_eq!(result.coverage.sources[0].source, SOURCE_PATHS);
        assert!(!result.scope.local_drives && !result.scope.system_sources);
    }
}
