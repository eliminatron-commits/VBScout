//! Autostart entries that run VBScript: Run/RunOnce keys (machine and every loaded user
//! hive, 64- and 32-bit views), Winlogon, Active Setup, the command processor's AutoRun,
//! and the startup folders of all users and of every profile.

use std::path::Path;

use vbs_core::model::{Activation, Detail, FindingKind, Location, LocationKind, NotCheckableReason, SourceStatus};
use vbs_core::module::{CREDENTIAL_RULE, Module, ModuleInfo, Report};
use vbs_core::views::{Bitness, Hive, RegValue, RegistryView, SystemView, ViewError};

use super::{
    CommandCheck, Rules, check_command, check_script_file, join_windows, loaded_user_hives, offline_hive,
    report_command, user_profiles,
};
use crate::analysis::command::{Reference, Usage};
use crate::analysis::{lnk, text};

pub struct Autostart;

static INFO: ModuleInfo = ModuleInfo {
    id: "autostart",
    system_source: Some("autostart.entries"),
    rules: &["VBS-311", "VBS-312", CREDENTIAL_RULE],
    fallback_rule: "VBS-300",
    needs_admin: true,
};

const RULES: Rules =
    Rules { breaks: "VBS-311", review: "VBS-312", not_checkable: "VBS-300", kind: FindingKind::Autostart };

/// Keys whose values are commands, with the kind of autostart they are.
const RUN_KEYS: [(&str, &str); 5] = [
    (r"SOFTWARE\Microsoft\Windows\CurrentVersion\Run", "run"),
    (r"SOFTWARE\Microsoft\Windows\CurrentVersion\RunOnce", "runOnce"),
    (r"SOFTWARE\Microsoft\Windows\CurrentVersion\RunServices", "runServices"),
    (r"SOFTWARE\Microsoft\Windows\CurrentVersion\RunServicesOnce", "runServicesOnce"),
    (r"SOFTWARE\Microsoft\Windows\CurrentVersion\Policies\Explorer\Run", "policyRun"),
];
const RUN_ONCE_EX: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\RunOnceEx";
const WINLOGON: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\Winlogon";
const WINDOWS: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\Windows";
const ACTIVE_SETUP: &str = r"SOFTWARE\Microsoft\Active Setup\Installed Components";
const COMMAND_PROCESSOR: &str = r"SOFTWARE\Microsoft\Command Processor";
const STARTUP: &str = r"Microsoft\Windows\Start Menu\Programs\StartUp";
const USER_STARTUP: &str = r"AppData\Roaming\Microsoft\Windows\Start Menu\Programs\Startup";

impl Module for Autostart {
    fn info(&self) -> &'static ModuleInfo {
        &INFO
    }

    fn scan_system(&self, system: &SystemView<'_>, report: &mut Report) {
        let registry = system.registry;
        for bitness in [Bitness::Native, Bitness::Wow32] {
            let hive = Place::machine(bitness);
            for (key, kind) in RUN_KEYS {
                run_key(system, registry, report, &hive, key, kind);
            }
            if let Ok(entries) = registry.subkeys(Hive::LocalMachine, RUN_ONCE_EX, bitness) {
                for entry in entries {
                    run_key(system, registry, report, &hive, &format!(r"{RUN_ONCE_EX}\{entry}"), "runOnceEx");
                }
            }
            if let Ok(components) = registry.subkeys(Hive::LocalMachine, ACTIVE_SETUP, bitness) {
                for component in components {
                    let key = format!(r"{ACTIVE_SETUP}\{component}");
                    values(system, registry, report, &hive, &key, &["StubPath"], "activeSetup");
                }
            }
            values(system, registry, report, &hive, COMMAND_PROCESSOR, &["AutoRun"], "commandProcessor");
        }
        let machine = Place::machine(Bitness::Native);
        values(system, registry, report, &machine, WINLOGON, &["Userinit", "Shell", "Taskman", "AppSetup"], "winlogon");

        let loaded = match loaded_user_hives(system) {
            Ok(sids) => sids,
            Err(error) => {
                report.record_error(format!("HKEY_USERS ({error})"));
                Vec::new()
            }
        };
        for sid in &loaded {
            user_hive(system, registry, report, &Place::user(sid, true));
        }

        if let Some(program_data) = &system.env.program_data {
            startup_folder(system, report, &join_windows(program_data, STARTUP));
        }
        let mut not_read = 0;
        for (sid, profile) in user_profiles(system) {
            startup_folder(system, report, &join_windows(&profile, USER_STARTUP));
            if loaded.iter().any(|loaded| loaded.eq_ignore_ascii_case(&sid)) {
                continue;
            }
            // Not logged on: the hive is not loaded (and loading it would change the system), so its
            // file is read instead.
            match offline_hive(system, &profile) {
                Ok(hive) => user_hive(system, &hive, report, &Place::user(&sid, false)),
                Err(error) => {
                    report.record_error(format!("{} ({error})", profile.join("NTUSER.DAT").display()));
                    not_read += 1;
                }
            }
        }
        if not_read > 0 {
            report.count_skipped(not_read);
            report.set_source_status(SourceStatus::Partial, "userHivesNotRead");
        }
    }
}

