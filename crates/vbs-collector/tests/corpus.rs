//! Definition of Done #4: positive and negative collections (`tests/corpus/`).
//!
//! Each collection is scanned like a machine: the file walk over the folder, and all system
//! modules over the collection's `system/` fixtures – registry (`registry.json`), event logs
//! (`events.json`), WMI (`wmi.json`) as in-memory views, task definitions, startup folders
//! and installer packages as files below `system/Windows`, `system/ProgramData` and
//! `system/Users`.
//!
//! * Every expected finding of `positive/` is reported and nothing else (recall 100 %).
//! * `negative/` yields no finding at all – not even "not checkable".
//! * Every rule a module can report has a positive case, and every finding type has a module
//!   (`PENDING_KINDS` lists types still without one – empty since phase 3).

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::Value;
use time::OffsetDateTime;
use vbs_collector::engine::{self, Plan};
use vbs_collector::modules;
use vbs_collector::platform::Host;
use vbs_collector::read_only::ReadOnlyFiles;
use vbs_core::model::{Finding, FindingKind, LocationKind, Machine, OperatingSystem};
use vbs_core::rules;
use vbs_core::views::{
    Bitness, EventRecord, Hive, MemoryEventLogs, MemoryRegistry, MemoryWmi, RegValue, SystemEnvironment, WmiObject,
    WmiValue,
};

/// Finding types without a module yet – empty since phase 3 (Office macros): every type has one.
const PENDING_KINDS: &[FindingKind] = &[];

#[derive(Debug, Deserialize)]
struct Expectations {
    cases: Vec<Case>,
}

/// One expected finding. File paths are relative to the collection with `/`; other
/// locations (registry keys, task paths, product codes, log channels) as reported.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(deny_unknown_fields)]
struct Case {
    path: String,
    #[serde(default)]
    item: Option<String>,
    rule: String,
    status: String,
    #[serde(default)]
    target: Option<String>,
}

fn collection(name: &str) -> PathBuf {
    common::corpus().join(name)
}

/// The collection's `system/` fixtures as a machine.
fn host(root: &Path) -> Host {
    let system = root.join("system");
    let root_text = system.display().to_string();
    let json = |name: &str| -> Value {
        fs::read_to_string(system.join(name)).map_or(Value::Null, |text| {
            serde_json::from_str(&text.replace("{root}", &root_text.replace('\\', "\\\\"))).unwrap()
        })
    };
    let variables: BTreeMap<String, String> = [
        ("SYSTEMROOT", system.join("Windows")),
        ("WINDIR", system.join("Windows")),
        ("PROGRAMDATA", system.join("ProgramData")),
        ("ALLUSERSPROFILE", system.join("ProgramData")),
    ]
    .into_iter()
    .map(|(name, path)| (name.to_owned(), path.display().to_string()))
    .collect();
    Host {
        machine: Machine {
            hostname: "CORPUS".into(),
            fqdn: None,
            domain: None,
            machine_id: None,
            os: OperatingSystem { family: "windows".into(), ..OperatingSystem::default() },
        },
        env: SystemEnvironment {
            elevated: true,
            system_root: Some(system.join("Windows")),
            program_data: Some(system.join("ProgramData")),
            variables,
        },
        local_roots: Vec::new(),
        registry: Box::new(registry(&json("registry.json"))),
        files: Box::new(ReadOnlyFiles),
        event_logs: Box::new(event_logs(&json("events.json"))),
        wmi: Box::new(wmi(&json("wmi.json"))),
        supported: true,
    }
}

fn registry(fixture: &Value) -> MemoryRegistry {
    let mut registry = MemoryRegistry::new();
    for key in fixture["keys"].as_array().into_iter().flatten() {
        let hive = match key["hive"].as_str().unwrap() {
            "HKLM" => Hive::LocalMachine,
            "HKU" => Hive::Users,
            "HKCU" => Hive::CurrentUser,
            other => panic!("unknown hive {other}"),
        };
        let values: Vec<(String, RegValue)> = key["values"]
            .as_object()
            .into_iter()
            .flatten()
            .map(|(name, value)| {
                let (kind, data) = value.as_object().unwrap().iter().next().unwrap();
                let value = match kind.as_str() {
                    "sz" => RegValue::Text(data.as_str().unwrap().into()),
                    "expandSz" => RegValue::ExpandText(data.as_str().unwrap().into()),
                    "multiSz" => RegValue::MultiText(
                        data.as_array().unwrap().iter().map(|v| v.as_str().unwrap().into()).collect(),
                    ),
                    "dword" => RegValue::Dword(u32::try_from(data.as_u64().unwrap()).unwrap()),
                    "qword" => RegValue::Qword(data.as_u64().unwrap()),
                    other => panic!("unknown value type {other}"),
                };
                (name.clone(), value)
            })
            .collect();
        let borrowed: Vec<(&str, RegValue)> = values.iter().map(|(n, v)| (n.as_str(), v.clone())).collect();
        let path = key["path"].as_str().unwrap();
        registry = match key["view"].as_str().unwrap_or("native") {
            "native" => registry.with_key_in(hive, Bitness::Native, path, &borrowed),
            "wow32" => registry.with_key_in(hive, Bitness::Wow32, path, &borrowed),
            "both" => registry.with_key(hive, path, &borrowed),
            other => panic!("unknown view {other}"),
        };
    }
    registry
}

