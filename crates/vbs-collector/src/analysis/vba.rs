//! VBScript in VBA projects – what stops working when VBScript is disabled or removed, and
//! what deserves a look (rules `VBS-6xx`, sources in `docs/research-notes.md`):
//!
//! * VBScript regular expressions: `CreateObject("VBScript.RegExp")`, or a reference to the
//!   type library in `vbscript.dll` ("Microsoft VBScript Regular Expressions") – a missing
//!   reference breaks the compilation of the whole project;
//! * VBScript code run inside the macro: the Script Control (`MSScriptControl.ScriptControl`)
//!   or `execScript` of an HTML document window with the language VBScript – or a language the
//!   code does not name (`execScript` without a language runs JScript);
//! * starting VBScript: a Script Host command with a `.vbs`/`.vbe` file or `//E:VBScript`,
//!   `mshta vbscript:…`, or a `.vbs` file opened directly (`.Run`, `ShellExecute`, …) – and
//!   scripts of unknown language (`.wsf`, `.hta`, `.wsc`);
//! * objects of the Windows Script Host (`WScript.Shell`, `WScript.Network`): they stay
//!   available for JScript as far as is known, so they are only worth a review.
//!
//! The Scripting Runtime (`Scripting.FileSystemObject`, `Scripting.Dictionary`, `scrrun.dll`) is
//! not part of VBScript and is not reported; JScript is never a finding. Code is analysed per
//! logical line (continuations joined, comments removed), with line numbers as the VBA editor
//! shows them (hidden `Attribute` lines not counted).

use super::command::{self, Usage};
use super::ovba::{Project, Reference};
use super::text;

/// What a finding is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    /// VBScript regular expressions (late binding or a reference to `vbscript.dll`).
    RegExp,
    /// VBScript code run through a script engine host: the Script Control or `execScript`.
    ScriptEngine,
    /// A script engine host with a language the code does not name.
    ScriptEngineUnknown,
    /// Starts VBScript (a `.vbs`/`.vbe` file, `//E:VBScript`, `vbscript:` code).
    StartsVbScript,
    /// Starts a script whose language only its content shows.
    StartsUnknown,
    /// Uses objects of the Windows Script Host.
    WshObject,
}

/// One use of VBScript (or a related technology) in a project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Use {
    pub kind: Kind,
    /// Module the code is in; `None` for a reference of the project.
    pub module: Option<String>,
    /// Name of the reference (`VBScript_RegExp_55`) for reference findings.
    pub reference: Option<String>,
    /// What is used or started, as written (`VBScript.RegExp`, `C:\Scripts\x.vbs`, …).
    pub target: Option<String>,
    /// How a script is started (`wscript`, `cscript`, `mshta`, `direct`, …).
    pub via: Option<&'static str>,
    /// Affected lines: (line number as the VBA editor shows it, text).
    pub lines: Vec<(Option<u32>, String)>,
}

/// Type library GUIDs of `vbscript.dll`: VBScript Regular Expressions 1.0 and 5.5.
const VBSCRIPT_TYPE_LIBRARIES: [&str; 2] =
    ["3F4DACA0-160D-11D2-A8E9-00104B365C9F", "3F4DACA7-160D-11D2-A8E9-00104B365C9F"];

/// Most evidence lines collected per finding (the report keeps fewer).
const MAX_LINES: usize = 8;

/// A reference to a type library of `vbscript.dll` (by GUID or by file).
pub fn is_vbscript_library(reference: &Reference) -> bool {
    let guid_matches = reference.guid().is_some_and(|guid| VBSCRIPT_TYPE_LIBRARIES.contains(&guid.as_str()));
    let file_matches = reference.path().is_some_and(|path| {
        let lower = path.to_ascii_lowercase().replace('/', "\\");
        let file = lower.rsplit('\\').find(|part| !part.chars().all(|c| c.is_ascii_digit())).unwrap_or(&lower);
        file == "vbscript.dll"
    });
    guid_matches || file_matches
}