/// The Run keys and other autostart values of one user hive (loaded, or read from its file).
fn user_hive(system: &SystemView<'_>, registry: &dyn RegistryView, report: &mut Report, user: &Place) {
    for (key, kind) in RUN_KEYS {
        run_key(system, registry, report, user, key, kind);
    }
    values(system, registry, report, user, WINLOGON, &["Shell"], "winlogon");
    values(system, registry, report, user, WINDOWS, &["Load", "Run"], "windowsLoad");
    values(system, registry, report, user, COMMAND_PROCESSOR, &["AutoRun"], "commandProcessor");
}

/// A registry root: HKLM (64- or 32-bit view) or one user hive – loaded under `HKEY_USERS`,
/// or read from the profile's `NTUSER.DAT`.
struct Place {
    hive: Hive,
    /// Prefix of registry paths in the view (`S-1-5-21-…\` for loaded user hives).
    prefix: String,
    /// Prefix shown in locations (`HKLM\`, `HKU\S-1-5-21-…\`).
    label: String,
    bitness: Bitness,
    /// Read from the hive file of a user who is not logged on.
    offline: bool,
}

impl Place {
    fn machine(bitness: Bitness) -> Self {
        Place { hive: Hive::LocalMachine, prefix: String::new(), label: r"HKLM\".into(), bitness, offline: false }
    }

    fn user(sid: &str, loaded: bool) -> Self {
        Place {
            hive: Hive::Users,
            prefix: if loaded { format!(r"{sid}\") } else { String::new() },
            label: format!(r"HKU\{sid}\"),
            bitness: Bitness::Native,
            offline: !loaded,
        }
    }

    fn display(&self, key: &str) -> String {
        let key = match (self.bitness, key.strip_prefix(r"SOFTWARE\")) {
            (Bitness::Wow32, Some(rest)) => format!(r"SOFTWARE\WOW6432Node\{rest}"),
            _ => key.to_owned(),
        };
        format!("{}{key}", self.label)
    }
}

fn run_key(
    system: &SystemView<'_>,
    registry: &dyn RegistryView,
    report: &mut Report,
    place: &Place,
    key: &str,
    kind: &str,
) {
    let full = format!("{}{key}", place.prefix);
    match registry.values(place.hive, &full, place.bitness) {
        Ok(values) => {
            report.count_entries(values.len() as u64);
            for (name, value) in values {
                command_value(system, report, place, key, &name, &value, kind);
            }
        }
        Err(ViewError::NotFound) => {}
        Err(error) => report.record_error(format!("{} ({error})", place.display(key))),
    }
}

#[allow(clippy::too_many_arguments)]
fn values(
    system: &SystemView<'_>,
    registry: &dyn RegistryView,
    report: &mut Report,
    place: &Place,
    key: &str,
    names: &[&str],
    kind: &str,
) {
    let full = format!("{}{key}", place.prefix);
    for name in names {
        match registry.value(place.hive, &full, name, place.bitness) {
            Ok(Some(value)) => {
                report.count_entries(1);
                command_value(system, report, place, key, name, &value, kind);
            }
            Ok(None) | Err(ViewError::NotFound) => {}
            Err(error) => report.record_error(format!("{} ({error})", place.display(key))),
        }
    }
}

fn command_value(
    system: &SystemView<'_>,
    report: &mut Report,
    place: &Place,
    key: &str,
    name: &str,
    value: &RegValue,
    kind: &str,
) {
    let Some(command) = value.as_text().map(str::trim).filter(|c| !c.is_empty()) else { return };
    // Userinit is a comma-separated list of programs.
    let analysed = if name.eq_ignore_ascii_case("Userinit") { command.replace(',', " & ") } else { command.to_owned() };
    let check = check_command(system, &analysed, None);
    let location = Location { kind: LocationKind::Registry, path: place.display(key), item: Some(name.to_owned()) };
    let mut details = vec![("autostartType", Detail::Text(kind.to_owned()))];
    if place.offline {
        details.push(("offlineHive", Detail::Flag(true)));
    }
    report_command(report, RULES, location, Activation::Automatic, command, &check, &details);
}

fn startup_folder(system: &SystemView<'_>, report: &mut Report, folder: &Path) {
    let entries = match system.files.list_dir(folder) {
        Ok(entries) => entries,
        Err(ViewError::NotFound) => return,
        Err(error) => {
            report.record_error(format!("{} ({error})", folder.display()));
            return;
        }
    };
    report.add_root(folder.display().to_string());
    for entry in entries.iter().filter(|entry| !entry.is_dir) {
        report.count_entries(1);
        let path = folder.join(&entry.name);
        let location = Location { kind: LocationKind::File, path: path.display().to_string(), item: None };
        let details = [("autostartType", Detail::Text("startupFolder".into()))];
        let written = path.display().to_string();
        match text::extension(&entry.name).as_deref() {
            Some("lnk") => match system.files.read(&path, 1024 * 1024) {
                Ok(bytes) => match lnk::parse(&bytes) {
                    Ok(link) => {
                        let target = link.target.unwrap_or_default();
                        let arguments = link.arguments.unwrap_or_default();
                        let quoted = format!("\"{}\" {arguments}", target.trim_matches('"'));
                        let check = check_command(system, &quoted, link.working_dir.as_deref());
                        let shown = format!("{target} {arguments}");
                        report_command(report, RULES, location, Activation::Automatic, shown.trim(), &check, &details);
                    }
                    Err(_) => not_checkable(report, location, NotCheckableReason::Corrupt),
                },
                Err(error) => not_checkable(report, location, super::view_reason(&error)),
            },
            Some("vbs" | "vbe") => {
                let check = CommandCheck {
                    references: vec![Reference {
                        usage: Usage::VbScript,
                        script: Some(written.clone()),
                        via: "direct",
                    }],
                    ..CommandCheck::default()
                };
                report_command(report, RULES, location, Activation::Automatic, &written, &check, &details);
            }
            Some(extension @ ("wsf" | "wsc" | "hta")) => match system.files.read(&path, super::MAX_SCRIPT_BYTES) {
                Ok(bytes) => {
                    let content = text::decode(&bytes);
                    if super::script_files::analyze(extension, &content).vbscript {
                        let check = CommandCheck {
                            references: vec![Reference {
                                usage: Usage::VbScript,
                                script: Some(written.clone()),
                                via: "direct",
                            }],
                            ..CommandCheck::default()
                        };
                        report_command(report, RULES, location, Activation::Automatic, &written, &check, &details);
                    }
                }
                Err(error) => not_checkable(report, location, super::view_reason(&error)),
            },
            Some("bat" | "cmd" | "ps1" | "kix") => {
                let mut check = CommandCheck::default();
                check_script_file(system, &path, &mut check);
                report_command(report, RULES, location, Activation::Automatic, &written, &check, &details);
            }
            _ => {}
        }
    }
}

fn not_checkable(report: &mut Report, location: Location, reason: NotCheckableReason) {
    report
        .not_checkable("VBS-300", location, reason)
        .kind(FindingKind::Autostart)
        .activation(Activation::Automatic)
        .detail("autostartType", "startupFolder")
        .emit();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::lnk::tests::build;
    use crate::modules::testing::scan_system;
    use vbs_core::views::{MemoryEventLogs, MemoryRegistry, MemoryWmi, SystemEnvironment};

    const SID: &str = "S-1-5-21-1111111111-2222222222-3333333333-1001";

    #[test]
    fn run_keys_winlogon_and_startup_folders() {
        let dir = tempfile::tempdir().unwrap();
        let common = join_windows(&dir.path().join("ProgramData"), STARTUP);
        std::fs::create_dir_all(&common).unwrap();
        std::fs::write(common.join("map.vbs"), "MsgBox 1").unwrap();
        std::fs::write(common.join("desktop.ini"), "[.ShellClassInfo]").unwrap();
        let profile = dir.path().join("Users").join("alice");
        let user_startup = join_windows(&profile, USER_STARTUP);
        std::fs::create_dir_all(&user_startup).unwrap();
        std::fs::write(
            user_startup.join("Inventory.lnk"),
            build(r"C:\Windows\System32\wscript.exe", Some(r"C:\Tools\inv.vbs"), None),
        )
        .unwrap();
        std::fs::write(user_startup.join("Word.lnk"), build(r"C:\Program Files\Office\WINWORD.EXE", None, None))
            .unwrap();

        let registry = MemoryRegistry::new()
            .with_key_in(
                Hive::LocalMachine,
                Bitness::Native,
                RUN_KEYS[0].0,
                &[
                    ("Backup", RegValue::Text(r"wscript.exe //B C:\Scripts\backup.vbs".into())),
                    ("Tray", RegValue::Text(r#""C:\Program Files\Tray\tray.exe" /min"#.into())),
                ],
            )
            .with_key_in(
                Hive::LocalMachine,
                Bitness::Wow32,
                RUN_KEYS[1].0,
                &[("Setup", RegValue::Text(r"mshta C:\Setup\finish.hta".into()))],
            )
            .with_key_in(
                Hive::LocalMachine,
                Bitness::Native,
                WINLOGON,
                &[("Userinit", RegValue::Text(r"C:\Windows\system32\userinit.exe,wscript C:\x\login.vbs,".into()))],
            )
            .with_key_in(
                Hive::Users,
                Bitness::Native,
                &format!(r"{SID}\{}", RUN_KEYS[0].0),
                &[("Map", RegValue::ExpandText(r"cscript %SystemRoot%\map.vbs".into()))],
            )
            .with_key_in(
                Hive::LocalMachine,
                Bitness::Native,
                &format!(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\ProfileList\{SID}"),
                &[("ProfileImagePath", RegValue::ExpandText(profile.display().to_string()))],
            )
            .with_key_in(
                Hive::LocalMachine,
                Bitness::Native,
                r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\ProfileList\S-1-5-21-9-9-9-1002",
                &[(
                    "ProfileImagePath",
                    RegValue::ExpandText(dir.path().join("Users").join("bob").display().to_string()),
                )],
            );
        let env =
            SystemEnvironment { program_data: Some(dir.path().join("ProgramData")), ..SystemEnvironment::default() };
        let report = scan_system(&Autostart, &env, &registry, &MemoryEventLogs::new(), &MemoryWmi::new());
        let mut summary: Vec<(String, String)> = report
            .findings()
            .iter()
            .map(|f| {
                let label = match &f.location.item {
                    Some(item) => format!("{}|{item}", f.location.path),
                    None => text::file_name(&f.location.path).to_owned(),
                };
                (label, f.rule.clone())
            })
            .collect();
        summary.sort();
        let mut expected: Vec<(String, String)> = [
            (r"HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Winlogon|Userinit", "VBS-311"),
            (r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Run|Backup", "VBS-311"),
            (r"HKLM\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\RunOnce|Setup", "VBS-312"),
            ("Inventory.lnk", "VBS-311"),
            ("map.vbs", "VBS-311"),
        ]
        .iter()
        .map(|(label, rule)| ((*label).to_owned(), (*rule).to_owned()))
        .chain([(format!(r"HKU\{SID}\SOFTWARE\Microsoft\Windows\CurrentVersion\Run|Map"), "VBS-311".to_owned())])
        .collect();
        expected.sort();
        assert_eq!(summary, expected);
        // alice's hive is loaded (it holds the Run key above); bob's is not, and his profile has no NTUSER.DAT.
        assert_eq!(report.skipped(), 1);
        assert_eq!(report.source_status().map(|s| s.1.as_str()), Some("userHivesNotRead"));
    }
}
