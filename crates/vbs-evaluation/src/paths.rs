//! Windows paths as the evaluation compares them.
//!
//! Result files keep paths as the collector saw them. For comparisons (the same file seen from
//! several machines, the script a task starts) they are normalised: backslashes, lower case, no
//! quotes, no `\\?\` prefix and no trailing backslash. Windows paths are case-insensitive.

/// Normalised form of a path for comparisons.
pub fn normalize(path: &str) -> String {
    let mut path = path.trim().trim_matches('"').replace('/', "\\");
    if let Some(rest) = path.strip_prefix(r"\\?\UNC\") {
        path = format!(r"\\{rest}");
    } else if let Some(rest) = path.strip_prefix(r"\\?\") {
        path = rest.to_owned();
    }
    while path.len() > 3 && path.ends_with('\\') {
        path.pop();
    }
    path.to_lowercase()
}

/// A path on a network share (`\\server\share\…`), in normalised form.
pub fn is_network(normalized: &str) -> bool {
    normalized.starts_with(r"\\")
}

/// A complete local path (`C:\…`) or network path – not relative and without `%variables%`.
pub fn is_absolute(normalized: &str) -> bool {
    let bytes = normalized.as_bytes();
    let drive = bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'\\';
    (drive || is_network(normalized)) && !normalized.contains('%')
}

/// The last component of a normalised path.
pub fn file_name(normalized: &str) -> &str {
    normalized.rsplit('\\').next().unwrap_or(normalized)
}

/// File extension of a normalised path, without the dot.
pub fn extension(normalized: &str) -> Option<&str> {
    file_name(normalized).rsplit_once('.').map(|(_, extension)| extension)
}

/// Script files that need the VBScript engine or may contain VBScript.
pub fn is_script_file(normalized: &str) -> bool {
    matches!(extension(normalized), Some("vbs" | "vbe" | "wsf" | "wsc" | "hta"))
}

/// The component store (`%windir%\WinSxS`, also inside container image layers): Windows keeps
/// every file it installs there.
pub fn in_component_store(normalized: &str) -> bool {
    normalized.contains(r"\windows\winsxs\")
}

/// Places that only hold Windows' own files or data:
///
/// * the component store and the servicing and update caches,
/// * the databases of User Access Logging (`System32\LogFiles\Sum\*.mdb`, locked while Windows runs),
/// * the Windows folder of container image layers (`…\windowsfilter\<layer>\Files\Windows\…`,
///   also the utility VM of Hyper-V isolation).
pub fn is_windows_location(normalized: &str) -> bool {
    in_component_store(normalized)
        || normalized.contains(r"\windows\servicing\")
        || normalized.contains(r"\windows\softwaredistribution\")
        || (normalized.contains(r"\windows\system32\logfiles\sum\") && extension(normalized) == Some("mdb"))
        || container_layer_windows(normalized)
}

fn container_layer_windows(normalized: &str) -> bool {
    let Some((_, rest)) = normalized.split_once(r"\windowsfilter\") else { return false };
    let Some((_layer, inside)) = rest.split_once('\\') else { return false };
    inside.starts_with(r"files\windows\") || inside.starts_with(r"utilityvm\files\windows\")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_for_comparison() {
        assert_eq!(normalize(r#""C:\Scripts\Backup.VBS""#), r"c:\scripts\backup.vbs");
        assert_eq!(normalize("C:/Scripts/"), r"c:\scripts");
        assert_eq!(normalize(r"C:\"), r"c:\");
        assert_eq!(normalize(r"\\?\UNC\FS01\Share\Logon.vbs"), r"\\fs01\share\logon.vbs");
        assert_eq!(normalize(r"\\?\D:\Tools\x.vbs"), r"d:\tools\x.vbs");
        assert!(is_network(&normalize(r"\\corp.example\NETLOGON\logon.vbs")));
        assert!(!is_network(&normalize(r"C:\x.vbs")));
    }

    #[test]
    fn absolute_names_and_extensions() {
        assert!(is_absolute(r"c:\scripts\x.vbs"));
        assert!(is_absolute(r"\\srv\share\x.vbs"));
        assert!(!is_absolute(r"%systemroot%\x.vbs"));
        assert!(!is_absolute("x.vbs"));
        assert!(!is_absolute(r"scripts\x.vbs"));
        assert_eq!(file_name(r"c:\scripts\x.vbs"), "x.vbs");
        assert_eq!(file_name("x.vbs"), "x.vbs");
        assert_eq!(extension(r"c:\a.b\x"), None);
        assert!(is_script_file(r"c:\a\tool.hta"));
        assert!(!is_script_file(r"c:\a\tool.ps1"));
    }

    #[test]
    fn windows_locations() {
        for path in [
            r"C:\Windows\WinSxS\amd64_microsoft-windows-slmgr_31bf3856ad364e35_10.0.20348.1_none_1\slmgr.vbs",
            r"C:\Windows\servicing\Packages\x.vbs",
            r"C:\Windows\SoftwareDistribution\Download\a\b.vbs",
            r"C:\Windows\System32\LogFiles\Sum\Current.mdb",
            r"C:\ProgramData\docker\windowsfilter\158c674d\Files\Windows\System32\slmgr.vbs",
            r"D:\docker\windowsfilter\6f797de8\UtilityVM\Files\Windows\System32\winrm.cmd",
        ] {
            assert!(is_windows_location(&normalize(path)), "{path}");
        }
        for path in [
            r"C:\Windows\System32\slmgr.vbs",
            r"C:\Windows\System32\GroupPolicy\Machine\Scripts\Startup\map.vbs",
            r"C:\Windows\SYSVOL\domain\scripts\logon.vbs",
            r"C:\Windows\System32\LogFiles\Sum\notes.vbs",
            r"C:\ProgramData\docker\windowsfilter\158c674d\Files\inetpub\app\run.vbs",
            r"C:\Scripts\windowsfilter\x.vbs",
        ] {
            assert!(!is_windows_location(&normalize(path)), "{path}");
        }
    }
}
