//! Recognises VBScript in command lines – registry values, task actions, shortcut targets,
//! service image paths, WMI command-line consumers and lines of batch or PowerShell scripts.
//!
//! A command runs VBScript when it
//! * starts the Windows Script Host (`wscript`/`cscript`) with a `.vbs`/`.vbe` file or the
//!   engine switch `//E:VBScript`,
//! * starts a `.vbs`/`.vbe` file directly (Windows opens it with the Script Host), or
//! * contains inline code of the `vbscript:` protocol (`mshta vbscript:…`, `rundll32 … vbscript:…`).
//!
//! A `.wsf`, `.hta` or `.wsc` file, or the Script Host with a file of another type, may or may
//! not be VBScript – only the content tells ([`Usage::Unknown`], reported as "review").
//! JScript (`.js`, `.jse`, `//E:JScript`, `javascript:`) is not a VBScript dependency.
//!
//! The analysis looks through `cmd /c`, `start`, `call`, PowerShell (`-Command`,
//! `-EncodedCommand`, `Start-Process`, `&`) and quoted inner commands (e.g. `schtasks /tr "…"`,
//! `$shell.Run("…")`), a few levels deep. It never resolves or reads files itself.

use super::text;

/// How a command involves VBScript.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Usage {
    /// Certainly VBScript.
    VbScript,
    /// A script whose language only its content shows.
    Unknown,
}

/// One use of VBScript (or of a script of unknown language) in a command.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Reference {
    pub usage: Usage,
    /// The script as written (quotes removed), if the command names one.
    pub script: Option<String>,
    /// How it is started: `wscript`, `cscript`, `mshta`, `rundll32`, `direct` or `inline`.
    pub via: &'static str,
}

/// How deep quoted and nested commands are followed.
const MAX_DEPTH: u8 = 4;

/// Analyses one command line.
pub fn analyze(command: &str) -> Vec<Reference> {
    let mut found = Vec::new();
    analyze_into(command, 0, &mut found);
    found.sort();
    found.dedup();
    found
}

/// Analyses a program and its arguments given separately (task actions, shortcuts, WMI consumers).
pub fn analyze_parts(program: &str, arguments: &str) -> Vec<Reference> {
    let program = program.trim().trim_matches('"');
    if program.is_empty() {
        return analyze(arguments);
    }
    analyze(&format!("\"{program}\" {arguments}"))
}

/// Script files of other languages (batch, PowerShell, KiXtart) a command names, as written –
/// the places where a call of VBScript may hide one level further down.
pub fn called_scripts(command: &str) -> Vec<String> {
    let mut found = Vec::new();
    collect_called_scripts(command, 0, &mut found);
    found.dedup();
    found
}

fn collect_called_scripts(command: &str, depth: u8, out: &mut Vec<String>) {
    if depth > MAX_DEPTH {
        return;
    }
    for segment in segments(command) {
        let (tokens, quoted_parts) = tokenize(segment);
        for token in &tokens {
            let candidate = clean(&token.text);
            let spaced = candidate.contains(char::is_whitespace);
            if spaced && !looks_like_path(candidate) {
                continue;
            }
            if matches!(text::extension(candidate).as_deref(), Some("bat" | "cmd" | "ps1" | "kix"))
                && !out.iter().any(|f| f == candidate)
            {
                out.push(candidate.to_owned());
            }
        }
        for part in quoted_parts {
            if part.contains(char::is_whitespace) && !looks_like_path(&part) {
                collect_called_scripts(&part, depth + 1, out);
            }
        }
    }
}

/// `C:\…`, `\\server\…` or `%VAR%\…` without option-like words inside.
fn looks_like_path(text: &str) -> bool {
    let bytes = text.as_bytes();
    let rooted = (bytes.len() > 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic())
        || text.starts_with("\\\\")
        || text.starts_with('%');
    rooted && !text.contains(" /") && !text.contains(" -")
}

/// The strongest usage among `references`, if any.
pub fn strongest(references: &[Reference]) -> Option<Usage> {
    references.iter().map(|reference| reference.usage).min()
}

fn analyze_into(command: &str, depth: u8, out: &mut Vec<Reference>) {
    if depth > MAX_DEPTH || command.trim().is_empty() {
        return;
    }
    let lower = command.to_ascii_lowercase();
    if lower.contains("vbscript:") {
        let via = if lower.contains("mshta") {
            "mshta"
        } else if lower.contains("rundll32") {
            "rundll32"
        } else {
            "inline"
        };
        out.push(Reference { usage: Usage::VbScript, script: None, via });
    }
    for segment in segments(command) {
        analyze_segment(segment, depth, out);
    }
}