/// Finds the uses of VBScript in a project; modules without readable source are skipped
/// (the caller reports them as not checkable).
pub fn analyze(project: &Project) -> Vec<Use> {
    let mut uses = Vec::new();
    let regexp_reference = project.references.iter().find(|reference| is_vbscript_library(reference));
    let mut early_bound_lines: Vec<(Option<u32>, String)> = Vec::new();
    for module in &project.modules {
        let Ok(source) = &module.source else { continue };
        let lines = logical_lines(source);
        analyze_module(&module.name, &lines, &mut uses);
        if regexp_reference.is_some() {
            for line in &lines {
                if uses_regexp_types(&line.code_lower()) && early_bound_lines.len() < MAX_LINES {
                    early_bound_lines
                        .push((Some(line.number), format!("{}:{}: {}", module.name, line.number, line.text)));
                }
            }
        }
    }
    for reference in project.references.iter().filter(|reference| is_vbscript_library(reference)) {
        let mut lines = vec![(None, reference_line(reference))];
        lines.extend(early_bound_lines.iter().cloned());
        uses.push(Use {
            kind: Kind::RegExp,
            module: None,
            reference: Some(if reference.name.is_empty() { "VBScript".to_owned() } else { reference.name.clone() }),
            target: reference.path().map(str::to_owned).or_else(|| Some(reference.libid.clone())),
            via: None,
            lines,
        });
    }
    uses
}

fn reference_line(reference: &Reference) -> String {
    match (reference.description(), reference.path()) {
        (Some(description), Some(path)) => format!("Reference: {description} ({path})"),
        (Some(description), None) => format!("Reference: {description}"),
        _ => format!("Reference: {}", reference.libid),
    }
}

