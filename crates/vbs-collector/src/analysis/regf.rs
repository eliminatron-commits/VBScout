//! Registry hive files (`NTUSER.DAT`) read as plain files. The hives of users who are not
//! logged on are not loaded in the registry, and loading them (`RegLoadKey`) would change the
//! system – so their Run keys and logon scripts are read from the file instead.
//!
//! Format ("regf"): a 4 KiB base block, then hive bins with cells. Key nodes (`nk`) point to
//! subkey lists (`lf`, `lh`, `li`, `ri`) and value lists; value keys (`vk`) hold small data
//! inline and larger data in a cell or in segments (`db`). Offsets are relative to the first
//! hive bin. Everything is bounds-checked: a damaged file yields fewer keys, never a panic.

use vbs_core::views::{Bitness, Hive as HiveKind, RegValue, RegistryView, ViewError};

/// A parsed hive file.
#[derive(Debug, Clone)]
pub struct HiveFile {
    data: Vec<u8>,
    root: u32,
    /// The primary and secondary sequence numbers differ: the file has changes only in its
    /// transaction logs (not read), so the newest changes may be missing.
    pub dirty: bool,
}

const BASE_BLOCK: usize = 4096;
/// Nesting of `ri` lists and key paths is bounded (loops in damaged files).
const MAX_DEPTH: usize = 64;

impl HiveFile {
    /// Checks the base block and keeps the file.
    pub fn parse(data: Vec<u8>) -> Result<Self, String> {
        if data.len() < BASE_BLOCK + 32 || &data[..4] != b"regf" {
            return Err("not a registry hive".into());
        }
        let primary = u32_at(&data, 4).unwrap_or(0);
        let secondary = u32_at(&data, 8).unwrap_or(0);
        let root = u32_at(&data, 0x24).ok_or("no root key")?;
        let hive = HiveFile { data, root, dirty: primary != secondary };
        if hive.signature(root) != Some(*b"nk") {
            return Err("root key missing".into());
        }
        Ok(hive)
    }

    /// Cell data (after the 4-byte size) at a hive-bin offset.
    fn cell(&self, offset: u32) -> Option<&[u8]> {
        let start = BASE_BLOCK.checked_add(offset as usize)?;
        let size = i32::from_le_bytes(self.data.get(start..start + 4)?.try_into().ok()?);
        let length = (size.unsigned_abs() as usize).checked_sub(4)?;
        self.data.get(start + 4..start + 4 + length)
    }

    fn signature(&self, offset: u32) -> Option<[u8; 2]> {
        let cell = self.cell(offset)?;
        Some([*cell.first()?, *cell.get(1)?])
    }

    /// Offset of the key at `path` (`Software\Microsoft\…`, case-insensitive; `""` = root).
    pub fn find(&self, path: &str) -> Option<u32> {
        let mut key = self.root;
        for part in path.split('\\').filter(|part| !part.is_empty()) {
            key = self.subkeys(key).into_iter().find(|(name, _)| name.eq_ignore_ascii_case(part))?.1;
        }
        Some(key)
    }

    /// Names and offsets of a key's subkeys.
    pub fn subkeys(&self, key: u32) -> Vec<(String, u32)> {
        let Some(node) = self.cell(key).filter(|c| c.starts_with(b"nk")) else { return Vec::new() };
        let (Some(count), Some(list)) = (u32_at(node, 0x14), u32_at(node, 0x1C)) else { return Vec::new() };
        let mut offsets = Vec::new();
        if count > 0 {
            self.list_offsets(list, 0, &mut offsets);
        }
        offsets.into_iter().filter_map(|offset| Some((self.key_name(offset)?, offset))).collect()
    }

    fn list_offsets(&self, list: u32, depth: usize, out: &mut Vec<u32>) {
        let Some(cell) = self.cell(list) else { return };
        if depth > MAX_DEPTH || cell.len() < 4 {
            return;
        }
        let count = usize::from(u16::from_le_bytes([cell[2], cell[3]]));
        match &cell[..2] {
            b"lf" | b"lh" => out.extend((0..count).filter_map(|i| u32_at(cell, 4 + i * 8))),
            b"li" => out.extend((0..count).filter_map(|i| u32_at(cell, 4 + i * 4))),
            b"ri" => {
                for i in 0..count {
                    if let Some(sub) = u32_at(cell, 4 + i * 4) {
                        self.list_offsets(sub, depth + 1, out);
                    }
                }
            }
            _ => {}
        }
    }

    fn key_name(&self, key: u32) -> Option<String> {
        let node = self.cell(key).filter(|c| c.starts_with(b"nk"))?;
        let flags = u16_at(node, 2)?;
        let length = usize::from(u16_at(node, 0x48)?);
        let raw = node.get(0x4C..0x4C + length)?;
        Some(if flags & 0x20 != 0 { latin1(raw) } else { utf16(raw) })
    }

    /// The values of a key.
    pub fn values(&self, key: u32) -> Vec<(String, RegValue)> {
        let Some(node) = self.cell(key).filter(|c| c.starts_with(b"nk")) else { return Vec::new() };
        let (Some(count), Some(list)) = (u32_at(node, 0x24), u32_at(node, 0x28)) else { return Vec::new() };
        let Some(list) = self.cell(list).filter(|_| count > 0) else { return Vec::new() };
        (0..count as usize).filter_map(|i| u32_at(list, i * 4)).filter_map(|offset| self.value(offset)).collect()
    }

