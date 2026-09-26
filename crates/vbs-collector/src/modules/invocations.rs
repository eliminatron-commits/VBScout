//! Batch, PowerShell and KiXtart scripts that start VBScript (`wscript`/`cscript` with a
//! `.vbs` file, a `.vbs` file started directly, `mshta vbscript:…`).

use vbs_core::model::FindingKind;
use vbs_core::module::{CREDENTIAL_RULE, CandidateFile, Module, ModuleInfo, Report};

use super::{MAX_SCRIPT_BYTES, file_activation, file_credentials};
use crate::analysis::command::{self, Usage};
use crate::analysis::script::{self, Language};
use crate::analysis::{servicing, text};

pub struct Invocations;

static INFO: ModuleInfo = ModuleInfo {
    id: "script-invocation",
    system_source: None,
    rules: &["VBS-201", "VBS-202", CREDENTIAL_RULE],
    fallback_rule: "VBS-200",
    needs_admin: false,
};

impl Module for Invocations {
    fn info(&self) -> &'static ModuleInfo {
        &INFO
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["bat", "cmd", "ps1", "kix"]
    }

    fn inspect_file(&self, file: &dyn CandidateFile, report: &mut Report) {
        let bytes = match file.contents(MAX_SCRIPT_BYTES) {
            Ok(bytes) => bytes,
            Err(error) => {
                if let Some(reason) = error.reason() {
                    report
                        .not_checkable("VBS-200", file.location(), reason)
                        .kind(FindingKind::ScriptInvocation)
                        .file(file.facts())
                        .emit();
                }
                return;
            }
        };
        if servicing::is_servicing_data(&bytes) {
            return; // a differential of the component store, not a script
        }
        let language = match file.extension() {
            "ps1" => Language::PowerShell,
            "kix" => Language::KiXtart,
            _ => Language::Batch,
        };
        let content = text::decode(&bytes);
        let code = script::code_lines(&content, language);
        let activation = file_activation(file.path());
        let mut reported = false;
        for (usage, rule) in [(Usage::VbScript, "VBS-201"), (Usage::Unknown, "VBS-202")] {
            let mut calls: Vec<(u32, &str, Option<String>, &'static str)> = Vec::new();
            for (number, line) in &code {
                let references = command::analyze(line);
                if command::strongest(&references) != Some(usage) {
                    continue;
                }
                let reference = references.iter().filter(|r| r.usage == usage).min_by_key(|r| r.script.is_none());
                if let Some(reference) = reference {
                    calls.push((*number, line, reference.script.clone(), reference.via));
                }
            }
            let Some((_, _, script, via)) = calls.first().cloned() else { continue };
            let mut builder = report
                .file_finding(rule, file)
                .activation(activation.clone())
                .detail("via", via)
                .detail("calls", i64::try_from(calls.len()).unwrap_or(i64::MAX));
            if let Some(script) = script {
                builder = builder.target(script);
            }
            for (number, line, _, _) in &calls {
                builder = builder.evidence(Some(*number), line);
            }
            builder.emit();
            reported = true;
        }
        if reported {
            file_credentials(report, file, activation, &code);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::testing::{MemoryFile, inspect};
    use vbs_core::model::{Activation, Detail};

    #[test]
    fn batch_files_starting_vbscript() {
        let bat = "@echo off\r\nrem cscript old.vbs\r\nset DBPASSWORD=Geheim123\r\ncscript //nologo \"%~dp0inventory.vbs\" /q\r\nwscript C:\\Tools\\menu.wsf\r\n";
        let findings = inspect(&Invocations, &MemoryFile::new("C:/Scripts/run.cmd", bat));
        let rules: Vec<&str> = findings.iter().map(|f| f.rule.as_str()).collect();
        assert_eq!(rules, ["VBS-201", "VBS-202", CREDENTIAL_RULE]);
        assert_eq!(findings[0].target.as_deref(), Some("%~dp0inventory.vbs"));
        assert_eq!(findings[0].details["via"], Detail::Text("cscript".into()));
        assert_eq!(findings[0].evidence[0].line, Some(4));
        assert_eq!(findings[1].target.as_deref(), Some(r"C:\Tools\menu.wsf"));
        assert!(findings[2].evidence[0].masked);
    }

    #[test]
    fn powershell_and_logon_folders() {
        let ps = "# old: wscript x.vbs\r\n$shell = New-Object -ComObject WScript.Shell\r\n$shell.Run(\"wscript.exe //B \\\\srv\\netlogon\\map.vbs\")\r\n";
        let file = MemoryFile::new(r"C:\Windows\System32\GroupPolicy\User\Scripts\Logon\logon.ps1", ps);
        let findings = inspect(&Invocations, &file);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].activation, Activation::Automatic);
        assert_eq!(findings[0].target.as_deref(), Some(r"\\srv\netlogon\map.vbs"));
    }

    #[test]
    fn scripts_without_vbscript_yield_nothing() {
        for (name, body) in [
            ("build.cmd", "@echo off\r\nrem Builds\r\necho Building\r\nmsbuild /p:Configuration=Release\r\n"),
            ("backup.ps1", "# Backup\r\nCopy-Item C:\\Data D:\\Backup -Recurse\r\n$pwd = \"not reported\"\r\n"),
            ("clean.bat", "del /q C:\\Temp\\*.vbs\r\ncopy \\\\srv\\share\\logon.vbs C:\\Temp\\\r\n"),
        ] {
            assert!(inspect(&Invocations, &MemoryFile::new(&format!("C:/x/{name}"), body)).is_empty(), "{name}");
        }
    }
}