fn analyze_module(module: &str, lines: &[LogicalLine], uses: &mut Vec<Use>) {
    let mut regexp: Vec<&LogicalLine> = Vec::new();
    let mut script_control: Vec<&LogicalLine> = Vec::new();
    let mut languages: Vec<(&LogicalLine, Option<String>)> = Vec::new();
    let mut exec_script: Vec<(&LogicalLine, Option<String>)> = Vec::new();
    let mut wsh: Vec<&LogicalLine> = Vec::new();
    let mut wsh_target: Option<String> = None;
    let mut starts: Vec<(Kind, &LogicalLine, command::Reference)> = Vec::new();
    for line in lines {
        let code = line.code_lower();
        let literals: Vec<String> = line.literals().map(|literal| literal.trim().to_ascii_lowercase()).collect();
        // Late binding; early-bound types (`VBScript_RegExp_55.RegExp`) need the reference,
        // which is reported for the project with these lines as evidence.
        if literals.iter().any(|literal| literal == "vbscript.regexp" || literal.starts_with("vbscript.regexp.")) {
            regexp.push(line);
        }
        if literals.iter().any(|literal| literal.contains("scriptcontrol")) || uses_script_control_type(&code) {
            script_control.push(line);
        }
        if let Some(language) = language_assignment(line) {
            languages.push((line, language));
        }
        if let Some(language) = exec_script_language(line) {
            exec_script.push((line, language));
        }
        let wsh_literal = line.literals().find(|literal| is_wsh_prog_id(literal));
        if wsh_literal.is_some() || uses_wsh_types(&code) {
            if wsh_target.is_none() {
                wsh_target = Some(wsh_literal.map_or_else(|| "IWshRuntimeLibrary".to_owned(), |l| l.trim().to_owned()));
            }
            wsh.push(line);
        }
        for reference in script_starts(line) {
            let kind = if reference.usage == Usage::VbScript { Kind::StartsVbScript } else { Kind::StartsUnknown };
            starts.push((kind, line, reference));
        }
    }
    let evidence = |lines: &[&LogicalLine]| -> Vec<(Option<u32>, String)> {
        lines.iter().take(MAX_LINES).map(|line| (Some(line.number), line.text.clone())).collect()
    };
    if !regexp.is_empty() {
        uses.push(Use {
            kind: Kind::RegExp,
            module: Some(module.to_owned()),
            reference: None,
            target: Some("VBScript.RegExp".to_owned()),
            via: None,
            lines: evidence(&regexp),
        });
    }
    if !script_control.is_empty() {
        // The Script Control accepts "VBScript" and "VBS".
        let vbscript = languages.iter().any(|(_, language)| matches!(language.as_deref(), Some("vbscript" | "vbs")));
        let unknown = languages.is_empty() || languages.iter().any(|(_, language)| language.is_none());
        let kind = if vbscript {
            Some(Kind::ScriptEngine)
        } else if unknown {
            Some(Kind::ScriptEngineUnknown)
        } else {
            None // JScript or another named language
        };
        if let Some(kind) = kind {
            let mut lines: Vec<&LogicalLine> = script_control.clone();
            for (line, _) in &languages {
                if !lines.iter().any(|known| known.number == line.number) {
                    lines.push(line);
                }
            }
            lines.sort_by_key(|line| line.number);
            uses.push(Use {
                kind,
                module: Some(module.to_owned()),
                reference: None,
                target: Some("MSScriptControl.ScriptControl".to_owned()),
                via: None,
                lines: evidence(&lines),
            });
        }
    }
    // `execScript` of an HTML document window: JScript unless the call names another language.
    let exec_vbscript: Vec<&LogicalLine> = exec_script
        .iter()
        .filter(|(_, language)| matches!(language.as_deref(), Some("vbscript" | "vbs")))
        .map(|(line, _)| *line)
        .collect();
    let exec_unknown: Vec<&LogicalLine> =
        exec_script.iter().filter(|(_, language)| language.is_none()).map(|(line, _)| *line).collect();
    let (kind, exec_lines) = if !exec_vbscript.is_empty() {
        (Some(Kind::ScriptEngine), exec_vbscript)
    } else if !exec_unknown.is_empty() {
        (Some(Kind::ScriptEngineUnknown), exec_unknown)
    } else {
        (None, Vec::new())
    };
    if let Some(kind) = kind {
        uses.push(Use {
            kind,
            module: Some(module.to_owned()),
            reference: None,
            target: Some("execScript".to_owned()),
            via: None,
            lines: evidence(&exec_lines),
        });
    }
    // One finding per started script (the first line that starts it).
    let mut seen: Vec<(Kind, Option<String>)> = Vec::new();
    for (kind, line, reference) in &starts {
        let target = reference.script.clone().filter(|script| script.trim() != PLACEHOLDER);
        if seen.contains(&(*kind, target.clone())) {
            continue;
        }
        // "Starts VBScript" wins over "unknown language" for the same line.
        if *kind == Kind::StartsUnknown
            && starts.iter().any(|(other, same, _)| *other == Kind::StartsVbScript && same.number == line.number)
        {
            continue;
        }
        seen.push((*kind, target.clone()));
        uses.push(Use {
            kind: *kind,
            module: Some(module.to_owned()),
            reference: None,
            target,
            via: Some(reference.via),
            lines: vec![(Some(line.number), line.text.clone())],
        });
    }
    if !wsh.is_empty() {
        uses.push(Use {
            kind: Kind::WshObject,
            module: Some(module.to_owned()),
            reference: None,
            target: wsh_target,
            via: None,
            lines: evidence(&wsh),
        });
    }
}

// ---- lines ---------------------------------------------------------------------------------

/// Stands for a part of a command that is not a string literal (a variable, a function call).
const PLACEHOLDER: &str = "…";

/// A statement line: continuations joined, the comment removed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogicalLine {
    /// Line number as the VBA editor shows it (without the hidden `Attribute` lines).
    pub number: u32,
    /// The line as written (continuations joined).
    pub text: String,
    /// Code and string literals, without the comment.
    segments: Vec<Segment>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Segment {
    Code(String),
    Literal(String),
}

impl LogicalLine {
    /// The code without literals, lower case.
    fn code_lower(&self) -> String {
        self.segments
            .iter()
            .map(|segment| match segment {
                Segment::Code(code) => code.to_ascii_lowercase(),
                Segment::Literal(_) => " \"\" ".to_owned(),
            })
            .collect()
    }

    fn literals(&self) -> impl Iterator<Item = &str> {
        self.segments.iter().filter_map(|segment| match segment {
            Segment::Literal(text) => Some(text.as_str()),
            Segment::Code(_) => None,
        })
    }

    /// Whether the line contains code at all (not only a comment).
    pub fn is_code(&self) -> bool {
        self.segments.iter().any(|segment| match segment {
            Segment::Code(code) => !code.trim().is_empty(),
            Segment::Literal(_) => true,
        })
    }
}

