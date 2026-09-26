//! Lines of scripts: which are code (not comments or `echo` output) and which make good
//! evidence – only a few affected lines are ever kept (see `vbs_core::validate::limits`).

use vbs_core::validate::limits::MAX_EVIDENCE_LINES;

use super::text;

/// Languages whose lines are analysed for calls of VBScript.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    /// `.bat`, `.cmd`
    Batch,
    /// `.ps1`
    PowerShell,
    /// `.kix` (KiXtart logon scripts)
    KiXtart,
    /// VBScript itself (`.vbs`, script blocks of `.wsf`/`.hta`/`.wsc`)
    VbScript,
}

/// Non-empty lines that are code: comments, `echo` lines and PowerShell block comments are left out.
pub fn code_lines(content: &str, language: Language) -> Vec<(u32, &str)> {
    let mut lines = Vec::new();
    let mut in_block_comment = false;
    for (number, line) in text::lines(content) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let lower = trimmed.to_ascii_lowercase();
        let comment = match language {
            Language::Batch => {
                let command = lower.trim_start_matches('@').trim_start();
                command == "rem"
                    || command.starts_with("rem ")
                    || command.starts_with("rem\t")
                    || command.starts_with("::")
                    || command == "echo"
                    || command.starts_with("echo ")
                    || command.starts_with("echo.")
                    || command.starts_with("echo\t")
            }
            Language::PowerShell => {
                if in_block_comment {
                    if trimmed.contains("#>") {
                        in_block_comment = false;
                    }
                    true
                } else if trimmed.starts_with("<#") {
                    in_block_comment = !trimmed.contains("#>");
                    true
                } else {
                    trimmed.starts_with('#')
                }
            }
            Language::KiXtart => trimmed.starts_with(';'),
            Language::VbScript => trimmed.starts_with('\'') || lower == "rem" || lower.starts_with("rem "),
        };
        if !comment {
            lines.push((number, line));
        }
    }
    lines
}

/// Keywords of lines that show what a VBScript does with the system.
const SIGNALS: [&str; 10] = [
    "createobject",
    "getobject",
    "wscript.",
    ".run ",
    ".run(",
    ".exec",
    "executeglobal",
    "shellexecute",
    ".regwrite",
    "mapnetworkdrive",
];

/// Up to [`MAX_EVIDENCE_LINES`] lines of VBScript code: telling lines (`CreateObject`,
/// `WScript.…`, `.Run`, …) first, then the first code lines; in source order.
pub fn vbscript_evidence<'a>(lines: &[(u32, &'a str)]) -> Vec<(u32, &'a str)> {
    let signal = |line: &str| {
        let lower = line.to_ascii_lowercase();
        SIGNALS.iter().any(|keyword| lower.contains(keyword))
    };
    let mut chosen: Vec<(u32, &str)> =
        lines.iter().copied().filter(|(_, line)| signal(line)).take(MAX_EVIDENCE_LINES).collect();
    for line in lines {
        if chosen.len() >= MAX_EVIDENCE_LINES {
            break;
        }
        if !chosen.contains(line) {
            chosen.push(*line);
        }
    }
    chosen.sort_by_key(|(number, _)| *number);
    chosen
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leaves_out_comments_and_echo() {
        let batch = "@echo off\r\nrem cscript old.vbs\r\n:: wscript x.vbs\r\necho Run cscript x.vbs\r\ncscript //nologo run.vbs\r\n";
        assert_eq!(code_lines(batch, Language::Batch), [(5, "cscript //nologo run.vbs")]);
        let ps = "# cscript a.vbs\r\n<#\r\nwscript b.vbs\r\n#>\r\n& cscript c.vbs\r\n";
        assert_eq!(code_lines(ps, Language::PowerShell), [(5, "& cscript c.vbs")]);
        let vbs = "' comment\r\nRem also\r\nremark = 1\r\n";
        assert_eq!(code_lines(vbs, Language::VbScript), [(3, "remark = 1")]);
    }

    #[test]
    fn evidence_prefers_telling_lines() {
        let code: Vec<(u32, &str)> = vec![
            (1, "Option Explicit"),
            (2, "Dim a, b"),
            (3, "Set fso = CreateObject(\"Scripting.FileSystemObject\")"),
            (4, "a = 1"),
            (5, "b = 2"),
            (6, "c = 3"),
            (7, "WScript.Quit 0"),
        ];
        let chosen: Vec<u32> = vbscript_evidence(&code).iter().map(|(n, _)| *n).collect();
        assert_eq!(chosen, [1, 2, 3, 4, 7]);
    }
}