fn event_logs(fixture: &Value) -> MemoryEventLogs {
    let mut logs = MemoryEventLogs::new();
    for (channel, records) in fixture["channels"].as_object().into_iter().flatten() {
        let records = records
            .as_array()
            .unwrap()
            .iter()
            .map(|record| EventRecord {
                record_id: record["recordId"].as_u64().unwrap(),
                event_id: u32::try_from(record["eventId"].as_u64().unwrap()).unwrap(),
                provider: record["provider"].as_str().unwrap().into(),
                time: record["time"]
                    .as_str()
                    .map(|t| OffsetDateTime::parse(t, &time::format_description::well_known::Rfc3339).unwrap()),
                data: record["data"]
                    .as_object()
                    .unwrap()
                    .iter()
                    .map(|(k, v)| (k.clone(), v.as_str().unwrap().into()))
                    .collect(),
            })
            .collect();
        logs = logs.with_channel(channel, records);
    }
    logs
}

fn wmi(fixture: &Value) -> MemoryWmi {
    let mut wmi = MemoryWmi::new();
    for (namespace, classes) in fixture["namespaces"].as_object().into_iter().flatten() {
        for (class, objects) in classes.as_object().unwrap() {
            let objects = objects
                .as_array()
                .unwrap()
                .iter()
                .map(|object| WmiObject {
                    path: object["path"].as_str().unwrap().into(),
                    properties: object["properties"]
                        .as_object()
                        .unwrap()
                        .iter()
                        .map(|(name, value)| {
                            let value = match value {
                                Value::String(text) => WmiValue::Text(text.clone()),
                                Value::Bool(flag) => WmiValue::Bool(*flag),
                                Value::Number(number) => WmiValue::Int(number.as_i64().unwrap()),
                                Value::Array(items) => {
                                    WmiValue::TextList(items.iter().map(|i| i.as_str().unwrap().into()).collect())
                                }
                                _ => WmiValue::Null,
                            };
                            (name.clone(), value)
                        })
                        .collect(),
                })
                .collect();
            wmi = wmi.with_instances(namespace, class, objects);
        }
    }
    wmi
}

/// Scans one collection like a machine and returns the findings.
fn scan(root: &Path) -> Vec<Finding> {
    let plan = Plan { paths: vec![root.to_path_buf()], network_paths: Vec::new(), system_sources: true, threads: 4 };
    engine::scan(&plan, &host(root), &modules::all()).findings
}

/// A path below the collection, relative and with `/`; anything else unchanged.
fn relative(root: &Path, text: &str) -> String {
    let root_text = root.display().to_string();
    match text.strip_prefix(&root_text) {
        Some(rest) => rest.trim_start_matches(['/', '\\']).replace('\\', "/"),
        None => text.to_owned(),
    }
}

fn case(root: &Path, finding: &Finding) -> Case {
    let path = if finding.location.kind == LocationKind::File {
        relative(root, &finding.location.path)
    } else {
        finding.location.path.clone()
    };
    Case {
        path,
        item: finding.location.item.clone(),
        rule: finding.rule.clone(),
        status: finding.status.as_str().to_owned(),
        target: finding.target.as_deref().map(|target| relative(root, target)),
    }
}

fn expectations() -> Expectations {
    serde_json::from_str(&fs::read_to_string(common::corpus().join("expected.json")).unwrap()).unwrap()
}

#[test]
fn negative_collection_yields_no_findings() {
    let root = collection("negative");
    let reported: Vec<Case> = scan(&root).iter().map(|f| case(&root, f)).collect();
    assert!(reported.is_empty(), "false positives: {reported:#?}");
}

#[test]
fn positive_collection_matches_the_expectations() {
    let root = collection("positive");
    let expected: BTreeSet<Case> = expectations().cases.into_iter().collect();
    let findings = scan(&root);
    let actual: BTreeSet<Case> = findings.iter().map(|f| case(&root, f)).collect();
    assert_eq!(actual.len(), findings.len(), "two findings look the same – make the cases distinguishable");
    let missed: Vec<_> = expected.difference(&actual).collect();
    let unexpected: Vec<_> = actual.difference(&expected).collect();
    assert!(missed.is_empty() && unexpected.is_empty(), "missed: {missed:#?}\nunexpected: {unexpected:#?}");
}

#[test]
fn module_rules_exist_and_have_positive_cases() {
    let catalog = rules::catalog();
    let covered: BTreeSet<String> = expectations().cases.into_iter().map(|case| case.rule).collect();
    let mut ids = BTreeSet::new();
    for module in modules::all() {
        let info = module.info();
        assert!(ids.insert(info.id), "module id {} is not unique", info.id);
        let fallback = catalog.rule(info.fallback_rule).unwrap_or_else(|| panic!("{}: unknown fallback rule", info.id));
        assert!(fallback.id.ends_with("00"), "{}: the fallback must be a VBS-x00 rule", info.id);
        for rule in info.rules {
            assert!(catalog.rule(rule).is_some(), "{}: rule {rule} is not in rules/catalog.json", info.id);
            assert!(covered.contains(*rule), "{}: rule {rule} has no positive case in tests/corpus", info.id);
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

/// Prints the positive collection's findings as `expected.json` cases (review before use):
/// `cargo test -p vbs-collector --test corpus -- --ignored --nocapture print_positive_cases`
#[test]
#[ignore = "helper for maintaining expected.json"]
fn print_positive_cases() {
    let root = collection("positive");
    let mut cases: Vec<Case> = scan(&root).iter().map(|f| case(&root, f)).collect();
    cases.sort();
    for case in cases {
        let mut entry = serde_json::json!({ "path": case.path, "rule": case.rule, "status": case.status });
        if let Some(item) = case.item {
            entry["item"] = item.into();
        }
        if let Some(target) = case.target {
            entry["target"] = target.into();
        }
        println!("    {entry},");
    }
}