/// The logical code lines of a module's source.
pub fn logical_lines(source: &str) -> Vec<LogicalLine> {
    let mut lines = Vec::new();
    let mut visible = 0u32;
    let mut pending: Option<(u32, String)> = None;
    for (_, physical) in text::lines(source) {
        let trimmed = physical.trim_start();
        if pending.is_none() && is_attribute(trimmed) {
            continue;
        }
        visible += 1;
        let continued = pending.is_some();
        let (number, mut joined) = pending.take().unwrap_or((visible, String::new()));
        let content = if continued { physical.trim() } else { physical.trim_end() };
        match content.strip_suffix('_').filter(|rest| rest.ends_with([' ', '\t'])) {
            Some(rest) => {
                joined.push_str(rest.trim_end());
                joined.push(' ');
                pending = Some((number, joined));
            }
            None => {
                joined.push_str(content);
                let segments = split(&joined);
                let line = LogicalLine { number, text: joined.trim().to_owned(), segments };
                if line.is_code() {
                    lines.push(line);
                }
            }
        }
    }
    if let Some((number, joined)) = pending {
        let segments = split(&joined);
        let line = LogicalLine { number, text: joined.trim().to_owned(), segments };
        if line.is_code() {
            lines.push(line);
        }
    }
    lines
}

/// Hidden lines such as `Attribute VB_Name = "Module1"` or `Attribute Foo.VB_UserMemId = 0`.
fn is_attribute(line: &str) -> bool {
    line.get(..10).is_some_and(|start| start.eq_ignore_ascii_case("attribute ")) && line.contains("VB_")
}

/// Splits a line into code and string literals (`""` is a quote inside a literal) and drops
/// the comment: `'` outside a literal, or a `Rem` statement.
fn split(line: &str) -> Vec<Segment> {
    let mut segments = Vec::new();
    let mut code = String::new();
    let mut chars = line.chars().peekable();
    let mut statement_start = true;
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                if !code.is_empty() {
                    segments.push(Segment::Code(std::mem::take(&mut code)));
                }
                let mut literal = String::new();
                loop {
                    match chars.next() {
                        Some('"') if chars.peek() == Some(&'"') => {
                            chars.next();
                            literal.push('"');
                        }
                        Some('"') | None => break,
                        Some(other) => literal.push(other),
                    }
                }
                segments.push(Segment::Literal(literal));
                statement_start = false;
            }
            '\'' => break,
            ':' => {
                code.push(c);
                statement_start = true;
            }
            c if c.is_whitespace() => code.push(c),
            _ => {
                if statement_start && (c == 'R' || c == 'r') {
                    let rest: String = std::iter::once(c).chain(chars.clone().take(3)).collect();
                    let word = rest.to_ascii_lowercase();
                    if word == "rem" || word.starts_with("rem ") || word.starts_with("rem\t") {
                        break;
                    }
                }
                statement_start = false;
                code.push(c);
            }
        }
    }
    if !code.is_empty() {
        segments.push(Segment::Code(code));
    }
    segments
}

// ---- detectors -----------------------------------------------------------------------------

/// Type names of the VBScript regular expressions, early bound (needs the reference).
fn uses_regexp_types(code: &str) -> bool {
    if code.contains("vbscript_regexp_55.") || code.contains("vbscript_regexp_10.") {
        return true;
    }
    let words: Vec<&str> = words(code).collect();
    words.windows(2).any(|pair| {
        matches!(pair[0], "as" | "new") && matches!(pair[1], "regexp" | "matchcollection" | "match" | "submatches")
    })
}

/// `New ScriptControl`, `As ScriptControl`, `MSScriptControl.ScriptControl`.
fn uses_script_control_type(code: &str) -> bool {
    if code.contains("msscriptcontrol.") {
        return true;
    }
    let words: Vec<&str> = words(code).collect();
    words.windows(2).any(|pair| matches!(pair[0], "as" | "new") && pair[1] == "scriptcontrol")
}

/// `WScript.Shell`, `WScript.Network` and the other ProgIDs of the Windows Script Host.
fn is_wsh_prog_id(literal: &str) -> bool {
    let lower = literal.trim().to_ascii_lowercase();
    let base = lower.strip_suffix(".1").unwrap_or(&lower);
    matches!(base, "wscript.shell" | "wscript.network" | "wscript.signer" | "wshcontroller")
}