/// Splits at `&&`, `||`, `&`, `|`, `;` and line breaks outside quotes.
fn segments(command: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut quote: Option<char> = None;
    let mut start = 0;
    let mut previous = ' ';
    for (i, c) in command.char_indices() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None if c == '"' => quote = Some(c),
            None if c == '\'' && !previous.is_alphanumeric() => quote = Some(c),
            None if matches!(c, '&' | '|' | ';' | '\n' | '\r') && previous != '^' => {
                parts.push(&command[start..i]);
                start = i + c.len_utf8();
            }
            None => {}
        }
        previous = c;
    }
    parts.push(&command[start..]);
    parts.into_iter().filter(|part| !part.trim().is_empty()).collect()
}

#[derive(Debug)]
struct Token {
    text: String,
    /// Byte offset in the segment where the token starts.
    start: usize,
    quoted: bool,
}

/// Whitespace-separated tokens (quotes group and are removed) plus every quoted part on its own.
fn tokenize(segment: &str) -> (Vec<Token>, Vec<String>) {
    let mut tokens = Vec::new();
    let mut quoted_parts = Vec::new();
    let mut current = String::new();
    let mut part = String::new();
    let mut start: Option<usize> = None;
    let mut quote: Option<char> = None;
    let mut quoted = false;
    let mut previous = ' ';
    for (i, c) in segment.char_indices() {
        match quote {
            Some(q) if c == q => {
                quote = None;
                quoted_parts.push(std::mem::take(&mut part));
            }
            Some(_) => {
                current.push(c);
                part.push(c);
            }
            None if c == '"' || (c == '\'' && !previous.is_alphanumeric()) => {
                quote = Some(c);
                quoted = true;
                start.get_or_insert(i);
            }
            None if c.is_whitespace() => {
                if let Some(begin) = start.take() {
                    tokens.push(Token { text: std::mem::take(&mut current), start: begin, quoted });
                    quoted = false;
                }
            }
            None => {
                start.get_or_insert(i);
                current.push(c);
            }
        }
        previous = c;
    }
    if !part.is_empty() {
        quoted_parts.push(part);
    }
    if let Some(begin) = start {
        tokens.push(Token { text: current, start: begin, quoted });
    }
    (tokens, quoted_parts)
}

/// Lower-case program name of a token: base name without `.exe`/`.com`, e.g.
/// `C:\Windows\System32\WScript.exe` → `wscript`, `$shell.Run(cscript` → `cscript`.
fn program_name(token: &str) -> String {
    let cleaned = clean(token);
    let base = cleaned.rsplit(['\\', '/', '(', '=', ',']).next().unwrap_or(cleaned);
    let lower = base.to_ascii_lowercase();
    let lower = lower.strip_prefix('@').unwrap_or(&lower);
    match lower {
        "%comspec%" => "cmd".to_owned(),
        other => other.strip_suffix(".exe").or_else(|| other.strip_suffix(".com")).unwrap_or(other).to_owned(),
    }
}

/// A token without surrounding punctuation of the enclosing language.
fn clean(token: &str) -> &str {
    token.trim().trim_matches(|c: char| matches!(c, '"' | '\'' | '(' | ')' | ',' | ';' | '{' | '}' | '[' | ']' | '`'))
}

const HOSTS: [&str; 6] = ["wscript", "cscript", "mshta", "cmd", "powershell", "pwsh"];

fn analyze_segment(segment: &str, depth: u8, out: &mut Vec<Reference>) {
    let (tokens, quoted_parts) = tokenize(segment);
    for part in &quoted_parts {
        if part.trim().contains(char::is_whitespace) {
            analyze_into(part, depth + 1, out);
        }
    }
    let Some(first) = first_program(&tokens) else { return };

    // The program of the segment, and every Script Host, shell or mshta further on
    // (`if exist x cscript x.vbs`, `for … do wscript %%f`, `psexec … cscript …`).
    let mut positions = vec![first];
    positions.extend((first + 1..tokens.len()).filter(|&i| HOSTS.contains(&program_name(&tokens[i].text).as_str())));
    for position in positions {
        analyze_program(segment, &tokens, position, depth, out);
    }
}

