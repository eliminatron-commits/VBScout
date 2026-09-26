//! Group Policy script lists (`scripts.ini`, `psscripts.ini`) as specified in [MS-GPSCR]:
//! sections `[Logon]`, `[Logoff]`, `[Startup]`, `[Shutdown]` with `<n>CmdLine` and
//! `<n>Parameters` keys. The files are usually UTF-16 with a byte order mark.

use std::collections::BTreeMap;

use super::text;

/// One configured script.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyScript {
    /// `Logon`, `Logoff`, `Startup` or `Shutdown` (as written in the file).
    pub phase: String,
    pub index: u32,
    pub command: String,
    pub parameters: String,
    /// Line of the `CmdLine` key.
    pub line: u32,
}

/// Parses a `scripts.ini` or `psscripts.ini` file.
pub fn parse(bytes: &[u8]) -> Vec<PolicyScript> {
    let content = text::decode(bytes);
    let mut section = String::new();
    let mut entries: BTreeMap<(String, u32), PolicyScript> = BTreeMap::new();
    for (number, line) in text::lines(&content) {
        let line = line.trim();
        if let Some(name) = line.strip_prefix('[').and_then(|rest| rest.strip_suffix(']')) {
            section = name.trim().to_owned();
            continue;
        }
        let known = ["logon", "logoff", "startup", "shutdown"].contains(&section.to_ascii_lowercase().as_str());
        let Some((key, value)) = line.split_once('=') else { continue };
        if !known {
            continue;
        }
        let key = key.trim();
        let digits = key.chars().take_while(char::is_ascii_digit).count();
        let Ok(index) = key[..digits].parse::<u32>() else { continue };
        let entry = entries.entry((section.to_ascii_lowercase(), index)).or_insert_with(|| PolicyScript {
            phase: section.clone(),
            index,
            command: String::new(),
            parameters: String::new(),
            line: number,
        });
        match key[digits..].to_ascii_lowercase().as_str() {
            "cmdline" => {
                entry.command = value.trim().to_owned();
                entry.line = number;
            }
            "parameters" => entry.parameters = value.trim().to_owned(),
            _ => {}
        }
    }
    entries.into_values().filter(|entry| !entry.command.is_empty()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_documented_format() {
        let content = "\r\n[Logon]\r\n0CmdLine=map-drives.vbs\r\n0Parameters=/quiet\r\n1CmdLine=\\\\srv\\netlogon\\printers.cmd\r\n[ScriptsConfig]\r\nStartExecutePSFirst=true\r\n[Logoff]\r\n0Parameters=only parameters\r\n";
        let bytes: Vec<u8> =
            [0xFF, 0xFE].into_iter().chain(content.encode_utf16().flat_map(u16::to_le_bytes)).collect();
        let scripts = parse(&bytes);
        assert_eq!(scripts.len(), 2);
        assert_eq!(
            (scripts[0].phase.as_str(), scripts[0].index, scripts[0].command.as_str()),
            ("Logon", 0, "map-drives.vbs")
        );
        assert_eq!((scripts[0].parameters.as_str(), scripts[0].line), ("/quiet", 3));
        assert_eq!(scripts[1].command, r"\\srv\netlogon\printers.cmd");
    }
}
