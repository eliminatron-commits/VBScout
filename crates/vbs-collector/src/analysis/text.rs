//! Text of scripts as Windows writes it: UTF-8 or UTF-16 with or without a byte order mark,
//! or the ANSI code page (Windows-1252 on most western systems).

/// Decodes script bytes: a byte order mark decides first (UTF-8, UTF-16 LE/BE), then
/// UTF-16 LE without a mark (recognised by its zero bytes), then UTF-8, and anything
/// else is read as Windows-1252.
pub fn decode(bytes: &[u8]) -> String {
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(rest).into_owned();
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return utf16(rest, u16::from_le_bytes);
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        return utf16(rest, u16::from_be_bytes);
    }
    if looks_like_utf16le(bytes) {
        return utf16(bytes, u16::from_le_bytes);
    }
    match std::str::from_utf8(bytes) {
        Ok(text) => text.to_owned(),
        Err(_) => bytes.iter().map(|&byte| windows_1252(byte)).collect(),
    }
}

fn utf16(bytes: &[u8], word: fn([u8; 2]) -> u16) -> String {
    let units: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|pair| word(*pair)).collect();
    String::from_utf16_lossy(&units)
}

/// Plain-ASCII text stored as UTF-16 LE has a zero in (almost) every second byte.
fn looks_like_utf16le(bytes: &[u8]) -> bool {
    let sample = &bytes[..bytes.len().min(4096) & !1];
    if sample.len() < 8 {
        return false;
    }
    let pairs = sample.len() / 2;
    let high_zero = sample.iter().skip(1).step_by(2).filter(|&&byte| byte == 0).count();
    let low_zero = sample.iter().step_by(2).filter(|&&byte| byte == 0).count();
    high_zero * 10 >= pairs * 7 && low_zero * 10 <= pairs
}

/// Windows-1252 (0x80–0x9F differ from Latin-1; undefined bytes keep their code point).
pub(crate) fn windows_1252(byte: u8) -> char {
    const HIGH: [char; 32] = [
        '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8d}', 'Ž', '\u{8f}', '\u{90}', '‘',
        '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9d}', 'ž', 'Ÿ',
    ];
    match byte {
        0x80..=0x9F => HIGH[usize::from(byte - 0x80)],
        other => char::from(other),
    }
}

/// Lines with 1-based numbers; `\r\n`, `\n` and a lone `\r` all end a line.
pub fn lines(text: &str) -> impl Iterator<Item = (u32, &str)> {
    let mut rest = Some(text);
    let mut number = 0u32;
    std::iter::from_fn(move || {
        let current = rest?;
        number = number.saturating_add(1);
        match current.find(['\r', '\n']) {
            Some(end) => {
                let skip = if current[end..].starts_with("\r\n") { 2 } else { 1 };
                rest = Some(&current[end + skip..]);
                Some((number, &current[..end]))
            }
            None => {
                rest = None;
                (!current.is_empty() || number == 1).then_some((number, current))
            }
        }
    })
}

/// 1-based line number of a byte offset.
pub fn line_of(text: &str, offset: usize) -> u32 {
    let before = &text[..offset.min(text.len())];
    let mut line = 1u32;
    let mut previous = '\0';
    for c in before.chars() {
        if c == '\n' && previous != '\r' || c == '\r' {
            line = line.saturating_add(1);
        }
        previous = c;
    }
    line
}

/// Lower-case extension (letters and digits only) of a path or file name as written, e.g.
/// `"C:\x\Backup.VBS"` → `vbs`.
pub fn extension(path: &str) -> Option<String> {
    let name = file_name(path);
    let (_, extension) = name.rsplit_once('.')?;
    (!extension.is_empty() && extension.chars().all(|c| c.is_ascii_alphanumeric()))
        .then(|| extension.to_ascii_lowercase())
}

/// File name part of a Windows or UNC path as written.
pub fn file_name(path: &str) -> &str {
    let path = path.trim().trim_matches(['"', '\'']);
    path.rsplit(['\\', '/']).next().unwrap_or(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_the_encodings_windows_writes() {
        assert_eq!(decode(b"\xEF\xBB\xBFMsgBox \xC3\xA4"), "MsgBox ä");
        let utf16: Vec<u8> = "WScript.Echo 1".encode_utf16().flat_map(u16::to_le_bytes).collect();
        assert_eq!(decode(&[&[0xFF, 0xFE][..], &utf16].concat()), "WScript.Echo 1");
        assert_eq!(decode(&utf16), "WScript.Echo 1", "UTF-16 LE without a byte order mark");
        let be: Vec<u8> = "x = 1".encode_utf16().flat_map(u16::to_be_bytes).collect();
        assert_eq!(decode(&[&[0xFE, 0xFF][..], &be].concat()), "x = 1");
        assert_eq!(decode(b"Kennwort f\xFCr \x80 \x93x\x94"), "Kennwort für € “x”");
        assert_eq!(decode(b"plain"), "plain");
    }

    #[test]
    fn splits_lines_like_notepad() {
        let found: Vec<_> = lines("a\r\nb\nc\rd").collect();
        assert_eq!(found, [(1, "a"), (2, "b"), (3, "c"), (4, "d")]);
        assert_eq!(lines("a\r\n").collect::<Vec<_>>(), [(1, "a")]);
        assert_eq!(lines("").collect::<Vec<_>>(), [(1, "")]);
        assert_eq!(line_of("a\r\nb\nc", 5), 3);
        assert_eq!(line_of("abc", 1), 1);
    }

    #[test]
    fn extensions_and_names() {
        assert_eq!(extension(r#""C:\Scripts\Backup.VBS""#).as_deref(), Some("vbs"));
        assert_eq!(extension(r"\\srv\share\logon.vbs").as_deref(), Some("vbs"));
        assert_eq!(extension("C:\\dir.d\\noext"), None);
        assert_eq!(extension("x.vbs)"), None);
        assert_eq!(file_name(r"C:\Windows\System32\wscript.exe"), "wscript.exe");
        assert_eq!(file_name("wscript"), "wscript");
    }
}
