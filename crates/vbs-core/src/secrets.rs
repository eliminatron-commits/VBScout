//! Masking of secrets in evidence lines.
//!
//! Evidence excerpts must never carry passwords or credentials. Every line that
//! enters a result file passes [`mask_line`] – once when a module reports it and
//! again when the container is written – so a module bug cannot leak a secret.
//! Only literal values are masked and reported: a quoted string assigned to a
//! password-like name, a `Password=`/`Pwd=` pair of a connection string, a batch
//! `set NAME=value` with a password-like name and credentials embedded in URLs.
//! Expressions such as `pwd = GetPassword()` are left alone.

use serde::{Deserialize, Serialize};

/// Replacement for a masked value (fixed length, so the secret's length is not revealed).
pub const MASK: &str = "********";

/// Kind of secret that was masked; reported as a security finding without the value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SecretKind {
    /// A literal assigned to a password-like name.
    Password,
    /// `Password=`/`Pwd=` inside a connection string.
    ConnectionString,
    /// `scheme://user:password@host`.
    UrlCredentials,
}

/// A line with all recognised secrets replaced by [`MASK`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaskedLine {
    pub text: String,
    /// Kinds of the masked secrets, in order of appearance.
    pub secrets: Vec<SecretKind>,
}

impl MaskedLine {
    pub fn masked(&self) -> bool {
        !self.secrets.is_empty()
    }
}

/// Words that mark a name as holding a password when they appear anywhere in it.
const STRONG_WORDS: [&str; 7] = ["password", "passwort", "kennwort", "passwd", "secret", "apikey", "credential"];
/// Short words that only count as a whole name part (`strPwd`, `DB_PASS`, but not `bypass`).
const WEAK_PARTS: [&str; 4] = ["pwd", "pass", "pw", "pword"];
/// Keys of connection strings whose value is a password.
const CONNECTION_KEYS: [&str; 2] = ["password", "pwd"];

/// Masks the secrets in one line of source text.
pub fn mask_line(line: &str) -> MaskedLine {
    let mut spans: Vec<(usize, usize, SecretKind)> = Vec::new();
    find_url_credentials(line, &mut spans);
    find_assignments(line, &mut spans);
    spans.sort_by_key(|&(start, _, _)| start);

    let mut text = String::with_capacity(line.len());
    let mut secrets = Vec::new();
    let mut cursor = 0;
    for (start, end, kind) in spans {
        if start < cursor {
            continue; // overlapping match – the earlier one already covers it
        }
        text.push_str(&line[cursor..start]);
        text.push_str(MASK);
        secrets.push(kind);
        cursor = end;
    }
    text.push_str(&line[cursor..]);
    MaskedLine { text, secrets }
}

/// `scheme://user:password@host` – masks `password`.
fn find_url_credentials(line: &str, spans: &mut Vec<(usize, usize, SecretKind)>) {
    let mut from = 0;
    while let Some(offset) = line[from..].find("://") {
        let authority_start = from + offset + 3;
        let authority_end = line[authority_start..]
            .find(|c: char| c == '/' || c.is_whitespace() || c == '"' || c == '\'')
            .map_or(line.len(), |len| authority_start + len);
        let authority = &line[authority_start..authority_end];
        if let Some(at) = authority.rfind('@')
            && let Some(colon) = authority[..at].find(':')
            && colon + 1 < at
            && &authority[colon + 1..at] != MASK
        {
            spans.push((authority_start + colon + 1, authority_start + at, SecretKind::UrlCredentials));
        }
        from = authority_end;
    }
}

fn is_name_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '.' || c == '-' || c == '$'
}

/// Splits a name into lower-case words: `strDbPassword_2` → `str`, `db`, `password`, `2`;
/// `DBPass1` → `db`, `pass`, `1`.
fn name_parts(name: &str) -> Vec<String> {
    let chars: Vec<char> = name.chars().collect();
    let mut parts = Vec::new();
    let mut current = String::new();
    for (i, &c) in chars.iter().enumerate() {
        if !c.is_alphanumeric() {
            if !current.is_empty() {
                parts.push(std::mem::take(&mut current));
            }
            continue;
        }
        if let Some(&previous) = i.checked_sub(1).and_then(|p| chars.get(p))
            && previous.is_alphanumeric()
        {
            let next_lower = chars.get(i + 1).is_some_and(|n| n.is_lowercase());
            let boundary = (c.is_uppercase() && (previous.is_lowercase() || previous.is_ascii_digit()))
                || (c.is_uppercase() && previous.is_uppercase() && next_lower)
                || (c.is_ascii_digit() != previous.is_ascii_digit());
            if boundary && !current.is_empty() {
                parts.push(std::mem::take(&mut current));
            }
        }
        current.extend(c.to_lowercase());
    }
    if !current.is_empty() {
        parts.push(current);
    }
    parts
}

