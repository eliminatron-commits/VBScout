//! Data of Windows servicing that only carries the name of the file it stands for.
//!
//! * Differentials of the component store (`WinSxS\<component>\f\`, `r\`, `n\`, also in expanded
//!   updates and container image layers) in the MSDelta format: `PA30` (`PA31`), usually behind a
//!   CRC-32, followed by the target's file time.
//! * Payloads of components that are not installed, which the component store keeps compressed:
//!   `DCN`, `DCS`, `DCD`, `DCM`… with version 1.
//!
//! Windows rebuilds the real file from them when it installs, updates or rolls back a component;
//! as they are, neither the Script Host, the shell, Windows Installer nor Office can use them, so
//! they are no finding. The real file is checked where Windows puts it.

/// Whether `head` – the first bytes of a file, at least 16 – is servicing data.
pub fn is_servicing_data(head: &[u8]) -> bool {
    compressed_payload(head) || differential(head)
}

/// Compressed component payload: `DC`, an upper-case letter and version 1.
fn compressed_payload(head: &[u8]) -> bool {
    matches!(head, [b'D', b'C', kind, 0x01, ..] if kind.is_ascii_uppercase())
}

/// MSDelta differential at the start or behind a CRC-32. The file time after the signature has
/// control bytes that text never contains (its high byte is 0–2 for any date up to the year 2400),
/// so a script that happens to start with "PA30" is not mistaken for one.
fn differential(head: &[u8]) -> bool {
    [0, 4].into_iter().any(|start| {
        head.get(start..start + 4).is_some_and(|signature| signature == b"PA30" || signature == b"PA31")
            && head.get(start + 4..start + 12).is_some_and(|time| time.iter().any(|&byte| byte < 0x09))
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A differential as the component store keeps it: CRC-32, `PA30`, file time, bit stream.
    pub(crate) fn differential_bytes() -> Vec<u8> {
        let mut bytes = vec![0x5E, 0x91, 0x2C, 0x7A];
        bytes.extend_from_slice(b"PA30");
        bytes.extend_from_slice(&0x01DB_2F3C_4A5B_6C7Du64.to_le_bytes());
        bytes.extend_from_slice(&[0x18, 0x23, 0xC8, 0x81, 0x03, 0x62, 0x40, 0x10, 0xC5, 0x00]);
        bytes
    }

    #[test]
    fn differentials_with_and_without_checksum() {
        let with_crc = differential_bytes();
        assert!(is_servicing_data(&with_crc));
        assert!(is_servicing_data(&with_crc[4..]));
        let mut newer = with_crc.clone();
        newer[4..8].copy_from_slice(b"PA31");
        assert!(is_servicing_data(&newer));
        let mut zero_time = with_crc.clone();
        zero_time[8..16].fill(0);
        assert!(is_servicing_data(&zero_time));
    }

    #[test]
    fn compressed_payloads() {
        for kind in *b"NSDM" {
            let mut bytes = vec![b'D', b'C', kind, 0x01];
            bytes.extend_from_slice(&[0x02, 0, 0, 0, 0x40, 0x1F, 0, 0, 0x7B, 0x2C, 0x11, 0x90]);
            assert!(is_servicing_data(&bytes), "{}", kind as char);
        }
        assert!(!is_servicing_data(b"DCs\x01 lower case kind"));
        assert!(!is_servicing_data(b"DCN\x02 other version"));
    }

    #[test]
    fn scripts_and_other_files_are_not_servicing_data() {
        for text in [
            &b"PA30 = 1\r\nWScript.Echo PA30\r\n"[..],
            b"Dim PA30\r\nPA30 = \"x\"\r\n",
            b"Dim\tPA30\t\t\t\t\t\t\t\t\r\n",
            b"DCN\r\nWScript.Echo 1\r\n",
            b"L\x00\x00\x00\x01\x14\x02\x00\x00\x00\x00\x00\xC0\x00\x00\x00",
            b"\xD0\xCF\x11\xE0\xA1\xB1\x1A\xE1\x00\x00\x00\x00\x00\x00\x00\x00",
            b"PA30",
            b"",
        ] {
            assert!(!is_servicing_data(text), "{:?}", String::from_utf8_lossy(text));
        }
    }
}