/// Early-bound types of the "Windows Script Host Object Model" (`IWshRuntimeLibrary`).
fn uses_wsh_types(code: &str) -> bool {
    if code.contains("iwshruntimelibrary.") {
        return true;
    }
    let words: Vec<&str> = words(code).collect();
    words.windows(2).any(|pair| {
        matches!(pair[0], "as" | "new")
            && matches!(
                pair[1].trim_end_matches(|c: char| c.is_ascii_digit()),
                "wshshell"
                    | "iwshshell"
                    | "wshnetwork"
                    | "iwshnetwork"
                    | "wshshortcut"
                    | "iwshshortcut"
                    | "wshexec"
                    | "iwshexec"
            )
    })
}

/// Identifiers (with qualifiers such as `IWshRuntimeLibrary.WshShell`) of a lower-case code text.
fn words(code: &str) -> impl Iterator<Item = &str> {
    code.split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '.')).filter(|word| !word.is_empty())
}

/// `.Language = "VBScript"` – `Some(Some(language))` for a literal (lower case), `Some(None)`
/// when the language comes from a variable or expression.
fn language_assignment(line: &LogicalLine) -> Option<Option<String>> {
    let mut segments = line.segments.iter().peekable();
    while let Some(segment) = segments.next() {
        let Segment::Code(code) = segment else { continue };
        let lower = code.to_ascii_lowercase();
        let Some(at) = lower.find(".language") else { continue };
        let rest = lower[at + ".language".len()..].trim_start();
        if !rest.starts_with('=') {
            continue;
        }
        let after = rest[1..].trim();
        if !after.is_empty() {
            return Some(None); // a variable or an expression
        }
        return Some(match segments.peek() {
            Some(Segment::Literal(language)) => Some(language.trim().to_ascii_lowercase()),
            _ => None,
        });
    }
    None
}

/// The language argument of an `execScript` call (`window.execScript code, "VBScript"`):
/// `Some(Some(language))` for a literal (lower case), `Some(None)` when it comes from a variable
/// or expression; `None` without a language (JScript by default) or without such a call.
fn exec_script_language(line: &LogicalLine) -> Option<Option<String>> {
    // The line after `execScript`, literals replaced by their index.
    let mut after: Option<String> = None;
    let mut literals: Vec<&str> = Vec::new();
    for segment in &line.segments {
        match segment {
            Segment::Code(code) => {
                let lower = code.to_ascii_lowercase();
                match after.as_mut() {
                    Some(text) => text.push_str(&lower),
                    None => {
                        if let Some(at) = lower.find("execscript") {
                            after = Some(lower[at + "execscript".len()..].to_owned());
                        }
                    }
                }
            }
            Segment::Literal(literal) => {
                if let Some(text) = after.as_mut() {
                    text.push_str(&format!("\u{1}{}\u{1}", literals.len()));
                    literals.push(literal);
                }
            }
        }
    }
    let after = after?;
    // Arguments at the top level of the call: code, language.
    let mut depth = 0i32;
    let mut arguments: Vec<String> = vec![String::new()];
    for c in after.trim_start().trim_start_matches('(').chars() {
        match c {
            '(' => depth += 1,
            ')' if depth == 0 => break,
            ')' => depth -= 1,
            ',' if depth == 0 => {
                arguments.push(String::new());
                continue;
            }
            _ => {}
        }
        if let Some(last) = arguments.last_mut() {
            last.push(c);
        }
    }
    let language = arguments.get(1)?.trim();
    if language.is_empty() {
        return None;
    }
    let literal = language
        .strip_prefix('\u{1}')
        .and_then(|rest| rest.strip_suffix('\u{1}'))
        .and_then(|index| index.parse::<usize>().ok())
        .and_then(|index| literals.get(index));
    Some(literal.map(|literal| literal.trim().to_ascii_lowercase()))
}

