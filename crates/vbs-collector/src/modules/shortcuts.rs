//! Shortcuts (`.lnk`) that start VBScript: the Script Host with a `.vbs` file, the `.vbs`
//! file itself, or `mshta vbscript:…`.

use vbs_core::model::{Activation, FindingKind, NotCheckableReason};
use vbs_core::module::{CREDENTIAL_RULE, CandidateFile, Module, ModuleInfo, Report};

use super::{file_activation, read_error};
use crate::analysis::command::{self, Usage};
use crate::analysis::{lnk, servicing};

pub struct Shortcuts;

static INFO: ModuleInfo = ModuleInfo {
    id: "shortcut",
    system_source: None,
    rules: &["VBS-211", "VBS-212", CREDENTIAL_RULE],
    fallback_rule: "VBS-200",
    needs_admin: false,
};

/// Shell links are small; anything bigger is not a shortcut.
const MAX_LINK_BYTES: u64 = 1024 * 1024;

impl Module for Shortcuts {
    fn info(&self) -> &'static ModuleInfo {
        &INFO
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["lnk"]
    }

    fn inspect_file(&self, file: &dyn CandidateFile, report: &mut Report) {
        let not_checkable = |report: &mut Report, reason: NotCheckableReason, problem: Option<&str>| {
            let mut builder =
                report.not_checkable("VBS-200", file.location(), reason).kind(FindingKind::Shortcut).file(file.facts());
            if let Some(problem) = problem {
                builder = builder.detail("readError", read_error(problem));
            }
            builder.emit();
        };
        let bytes = match file.contents(MAX_LINK_BYTES) {
            Ok(bytes) => bytes,
            Err(error) => {
                if let Some(reason) = error.reason() {
                    not_checkable(report, reason, None);
                }
                return;
            }
        };
        if servicing::is_servicing_data(&bytes) {
            return; // a differential of the component store, not a shortcut
        }
        let link = match lnk::parse(&bytes) {
            Ok(link) => link,
            Err(lnk::LinkError::Corrupt(what)) => {
                return not_checkable(report, NotCheckableReason::Corrupt, Some(what));
            }
        };
        let target = link.target.as_deref().unwrap_or_default();
        let arguments = link.arguments.as_deref().unwrap_or_default();
        let references = command::analyze_parts(target, arguments);
        let Some(usage) = command::strongest(&references) else { return };
        let rule = if usage == Usage::VbScript { "VBS-211" } else { "VBS-212" };
        let reference = references.iter().filter(|r| r.usage == usage).min_by_key(|r| r.script.is_none());
        let lower = file.path().to_string_lossy().replace('/', "\\").to_lowercase();
        let recent = lower.contains("\\microsoft\\windows\\recent\\");
        let activation = match file_activation(file.path()) {
            Activation::Automatic => Activation::Automatic,
            _ => Activation::Manual,
        };
        let command_line = if arguments.is_empty() { target.to_owned() } else { format!("{target} {arguments}") };
        let mut builder = report.file_finding(rule, file).activation(activation.clone()).evidence(None, &command_line);
        if let Some(reference) = reference {
            builder = builder.detail("via", reference.via);
            if let Some(script) = &reference.script {
                builder = builder.target(script.clone());
            }
        }
        if let Some(folder) = &link.working_dir {
            builder = builder.detail("workingDirectory", folder.clone());
        }
        if recent {
            builder = builder.detail("recentItem", true);
        }
        builder.emit();
        report.credentials(file.location(), Some(file.facts()), activation, [(None, command_line.as_str())]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::lnk::tests::build;
    use crate::modules::testing::{MemoryFile, inspect};
    use vbs_core::model::{Detail, FindingStatus};

    #[test]
    fn shortcuts_to_the_script_host() {
        let data = build(r"C:\Windows\System32\wscript.exe", Some(r#""C:\Scripts\menu.vbs""#), Some(r"C:\Scripts"));
        let findings = inspect(&Shortcuts, &MemoryFile::new(r"C:\Users\Public\Desktop\Menu.lnk", data));
        assert_eq!(findings.len(), 1);
        assert_eq!((findings[0].rule.as_str(), &findings[0].kind), ("VBS-211", &FindingKind::Shortcut));
        assert_eq!(findings[0].activation, Activation::Manual);
        assert_eq!(findings[0].target.as_deref(), Some(r"C:\Scripts\menu.vbs"));
        assert_eq!(findings[0].details["via"], Detail::Text("wscript".into()));
    }

    #[test]
    fn shortcuts_to_scripts_and_other_programs() {
        let startup = r"C:\ProgramData\Microsoft\Windows\Start Menu\Programs\StartUp\Map.lnk";
        let findings = inspect(&Shortcuts, &MemoryFile::new(startup, build(r"\\srv\netlogon\map.vbs", None, None)));
        assert_eq!((findings[0].rule.as_str(), &findings[0].activation), ("VBS-211", &Activation::Automatic));
        let hta = inspect(&Shortcuts, &MemoryFile::new("C:/x/Tool.lnk", build(r"C:\Tools\tool.hta", None, None)));
        assert_eq!(hta[0].rule, "VBS-212");
        assert!(
            inspect(
                &Shortcuts,
                &MemoryFile::new("C:/x/Word.lnk", build(r"C:\Program Files\Office\WINWORD.EXE", None, None))
            )
            .is_empty()
        );
    }

    #[test]
    fn broken_shortcuts_are_not_checkable() {
        let findings = inspect(&Shortcuts, &MemoryFile::new("C:/x/Broken.lnk", b"garbage".to_vec()));
        assert_eq!(
            (findings[0].rule.as_str(), &findings[0].status, &findings[0].kind),
            ("VBS-200", &FindingStatus::NotCheckable, &FindingKind::Shortcut)
        );
        assert_eq!(findings[0].details["readError"], Detail::Text("not a shell link".into()));
    }
}
