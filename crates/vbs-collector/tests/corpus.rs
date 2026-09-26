//! Definition of Done #4: positive and negative collections (`tests/corpus/`).
//!
//! * Every expected finding of `positive/` is reported and nothing else (recall 100 %).
//! * `negative/` yields no finding at all.
//! * Every rule a module can report has a positive case, and every finding type has a module –
//!   types still without one are listed in `PENDING_KINDS`, which must be empty after phase 3.

mod common;

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use vbs_collector::engine::{self, Plan};
use vbs_collector::modules;
use vbs_collector::platform::Host;
use vbs_collector::read_only::ReadOnlyFiles;
use vbs_core::model::{Finding, FindingKind, Machine, OperatingSystem};
use vbs_core::rules;
use vbs_core::views::{SystemEnvironment, Unavailable};

/// Finding types without a module yet. Phase 2 removes the system-level types, phase 3 the
/// Office macros; the list must be empty when the collector is complete.
const PENDING_KINDS: &[FindingKind] = &[
    FindingKind::ScriptFile,
    FindingKind::ScriptInvocation,
    FindingKind::Shortcut,
    FindingKind::ScheduledTask,
    FindingKind::Autostart,
    FindingKind::Service,
    FindingKind::WmiSubscription,
    FindingKind::LogonScript,
    FindingKind::MsiCustomAction,
    FindingKind::EventLogUsage,
    FindingKind::OfficeMacro,
    FindingKind::HardcodedCredential,
];

#[derive(Debug, Deserialize)]
struct Expectations {
    cases: Vec<Case>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
struct Case {
    /// Relative to `positive/`, with `/` separators.
    path: String,
    rule: String,
    status: String,
}

fn test_host() -> Host {
    Host {
        machine: Machine {
            hostname: "CORPUS".into(),
            fqdn: None,
            domain: None,
            machine_id: None,
            os: OperatingSystem { family: "windows".into(), ..OperatingSystem::default() },
        },
        env: SystemEnvironment { elevated: true, ..SystemEnvironment::default() },
        local_roots: Vec::new(),
        registry: Box::new(Unavailable),
        files: Box::new(ReadOnlyFiles),
        event_logs: Box::new(Unavailable),
        wmi: Box::new(Unavailable),
        supported: true,
    }
}

/// Scans one folder with all modules (files only) and returns the findings.
fn scan(root: &Path) -> Vec<Finding> {
    let plan = Plan { paths: vec![root.to_path_buf()], network_paths: Vec::new(), system_sources: false, threads: 4 };
    engine::scan(&plan, &test_host(), &modules::all()).findings
}

fn relative(root: &Path, finding: &Finding) -> String {
    let path = PathBuf::from(&finding.location.path);
    path.strip_prefix(root).unwrap_or(&path).to_string_lossy().replace('\\', "/")
}

#[test]
fn negative_collection_yields_no_findings() {
    let root = common::corpus().join("negative");
    let findings = scan(&root);
    let reported: Vec<String> =
        findings.iter().map(|f| format!("{} {} {}", relative(&root, f), f.rule, f.status)).collect();
    assert!(reported.is_empty(), "false positives: {reported:#?}");
}

#[test]
fn positive_collection_matches_the_expectations() {
    let root = common::corpus().join("positive");
    let expectations: Expectations =
        serde_json::from_str(&fs::read_to_string(common::corpus().join("expected.json")).unwrap()).unwrap();
    let expected: BTreeSet<Case> = expectations.cases.into_iter().collect();
    let actual: BTreeSet<Case> = scan(&root)
        .iter()
        .map(|finding| Case {
            path: relative(&root, finding),
            rule: finding.rule.clone(),
            status: finding.status.as_str().to_owned(),
        })
        .collect();
    let missed: Vec<_> = expected.difference(&actual).collect();
    let unexpected: Vec<_> = actual.difference(&expected).collect();
    assert!(missed.is_empty() && unexpected.is_empty(), "missed: {missed:#?}\nunexpected: {unexpected:#?}");
}

#[test]
fn module_rules_exist_and_have_positive_cases() {
    let catalog = rules::catalog();
    let expectations: Expectations =
        serde_json::from_str(&fs::read_to_string(common::corpus().join("expected.json")).unwrap()).unwrap();
    let covered: BTreeSet<&str> = expectations.cases.iter().map(|case| case.rule.as_str()).collect();
    let mut ids = BTreeSet::new();
    for module in modules::all() {
        let info = module.info();
        assert!(ids.insert(info.id), "module id {} is not unique", info.id);
        let fallback = catalog.rule(info.fallback_rule).unwrap_or_else(|| panic!("{}: unknown fallback rule", info.id));
        assert!(fallback.id.ends_with("00"), "{}: the fallback must be a VBS-x00 rule", info.id);
        for rule in info.rules {
            assert!(catalog.rule(rule).is_some(), "{}: rule {rule} is not in rules/catalog.json", info.id);
            assert!(covered.contains(rule), "{}: rule {rule} has no positive case in tests/corpus", info.id);
        }
    }
}

#[test]
fn every_finding_kind_has_a_module_or_is_pending() {
    let catalog = rules::catalog();
    let implemented: BTreeSet<FindingKind> = modules::all()
        .iter()
        .flat_map(|module| module.info().rules.iter())
        .filter_map(|rule| catalog.rule(rule).map(|entry| entry.kind.clone()))
        .collect();
    let pending: BTreeSet<FindingKind> = PENDING_KINDS.iter().cloned().collect();
    let still_listed: Vec<_> = implemented.intersection(&pending).collect();
    assert!(still_listed.is_empty(), "implemented now – remove from PENDING_KINDS: {still_listed:?}");
    let missing: Vec<_> =
        FindingKind::KNOWN.iter().filter(|kind| !implemented.contains(kind) && !pending.contains(kind)).collect();
    assert!(missing.is_empty(), "finding kinds without a module: {missing:?}");
}