/// Commands in the literals of a line that start VBScript or a script of unknown language.
/// A `.vbs` file named on its own only counts where the line starts something (`.Run`,
/// `ShellExecute`, …) – writing, copying or deleting a script does not run it.
fn script_starts(line: &LogicalLine) -> Vec<command::Reference> {
    let code = line.code_lower();
    let starts_something = ["shell", ".run", ".exec", "winexec", "createprocess", "followhyperlink", ".create"]
        .iter()
        .any(|keyword| code.contains(keyword));
    let mut found: Vec<command::Reference> = Vec::new();
    for candidate in commands(line) {
        for reference in command::analyze(&candidate) {
            if reference.via == "direct" && !starts_something {
                continue;
            }
            if !found.contains(&reference) {
                found.push(reference);
            }
        }
    }
    found
}

/// Command texts of a line: every expression of literals joined by `&`/`+` (other parts of the
/// expression become a placeholder, `Chr(34)` a quote); a literal that is only part of such an
/// expression is not a command on its own.
fn commands(line: &LogicalLine) -> Vec<String> {
    let mut commands: Vec<String> = Vec::new();
    let mut chain = String::new();
    let push = |chain: &mut String, commands: &mut Vec<String>| {
        if !chain.is_empty() && chain.as_str() != PLACEHOLDER && !commands.contains(chain) {
            commands.push(std::mem::take(chain));
        }
        chain.clear();
    };
    for segment in &line.segments {
        match segment {
            Segment::Literal(text) => chain.push_str(text),
            Segment::Code(code) => {
                let trimmed = code.trim();
                let concatenates = trimmed.starts_with(['&', '+']) || trimmed.ends_with(['&', '+']);
                if !chain.is_empty() && concatenates {
                    let parts: Vec<&str> = trimmed.split(['&', '+']).map(str::trim).collect();
                    for (index, part) in parts.iter().enumerate() {
                        if part.is_empty() {
                            continue;
                        }
                        let lower = part.to_ascii_lowercase().replace(' ', "");
                        let quote = matches!(lower.as_str(), "chr(34)" | "chr$(34)" | "chrw(34)" | "chrw$(34)");
                        let space = matches!(lower.as_str(), "vbcrlf" | "vbnewline" | "vbtab" | "vbcr" | "vblf");
                        // Only the parts between `&` belong to the expression.
                        if index == parts.len() - 1 && !trimmed.ends_with(['&', '+']) {
                            push(&mut chain, &mut commands);
                            break;
                        }
                        chain.push_str(if quote {
                            "\""
                        } else if space {
                            " "
                        } else {
                            PLACEHOLDER
                        });
                    }
                } else {
                    push(&mut chain, &mut commands);
                    // An expression may start with a variable: `folder & "\x.vbs"`.
                    if trimmed.ends_with(['&', '+']) {
                        let before = trimmed.trim_end_matches(['&', '+']).trim_end();
                        let operand = before.rsplit(|c: char| c.is_whitespace() || c == '(' || c == ',').next();
                        if operand.is_some_and(|operand| !operand.is_empty()) {
                            chain.push_str(PLACEHOLDER);
                        }
                    }
                }
            }
        }
    }
    push(&mut chain, &mut commands);
    commands
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::ovba::{Module, Project, ReferenceKind};

    fn project(modules: &[(&str, &str)], references: &[Reference]) -> Project {
        Project {
            name: Some("VBAProject".into()),
            code_page: 1252,
            references: references.to_vec(),
            modules: modules
                .iter()
                .map(|(name, source)| Module {
                    name: (*name).into(),
                    stream: (*name).into(),
                    procedural: true,
                    source: Ok((*source).into()),
                })
                .collect(),
            protection: Default::default(),
        }
    }

    fn kinds(uses: &[Use]) -> Vec<(Kind, Option<String>, Option<String>)> {
        uses.iter().map(|u| (u.kind, u.module.clone().or_else(|| u.reference.clone()), u.target.clone())).collect()
    }

    const HEADER: &str = "Attribute VB_Name = \"Module1\"\r\n";

    #[test]
    fn logical_lines_follow_the_editor() {
        let source = format!(
            "{HEADER}Option Explicit\r\n' comment only\r\nSub A() ' trailing\r\n    x = \"it's \"\"quoted\"\"\" _\r\n        & y\r\nRem remark\r\n    Remarks = 1 : Rem later\r\nEnd Sub\r\n"
        );
        let lines = logical_lines(&source);
        let summary: Vec<(u32, &str)> = lines.iter().map(|l| (l.number, l.text.as_str())).collect();
        assert_eq!(summary[0], (1, "Option Explicit"));
        assert_eq!(summary[1], (3, "Sub A() ' trailing"));
        assert_eq!(summary[2], (4, "x = \"it's \"\"quoted\"\"\" & y"));
        assert_eq!(summary[3].0, 7);
        assert_eq!(lines[2].literals().collect::<Vec<_>>(), ["it's \"quoted\""]);
        assert!(lines.iter().all(|l| !l.code_lower().contains("remark ") && !l.code_lower().contains("comment")));
        assert!(lines[3].code_lower().contains("remarks = 1"));
    }

    #[test]
    fn regexp_late_and_early_bound() {
        let late = format!("{HEADER}Sub A()\r\n    Set re = CreateObject(\"VBScript.RegExp\")\r\nEnd Sub\r\n");
        let uses = analyze(&project(&[("Module1", &late)], &[]));
        assert_eq!(kinds(&uses), [(Kind::RegExp, Some("Module1".into()), Some("VBScript.RegExp".into()))]);
        assert_eq!(uses[0].lines, [(Some(2), "Set re = CreateObject(\"VBScript.RegExp\")".to_owned())]);

        let reference = Reference {
            name: "VBScript_RegExp_55".into(),
            kind: ReferenceKind::Registered,
            libid: r"*\G{3F4DACA7-160D-11D2-A8E9-00104B365C9F}#5.5#0#C:\Windows\System32\vbscript.dll\3#Microsoft VBScript Regular Expressions 5.5".into(),
        };
        let early = format!("{HEADER}Sub A()\r\n    Dim re As New RegExp\r\nEnd Sub\r\n");
        let uses = analyze(&project(&[("Module1", &early)], std::slice::from_ref(&reference)));
        assert_eq!(
            kinds(&uses),
            [(Kind::RegExp, Some("VBScript_RegExp_55".into()), Some(r"C:\Windows\System32\vbscript.dll\3".into()))]
        );
        assert_eq!(uses[0].lines[1], (Some(2), "Module1:2: Dim re As New RegExp".to_owned()));
        // Without the reference, `New RegExp` is VBA's own class (Office 2508 and later).
        assert!(analyze(&project(&[("Module1", &early)], &[])).is_empty());
    }

    #[test]
    fn script_control_by_language() {
        let vbscript = format!(
            "{HEADER}Sub A()\r\n    Set sc = CreateObject(\"MSScriptControl.ScriptControl\")\r\n    sc.Language = \"VBScript\"\r\n    sc.AddCode code\r\nEnd Sub\r\n"
        );
        let jscript = vbscript.replace("\"VBScript\"", "\"JScript\"");
        let variable = vbscript.replace("\"VBScript\"", "lang");
        let early = format!("{HEADER}Dim sc As New ScriptControl\r\n");
        assert_eq!(analyze(&project(&[("M", &vbscript)], &[]))[0].kind, Kind::ScriptEngine);
        assert_eq!(analyze(&project(&[("M", &vbscript)], &[]))[0].lines.len(), 2);
        assert!(analyze(&project(&[("M", &jscript)], &[])).is_empty());
        assert_eq!(analyze(&project(&[("M", &variable)], &[]))[0].kind, Kind::ScriptEngineUnknown);
        assert_eq!(analyze(&project(&[("M", &early)], &[]))[0].kind, Kind::ScriptEngineUnknown);
    }

    #[test]
    fn exec_script_by_language() {
        let module = |call: &str| {
            format!("{HEADER}Sub A()\r\n    Set html = CreateObject(\"htmlfile\")\r\n    {call}\r\nEnd Sub\r\n")
        };
        let kinds_of = |call: &str| -> Vec<(Kind, Option<String>)> {
            analyze(&project(&[("M", &module(call))], &[])).into_iter().map(|u| (u.kind, u.target)).collect()
        };
        let exec = Some("execScript".to_owned());
        assert_eq!(
            kinds_of("html.parentWindow.execScript \"MsgBox 1\", \"VBScript\""),
            [(Kind::ScriptEngine, exec.clone())]
        );
        assert_eq!(
            kinds_of("Call html.parentWindow.execScript(code, \"vbscript\")"),
            [(Kind::ScriptEngine, exec.clone())]
        );
        assert_eq!(kinds_of("html.parentWindow.execScript code, language"), [(Kind::ScriptEngineUnknown, exec)]);
        assert!(kinds_of("html.parentWindow.execScript \"var x = f(1, 2);\"").is_empty(), "JScript by default");
        assert!(kinds_of("html.parentWindow.execScript Replace(code, \",\", \";\"), \"JScript\"").is_empty());
    }

    #[test]
    fn starting_scripts() {
        let source = format!(
            "{HEADER}Sub A()\r\n    Shell \"wscript.exe \"\"C:\\Scripts\\logon.vbs\"\"\", vbHide\r\n    CreateObject(\"WScript.Shell\").Run \"\\\\srv\\share\\map.vbs\"\r\n    Shell \"cscript //nologo \"\"\" & folder & \"\\report.vbs\"\"\"\r\n    Shell \"mshta.exe \" & Chr(34) & \"C:\\Tools\\menu.hta\" & Chr(34)\r\n    Open \"C:\\Temp\\gen.vbs\" For Output As #1\r\n    Shell \"cmd /c echo hello\"\r\nEnd Sub\r\n"
        );
        let uses = analyze(&project(&[("Module1", &source)], &[]));
        let summary: Vec<(Kind, Option<String>, Option<&str>)> =
            uses.iter().map(|u| (u.kind, u.target.clone(), u.via)).collect();
        assert_eq!(
            summary,
            [
                (Kind::StartsVbScript, Some(r"C:\Scripts\logon.vbs".into()), Some("wscript")),
                (Kind::StartsVbScript, Some(r"\\srv\share\map.vbs".into()), Some("direct")),
                (Kind::StartsVbScript, Some(r"…\report.vbs".into()), Some("cscript")),
                (Kind::StartsUnknown, Some(r"C:\Tools\menu.hta".into()), Some("mshta")),
                (Kind::WshObject, Some("WScript.Shell".into()), None),
            ]
        );
        assert_eq!(uses[0].lines[0].0, Some(2));
    }

    #[test]
    fn not_vbscript() {
        let source = format!(
            "{HEADER}Sub A()\r\n    ' Set re = CreateObject(\"VBScript.RegExp\")\r\n    Set fso = CreateObject(\"Scripting.FileSystemObject\")\r\n    Set d = CreateObject(\"Scripting.Dictionary\")\r\n    MsgBox \"Run the report.vbs later\"\r\n    Kill \"C:\\Temp\\old.vbs\"\r\n    Shell \"cscript //E:JScript run.js\"\r\n    Dim re As New RegExp\r\nEnd Sub\r\n"
        );
        assert!(
            analyze(&project(&[("Module1", &source)], &[])).is_empty(),
            "{:?}",
            analyze(&project(&[("Module1", &source)], &[]))
        );
    }

    #[test]
    fn windows_script_host_objects() {
        let early = format!("{HEADER}Dim sh As IWshRuntimeLibrary.WshShell\r\nDim net As New WshNetwork\r\n");
        let uses = analyze(&project(&[("Module1", &early)], &[]));
        assert_eq!(kinds(&uses), [(Kind::WshObject, Some("Module1".into()), Some("IWshRuntimeLibrary".into()))]);
        assert_eq!(uses[0].lines.len(), 2);
    }

    #[test]
    fn vbscript_libraries_by_guid_or_file() {
        let by_file = Reference {
            name: "VBScript_RegExp_10".into(),
            kind: ReferenceKind::Registered,
            libid: r"*\G{00000000-0000-0000-0000-000000000000}#1.0#0#C:\WINNT\System32\VBSCRIPT.DLL\2#Microsoft VBScript Regular Expressions".into(),
        };
        let scrrun = Reference {
            name: "Scripting".into(),
            kind: ReferenceKind::Registered,
            libid: r"*\G{420B2830-E718-11CF-893D-00A0C9054228}#1.0#0#C:\Windows\System32\scrrun.dll#Microsoft Scripting Runtime".into(),
        };
        assert!(is_vbscript_library(&by_file));
        assert!(!is_vbscript_library(&scrrun));
    }
}
