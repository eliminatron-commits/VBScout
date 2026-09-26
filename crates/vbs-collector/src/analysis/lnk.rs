//! Shell links (`.lnk`) as specified in [MS-SHLLINK]: target path, arguments and working
//! folder. The target comes from the first source that has it: the link information (local
//! base path or network share), the environment-variable block, the item ID list (file
//! system items), or the relative path.

/// What a shortcut starts.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ShellLink {
    /// Target program or document as stored (may contain `%variables%`).
    pub target: Option<String>,
    pub arguments: Option<String>,
    pub working_dir: Option<String>,
    pub description: Option<String>,
}

/// Why a file is not a readable shell link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkError {
    /// Not a shell link at all (wrong header) or cut off.
    Corrupt(&'static str),
}

const HEADER_SIZE: usize = 0x4C;
const LINK_CLSID: [u8; 16] = [0x01, 0x14, 0x02, 0, 0, 0, 0, 0, 0xC0, 0, 0, 0, 0, 0, 0, 0x46];

const HAS_ID_LIST: u32 = 0x01;
const HAS_LINK_INFO: u32 = 0x02;
const HAS_NAME: u32 = 0x04;
const HAS_RELATIVE_PATH: u32 = 0x08;
const HAS_WORKING_DIR: u32 = 0x10;
const HAS_ARGUMENTS: u32 = 0x20;
const HAS_ICON_LOCATION: u32 = 0x40;
const IS_UNICODE: u32 = 0x80;

const ENVIRONMENT_BLOCK: u32 = 0xA000_0001;

/// Parses a shell link.
pub fn parse(data: &[u8]) -> Result<ShellLink, LinkError> {
    if data.len() < HEADER_SIZE || u32_at(data, 0) != Some(HEADER_SIZE as u32) || data[4..20] != LINK_CLSID {
        return Err(LinkError::Corrupt("not a shell link"));
    }
    let flags = u32_at(data, 0x14).unwrap_or(0);
    let mut pos = HEADER_SIZE;
    let mut link = ShellLink::default();
    let mut id_list_target = None;

    if flags & HAS_ID_LIST != 0 {
        let size = usize::from(u16_at(data, pos).ok_or(LinkError::Corrupt("item ID list"))?);
        let list = data.get(pos + 2..pos + 2 + size).ok_or(LinkError::Corrupt("item ID list"))?;
        id_list_target = id_list_path(list);
        pos += 2 + size;
    }
    let mut info_target = None;
    if flags & HAS_LINK_INFO != 0 {
        let size = u32_at(data, pos).ok_or(LinkError::Corrupt("link info"))? as usize;
        let info = data.get(pos..pos + size).ok_or(LinkError::Corrupt("link info"))?;
        info_target = link_info_path(info);
        pos += size;
    }
    let unicode = flags & IS_UNICODE != 0;
    let mut strings = [None, None, None, None, None];
    for (slot, flag) in
        [HAS_NAME, HAS_RELATIVE_PATH, HAS_WORKING_DIR, HAS_ARGUMENTS, HAS_ICON_LOCATION].into_iter().enumerate()
    {
        if flags & flag == 0 {
            continue;
        }
        let count = usize::from(u16_at(data, pos).ok_or(LinkError::Corrupt("string data"))?);
        let bytes = if unicode { count * 2 } else { count };
        let raw = data.get(pos + 2..pos + 2 + bytes).ok_or(LinkError::Corrupt("string data"))?;
        strings[slot] = Some(if unicode { utf16(raw) } else { ansi(raw) });
        pos += 2 + bytes;
    }
    let [name, relative_path, working_dir, arguments, _icon] = strings;

    let mut environment_target = None;
    while let Some(block_size) = u32_at(data, pos) {
        let block_size = block_size as usize;
        if block_size < 8 {
            break;
        }
        let Some(block) = data.get(pos..pos + block_size) else { break };
        if u32_at(block, 4) == Some(ENVIRONMENT_BLOCK) && block.len() >= 0x314 {
            let unicode_target = utf16_z(&block[0x10C..0x314]);
            environment_target = Some(unicode_target).filter(|t| !t.is_empty()).or_else(|| {
                let ansi_target = ansi_z(&block[8..0x10C]);
                (!ansi_target.is_empty()).then_some(ansi_target)
            });
        }
        pos += block_size;
    }

    link.target = info_target
        .or(environment_target)
        .or(id_list_target)
        .or_else(|| relative_path.clone())
        .filter(|target| !target.trim().is_empty());
    link.arguments = arguments.filter(|a| !a.trim().is_empty());
    link.working_dir = working_dir.filter(|w| !w.trim().is_empty());
    link.description = name.filter(|n| !n.trim().is_empty());
    Ok(link)
}