    fn value(&self, offset: u32) -> Option<(String, RegValue)> {
        let vk = self.cell(offset).filter(|c| c.starts_with(b"vk"))?;
        let name_length = usize::from(u16_at(vk, 2)?);
        let size = u32_at(vk, 4)?;
        let data_offset = u32_at(vk, 8)?;
        let kind = u32_at(vk, 0x0C)?;
        let flags = u16_at(vk, 0x10)?;
        let raw_name = vk.get(0x14..0x14 + name_length)?;
        let name = if flags & 0x01 != 0 { latin1(raw_name) } else { utf16(raw_name) };
        let inline = size & 0x8000_0000 != 0;
        let length = (size & 0x7FFF_FFFF) as usize;
        let data: Vec<u8> = if inline {
            data_offset.to_le_bytes()[..length.min(4)].to_vec()
        } else {
            self.value_data(data_offset, length)?
        };
        Some((name, decode(kind, &data)))
    }

    /// Value data from a cell, or from the segments of a big-data (`db`) record.
    fn value_data(&self, offset: u32, length: usize) -> Option<Vec<u8>> {
        let cell = self.cell(offset)?;
        if length > 16_344 && cell.starts_with(b"db") {
            let segments = usize::from(u16_at(cell, 2)?);
            let list = self.cell(u32_at(cell, 4)?)?;
            let mut data = Vec::with_capacity(length);
            for i in 0..segments {
                let segment = self.cell(u32_at(list, i * 4)?)?;
                let take = segment.len().min(16_344).min(length - data.len());
                data.extend_from_slice(&segment[..take]);
            }
            return Some(data);
        }
        cell.get(..length).map(<[u8]>::to_vec)
    }
}

/// A hive file as a registry view: every hive and view maps to the file's root.
impl RegistryView for HiveFile {
    fn subkeys(&self, _: HiveKind, path: &str, _: Bitness) -> Result<Vec<String>, ViewError> {
        let key = self.find(path).ok_or(ViewError::NotFound)?;
        Ok(HiveFile::subkeys(self, key).into_iter().map(|(name, _)| name).collect())
    }

    fn values(&self, _: HiveKind, path: &str, _: Bitness) -> Result<Vec<(String, RegValue)>, ViewError> {
        let key = self.find(path).ok_or(ViewError::NotFound)?;
        Ok(HiveFile::values(self, key))
    }
}

fn decode(kind: u32, data: &[u8]) -> RegValue {
    let text = || {
        let units: Vec<u16> = data.as_chunks::<2>().0.iter().map(|pair| u16::from_le_bytes(*pair)).collect();
        let end = units.iter().position(|&u| u == 0).unwrap_or(units.len());
        String::from_utf16_lossy(&units[..end])
    };
    match kind {
        1 => RegValue::Text(text()),
        2 => RegValue::ExpandText(text()),
        7 => {
            let units: Vec<u16> = data.as_chunks::<2>().0.iter().map(|pair| u16::from_le_bytes(*pair)).collect();
            RegValue::MultiText(
                units.split(|&u| u == 0).filter(|part| !part.is_empty()).map(String::from_utf16_lossy).collect(),
            )
        }
        4 if data.len() >= 4 => RegValue::Dword(u32::from_le_bytes([data[0], data[1], data[2], data[3]])),
        11 if data.len() >= 8 => {
            let mut bytes = [0u8; 8];
            bytes.copy_from_slice(&data[..8]);
            RegValue::Qword(u64::from_le_bytes(bytes))
        }
        3 => RegValue::Binary(data.to_vec()),
        other => RegValue::Other(other),
    }
}

fn u16_at(data: &[u8], at: usize) -> Option<u16> {
    data.get(at..at + 2).map(|b| u16::from_le_bytes([b[0], b[1]]))
}

fn u32_at(data: &[u8], at: usize) -> Option<u32> {
    data.get(at..at + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

fn latin1(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| char::from(b)).collect()
}

fn utf16(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|pair| u16::from_le_bytes(*pair)).collect();
    String::from_utf16_lossy(&units)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Vec<u8> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/corpus/positive/system/Users/carol")
            .join(name);
        std::fs::read(path).unwrap()
    }

    #[test]
    fn reads_a_hive_written_by_hivex() {
        let hive = HiveFile::parse(fixture("NTUSER.DAT")).unwrap();
        let run = r"Software\Microsoft\Windows\CurrentVersion\Run";
        let values = RegistryView::values(&hive, HiveKind::Users, run, Bitness::Native).unwrap();
        let legacy = values.iter().find(|(name, _)| name == "LegacyOffline").map(|(_, v)| v.clone());
        assert_eq!(legacy, Some(RegValue::Text(r"wscript.exe //B C:\Offline\offline-agent.vbs".into())));
        assert!(values.iter().any(|(name, value)| name == "Tray" && matches!(value, RegValue::ExpandText(_))));
        let subkeys =
            RegistryView::subkeys(&hive, HiveKind::Users, r"SOFTWARE\MICROSOFT\Windows", Bitness::Native).unwrap();
        assert_eq!(subkeys, ["CurrentVersion"]);
        assert_eq!(
            RegistryView::values(&hive, HiveKind::Users, r"Software\Missing", Bitness::Native),
            Err(ViewError::NotFound)
        );
    }

    #[test]
    fn rejects_other_and_damaged_files() {
        assert!(HiveFile::parse(b"not a hive".to_vec()).is_err());
        let mut damaged = fixture("NTUSER.DAT");
        damaged.truncate(BASE_BLOCK + 64);
        if let Ok(hive) = HiveFile::parse(damaged) {
            assert!(hive.find(r"Software\Microsoft").is_none());
        }
    }
}