fn is_secret_name(name: &str) -> bool {
    let lower = name.to_lowercase();
    STRONG_WORDS.iter().any(|word| lower.contains(word))
        || name_parts(name).iter().any(|part| WEAK_PARTS.contains(&part.as_str()))
}

/// Finds `name = "literal"`, `name: 'literal'`, `Password=value;` and `set NAME=value`.
fn find_assignments(line: &str, spans: &mut Vec<(usize, usize, SecretKind)>) {
    let batch_set = line.trim_start().to_ascii_lowercase().starts_with("set ");
    let chars: Vec<(usize, char)> = line.char_indices().collect();
    let mut i = 0;
    while i < chars.len() {
        let (start, c) = chars[i];
        if !is_name_char(c) || (i > 0 && is_name_char(chars[i - 1].1)) {
            i += 1;
            continue;
        }
        let mut j = i;
        while j < chars.len() && is_name_char(chars[j].1) {
            j += 1;
        }
        let name_end = chars.get(j).map_or(line.len(), |&(index, _)| index);
        let name = &line[start..name_end];
        i = j;
        if !is_secret_name(name) {
            continue;
        }
        // Command-line option such as `--password=x`, `/password:x` or `-Password "x"`.
        let option = name.starts_with('-') || line[..start].ends_with('/');

        // Operator: `=`, `:=` or `:` – but not `==`, `=>` or `<=`. Options may use a space instead.
        let mut k = j;
        while k < chars.len() && chars[k].1 == ' ' {
            k += 1;
        }
        let operator_len = match (chars.get(k).map(|c| c.1), chars.get(k + 1).map(|c| c.1)) {
            (Some(':'), Some('=')) => 2,
            (Some('='), Some('=' | '>')) => continue,
            (Some('=' | ':'), _) => 1,
            (Some('"' | '\''), _) if option && k > j => 0,
            _ => continue,
        };
        let mut v = k + operator_len;
        let spaced = v < chars.len() && chars[v].1 == ' ';
        while v < chars.len() && chars[v].1 == ' ' {
            v += 1;
        }
        let Some(&(value_start, first)) = chars.get(v) else { continue };

        if first == '"' || first == '\'' {
            // Quoted literal; VBScript escapes a quote by doubling it.
            let mut end = v + 1;
            while end < chars.len() {
                if chars[end].1 == first {
                    if chars.get(end + 1).is_some_and(|c| c.1 == first) {
                        end += 2;
                        continue;
                    }
                    break;
                }
                end += 1;
            }
            let content_start = value_start + first.len_utf8();
            let content_end = chars.get(end).map_or(line.len(), |&(index, _)| index);
            if content_end > content_start && &line[content_start..content_end] != MASK {
                spans.push((content_start, content_end, SecretKind::Password));
            }
            i = end + 1;
            continue;
        }

        // Unquoted value: a connection-string pair (`Pwd=x;`), an option (`--password=x`)
        // or a batch `set NAME=value`.
        let lower_name = name.to_lowercase();
        let attached = operator_len == 1 && !spaced && k == j;
        let connection_pair = attached && chars[k].1 == '=' && CONNECTION_KEYS.contains(&lower_name.as_str());
        let option_value = attached && option;
        let batch_assignment = batch_set && operator_len == 1 && chars[k].1 == '=';
        if !connection_pair && !option_value && !batch_assignment {
            continue;
        }
        let until_space = connection_pair || option_value;
        let stop = |c: char| c == ';' || c == '"' || c == '\'' || (until_space && c.is_whitespace());
        let mut end = v;
        while end < chars.len() && !stop(chars[end].1) {
            end += 1;
        }
        let value_end = chars.get(end).map_or(line.len(), |&(index, _)| index);
        let value = line[value_start..value_end].trim_end();
        if !value.is_empty() && value != MASK {
            let kind = if connection_pair { SecretKind::ConnectionString } else { SecretKind::Password };
            spans.push((value_start, value_start + value.len(), kind));
        }
        i = end;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn masked(line: &str) -> (String, Vec<SecretKind>) {
        let result = mask_line(line);
        (result.text, result.secrets)
    }

    #[test]
    fn masks_quoted_password_literals() {
        assert_eq!(
            masked(r#"strPassword = "Sommer2024!""#),
            (format!(r#"strPassword = "{MASK}""#), vec![SecretKind::Password])
        );
        assert_eq!(masked(r#"Const DB_PASS = "x""y""#).0, format!(r#"Const DB_PASS = "{MASK}""#));
        assert_eq!(masked(r#"$cred = @{ Password: 'hunter2' }"#).0, format!("$cred = @{{ Password: '{MASK}' }}"));
        assert_eq!(masked(r#"kennwort := "geheim""#).0, format!(r#"kennwort := "{MASK}""#));
    }

    #[test]
    fn masks_connection_strings() {
        let (text, secrets) = masked(r#"conn.Open "Provider=SQLOLEDB;Data Source=db1;User ID=sa;Password=Pa55;""#);
        assert_eq!(text, format!(r#"conn.Open "Provider=SQLOLEDB;Data Source=db1;User ID=sa;Password={MASK};""#));
        assert_eq!(secrets, [SecretKind::ConnectionString]);
        assert_eq!(masked("DSN=x;UID=sa;PWD=secret").0, format!("DSN=x;UID=sa;PWD={MASK}"));
    }

    #[test]
    fn masks_command_line_options() {
        assert_eq!(masked("tool.exe --password=abc123 --verbose").0, format!("tool.exe --password={MASK} --verbose"));
        assert_eq!(masked("sync.exe /password:abc123 /q").0, format!("sync.exe /password:{MASK} /q"));
        assert_eq!(
            masked(r#"Connect-Thing -Password "abc 123" -Force"#).0,
            format!(r#"Connect-Thing -Password "{MASK}" -Force"#)
        );
    }

    #[test]
    fn masks_batch_set_and_url_credentials() {
        assert_eq!(masked("set DBPASSWORD=Geheim 123").0, format!("set DBPASSWORD={MASK}"));
        let (text, secrets) = masked("copy \\\\srv\\x ftp://backup:S3cr3t@ftp.example.org/in");
        assert_eq!(text, format!("copy \\\\srv\\x ftp://backup:{MASK}@ftp.example.org/in"));
        assert_eq!(secrets, [SecretKind::UrlCredentials]);
    }

    #[test]
    fn leaves_expressions_and_harmless_names_alone() {
        for line in [
            "pwd = GetPassword()",
            "If Len(password) = 0 Then",
            "If strPwd == other Then",
            r#"strPassword = """#,
            r#"bypass = "no""#,
            r#"compass = "north""#,
            "Set objShell = CreateObject(\"WScript.Shell\")",
            "https://example.org/path",
        ] {
            let result = mask_line(line);
            assert_eq!(result.text, line, "{line}");
            assert!(!result.masked(), "{line}");
        }
    }

    #[test]
    fn splits_names_into_words() {
        assert_eq!(name_parts("strDbPassword_2"), ["str", "db", "password", "2"]);
        assert_eq!(name_parts("DBPass1"), ["db", "pass", "1"]);
        assert_eq!(name_parts("strPwd1"), ["str", "pwd", "1"]);
        assert_eq!(name_parts("bypass"), ["bypass"]);
    }

    #[test]
    fn masking_is_idempotent() {
        for line in [r#"pwd = "x""#, "DSN=a;PWD=b", "ftp://u:p@h/", "set PASSWORD=x", "run --password=x"] {
            let once = mask_line(line);
            assert!(once.masked(), "{line}");
            let twice = mask_line(&once.text);
            assert_eq!(twice.text, once.text);
            assert!(!twice.masked(), "{line}");
        }
    }

    #[test]
    fn masked_lines_never_contain_the_secret() {
        let secrets = ["Sommer2024!", "Pa55", "S3cr3t", "hunter2"];
        let lines = [
            r#"objConn.Open "DSN=x;UID=sa;PWD=Pa55""#,
            r#"strPwd = "Sommer2024!" ' admin"#,
            "net use x: ftp://u:S3cr3t@h/",
            r#"password: "hunter2", user: "a""#,
        ];
        for line in lines {
            let result = mask_line(line);
            assert!(result.masked(), "{line}");
            assert!(secrets.iter().all(|secret| !result.text.contains(secret)), "{line} → {}", result.text);
        }
    }
}