/// Local base path (+ common path suffix) or network share (+ suffix) of the LinkInfo structure.
fn link_info_path(info: &[u8]) -> Option<String> {
    let header_size = u32_at(info, 4)? as usize;
    let flags = u32_at(info, 8)?;
    let offset = |at: usize| u32_at(info, at).map(|o| o as usize).filter(|&o| o > 0 && o < info.len());
    let unicode_offsets = header_size >= 0x24;
    let suffix = if unicode_offsets {
        offset(0x20).map(|o| utf16_z(&info[o..])).or_else(|| offset(0x18).map(|o| ansi_z(&info[o..])))
    } else {
        offset(0x18).map(|o| ansi_z(&info[o..]))
    }
    .unwrap_or_default();
    if flags & 0x01 != 0 {
        let base = if unicode_offsets {
            offset(0x1C).map(|o| utf16_z(&info[o..])).or_else(|| offset(0x10).map(|o| ansi_z(&info[o..])))
        } else {
            offset(0x10).map(|o| ansi_z(&info[o..]))
        };
        if let Some(base) = base.filter(|b| !b.is_empty()) {
            return Some(join(&base, &suffix));
        }
    }
    if flags & 0x02 != 0 {
        // CommonNetworkRelativeLink: size, flags, NetNameOffset, DeviceNameOffset, provider, [unicode offsets].
        let network = offset(0x14)?;
        let block = &info[network..];
        let block_offset = |at: usize| u32_at(block, at).map(|o| o as usize).filter(|&o| o > 0 && o < block.len());
        let name_offset = block_offset(8)?;
        let net_name = if name_offset > 0x14 {
            block_offset(0x14).map(|o| utf16_z(&block[o..])).unwrap_or_else(|| ansi_z(&block[name_offset..]))
        } else {
            ansi_z(&block[name_offset..])
        };
        if !net_name.is_empty() {
            return Some(join(&net_name, &suffix));
        }
    }
    None
}

fn join(base: &str, suffix: &str) -> String {
    if suffix.is_empty() {
        base.to_owned()
    } else if base.ends_with('\\') {
        format!("{base}{suffix}")
    } else {
        format!("{base}\\{suffix}")
    }
}

/// Path from an item ID list: a drive item followed by file system items (long names from
/// the 0xBEEF0004 extension block where present, short names otherwise).
fn id_list_path(list: &[u8]) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    let mut pos = 0;
    while let Some(size) = u16_at(list, pos) {
        let size = usize::from(size);
        if size < 2 {
            break;
        }
        let Some(item) = list.get(pos..pos + size) else { break };
        let kind = item.get(2).copied().unwrap_or(0);
        match kind & 0x70 {
            // Volume ("C:\").
            0x20 if item.len() > 5 => {
                let drive = ansi_z(&item[3..]);
                if drive.len() >= 2 && drive.as_bytes()[1] == b':' {
                    parts.clear();
                    parts.push(drive.trim_end_matches('\\').to_owned());
                }
            }
            // File or folder.
            0x30 => {
                if let Some(name) = file_item_name(item, kind) {
                    parts.push(name);
                }
            }
            _ => {}
        }
        pos += size;
    }
    (parts.len() >= 2 && parts[0].ends_with(':')).then(|| parts.join("\\"))
}

fn file_item_name(item: &[u8], kind: u8) -> Option<String> {
    // size (2), type (1), unknown (1), file size (4), modified (4), attributes (2), primary name.
    let name_start = 14;
    let unicode = kind & 0x04 != 0;
    let (short, short_len) = if unicode {
        let name = utf16_z(item.get(name_start..)?);
        let len = (name.encode_utf16().count() + 1) * 2;
        (name, len)
    } else {
        let name = ansi_z(item.get(name_start..)?);
        (name.clone(), name.len() + 1)
    };
    let mut ext = name_start + short_len;
    if ext % 2 == 1 {
        ext += 1;
    }
    long_name(item.get(ext..).unwrap_or_default()).or(Some(short)).filter(|name| !name.is_empty())
}

/// Long name of the 0xBEEF0004 extension block (layout per version).
fn long_name(block: &[u8]) -> Option<String> {
    if u32_at(block, 4)? != 0xBEEF_0004 {
        return None;
    }
    let size = usize::from(u16_at(block, 0)?);
    let version = u16_at(block, 2)?;
    let block = block.get(..size.min(block.len()))?;
    let offset = match version {
        0..=2 => return None,
        3..=6 => 0x14,
        7 => 0x26,
        8 => 0x2A,
        _ => 0x2E,
    };
    let name = utf16_z(block.get(offset..)?);
    (!name.is_empty() && name.chars().all(|c| !c.is_control())).then_some(name)
}

fn u16_at(data: &[u8], at: usize) -> Option<u16> {
    data.get(at..at + 2).map(|b| u16::from_le_bytes([b[0], b[1]]))
}

