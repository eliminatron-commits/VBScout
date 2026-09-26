//! Logon, logoff, startup and shutdown scripts that run VBScript: scripts assigned by Group
//! Policy (as applied, from the registry – including the policy state of users who are not
//! logged on), `UserInitMprLogonScript`, and the script lists `scripts.ini`/`psscripts.ini`
//! of local policies and of SYSVOL (local on domain controllers, or given with
//! `--include-unc`).

use vbs_core::model::{Activation, Detail, FindingKind, Location, LocationKind};
use vbs_core::module::{CREDENTIAL_RULE, CandidateFile, Module, ModuleInfo, Report};
use vbs_core::views::{Bitness, Hive, RegistryView, SystemView, ViewError};

use super::{Rules, check_command, loaded_user_hives, offline_hive, report_command, user_profiles};
use crate::analysis::command::{self, Usage};
use crate::analysis::ini;

pub struct LogonScripts;

static INFO: ModuleInfo = ModuleInfo {
    id: "logon-script",
    system_source: Some("policies.scripts"),
    rules: &["VBS-341", "VBS-342", CREDENTIAL_RULE],
    fallback_rule: "VBS-300",
    needs_admin: false,
};

const RULES: Rules =
    Rules { breaks: "VBS-341", review: "VBS-342", not_checkable: "VBS-300", kind: FindingKind::LogonScript };
const GP_SCRIPTS: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Group Policy\Scripts";
const GP_STATE: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Group Policy\State";

impl Module for LogonScripts {
    fn info(&self) -> &'static ModuleInfo {
        &INFO
    }

    fn file_names(&self) -> &'static [&'static str] {
        &["scripts.ini", "psscripts.ini"]
    }

    fn inspect_file(&self, file: &dyn CandidateFile, report: &mut Report) {
        let bytes = match file.contents(1024 * 1024) {
            Ok(bytes) => bytes,
            Err(error) => {
                if let Some(reason) = error.reason() {
                    report
                        .not_checkable("VBS-300", file.location(), reason)
                        .kind(FindingKind::LogonScript)
                        .file(file.facts())
                        .emit();
                }
                return;
            }
        };
        let path = file.path().display().to_string();
        let folder = path.rfind(['\\', '/']).map_or("", |end| &path[..end]);
        for entry in ini::parse(&bytes) {
            let script = full_path(&entry.command, &format!(r"{folder}\{}", entry.phase));
            let references = command::analyze_parts(&script, &entry.parameters);
            let Some(usage) = command::strongest(&references) else { continue };
            let rule = if usage == Usage::VbScript { "VBS-341" } else { "VBS-342" };
            let location = Location { item: Some(format!("{} {}", entry.phase, entry.index)), ..file.location() };
            let line = format!("{}CmdLine={}", entry.index, entry.command);
            report
                .finding(rule, location.clone())
                .file(file.facts())
                .activation(Activation::Automatic)
                .target(script.clone())
                .detail("phase", entry.phase.to_ascii_lowercase())
                .evidence(Some(entry.line), &line)
                .emit();
            let parameters = format!("{}Parameters={}", entry.index, entry.parameters);
            report.credentials(
                location,
                Some(file.facts()),
                Activation::Automatic,
                [(Some(entry.line), parameters.as_str())],
            );
        }
    }

    fn scan_system(&self, system: &SystemView<'_>, report: &mut Report) {
        let mut seen: Vec<(String, String)> = Vec::new();
        let machine = Root {
            registry: system.registry,
            hive: Hive::LocalMachine,
            prefix: String::new(),
            label: "HKLM".into(),
            offline: false,
        };
        policy_scripts(system, report, &machine, GP_SCRIPTS, &["Startup", "Shutdown"], &mut seen);
        match system.registry.subkeys(Hive::LocalMachine, GP_STATE, Bitness::Native) {
            Ok(accounts) => {
                for account in accounts {
                    let base = format!(r"{GP_STATE}\{account}\Scripts");
                    let phases = ["Logon", "Logoff", "Startup", "Shutdown"];
                    policy_scripts(system, report, &machine, &base, &phases, &mut seen);
                }
            }
            Err(ViewError::NotFound) => {}
            Err(error) => report.record_error(format!(r"HKLM\{GP_STATE} ({error})")),
        }
        let loaded = loaded_user_hives(system).unwrap_or_default();
        for sid in &loaded {
            let user = Root {
                registry: system.registry,
                hive: Hive::Users,
                prefix: format!(r"{sid}\"),
                label: format!(r"HKU\{sid}"),
                offline: false,
            };
            user_scripts(system, report, &user, &mut seen);
        }
        // Users who are not logged on: their hive files, read as files (never loaded).
        for (sid, profile) in user_profiles(system) {
            if loaded.iter().any(|loaded| loaded.eq_ignore_ascii_case(&sid)) {
                continue;
            }
            if let Ok(hive) = offline_hive(system, &profile) {
                let user = Root {
                    registry: &hive,
                    hive: Hive::Users,
                    prefix: String::new(),
                    label: format!(r"HKU\{sid}"),
                    offline: true,
                };
                user_scripts(system, report, &user, &mut seen);
            }
        }
    }
}