/// Index of the program, after `@`, `call`, `start` (with its options and title), PowerShell's
/// `&`/`.` call operators, `Start-Process`/`Invoke-Item` and their parameters.
fn first_program(tokens: &[Token]) -> Option<usize> {
    let mut i = 0;
    while let Some(token) = tokens.get(i) {
        let name = program_name(&token.text);
        match name.as_str() {
            "" | "&" | "." | "call" => i += 1,
            "start" | "start-process" | "saps" | "invoke-item" | "ii" => {
                i += 1;
                let mut title_seen = name != "start";
                while let Some(next) = tokens.get(i) {
                    let lower = next.text.to_ascii_lowercase();
                    if lower == "/d" || PS_VALUE_PARAMETERS.contains(&lower.as_str()) {
                        i += 2;
                    } else if lower.starts_with('/') || (lower.starts_with('-') && lower.len() > 1) {
                        i += 1;
                    } else if next.quoted && !title_seen {
                        title_seen = true; // `start "title" program`
                        i += 1;
                    } else {
                        break;
                    }
                }
            }
            _ => return Some(i),
        }
    }
    None
}

/// PowerShell parameters of `Start-Process` that take a value which is not the program.
const PS_VALUE_PARAMETERS: [&str; 8] = [
    "-windowstyle",
    "-workingdirectory",
    "-verb",
    "-credential",
    "-redirectstandardoutput",
    "-redirectstandarderror",
    "-redirectstandardinput",
    "-argumentlist",
];

fn analyze_program(segment: &str, tokens: &[Token], position: usize, depth: u8, out: &mut Vec<Reference>) {
    let token = &tokens[position];
    let args = &tokens[position + 1..];
    let name = program_name(&token.text);
    match name.as_str() {
        "wscript" | "cscript" => {
            let via = if name == "wscript" { "wscript" } else { "cscript" };
            if let Some(reference) = script_host(via, args) {
                out.push(reference);
            }
        }
        "mshta" => {
            let target = args.iter().map(|t| clean(&t.text)).find(|t| !t.is_empty() && !t.starts_with(['-', '/']));
            if let Some(target) = target {
                let lower = target.to_ascii_lowercase();
                if !lower.starts_with("vbscript:") && !lower.starts_with("javascript:") && !lower.starts_with("about:")
                {
                    out.push(Reference { usage: Usage::Unknown, script: Some(target.to_owned()), via: "mshta" });
                }
            }
        }
        "cmd" => {
            let switch = args.iter().find(|t| {
                let lower = t.text.to_ascii_lowercase();
                lower.starts_with("/c") || lower.starts_with("/k")
            });
            if let Some(switch) = switch {
                let rest = segment[switch.start..].get(2..).unwrap_or_default();
                analyze_into(rest, depth + 1, out);
            }
        }
        "powershell" | "pwsh" => powershell(segment, args, depth, out),
        _ => {
            let script = clean(&token.text);
            if script.contains(char::is_whitespace) && !looks_like_path(script) {
                return; // quoted code such as `$shell.Run("…")` – analysed as a quoted part
            }
            match text::extension(script).as_deref() {
                Some("vbs" | "vbe") => {
                    out.push(Reference { usage: Usage::VbScript, script: Some(script.to_owned()), via: "direct" })
                }
                Some("wsf" | "hta" | "wsc") => {
                    out.push(Reference { usage: Usage::Unknown, script: Some(script.to_owned()), via: "direct" })
                }
                _ => {}
            }
        }
    }
}

/// `wscript`/`cscript` arguments: `//` options (and the documented `/` forms before the
/// script), then the script, then the script's own arguments.
fn script_host(via: &'static str, args: &[Token]) -> Option<Reference> {
    let mut engine: Option<bool> = None; // Some(true) = VBScript, Some(false) = another engine
    let mut script: Option<String> = None;
    for token in args {
        let arg = clean(&token.text);
        let lower = arg.to_ascii_lowercase();
        let option = lower.starts_with("//") || (script.is_none() && is_wsh_option(&lower));
        if option {
            if let Some(name) = lower.trim_start_matches('/').strip_prefix("e:") {
                engine = Some(name.starts_with("vbscript"));
            }
            continue;
        }
        if script.is_none() && !arg.is_empty() && !(lower.starts_with('-') && lower.len() > 1) {
            script = Some(arg.to_owned());
        }
    }
    let extension = script.as_deref().and_then(text::extension);
    let usage = match (engine, extension.as_deref()) {
        (Some(true), _) | (None, Some("vbs" | "vbe")) => Usage::VbScript,
        (Some(false), _) | (None, Some("js" | "jse")) => return None,
        (None, _) if script.is_some() => Usage::Unknown,
        (None, _) => return None,
    };
    Some(Reference { usage, script, via })
}