fn u32_at(data: &[u8], at: usize) -> Option<u32> {
    data.get(at..at + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

fn utf16(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|pair| u16::from_le_bytes(*pair)).collect();
    String::from_utf16_lossy(&units)
}

fn utf16_z(bytes: &[u8]) -> String {
    let units: Vec<u16> =
        bytes.as_chunks::<2>().0.iter().map(|pair| u16::from_le_bytes(*pair)).take_while(|&u| u != 0).collect();
    String::from_utf16_lossy(&units)
}

/// ANSI strings: bytes above 0x7F are read as Windows-1252 (good enough for paths and names).
fn ansi(bytes: &[u8]) -> String {
    super::text::decode(bytes)
}

fn ansi_z(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    super::text::decode(&bytes[..end])
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Builds a minimal shell link as Windows writes it (link info + Unicode string data).
    pub(crate) fn build(target: &str, arguments: Option<&str>, working_dir: Option<&str>) -> Vec<u8> {
        let mut flags = HAS_LINK_INFO | IS_UNICODE;
        if arguments.is_some() {
            flags |= HAS_ARGUMENTS;
        }
        if working_dir.is_some() {
            flags |= HAS_WORKING_DIR;
        }
        let mut data = Vec::new();
        data.extend((HEADER_SIZE as u32).to_le_bytes());
        data.extend(LINK_CLSID);
        data.extend(flags.to_le_bytes());
        data.resize(HEADER_SIZE, 0);
        // LinkInfo with a local base path (ANSI) and an empty common path suffix.
        let base: Vec<u8> = target.bytes().chain([0]).collect();
        let header = 0x1Cu32;
        let base_offset = header;
        let suffix_offset = base_offset + base.len() as u32;
        let volume_offset = suffix_offset + 1;
        let volume = [0x10u8, 0, 0, 0, 3, 0, 0, 0, 0x12, 0x34, 0x56, 0x78, 0x10, 0, 0, 0, 0];
        let size = volume_offset + volume.len() as u32;
        for value in [size, header, 0x01, volume_offset, base_offset, 0, suffix_offset] {
            data.extend(value.to_le_bytes());
        }
        data.extend(&base);
        data.push(0);
        data.extend(volume);
        for text in [working_dir, arguments].into_iter().flatten() {
            let units: Vec<u16> = text.encode_utf16().collect();
            data.extend((units.len() as u16).to_le_bytes());
            data.extend(units.iter().flat_map(|u| u.to_le_bytes()));
        }
        data.extend(0u32.to_le_bytes());
        data
    }

    #[test]
    fn reads_target_arguments_and_folder() {
        let data =
            build(r"C:\Windows\System32\wscript.exe", Some(r#""C:\Scripts\backup.vbs" /q"#), Some(r"C:\Scripts"));
        let link = parse(&data).unwrap();
        assert_eq!(link.target.as_deref(), Some(r"C:\Windows\System32\wscript.exe"));
        assert_eq!(link.arguments.as_deref(), Some(r#""C:\Scripts\backup.vbs" /q"#));
        assert_eq!(link.working_dir.as_deref(), Some(r"C:\Scripts"));
    }

    #[test]
    fn reads_the_item_id_list() {
        // My Computer, drive C:, folder "Scripts", file "backup.vbs" (short name only).
        let mut list = Vec::new();
        let root = [
            0x14u8, 0, 0x1F, 0x50, 0xE0, 0x4F, 0xD0, 0x20, 0xEA, 0x3A, 0x69, 0x10, 0xA2, 0xD8, 0x08, 0, 0x2B, 0x30,
            0x30, 0x9D,
        ];
        list.extend(root);
        let mut drive = vec![0x19, 0, 0x2F];
        drive.extend(b"C:\\");
        drive.resize(0x19, 0);
        list.extend(drive);
        for (kind, name) in [(0x31u8, "Scripts"), (0x32u8, "backup.vbs")] {
            let mut item = vec![0, 0, kind, 0];
            item.extend([0u8; 10]);
            item.extend(name.bytes());
            item.push(0);
            if item.len() % 2 == 1 {
                item.push(0);
            }
            let len = item.len() as u16;
            item[..2].copy_from_slice(&len.to_le_bytes());
            list.extend(item);
        }
        list.extend([0, 0]);
        let mut data = Vec::new();
        data.extend((HEADER_SIZE as u32).to_le_bytes());
        data.extend(LINK_CLSID);
        data.extend((HAS_ID_LIST | IS_UNICODE).to_le_bytes());
        data.resize(HEADER_SIZE, 0);
        data.extend((list.len() as u16).to_le_bytes());
        data.extend(list);
        data.extend(0u32.to_le_bytes());
        let link = parse(&data).unwrap();
        assert_eq!(link.target.as_deref(), Some(r"C:\Scripts\backup.vbs"));
    }

    #[test]
    fn rejects_other_files() {
        assert!(parse(b"not a link").is_err());
        let mut data = build(r"C:\x.exe", None, None);
        data.truncate(HEADER_SIZE + 6);
        assert!(parse(&data).is_err());
    }
}
