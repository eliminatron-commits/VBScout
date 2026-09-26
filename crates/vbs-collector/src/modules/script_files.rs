//! VBScript files: `.vbs`, `.vbe` (encoded), and `.wsf`, `.wsc`, `.hta` that contain VBScript.

use vbs_core::module::{CREDENTIAL_RULE, CandidateFile, Module, ModuleInfo, Report};

use super::{MAX_SCRIPT_BYTES, file_activation, file_credentials};
use crate::analysis::markup;
use crate::analysis::script::{self, Language};
use crate::analysis::{text, vbe};

pub struct ScriptFiles;

static INFO: ModuleInfo = ModuleInfo {
    id: "script-file",
    system_source: None,
    rules: &["VBS-101", "VBS-102", "VBS-103", "VBS-104", "VBS-105", CREDENTIAL_RULE],
    fallback_rule: "VBS-100",
    needs_admin: false,
};

impl Module for ScriptFiles {
    fn info(&self) -> &'static ModuleInfo {
        &INFO
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["vbs", "vbe", "wsf", "wsc", "hta"]
    }

    fn inspect_file(&self, file: &dyn CandidateFile, report: &mut Report) {
        let bytes = match file.contents(MAX_SCRIPT_BYTES) {
            Ok(bytes) => bytes,
            Err(error) => {
                if let Some(reason) = error.reason() {
                    report.not_checkable("VBS-100", file.location(), reason).file(file.facts()).emit();
                }
                return;
            }
        };
        let content = text::decode(&bytes);
        let found = analyze(file.extension(), &content);
        if !found.vbscript {
            return;
        }
        let rule = match file.extension() {
            "vbs" => "VBS-101",
            "vbe" => "VBS-102",
            "wsf" => "VBS-103",
            "wsc" => "VBS-104",
            _ => "VBS-105",
        };
        let activation = file_activation(file.path());
        let code: Vec<(u32, &str)> = found.lines.iter().map(|(n, l)| (*n, l.as_str())).collect();
        let mut builder = report.file_finding(rule, file).activation(activation.clone());
        for (number, line) in
            found.markers.iter().map(|(n, l)| (*n, l.as_str())).chain(script::vbscript_evidence(&code)).take(8)
        {
            builder = builder.evidence(Some(number), line);
        }
        if found.encoded {
            builder = builder.detail("encoded", true);
        }
        if found.blocks > 0 {
            builder = builder.detail("vbscriptBlocks", i64::try_from(found.blocks).unwrap_or(i64::MAX));
        }
        if let Some(src) = &found.external {
            builder = builder.target(src.clone());
        }
        builder.emit();
        file_credentials(report, file, activation, &code);
    }
}

/// VBScript found in a script file.
#[derive(Debug, Default)]
pub(crate) struct ScriptContent {
    pub vbscript: bool,
    /// VBScript code lines (decoded where the file was encoded), with their line numbers.
    pub lines: Vec<(u32, String)>,
    /// Lines that mark VBScript in a document (script tags, event handlers).
    pub markers: Vec<(u32, String)>,
    /// Encoded with the Script Encoder.
    pub encoded: bool,
    /// VBScript elements of a .wsf, .wsc or .hta file.
    pub blocks: usize,
    /// `src` of an external VBScript file.
    pub external: Option<String>,
}

/// Finds the VBScript in the text of a `.vbs`, `.vbe`, `.wsf`, `.wsc` or `.hta` file.
pub(crate) fn analyze(extension: &str, content: &str) -> ScriptContent {
    match extension {
        "vbs" => ScriptContent {
            vbscript: true,
            lines: owned(script::code_lines(content, Language::VbScript)),
            ..Default::default()
        },
        "vbe" => {
            let decoded = vbe::decode_all(content);
            let source = decoded.as_deref().unwrap_or(content);
            ScriptContent {
                vbscript: true,
                lines: owned(script::code_lines(source, Language::VbScript)),
                encoded: decoded.is_some(),
                ..Default::default()
            }
        }
        _ => documents(content, extension == "hta"),
    }
}

fn owned(lines: Vec<(u32, &str)>) -> Vec<(u32, String)> {
    lines.into_iter().map(|(n, l)| (n, l.to_owned())).collect()
}