fn is_wsh_option(lower: &str) -> bool {
    let Some(name) = lower.strip_prefix('/') else { return false };
    let name = name.split(':').next().unwrap_or(name);
    matches!(name, "b" | "d" | "e" | "h" | "i" | "job" | "logo" | "nologo" | "s" | "t" | "x" | "u" | "?")
}

/// `powershell -Command …`, `-EncodedCommand …`; other arguments are read as a command too.
fn powershell(segment: &str, args: &[Token], depth: u8, out: &mut Vec<Reference>) {
    let mut i = 0;
    while let Some(token) = args.get(i) {
        let lower = token.text.to_ascii_lowercase();
        let parameter = lower.strip_prefix('-').or_else(|| lower.strip_prefix('/'));
        match parameter {
            Some("c" | "command") => {
                let rest = args.get(i + 1).map_or("", |next| &segment[next.start..]);
                analyze_into(rest, depth + 1, out);
                return;
            }
            Some("e" | "ec" | "enc" | "encodedcommand") => {
                if let Some(decoded) = args.get(i + 1).and_then(|next| decode_encoded_command(&next.text)) {
                    analyze_into(&decoded, depth + 1, out);
                }
                return;
            }
            // Parameters with a value (the value is not a command).
            Some(
                "f" | "file" | "executionpolicy" | "ep" | "windowstyle" | "w" | "workingdirectory" | "wd"
                | "configurationname" | "version" | "psconsolefile" | "inputformat" | "outputformat",
            ) => i += 2,
            Some(_) => i += 1,
            None => {
                analyze_into(&segment[token.start..], depth + 1, out);
                return;
            }
        }
    }
}

/// `-EncodedCommand` takes Base64 of UTF-16 LE text.
fn decode_encoded_command(value: &str) -> Option<String> {
    let bytes = base64(clean(value))?;
    let units: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|pair| u16::from_le_bytes(*pair)).collect();
    Some(String::from_utf16_lossy(&units))
}

