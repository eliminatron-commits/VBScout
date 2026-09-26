//! Permanent WMI event subscriptions that run VBScript: `ActiveScriptEventConsumer` with the
//! VBScript engine and `CommandLineEventConsumer` starting VBScript. Consumers bound to a
//! filter (`__FilterToConsumerBinding`) run automatically.

use vbs_core::model::{Activation, Detail, FindingKind, Location, LocationKind, SourceStatus};
use vbs_core::module::{CREDENTIAL_RULE, Module, ModuleInfo, Report};
use vbs_core::views::{SystemView, ViewError, WmiObject, WmiValue};

use super::{Rules, check_command, report_command};
use crate::analysis::script::{self, Language};
use crate::analysis::text;

pub struct WmiSubscriptions;

static INFO: ModuleInfo = ModuleInfo {
    id: "wmi-subscription",
    system_source: Some("wmi.subscriptions"),
    rules: &["VBS-331", "VBS-332", CREDENTIAL_RULE],
    fallback_rule: "VBS-300",
    needs_admin: true,
};

const RULES: Rules =
    Rules { breaks: "VBS-331", review: "VBS-332", not_checkable: "VBS-300", kind: FindingKind::WmiSubscription };

/// Namespaces that hold permanent consumers (the classes are registered in `root\subscription`).
const NAMESPACES: [&str; 2] = [r"ROOT\subscription", r"ROOT\default"];

impl Module for WmiSubscriptions {
    fn info(&self) -> &'static ModuleInfo {
        &INFO
    }

    fn scan_system(&self, system: &SystemView<'_>, report: &mut Report) {
        let mut reachable = false;
        for namespace in NAMESPACES {
            let instances = |class: &str| system.wmi.instances(namespace, class);
            let bindings = match instances("__FilterToConsumerBinding") {
                Ok(bindings) => bindings,
                Err(ViewError::NotFound) => continue,
                Err(error) => {
                    report.record_error(format!("{namespace} ({error})"));
                    continue;
                }
            };
            reachable = true;
            report.add_root(namespace);
            let filters = instances("__EventFilter").unwrap_or_default();
            let bound = |consumer: &WmiObject| -> Option<String> {
                let own = relative(&consumer.path);
                bindings.iter().find_map(|binding| {
                    let target = text_of(binding, "Consumer")?;
                    if !relative(&target).eq_ignore_ascii_case(&own) {
                        return None;
                    }
                    let filter = relative(&text_of(binding, "Filter").unwrap_or_default());
                    let query = filters
                        .iter()
                        .find(|f| relative(&f.path).eq_ignore_ascii_case(&filter))
                        .and_then(|f| text_of(f, "Query"));
                    Some(query.unwrap_or_default())
                })
            };
            for consumer in instances("ActiveScriptEventConsumer").unwrap_or_default() {
                report.count_entries(1);
                let engine = text_of(&consumer, "ScriptingEngine").unwrap_or_default();
                if !engine.to_ascii_lowercase().contains("vbscript") {
                    continue;
                }
                report.count_inspected(1);
                let query = bound(&consumer);
                let activation = if query.is_some() { Activation::Automatic } else { Activation::Dormant };
                let location = Location {
                    kind: LocationKind::Wmi,
                    path: format!("{namespace}:{}", relative(&consumer.path)),
                    item: None,
                };
                let script_text = text_of(&consumer, "ScriptText").unwrap_or_default();
                let lines = script::code_lines(&script_text, Language::VbScript);
                let mut builder = report
                    .finding("VBS-331", location.clone())
                    .activation(activation.clone())
                    .detail("consumerClass", "ActiveScriptEventConsumer")
                    .detail("scriptingEngine", engine.clone())
                    .detail("bound", query.is_some());
                if let Some(query) = query.filter(|q| !q.is_empty()) {
                    builder = builder.detail("eventQuery", query);
                }
                if let Some(file) = text_of(&consumer, "ScriptFileName").filter(|f| !f.trim().is_empty()) {
                    builder = builder.target(file.clone()).evidence(None, &format!("ScriptFileName = {file}"));
                }
                for (number, line) in script::vbscript_evidence(&lines) {
                    builder = builder.evidence(Some(number), line);
                }
                builder.emit();
                report.credentials(location, None, activation, lines.iter().map(|(n, l)| (Some(*n), *l)));
            }
            for consumer in instances("CommandLineEventConsumer").unwrap_or_default() {
                report.count_entries(1);
                let program = text_of(&consumer, "ExecutablePath").unwrap_or_default();
                let template = text_of(&consumer, "CommandLineTemplate").unwrap_or_default();
                let command = match (program.trim().is_empty(), template.trim().is_empty()) {
                    (false, false)
                        if !template.to_ascii_lowercase().contains(&text::file_name(&program).to_ascii_lowercase()) =>
                    {
                        format!("\"{}\" {template}", program.trim_matches('"'))
                    }
                    (_, false) => template.clone(),
                    _ => program.clone(),
                };
                let check = check_command(system, &command, text_of(&consumer, "WorkingDirectory").as_deref());
                if check.usage().is_none() && check.unreadable.is_none() {
                    continue;
                }
                report.count_inspected(1);
                let query = bound(&consumer);
                let activation = if query.is_some() { Activation::Automatic } else { Activation::Dormant };
                let location = Location {
                    kind: LocationKind::Wmi,
                    path: format!("{namespace}:{}", relative(&consumer.path)),
                    item: None,
                };
                let mut details = vec![
                    ("consumerClass", Detail::Text("CommandLineEventConsumer".into())),
                    ("bound", Detail::Flag(query.is_some())),
                ];
                if let Some(query) = query.filter(|q| !q.is_empty()) {
                    details.push(("eventQuery", Detail::Text(query)));
                }
                report_command(report, RULES, location, activation, &command, &check, &details);
            }
        }
        if !reachable {
            report.set_source_status(SourceStatus::Failed, "unavailable");
        }
    }
}