/// VBScript in `<script>` elements; in HTML applications also event handlers declared as
/// VBScript and `vbscript:` links.
fn documents(content: &str, html: bool) -> ScriptContent {
    let mut found = ScriptContent::default();
    for block in markup::script_blocks(content).into_iter().filter(markup::ScriptBlock::is_vbscript) {
        found.vbscript = true;
        found.blocks += 1;
        let tag_line = text::line_of(content, block.content.start);
        let language = block.language.clone().unwrap_or_default();
        found.markers.push((tag_line, format!("<script language=\"{language}\">")));
        if let Some(src) = &block.src {
            found.external.get_or_insert_with(|| src.clone());
        }
        let raw = &content[block.content.clone()];
        let (code, encoded) = match vbe::decode_all(raw) {
            Some(decoded) => (decoded, true),
            None => (raw.to_owned(), false),
        };
        found.encoded |= encoded || language.to_ascii_lowercase().contains("encode");
        let first = tag_line.saturating_sub(1);
        for (number, line) in script::code_lines(&code, Language::VbScript) {
            let trimmed = line.trim();
            if trimmed.starts_with("<![CDATA[") || trimmed.starts_with("]]>") || trimmed == "<!--" || trimmed == "-->" {
                continue;
            }
            found.lines.push((first + number, line.to_owned()));
        }
    }
    if html {
        for tag in markup::tags(content) {
            if tag.closing || tag.is("script") {
                continue;
            }
            let declares = tag.attribute("language").is_some_and(|l| l.to_ascii_lowercase().contains("vbscript"));
            let protocol = tag
                .attributes
                .iter()
                .any(|(_, value)| value.trim_start().to_ascii_lowercase().starts_with("vbscript:"));
            if declares || protocol {
                found.vbscript = true;
                let line = text::line_of(content, tag.start);
                let snippet = content[tag.start..tag.end].trim();
                found.markers.push((line, snippet.to_owned()));
            }
        }
    }
    found.markers.sort();
    found.markers.dedup();
    found.markers.truncate(3);
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::testing::{MemoryFile, inspect};
    use vbs_core::model::{Activation, Detail, FindingKind, FindingStatus};

    #[test]
    fn vbs_files_break_and_show_telling_lines() {
        let file = MemoryFile::new(
            "C:/Scripts/backup.vbs",
            "' Nightly backup\r\nOption Explicit\r\nSet fso = CreateObject(\"Scripting.FileSystemObject\")\r\nstrPwd = \"Sommer2024!\"\r\n",
        );
        let findings = inspect(&ScriptFiles, &file);
        assert_eq!(findings.len(), 2);
        let main = findings.iter().find(|f| f.rule == "VBS-101").unwrap();
        assert_eq!(main.kind, FindingKind::ScriptFile);
        assert_eq!(main.activation, Activation::Dormant);
        assert!(main.evidence.iter().any(|e| e.line == Some(3) && e.text.contains("CreateObject")));
        assert!(main.evidence.iter().all(|e| !e.text.contains("Sommer2024!")));
        let secret = findings.iter().find(|f| f.rule == CREDENTIAL_RULE).unwrap();
        assert_eq!(secret.evidence[0].line, Some(4));
    }

    #[test]
    fn startup_folder_scripts_run_automatically() {
        let file = MemoryFile::new(r"C:\ProgramData\Microsoft\Windows\Start Menu\Programs\StartUp\map.vbs", "MsgBox 1");
        assert_eq!(inspect(&ScriptFiles, &file)[0].activation, Activation::Automatic);
    }

    #[test]
    fn documents_count_only_with_vbscript() {
        let wsf = "<job>\r\n<script language=\"JScript\">WScript.Echo(1);</script>\r\n<script language=\"VBScript\">\r\nWScript.Echo 2\r\n</script>\r\n</job>";
        let findings = inspect(&ScriptFiles, &MemoryFile::new("C:/x/job.wsf", wsf));
        assert_eq!(findings[0].rule, "VBS-103");
        assert_eq!(findings[0].details["vbscriptBlocks"], Detail::Number(1));
        assert!(findings[0].evidence.iter().any(|e| e.line == Some(4) && e.text == "WScript.Echo 2"));
        let js_only = "<job><script language=\"JScript\">WScript.Echo(1);</script></job>";
        assert!(inspect(&ScriptFiles, &MemoryFile::new("C:/x/js.wsf", js_only)).is_empty());
        let hta = "<html><body><input type=button value=Go language=\"VBScript\" onclick=\"Go\"><a href=\"vbscript:Run\">x</a></body></html>";
        assert_eq!(inspect(&ScriptFiles, &MemoryFile::new("C:/x/app.hta", hta))[0].rule, "VBS-105");
        assert!(inspect(&ScriptFiles, &MemoryFile::new("C:/x/js.hta", "<script>alert(1)</script>")).is_empty());
    }

    #[test]
    fn unreadable_files_are_not_checkable() {
        struct Locked;
        impl CandidateFile for Locked {
            fn path(&self) -> &std::path::Path {
                std::path::Path::new("C:/x/locked.vbs")
            }
            fn extension(&self) -> &str {
                "vbs"
            }
            fn size(&self) -> u64 {
                1
            }
            fn modified(&self) -> Option<time::OffsetDateTime> {
                None
            }
            fn network(&self) -> bool {
                false
            }
            fn contents(&self, _: u64) -> Result<std::sync::Arc<[u8]>, vbs_core::module::ReadError> {
                Err(vbs_core::module::ReadError::Locked)
            }
            fn open(&self) -> Result<Box<dyn vbs_core::module::ReadSeek + '_>, vbs_core::module::ReadError> {
                Err(vbs_core::module::ReadError::Locked)
            }
            fn sha256(&self) -> Option<String> {
                None
            }
        }
        let mut report = Report::new();
        ScriptFiles.inspect_file(&Locked, &mut report);
        let finding = &report.findings()[0];
        assert_eq!((finding.rule.as_str(), &finding.status), ("VBS-100", &FindingStatus::NotCheckable));
    }
}