fn base64(text: &str) -> Option<Vec<u8>> {
    let mut bits = 0u32;
    let mut count = 0;
    let mut bytes = Vec::with_capacity(text.len() * 3 / 4);
    for c in text.bytes() {
        let value = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' => break,
            _ => return None,
        };
        bits = (bits << 6) | u32::from(value);
        count += 6;
        if count >= 8 {
            count -= 8;
            bytes.push((bits >> count) as u8);
        }
    }
    Some(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usage(command: &str) -> Option<Usage> {
        strongest(&analyze(command))
    }

    fn script(command: &str) -> Option<String> {
        analyze(command).into_iter().find_map(|reference| reference.script)
    }

    #[test]
    fn script_host_with_vbscript_files() {
        for command in [
            r"wscript.exe C:\Scripts\backup.vbs",
            r#"C:\Windows\System32\cscript.exe //nologo "C:\Program Files\Tools\run.vbs" /verbose"#,
            r"%SystemRoot%\SysWOW64\WScript.exe //B \\srv\netlogon\logon.vbs",
            r"cscript //E:VBScript C:\Scripts\task.txt",
            r#"cscript //nologo //e:vbscript "%~f0""#,
            r"cscript /nologo map.vbe",
        ] {
            assert_eq!(usage(command), Some(Usage::VbScript), "{command}");
        }
        assert_eq!(
            script(r#"wscript.exe "C:\Program Files\x\a b.vbs" 1 2"#).as_deref(),
            Some(r"C:\Program Files\x\a b.vbs")
        );
    }

    #[test]
    fn jscript_is_not_vbscript() {
        for command in [
            r"wscript.exe C:\Scripts\clean.js",
            r"cscript //E:JScript C:\Scripts\task.txt",
            r#"cscript //nologo //e:jscript "%~f0""#,
            "mshta javascript:alert(1)",
            "wscript.exe",
            r"C:\Scripts\hello.js",
        ] {
            assert_eq!(usage(command), None, "{command}");
        }
    }

    #[test]
    fn direct_starts_and_unknown_languages() {
        assert_eq!(usage(r#""C:\Scripts\backup.vbs""#), Some(Usage::VbScript));
        assert_eq!(usage(r#"start "" /wait C:\Scripts\x.vbs"#), Some(Usage::VbScript));
        assert_eq!(usage(r"call \\srv\share\logon.vbs"), Some(Usage::VbScript));
        assert_eq!(usage(r"wscript C:\Jobs\all.wsf"), Some(Usage::Unknown));
        assert_eq!(usage(r"mshta.exe C:\Tools\inventory.hta"), Some(Usage::Unknown));
        assert_eq!(usage(r"cscript %1"), Some(Usage::Unknown));
        assert_eq!(usage(r"for %%f in (*.vbs) do cscript //nologo %%f"), Some(Usage::Unknown));
        assert_eq!(usage(r"C:\Tools\setup.hta"), Some(Usage::Unknown));
    }

    #[test]
    fn inline_vbscript() {
        let found = analyze(r#"mshta vbscript:Execute("CreateObject(""WScript.Shell"").Run ""calc"":close")"#);
        assert_eq!(found[0], Reference { usage: Usage::VbScript, script: None, via: "mshta" });
        let found = analyze(r#"rundll32.exe vbscript:"\..\mshtml,RunHTMLApplication "+String(1)"#);
        assert_eq!(found[0].via, "rundll32");
    }

    #[test]
    fn looks_through_shells_and_quoted_commands() {
        for command in [
            r#"cmd /c "cscript //nologo C:\x\y.vbs & exit""#,
            r"cmd.exe /q /c C:\x\y.vbs",
            r"%ComSpec% /k wscript x.vbs",
            r#"powershell -NoProfile -Command "& cscript.exe //nologo 'C:\x\y.vbs'""#,
            r#"powershell.exe -ExecutionPolicy Bypass -Command Start-Process wscript -ArgumentList 'C:\x\y.vbs'"#,
            r#"Start-Process -FilePath "C:\x\y.vbs" -Wait"#,
            r#"$shell.Run("wscript.exe //B C:\x\y.vbs", 0)"#,
            r#"schtasks /create /tn Backup /tr "wscript.exe C:\x\y.vbs" /sc daily"#,
            r"if exist C:\x\y.vbs cscript C:\x\y.vbs",
            r"for /f %%a in ('cscript //nologo C:\x\query.vbs') do set R=%%a",
            r"& 'C:\x\y.vbs'",
            r"@wscript x.vbs",
        ] {
            assert_eq!(usage(command), Some(Usage::VbScript), "{command}");
        }
        let encoded: String = {
            let text = r"cscript C:\x\y.vbs";
            let bytes: Vec<u8> = text.encode_utf16().flat_map(u16::to_le_bytes).collect();
            // Base64 via the decoder's inverse for the test.
            const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
            let mut out = String::new();
            for chunk in bytes.chunks(3) {
                let n = chunk.iter().enumerate().fold(0u32, |acc, (i, &b)| acc | (u32::from(b) << (16 - 8 * i)));
                for i in 0..=chunk.len() {
                    out.push(ALPHABET[((n >> (18 - 6 * i)) & 63) as usize] as char);
                }
            }
            out
        };
        assert_eq!(usage(&format!("powershell -enc {encoded}")), Some(Usage::VbScript));
    }

    #[test]
    fn names_called_scripts() {
        assert_eq!(
            called_scripts(r#"cmd.exe /c "C:\Program Files\App\nightly job.cmd" /q"#),
            [r"C:\Program Files\App\nightly job.cmd"]
        );
        assert_eq!(called_scripts(r"powershell -File C:\x\a.ps1 & call b.bat"), [r"C:\x\a.ps1", "b.bat"]);
        assert_eq!(called_scripts(r#"schtasks /tr "cmd /c C:\x\run.cmd""#), [r"C:\x\run.cmd"]);
        assert!(called_scripts(r"C:\x\app.exe /config C:\x\app.ini").is_empty());
    }

    #[test]
    fn ordinary_commands_are_not_vbscript() {
        for command in [
            r#""C:\Program Files\App\app.exe" /startup"#,
            r"copy \\srv\share\logon.vbs C:\Temp\",
            r"del C:\Scripts\old.vbs",
            r"if exist C:\x\y.vbs del C:\x\y.vbs",
            r#"$wscript = New-Object -ComObject WScript.Shell"#,
            r"Get-Content C:\Logs\cscript.txt",
            r"notepad.exe C:\Scripts\backup.vbs.txt",
            r"echo transcript.log",
            r"C:\Windows\System32\svchost.exe -k netsvcs -p",
            r#"cmd /c "robocopy C:\a D:\b /mir""#,
            "",
        ] {
            assert_eq!(usage(command), None, "{command}");
        }
    }
}