/// A registry root to read from: HKLM, a loaded user hive or a user's hive file.
struct Root<'r> {
    registry: &'r dyn RegistryView,
    hive: Hive,
    /// Prefix of paths in the view (`S-1-5-21-…\` for loaded user hives).
    prefix: String,
    /// Location prefix (`HKLM`, `HKU\S-1-5-21-…`).
    label: String,
    offline: bool,
}

/// Group Policy logon/logoff scripts and `UserInitMprLogonScript` of one user hive.
fn user_scripts(system: &SystemView<'_>, report: &mut Report, user: &Root<'_>, seen: &mut Vec<(String, String)>) {
    policy_scripts(system, report, user, GP_SCRIPTS, &["Logon", "Logoff"], seen);
    let environment = format!("{}Environment", user.prefix);
    if let Ok(Some(value)) = user.registry.value(user.hive, &environment, "UserInitMprLogonScript", Bitness::Native)
        && let Some(command) = value.as_text().filter(|c| !c.trim().is_empty())
    {
        report.count_entries(1);
        let check = check_command(system, command, None);
        let location = Location {
            kind: LocationKind::Registry,
            path: format!(r"{}\Environment", user.label),
            item: Some("UserInitMprLogonScript".into()),
        };
        let mut details = vec![("phase", Detail::Text("logon".into()))];
        if user.offline {
            details.push(("offlineHive", Detail::Flag(true)));
        }
        report_command(report, RULES, location, Activation::Automatic, command, &check, &details);
    }
}

/// `<base>\<Phase>\<n>\<m>` keys with the values `Script` and `Parameters`; the policy's
/// `FileSysPath` completes relative script names.
fn policy_scripts(
    system: &SystemView<'_>,
    report: &mut Report,
    root: &Root<'_>,
    base: &str,
    phases: &[&str],
    seen: &mut Vec<(String, String)>,
) {
    let (registry, hive) = (root.registry, root.hive);
    for phase in phases {
        let phase_key = format!(r"{}{base}\{phase}", root.prefix);
        let Ok(policies) = registry.subkeys(hive, &phase_key, Bitness::Native) else { continue };
        for policy in policies {
            let policy_key = format!(r"{phase_key}\{policy}");
            let text = |key: &str, name: &str| {
                registry
                    .value(hive, key, name, Bitness::Native)
                    .ok()
                    .flatten()
                    .and_then(|v| v.as_text().map(str::to_owned))
            };
            let file_sys_path = text(&policy_key, "FileSysPath");
            let policy_name = text(&policy_key, "DisplayName").or_else(|| text(&policy_key, "GPOName"));
            let Ok(scripts) = registry.subkeys(hive, &policy_key, Bitness::Native) else { continue };
            for index in scripts {
                let script_key = format!(r"{policy_key}\{index}");
                let Some(script) = text(&script_key, "Script").filter(|s| !s.trim().is_empty()) else { continue };
                report.count_entries(1);
                let parameters = text(&script_key, "Parameters").unwrap_or_default();
                let folder =
                    file_sys_path.as_deref().map(|root| format!(r"{root}\Scripts\{phase}")).unwrap_or_default();
                let full = full_path(&script, &folder);
                let identity = (
                    phase.to_ascii_lowercase(),
                    format!("{} {}", full.to_ascii_lowercase(), parameters.to_ascii_lowercase()),
                );
                if seen.contains(&identity) {
                    continue;
                }
                seen.push(identity);
                let command = format!("\"{full}\" {parameters}");
                let check = check_command(system, &command, None);
                let shown = script_key.strip_prefix(&root.prefix).unwrap_or(&script_key);
                let location =
                    Location { kind: LocationKind::Registry, path: format!(r"{}\{shown}", root.label), item: None };
                let mut details = vec![("phase", Detail::Text(phase.to_ascii_lowercase()))];
                if root.offline {
                    details.push(("offlineHive", Detail::Flag(true)));
                }
                if let Some(name) = &policy_name {
                    details.push(("policyName", Detail::Text(name.clone())));
                }
                report_command(report, RULES, location, Activation::Automatic, command.trim(), &check, &details);
            }
        }
    }
}

