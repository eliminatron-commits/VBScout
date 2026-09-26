//! Windows services that run VBScript: the image path, the failure command and the
//! application of service wrappers such as srvany or NSSM (`Parameters\Application`).

use vbs_core::model::{Activation, Detail, FindingKind, Location, LocationKind, SourceStatus};
use vbs_core::module::{CREDENTIAL_RULE, Module, ModuleInfo, Report};
use vbs_core::views::{Bitness, Hive, RegValue, SystemView, ViewError};

use super::{Rules, check_command, report_command};

pub struct Services;

static INFO: ModuleInfo = ModuleInfo {
    id: "service",
    system_source: Some("services.configuration"),
    rules: &["VBS-321", "VBS-322", CREDENTIAL_RULE],
    fallback_rule: "VBS-300",
    needs_admin: false,
};

const RULES: Rules =
    Rules { breaks: "VBS-321", review: "VBS-322", not_checkable: "VBS-300", kind: FindingKind::Service };
const SERVICES: &str = r"SYSTEM\CurrentControlSet\Services";
/// Service types that run a program (own or shared process, user services); drivers are left out.
const WIN32_SERVICE: u32 = 0x10 | 0x20 | 0x40;

impl Module for Services {
    fn info(&self) -> &'static ModuleInfo {
        &INFO
    }

    fn scan_system(&self, system: &SystemView<'_>, report: &mut Report) {
        let names = match system.registry.subkeys(Hive::LocalMachine, SERVICES, Bitness::Native) {
            Ok(names) => names,
            Err(error) => {
                let status =
                    if error == ViewError::NotFound { SourceStatus::Unavailable } else { SourceStatus::Failed };
                report.set_source_status(status, error.reason_code());
                return;
            }
        };
        report.add_root(format!(r"HKLM\{SERVICES}"));
        for name in names {
            report.count_entries(1);
            let key = format!(r"{SERVICES}\{name}");
            let values = match system.registry.values(Hive::LocalMachine, &key, Bitness::Native) {
                Ok(values) => values,
                Err(error) => {
                    report.record_error(format!(r"HKLM\{key} ({error})"));
                    continue;
                }
            };
            let get = |wanted: &str| values.iter().find(|(n, _)| n.eq_ignore_ascii_case(wanted)).map(|(_, v)| v);
            let service_type = get("Type").and_then(RegValue::as_u32).unwrap_or(0);
            if service_type & WIN32_SERVICE == 0 {
                continue;
            }
            report.count_inspected(1);
            let start = get("Start").and_then(RegValue::as_u32);
            let (activation, start_type) = match start {
                Some(0..=2) => (Activation::Automatic, "automatic"),
                Some(3) => (Activation::Manual, "manual"),
                Some(4) => (Activation::Dormant, "disabled"),
                _ => (Activation::Manual, "unknown"),
            };
            let mut details = vec![("startType", Detail::Text(start_type.into()))];
            if let Some(display) = get("DisplayName").and_then(RegValue::as_text).filter(|d| !d.starts_with('@')) {
                details.push(("displayName", Detail::Text(display.to_owned())));
            }
            let mut commands: Vec<(String, String, Option<String>)> = Vec::new();
            for value in ["ImagePath", "FailureCommand"] {
                if let Some(command) = get(value).and_then(RegValue::as_text).map(normalize_image_path) {
                    commands.push((value.to_owned(), command, None));
                }
            }
            let parameters = format!(r"{key}\Parameters");
            if let Ok(wrapper) = system.registry.values(Hive::LocalMachine, &parameters, Bitness::Native) {
                let value = |wanted: &str| {
                    wrapper
                        .iter()
                        .find(|(n, _)| n.eq_ignore_ascii_case(wanted))
                        .and_then(|(_, v)| v.as_text())
                        .map(str::to_owned)
                };
                if let Some(application) = value("Application").filter(|a| !a.trim().is_empty()) {
                    let arguments = value("AppParameters").unwrap_or_default();
                    let program = application.trim().trim_matches('"');
                    commands.push((
                        r"Parameters\Application".into(),
                        format!("\"{program}\" {arguments}"),
                        value("AppDirectory"),
                    ));
                }
            }
            for (item, command, working_dir) in commands {
                let check = check_command(system, &command, working_dir.as_deref());
                let location = Location { kind: LocationKind::Service, path: name.clone(), item: Some(item) };
                report_command(report, RULES, location, activation.clone(), command.trim(), &check, &details);
            }
        }
    }
}

/// Image paths use kernel forms: `\SystemRoot\…` and `\??\C:\…`.
fn normalize_image_path(path: &str) -> String {
    let trimmed = path.trim();
    if let Some(rest) = trimmed.strip_prefix(r"\??\") {
        return rest.to_owned();
    }
    if trimmed.len() > 12 && trimmed[..12].eq_ignore_ascii_case(r"\SystemRoot\") {
        return format!(r"%SystemRoot%\{}", &trimmed[12..]);
    }
    trimmed.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::testing::scan_system;
    use vbs_core::views::{MemoryEventLogs, MemoryRegistry, MemoryWmi, SystemEnvironment};

    #[test]
    fn services_and_wrappers() {
        let registry = MemoryRegistry::new()
            .with_key(
                Hive::LocalMachine,
                &format!(r"{SERVICES}\LegacyMonitor"),
                &[
                    ("Type", RegValue::Dword(0x10)),
                    ("Start", RegValue::Dword(2)),
                    ("ImagePath", RegValue::ExpandText(r"C:\Tools\srvany.exe".into())),
                ],
            )
            .with_key(
                Hive::LocalMachine,
                &format!(r"{SERVICES}\LegacyMonitor\Parameters"),
                &[
                    ("Application", RegValue::Text(r"C:\Windows\System32\cscript.exe".into())),
                    ("AppParameters", RegValue::Text(r"//B C:\Monitor\watch.vbs".into())),
                ],
            )
            .with_key(
                Hive::LocalMachine,
                &format!(r"{SERVICES}\Spooler"),
                &[
                    ("Type", RegValue::Dword(0x110)),
                    ("Start", RegValue::Dword(2)),
                    ("ImagePath", RegValue::ExpandText(r"%SystemRoot%\System32\spoolsv.exe".into())),
                    ("FailureCommand", RegValue::Text(r"wscript C:\Ops\restart-alert.vbs".into())),
                ],
            )
            .with_key(
                Hive::LocalMachine,
                &format!(r"{SERVICES}\SomeDriver"),
                &[
                    ("Type", RegValue::Dword(1)),
                    ("ImagePath", RegValue::ExpandText(r"\SystemRoot\System32\drivers\x.vbs".into())),
                ],
            );
        let report = scan_system(
            &Services,
            &SystemEnvironment::default(),
            &registry,
            &MemoryEventLogs::new(),
            &MemoryWmi::new(),
        );
        let summary: Vec<(&str, Option<&str>, &str, &Activation)> = report
            .findings()
            .iter()
            .map(|f| (f.location.path.as_str(), f.location.item.as_deref(), f.rule.as_str(), &f.activation))
            .collect();
        assert_eq!(
            summary,
            [
                ("LegacyMonitor", Some(r"Parameters\Application"), "VBS-321", &Activation::Automatic),
                ("Spooler", Some("FailureCommand"), "VBS-321", &Activation::Automatic),
            ]
        );
        assert_eq!(normalize_image_path(r"\??\C:\x\y.exe"), r"C:\x\y.exe");
        assert_eq!(normalize_image_path(r"\SystemRoot\System32\a.exe"), r"%SystemRoot%\System32\a.exe");
    }
}