fn text_of(object: &WmiObject, name: &str) -> Option<String> {
    object.properties.iter().find(|(key, _)| key.eq_ignore_ascii_case(name)).and_then(|(_, value)| match value {
        WmiValue::Text(text) => Some(text.clone()),
        WmiValue::TextList(items) => Some(items.join("\n")),
        _ => None,
    })
}

/// `\\HOST\ROOT\subscription:Class.Name="x"` → `Class.Name="x"`.
fn relative(path: &str) -> String {
    let path = path.trim();
    match path.find(':') {
        Some(colon) if path.starts_with("\\\\") || path[..colon].contains('\\') => path[colon + 1..].to_owned(),
        _ => path.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::testing::scan_system;
    use std::collections::BTreeMap;
    use vbs_core::views::{MemoryEventLogs, MemoryRegistry, MemoryWmi, SystemEnvironment};

    fn object(path: &str, properties: &[(&str, &str)]) -> WmiObject {
        WmiObject {
            path: path.into(),
            properties: properties
                .iter()
                .map(|(k, v)| ((*k).to_owned(), WmiValue::Text((*v).to_owned())))
                .collect::<BTreeMap<_, _>>(),
        }
    }

    #[test]
    fn script_and_command_line_consumers() {
        let ns = r"ROOT\subscription";
        let wmi = MemoryWmi::new()
            .with_instances(
                ns,
                "__FilterToConsumerBinding",
                vec![object(
                    "__FilterToConsumerBinding.Consumer=\"x\"",
                    &[
                        ("Consumer", r#"\\PC\ROOT\subscription:ActiveScriptEventConsumer.Name="Cleanup""#),
                        ("Filter", r#"__EventFilter.Name="Every5Min""#),
                    ],
                )],
            )
            .with_instances(
                ns,
                "__EventFilter",
                vec![object(
                    r#"__EventFilter.Name="Every5Min""#,
                    &[("Query", "SELECT * FROM __InstanceModificationEvent WITHIN 300")],
                )],
            )
            .with_instances(
                ns,
                "ActiveScriptEventConsumer",
                vec![
                    object(
                        r#"ActiveScriptEventConsumer.Name="Cleanup""#,
                        &[
                            ("ScriptingEngine", "VBScript"),
                            (
                                "ScriptText",
                                "Set fso = CreateObject(\"Scripting.FileSystemObject\")\r\npwd = \"hunter2\"",
                            ),
                        ],
                    ),
                    object(
                        r#"ActiveScriptEventConsumer.Name="Js""#,
                        &[("ScriptingEngine", "JScript"), ("ScriptText", "var x = 1;")],
                    ),
                ],
            )
            .with_instances(
                ns,
                "CommandLineEventConsumer",
                vec![
                    object(
                        r#"CommandLineEventConsumer.Name="Notify""#,
                        &[("CommandLineTemplate", r"cscript.exe //nologo C:\Ops\notify.vbs")],
                    ),
                    object(
                        r#"CommandLineEventConsumer.Name="Ping""#,
                        &[
                            ("ExecutablePath", r"C:\Windows\System32\ping.exe"),
                            ("CommandLineTemplate", "ping localhost"),
                        ],
                    ),
                ],
            );
        let report = scan_system(
            &WmiSubscriptions,
            &SystemEnvironment::default(),
            &MemoryRegistry::new(),
            &MemoryEventLogs::new(),
            &wmi,
        );
        let summary: Vec<(&str, &str, &Activation)> =
            report.findings().iter().map(|f| (f.location.path.as_str(), f.rule.as_str(), &f.activation)).collect();
        assert_eq!(
            summary,
            [
                (r#"ROOT\subscription:ActiveScriptEventConsumer.Name="Cleanup""#, "VBS-331", &Activation::Automatic),
                (
                    r#"ROOT\subscription:ActiveScriptEventConsumer.Name="Cleanup""#,
                    CREDENTIAL_RULE,
                    &Activation::Automatic
                ),
                (r#"ROOT\subscription:CommandLineEventConsumer.Name="Notify""#, "VBS-331", &Activation::Dormant),
            ]
        );
        assert_eq!(
            report.findings()[0].details["eventQuery"],
            Detail::Text("SELECT * FROM __InstanceModificationEvent WITHIN 300".into())
        );
    }
}