/// A script name as the policy stores it, completed to a full path when relative.
fn full_path(script: &str, folder: &str) -> String {
    let script = script.trim().trim_matches('"');
    let bytes = script.as_bytes();
    let absolute = script.starts_with("\\\\")
        || (bytes.len() > 2 && bytes[1] == b':')
        || script.starts_with('%')
        || script.starts_with('/');
    if absolute || folder.is_empty() || !script.contains('.') {
        script.to_owned()
    } else {
        format!(r"{}\{script}", folder.trim_end_matches('\\'))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::testing::{MemoryFile, inspect, scan_system};
    use vbs_core::model::FindingStatus;
    use vbs_core::views::{MemoryEventLogs, MemoryRegistry, MemoryWmi, RegValue, SystemEnvironment};

    #[test]
    fn policy_scripts_from_the_registry() {
        let user = "S-1-5-21-1-2-3-1001";
        let registry = MemoryRegistry::new()
            .with_key(
                Hive::LocalMachine,
                &format!(r"{GP_SCRIPTS}\Startup\0"),
                &[
                    (
                        "FileSysPath",
                        RegValue::Text(r"\\contoso.local\SysVol\contoso.local\Policies\{31B2F340}\Machine".into()),
                    ),
                    ("DisplayName", RegValue::Text("Computer startup".into())),
                ],
            )
            .with_key(
                Hive::LocalMachine,
                &format!(r"{GP_SCRIPTS}\Startup\0\0"),
                &[("Script", RegValue::Text("inventory.vbs".into())), ("Parameters", RegValue::Text("/silent".into()))],
            )
            .with_key(
                Hive::LocalMachine,
                &format!(r"{GP_STATE}\{user}\Scripts\Logon\0"),
                &[(
                    "FileSysPath",
                    RegValue::Text(r"\\contoso.local\SysVol\contoso.local\Policies\{6AC1786C}\User".into()),
                )],
            )
            .with_key(
                Hive::LocalMachine,
                &format!(r"{GP_STATE}\{user}\Scripts\Logon\0\0"),
                &[("Script", RegValue::Text("drives.bat".into()))],
            )
            .with_key(
                Hive::LocalMachine,
                &format!(r"{GP_STATE}\{user}\Scripts\Logon\0\1"),
                &[("Script", RegValue::Text("menu.wsf".into()))],
            )
            .with_key_in(
                Hive::Users,
                Bitness::Native,
                &format!(r"{user}\Environment"),
                &[("UserInitMprLogonScript", RegValue::Text(r"cscript C:\Login\login.vbs".into()))],
            );
        let report = scan_system(
            &LogonScripts,
            &SystemEnvironment::default(),
            &registry,
            &MemoryEventLogs::new(),
            &MemoryWmi::new(),
        );
        let summary: Vec<(&str, &FindingStatus, Option<&str>)> =
            report.findings().iter().map(|f| (f.rule.as_str(), &f.status, f.target.as_deref())).collect();
        assert_eq!(
            summary,
            [
                (
                    "VBS-341",
                    &FindingStatus::Detected,
                    Some(
                        r"\\contoso.local\SysVol\contoso.local\Policies\{31B2F340}\Machine\Scripts\Startup\inventory.vbs"
                    )
                ),
                (
                    "VBS-300",
                    &FindingStatus::NotCheckable,
                    Some(r"\\contoso.local\SysVol\contoso.local\Policies\{6AC1786C}\User\Scripts\Logon\drives.bat")
                ),
                (
                    "VBS-342",
                    &FindingStatus::Detected,
                    Some(r"\\contoso.local\SysVol\contoso.local\Policies\{6AC1786C}\User\Scripts\Logon\menu.wsf")
                ),
                ("VBS-341", &FindingStatus::Detected, Some(r"C:\Login\login.vbs")),
            ]
        );
        assert_eq!(report.findings()[1].reason, Some(vbs_core::model::NotCheckableReason::NetworkLocation));
        assert_eq!(report.findings()[1].kind, FindingKind::LogonScript);
    }

    #[test]
    fn script_lists_on_disk() {
        let ini = "[Logon]\r\n0CmdLine=map.vbs\r\n0Parameters=\r\n1CmdLine=printers.cmd\r\n[Logoff]\r\n0CmdLine=\\\\srv\\share\\bye.hta\r\n";
        let file = MemoryFile::new(r"C:\Windows\System32\GroupPolicy\User\Scripts\scripts.ini", ini);
        let findings = inspect(&LogonScripts, &file);
        let summary: Vec<(&str, Option<&str>, Option<&str>)> =
            findings.iter().map(|f| (f.rule.as_str(), f.location.item.as_deref(), f.target.as_deref())).collect();
        assert_eq!(
            summary,
            [
                ("VBS-342", Some("Logoff 0"), Some(r"\\srv\share\bye.hta")),
                ("VBS-341", Some("Logon 0"), Some(r"C:\Windows\System32\GroupPolicy\User\Scripts\Logon\map.vbs")),
            ]
        );
    }
}
