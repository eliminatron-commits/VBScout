//! VBA projects as Office stores them ([MS-OVBA]), read from their streams – never through
//! Office. A project is a storage with a `PROJECT` stream (module list, protection) and a
//! `VBA` storage: the compressed `dir` stream lists the references and modules, and every
//! module stream holds the compiled code followed by the module's source code, compressed.
//! The containers (compound files, Office Open XML packages, Access databases) hand the
//! project's streams over through [`ProjectStreams`]; this file works on bytes only.
//!
//! The protection of a project ("lock project for viewing") only hides the code in the
//! editor – the source is stored the same way and is read like any other ([`Protection`]).

use super::text;

/// Largest decompressed stream (a module's source, the `dir` stream).
pub const MAX_DECOMPRESSED: usize = 16 * 1024 * 1024;
/// Largest stream read from a container for a project.
pub const MAX_STREAM: u64 = 32 * 1024 * 1024;
/// More modules than this make the project implausible (damaged or crafted).
const MAX_MODULES: usize = 4096;
/// More references than this make the project implausible.
const MAX_REFERENCES: usize = 1024;

/// Why a project or one of its streams could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OvbaError(pub String);

impl std::fmt::Display for OvbaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

fn error(message: impl Into<String>) -> OvbaError {
    OvbaError(message.into())
}

/// Access to the streams of one project storage.
pub trait ProjectStreams {
    /// A stream below the project storage, e.g. `["VBA", "dir"]`; names are compared
    /// without regard to case. `Ok(None)` if it does not exist.
    fn stream(&mut self, path: &[&str], limit: u64) -> Result<Option<Vec<u8>>, String>;
}

/// A VBA project as far as it could be read.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Project {
    /// `PROJECTNAME`, e.g. `VBAProject`.
    pub name: Option<String>,
    /// Code page of names and source code (`PROJECTCODEPAGE`).
    pub code_page: u16,
    pub references: Vec<Reference>,
    pub modules: Vec<Module>,
    pub protection: Protection,
}

/// A module and its source code – or why the source could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Module {
    pub name: String,
    /// Name of the module stream in the `VBA` storage.
    pub stream: String,
    /// Standard module (`MODULETYPE` procedural); otherwise document, class or form module.
    pub procedural: bool,
    pub source: Result<String, String>,
}

/// A reference of the project to a type library, a control or another project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reference {
    /// Name as the project uses it, e.g. `VBScript_RegExp_55`.
    pub name: String,
    pub kind: ReferenceKind,
    /// Library identifier, e.g. `*\G{3F4DACA7-…}#5.5#0#C:\Windows\System32\vbscript.dll\3#Microsoft VBScript Regular Expressions 5.5`.
    pub libid: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReferenceKind {
    /// A registered type library (`REFERENCEREGISTERED`).
    Registered,
    /// An ActiveX control library (`REFERENCECONTROL`).
    Control,
    /// Another VBA project (`REFERENCEPROJECT`).
    Project,
}

impl Reference {
    /// The file part of the library identifier (`C:\…\vbscript.dll\3`), if any.
    pub fn path(&self) -> Option<&str> {
        self.libid.split('#').nth(3).map(str::trim).filter(|path| !path.is_empty())
    }

    /// The description at the end of the library identifier, if any.
    pub fn description(&self) -> Option<&str> {
        self.libid.split('#').nth(4).map(str::trim).filter(|text| !text.is_empty())
    }

    /// The type library GUID in upper case without braces, if the identifier has one.
    pub fn guid(&self) -> Option<String> {
        let start = self.libid.find('{')? + 1;
        let end = start + self.libid[start..].find('}')?;
        Some(self.libid[start..end].to_ascii_uppercase())
    }
}

/// Protection of a project, from the `PROJECT` stream (`CMG`, `DPB`, `GC`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Protection {
    /// Locked for viewing by the user (`fUserProtected`), or a project password is set.
    pub locked: bool,
    /// The project is marked as not visible (`GC`), e.g. by tools that make it "unviewable".
    pub hidden: bool,
    /// The protection fields are present but could not be decoded (tampered with).
    pub unreadable: bool,
}

// ---- decompression (MS-OVBA 2.4.1) -------------------------------------------------------

/// Decompresses a `CompressedContainer`. After the first chunk, a chunk header with a wrong
/// signature ends the container (trailing bytes after the last chunk are common); any other
/// inconsistency is an error.
pub fn decompress(data: &[u8], limit: usize) -> Result<Vec<u8>, OvbaError> {
    if data.first() != Some(&1) {
        return Err(error("compressed container without signature"));
    }
    let mut out: Vec<u8> = Vec::with_capacity(data.len().saturating_mul(2).min(limit));
    let mut pos = 1usize;
    let mut chunks = 0usize;
    while pos + 2 <= data.len() {
        let header = u16::from_le_bytes([data[pos], data[pos + 1]]);
        let size = usize::from(header & 0x0FFF) + 3;
        if (header >> 12) & 0x07 != 0b011 {
            if chunks == 0 {
                return Err(error("compressed chunk with wrong signature"));
            }
            break;
        }
        let compressed = header & 0x8000 != 0;
        let chunk_end = data.len().min(pos + size);
        let chunk_start = out.len();
        pos += 2;
        if compressed {
            decompress_chunk(data, &mut pos, chunk_end, &mut out, chunk_start)?;
        } else {
            // Raw chunk: 4096 bytes as they are (fewer at the end of a truncated stream).
            let end = data.len().min(pos + 4096);
            out.extend_from_slice(&data[pos..end]);
            pos = end;
        }
        if out.len() > limit {
            return Err(error(format!("decompressed data larger than {limit} bytes")));
        }
        chunks += 1;
    }
    Ok(out)
}

fn decompress_chunk(
    data: &[u8],
    pos: &mut usize,
    chunk_end: usize,
    out: &mut Vec<u8>,
    chunk_start: usize,
) -> Result<(), OvbaError> {
    while *pos < chunk_end {
        let flags = data[*pos];
        *pos += 1;
        for bit in 0..8 {
            if *pos >= chunk_end {
                break;
            }
            if flags & (1 << bit) == 0 {
                out.push(data[*pos]);
                *pos += 1;
                continue;
            }
            if *pos + 2 > chunk_end {
                return Err(error("truncated copy token"));
            }
            let token = u16::from_le_bytes([data[*pos], data[*pos + 1]]);
            *pos += 2;
            let written = out.len() - chunk_start;
            // Bits for the offset: ceil(log2(bytes written in this chunk)), at least 4.
            let bit_count = (usize::BITS - written.saturating_sub(1).leading_zeros()).max(4);
            let length_mask = 0xFFFFu16 >> bit_count;
            let length = usize::from(token & length_mask) + 3;
            let offset = usize::from(token >> (16 - bit_count)) + 1;
            if offset > written {
                return Err(error("copy token points before the chunk"));
            }
            for _ in 0..length {
                let byte = out[out.len() - offset];
                out.push(byte);
            }
            if out.len() - chunk_start > 4096 {
                return Err(error("compressed chunk larger than 4096 bytes"));
            }
        }
    }
    Ok(())
}

// ---- dir stream (MS-OVBA 2.3.4.2) --------------------------------------------------------

/// What the `dir` stream says about the project.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Dir {
    pub name: Option<String>,
    pub code_page: u16,
    pub references: Vec<Reference>,
    pub modules: Vec<DirModule>,
}

/// A module entry of the `dir` stream.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DirModule {
    pub name: String,
    pub stream: String,
    /// Where the compressed source starts in the module stream (`MODULEOFFSET`).
    pub text_offset: u32,
    pub procedural: bool,
}

/// Parses the decompressed `dir` stream: a sequence of records (id, size, data).
pub fn parse_dir(bytes: &[u8]) -> Result<Dir, OvbaError> {
    let mut dir = Dir { code_page: 1252, ..Dir::default() };
    let mut pos = 0usize;
    let mut reference_name: Option<String> = None;
    let mut module: Option<DirModule> = None;
    let mut ended = false;
    while pos + 6 <= bytes.len() {
        let id = u16::from_le_bytes([bytes[pos], bytes[pos + 1]]);
        let size = u32::from_le_bytes([bytes[pos + 2], bytes[pos + 3], bytes[pos + 4], bytes[pos + 5]]) as usize;
        // PROJECTVERSION: the size field is a reserved 4, followed by 6 bytes of version.
        let size = if id == 0x0009 { 6 } else { size };
        let start = pos + 6;
        let Some(data) = bytes.get(start..start.saturating_add(size)) else {
            return Err(error(format!("dir record 0x{id:04X} runs past the end of the stream")));
        };
        pos = start + size;
        match id {
            0x0003 if data.len() >= 2 => dir.code_page = u16::from_le_bytes([data[0], data[1]]),
            0x0004 => dir.name = Some(decode(data, dir.code_page)),
            // REFERENCENAME (MBCS) and its Unicode twin (0x003E) – the Unicode name wins.
            0x0016 => reference_name = Some(decode(data, dir.code_page)),
            0x003E => reference_name = Some(utf16(data)),
            0x000D => {
                let libid = sized_text(data, dir.code_page).unwrap_or_default();
                push_reference(&mut dir, reference_name.take(), ReferenceKind::Registered, libid)?;
            }
            0x000E => {
                let libid = sized_text(data, dir.code_page).unwrap_or_default();
                push_reference(&mut dir, reference_name.take(), ReferenceKind::Project, libid)?;
            }
            // REFERENCECONTROL: the twiddled part; the name records and the extended part
            // (0x0030) follow as records of their own. REFERENCEORIGINAL (0x0033) precedes it.
            0x002F => {
                let libid = sized_text(data, dir.code_page).unwrap_or_default();
                push_reference(&mut dir, reference_name.take(), ReferenceKind::Control, libid)?;
            }
            0x0030 => {
                // The extended part of the control just added: its name records (read just
                // before) and its library identifier belong to it, not to the next reference.
                let extended_name = reference_name.take();
                if let Some(last) = dir.references.last_mut()
                    && last.kind == ReferenceKind::Control
                {
                    if let Some(libid) = sized_text(data, dir.code_page).filter(|libid| !libid.is_empty()) {
                        last.libid = libid;
                    }
                    if last.name.is_empty() {
                        last.name = extended_name.unwrap_or_default();
                    }
                }
            }
            0x0019 => {
                finish_module(&mut dir, module.take())?;
                module = Some(DirModule { name: decode(data, dir.code_page), ..DirModule::default() });
            }
            0x0047 => {
                if let Some(module) = module.as_mut() {
                    let name = utf16(data);
                    if !name.is_empty() {
                        module.name = name;
                    }
                }
            }
            0x001A => {
                if let Some(module) = module.as_mut() {
                    module.stream = decode(data, dir.code_page);
                }
            }
            0x0032 => {
                if let Some(module) = module.as_mut() {
                    let name = utf16(data);
                    if !name.is_empty() {
                        module.stream = name;
                    }
                }
            }
            0x0031 if data.len() >= 4 => {
                if let Some(module) = module.as_mut() {
                    module.text_offset = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
                }
            }
            0x0021 => {
                if let Some(module) = module.as_mut() {
                    module.procedural = true;
                }
            }
            0x002B => finish_module(&mut dir, module.take())?,
            0x0010 => {
                ended = true;
                break;
            }
            _ => {}
        }
    }
    finish_module(&mut dir, module.take())?;
    if !ended && dir.modules.is_empty() && dir.name.is_none() {
        return Err(error("not a dir stream"));
    }
    Ok(dir)
}

fn push_reference(dir: &mut Dir, name: Option<String>, kind: ReferenceKind, libid: String) -> Result<(), OvbaError> {
    if dir.references.len() >= MAX_REFERENCES {
        return Err(error("too many references"));
    }
    dir.references.push(Reference { name: name.unwrap_or_default(), kind, libid });
    Ok(())
}

fn finish_module(dir: &mut Dir, module: Option<DirModule>) -> Result<(), OvbaError> {
    let Some(mut module) = module else { return Ok(()) };
    if dir.modules.len() >= MAX_MODULES {
        return Err(error("too many modules"));
    }
    if module.stream.is_empty() {
        module.stream = module.name.clone();
    }
    dir.modules.push(module);
    Ok(())
}

/// A length-prefixed text (`SizeOfLibid` + `Libid`) at the start of a record.
fn sized_text(data: &[u8], code_page: u16) -> Option<String> {
    let length = u32::from_le_bytes(data.get(..4)?.try_into().ok()?) as usize;
    let text = data.get(4..4usize.checked_add(length)?)?;
    Some(decode(text, code_page))
}

/// Text in the project's code page. Names and identifiers are ASCII almost always; other
/// code pages keep their ASCII part (the rest is read as Windows-1252).
pub fn decode(bytes: &[u8], code_page: u16) -> String {
    if code_page == 65001 {
        return String::from_utf8_lossy(bytes).into_owned();
    }
    if bytes.is_ascii() {
        return String::from_utf8_lossy(bytes).into_owned();
    }
    bytes.iter().map(|&byte| text::windows_1252(byte)).collect()
}

fn utf16(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|pair| u16::from_le_bytes(*pair)).collect();
    String::from_utf16_lossy(&units)
}

// ---- module source -------------------------------------------------------------------------

/// The source code of a module stream: the compressed container at the text offset. Some
/// files carry a wrong offset (the source then sits elsewhere in the stream); like other
/// readers, the stream is then searched for a container whose text starts with `Attribute`.
pub fn module_source(stream: &[u8], text_offset: u32, code_page: u16) -> Result<String, OvbaError> {
    let at_offset = match stream.get(text_offset as usize..) {
        Some(container) if container.first() == Some(&1) => decompress(container, MAX_DECOMPRESSED),
        Some(_) => Err(error("no compressed source at the module offset")),
        None => Err(error("module offset beyond the end of the stream")),
    };
    let bytes = match at_offset {
        Ok(bytes) => bytes,
        Err(first) => search_source(stream).ok_or(first)?,
    };
    Ok(decode(&bytes, code_page))
}

/// Searches a module stream for a compressed container whose text starts with `Attribute`.
fn search_source(stream: &[u8]) -> Option<Vec<u8>> {
    const MAX_CANDIDATES: usize = 4096;
    let mut candidates = 0;
    for start in 0..stream.len().saturating_sub(3) {
        if stream[start] != 1 {
            continue;
        }
        let header = u16::from_le_bytes([stream[start + 1], stream[start + 2]]);
        if header == 0 || (header >> 12) & 0x07 != 0b011 {
            continue;
        }
        candidates += 1;
        if candidates > MAX_CANDIDATES {
            return None;
        }
        // The first chunk decides; only then is the rest decompressed.
        let first_chunk_end = stream.len().min(start + 1 + usize::from(header & 0x0FFF) + 3);
        let Ok(head) = decompress(&stream[start..first_chunk_end], 4096) else { continue };
        let lead = String::from_utf8_lossy(&head[..head.len().min(20)]).to_ascii_lowercase();
        if lead.contains("attribute") {
            return decompress(&stream[start..], MAX_DECOMPRESSED).ok();
        }
    }
    None
}

// ---- PROJECT stream (MS-OVBA 2.3.1) ------------------------------------------------------

/// Reads the protection from the text of the `PROJECT` stream.
pub fn parse_protection(project_stream: &[u8]) -> Protection {
    let text = decode(project_stream, 1252);
    let mut protection = Protection::default();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            break; // [Host Extender Info], [Workspace]
        }
        let Some((key, value)) = line.split_once('=') else { continue };
        let value = value.trim().trim_matches('"');
        match key.trim().to_ascii_uppercase().as_str() {
            "CMG" => match decrypt(value) {
                // ProjectProtectionState: fUserProtected is the lowest bit.
                Some(state) if state.len() == 4 => protection.locked |= state[0] & 1 != 0,
                _ => protection.unreadable = true,
            },
            "DPB" => match decrypt(value) {
                // ProjectPassword: a single 0x00 without a password.
                Some(password) if password.is_empty() => protection.unreadable = true,
                Some(password) => protection.locked |= !(password.len() == 1 && password[0] == 0),
                None => protection.unreadable = true,
            },
            "GC" => match decrypt(value) {
                Some(visibility) if visibility.len() == 1 => protection.hidden = visibility[0] == 0,
                _ => protection.unreadable = true,
            },
            _ => {}
        }
    }
    protection
}

/// Decrypts a value of the `PROJECT` stream (MS-OVBA 2.4.3.3, "Data Encryption"): a hex text of
/// seed, version, project key, ignored bytes, data length and data, each byte chained to the
/// ones before it.
pub fn decrypt(hex: &str) -> Option<Vec<u8>> {
    if !hex.len().is_multiple_of(2) || hex.len() < 16 {
        return None;
    }
    let bytes: Vec<u8> =
        (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok()).collect::<Option<_>>()?;
    let (seed, version_enc, key_enc) = (bytes[0], bytes[1], bytes[2]);
    if seed ^ version_enc != 2 {
        return None;
    }
    let project_key = seed ^ key_enc;
    let (mut unencrypted1, mut encrypted1, mut encrypted2) = (project_key, key_enc, version_enc);
    let mut next = |byte_enc: u8| {
        let byte = byte_enc ^ encrypted2.wrapping_add(unencrypted1);
        encrypted2 = encrypted1;
        encrypted1 = byte_enc;
        unencrypted1 = byte;
        byte
    };
    let ignored = usize::from((seed & 6) / 2);
    let mut rest = bytes[3..].iter().copied();
    for _ in 0..ignored {
        next(rest.next()?);
    }
    let mut length = [0u8; 4];
    for slot in &mut length {
        *slot = next(rest.next()?);
    }
    let length = u32::from_le_bytes(length) as usize;
    let data: Vec<u8> = rest.by_ref().take(length).map(&mut next).collect();
    (data.len() == length && rest.next().is_none()).then_some(data)
}

// ---- the whole project ---------------------------------------------------------------------

/// Reads a project: the `dir` stream is required; modules whose source cannot be read carry
/// the reason instead of the code; a missing `PROJECT` stream leaves the protection unknown.
pub fn read_project(streams: &mut dyn ProjectStreams) -> Result<Project, OvbaError> {
    let dir_bytes = streams
        .stream(&["VBA", "dir"], MAX_STREAM)
        .map_err(|e| error(format!("unreadable dir stream: {e}")))?
        .ok_or_else(|| error("no dir stream"))?;
    let dir = parse_dir(&decompress(&dir_bytes, MAX_DECOMPRESSED)?)?;
    let protection = match streams.stream(&["PROJECT"], 1024 * 1024) {
        Ok(Some(bytes)) => parse_protection(&bytes),
        _ => Protection::default(),
    };
    let mut modules = Vec::with_capacity(dir.modules.len());
    for module in &dir.modules {
        let source = match streams.stream(&["VBA", &module.stream], MAX_STREAM) {
            Ok(Some(bytes)) => module_source(&bytes, module.text_offset, dir.code_page).map_err(|e| e.0),
            Ok(None) => Err("module stream missing".to_owned()),
            Err(e) => Err(format!("unreadable module stream: {e}")),
        };
        modules.push(Module {
            name: module.name.clone(),
            stream: module.stream.clone(),
            procedural: module.procedural,
            source,
        });
    }
    Ok(Project { name: dir.name, code_page: dir.code_page, references: dir.references, modules, protection })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::collections::HashMap;

    /// Compresses like Office does (MS-OVBA 2.4.1.3.6): literal and copy tokens per chunk,
    /// the longest earlier match wins. For tests only – the collector never writes.
    pub(crate) fn compress(data: &[u8]) -> Vec<u8> {
        let mut out = vec![1u8];
        for chunk in data.chunks(4096) {
            let mut body = Vec::new();
            let mut pos = 0;
            while pos < chunk.len() {
                let flag_at = body.len();
                body.push(0u8);
                for bit in 0..8 {
                    if pos >= chunk.len() {
                        break;
                    }
                    let bit_count = (usize::BITS - pos.saturating_sub(1).leading_zeros()).max(4);
                    let max_length = (0xFFFFusize >> bit_count) + 3;
                    let (mut best_len, mut best_off) = (0, 0);
                    for candidate in (0..pos).rev() {
                        let mut len = 0;
                        while len < max_length && pos + len < chunk.len() && chunk[candidate + len] == chunk[pos + len]
                        {
                            len += 1;
                        }
                        if len > best_len {
                            (best_len, best_off) = (len, pos - candidate);
                        }
                    }
                    if best_len >= 3 {
                        let token = (((best_off - 1) << (16 - bit_count)) | (best_len - 3)) as u16;
                        body.extend(token.to_le_bytes());
                        body[flag_at] |= 1 << bit;
                        pos += best_len;
                    } else {
                        body.push(chunk[pos]);
                        pos += 1;
                    }
                }
            }
            let header = 0xB000u16 | ((body.len() + 2 - 3) as u16 & 0x0FFF);
            out.extend(header.to_le_bytes());
            out.extend(body);
        }
        out
    }

    /// Encrypts a value like the VBA editor does (MS-OVBA 2.4.3.2), with a fixed seed.
    pub(crate) fn encrypt(seed: u8, project_key: u8, data: &[u8]) -> String {
        let version_enc = seed ^ 2;
        let key_enc = seed ^ project_key;
        let mut out = vec![seed, version_enc, key_enc];
        let (mut unencrypted1, mut encrypted1, mut encrypted2) = (project_key, key_enc, version_enc);
        let ignored = usize::from((seed & 6) / 2);
        let plain: Vec<u8> = std::iter::repeat_n(0u8, ignored)
            .chain((data.len() as u32).to_le_bytes())
            .chain(data.iter().copied())
            .collect();
        for byte in plain {
            let byte_enc = byte ^ encrypted2.wrapping_add(unencrypted1);
            out.push(byte_enc);
            encrypted2 = encrypted1;
            encrypted1 = byte_enc;
            unencrypted1 = byte;
        }
        out.iter().map(|b| format!("{b:02X}")).collect()
    }

    fn record(id: u16, data: &[u8]) -> Vec<u8> {
        let mut out = id.to_le_bytes().to_vec();
        out.extend((data.len() as u32).to_le_bytes());
        out.extend(data);
        out
    }

    fn sized(text: &str) -> Vec<u8> {
        let mut out = (text.len() as u32).to_le_bytes().to_vec();
        out.extend(text.as_bytes());
        out
    }

    fn utf16le(text: &str) -> Vec<u8> {
        text.encode_utf16().flat_map(u16::to_le_bytes).collect()
    }

    /// A `dir` stream (uncompressed) with the given references and modules (name, offset, procedural).
    pub(crate) fn dir_stream(references: &[(&str, &str)], modules: &[(&str, u32, bool)]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend(record(0x0001, &3u32.to_le_bytes()));
        out.extend(record(0x0002, &0x409u32.to_le_bytes()));
        out.extend(record(0x0014, &0x409u32.to_le_bytes()));
        out.extend(record(0x0003, &1252u16.to_le_bytes()));
        out.extend(record(0x0004, b"VBAProject"));
        out.extend(record(0x0005, b""));
        out.extend(record(0x0040, b""));
        out.extend(record(0x0006, b""));
        out.extend(record(0x003D, b""));
        out.extend(record(0x0007, &0u32.to_le_bytes()));
        out.extend(record(0x0008, &0u32.to_le_bytes()));
        // PROJECTVERSION: reserved size 4, then major (4) and minor (2).
        out.extend(0x0009u16.to_le_bytes());
        out.extend(4u32.to_le_bytes());
        out.extend(0x1234_5678u32.to_le_bytes());
        out.extend(3u16.to_le_bytes());
        out.extend(record(0x000C, b""));
        out.extend(record(0x003C, b""));
        for (name, libid) in references {
            out.extend(record(0x0016, name.as_bytes()));
            out.extend(record(0x003E, &utf16le(name)));
            let mut data = sized(libid);
            data.extend([0u8; 6]);
            out.extend(record(0x000D, &data));
        }
        out.extend(record(0x000F, &(modules.len() as u16).to_le_bytes()));
        out.extend(record(0x0013, &0xFFFFu16.to_le_bytes()));
        for (name, offset, procedural) in modules {
            out.extend(record(0x0019, name.as_bytes()));
            out.extend(record(0x0047, &utf16le(name)));
            out.extend(record(0x001A, name.as_bytes()));
            out.extend(record(0x0032, &utf16le(name)));
            out.extend(record(0x001C, b""));
            out.extend(record(0x0048, b""));
            out.extend(record(0x0031, &offset.to_le_bytes()));
            out.extend(record(0x001E, &0u32.to_le_bytes()));
            out.extend(record(0x002C, &0xFFFFu16.to_le_bytes()));
            out.extend(record(if *procedural { 0x0021 } else { 0x0022 }, b""));
            out.extend(record(0x002B, b""));
        }
        out.extend(record(0x0010, b""));
        out
    }

    /// Streams of a project held in memory, keyed by upper-case path.
    #[derive(Default)]
    pub(crate) struct MemoryProject(pub HashMap<String, Vec<u8>>);

    impl MemoryProject {
        /// A project whose modules have `code` after `pcode` bytes of compiled code.
        pub(crate) fn new(references: &[(&str, &str)], modules: &[(&str, &str, bool)], project: &str) -> Self {
            let mut streams = HashMap::new();
            let pcode = vec![0xAAu8; 37];
            let dir_modules: Vec<(&str, u32, bool)> =
                modules.iter().map(|(name, _, procedural)| (*name, pcode.len() as u32, *procedural)).collect();
            streams.insert("VBA/DIR".to_owned(), compress(&dir_stream(references, &dir_modules)));
            for (name, code, _) in modules {
                let mut stream = pcode.clone();
                stream.extend(compress(code.as_bytes()));
                streams.insert(format!("VBA/{}", name.to_ascii_uppercase()), stream);
            }
            streams.insert("PROJECT".to_owned(), project.as_bytes().to_vec());
            Self(streams)
        }
    }

    impl ProjectStreams for MemoryProject {
        fn stream(&mut self, path: &[&str], _limit: u64) -> Result<Option<Vec<u8>>, String> {
            Ok(self.0.get(&path.join("/").to_ascii_uppercase()).cloned())
        }
    }

    #[test]
    fn decompression_round_trips() {
        let samples: Vec<Vec<u8>> = vec![
            b"abcdefghijklmnopqrstuv.".to_vec(),
            b"#aaabcdefaaaaghijaaaabcdefaaaaghijaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_vec(),
            (0..20_000u32).map(|i| (i % 251) as u8).collect(),
            "Attribute VB_Name = \"Module1\"\r\n".repeat(400).into_bytes(),
            Vec::new(),
        ];
        for sample in samples {
            let compressed = compress(&sample);
            assert_eq!(decompress(&compressed, MAX_DECOMPRESSED).unwrap(), sample);
        }
    }

    #[test]
    fn decompression_rejects_damage_and_ignores_trailing_bytes() {
        assert!(decompress(b"", 100).is_err());
        assert!(decompress(&[0x02, 0x00, 0x30], 100).is_err());
        // A copy token as the first token points before the chunk.
        assert!(decompress(&[0x01, 0x03, 0xB0, 0x01, 0x00, 0x00], 100).is_err());
        let mut compressed = compress(b"Sub Main()\r\nEnd Sub\r\n");
        compressed.extend([0u8; 16]);
        assert_eq!(decompress(&compressed, 1000).unwrap(), b"Sub Main()\r\nEnd Sub\r\n");
        assert!(decompress(&compress(&[b'x'; 5000]), 1000).is_err(), "limit");
    }

    #[test]
    fn raw_chunks_are_copied() {
        let mut container = vec![1u8];
        container.extend(0x3FFFu16.to_le_bytes()); // signature 0b011, not compressed
        container.extend([b'a'; 4096]);
        assert_eq!(decompress(&container, MAX_DECOMPRESSED).unwrap(), vec![b'a'; 4096]);
    }

    #[test]
    fn dir_stream_lists_references_and_modules() {
        let libid = r"*\G{3F4DACA7-160D-11D2-A8E9-00104B365C9F}#5.5#0#C:\Windows\System32\vbscript.dll\3#Microsoft VBScript Regular Expressions 5.5";
        let dir = parse_dir(&dir_stream(
            &[("VBScript_RegExp_55", libid)],
            &[("Module1", 100, true), ("ThisWorkbook", 0, false)],
        ))
        .unwrap();
        assert_eq!(dir.name.as_deref(), Some("VBAProject"));
        assert_eq!(dir.code_page, 1252);
        let reference = &dir.references[0];
        assert_eq!(reference.name, "VBScript_RegExp_55");
        assert_eq!(reference.guid().as_deref(), Some("3F4DACA7-160D-11D2-A8E9-00104B365C9F"));
        assert_eq!(reference.path(), Some(r"C:\Windows\System32\vbscript.dll\3"));
        assert_eq!(reference.description(), Some("Microsoft VBScript Regular Expressions 5.5"));
        assert_eq!(dir.modules.len(), 2);
        assert_eq!(
            (dir.modules[0].name.as_str(), dir.modules[0].text_offset, dir.modules[0].procedural),
            ("Module1", 100, true)
        );
        assert!(!dir.modules[1].procedural);
        assert!(parse_dir(b"\x04\x00\xFF\xFF\xFF\x7F").is_err());
    }

    #[test]
    fn protection_fields_are_decrypted() {
        // Values of a real, unprotected workbook (Apache POI's SimpleMacro.xls).
        let open = parse_protection(
            b"ID=\"{C812BC5D-3DF7-4BA1-922A-65CF6CDF45FB}\"\r\nCMG=\"D1D3FBF9FFF9FFF9FFF9FF\"\r\nDPB=\"A2A0881B581C581C58\"\r\nGC=\"7371594A2B4B2B4BD4\"\r\n",
        );
        assert_eq!(open, Protection::default());
        let locked = format!(
            "CMG=\"{}\"\r\nDPB=\"{}\"\r\nGC=\"{}\"\r\n[Workspace]\r\nCMG=\"00\"\r\n",
            encrypt(0x31, 0x2A, &1u32.to_le_bytes()),
            encrypt(0x57, 0x2A, &[0xFF; 29]),
            encrypt(0x44, 0x2A, &[0xFF]),
        );
        assert_eq!(parse_protection(locked.as_bytes()), Protection { locked: true, hidden: false, unreadable: false });
        // Tools that make a project "unviewable" write values that do not decrypt.
        let unviewable = "CMG=\"0000000000000000\"\r\nDPB=\"0000000000000000\"\r\nGC=\"0000000000000000\"\r\n";
        assert!(parse_protection(unviewable.as_bytes()).unreadable);
        assert_eq!(decrypt(&encrypt(0x0B, 0x77, b"hello")).as_deref(), Some(&b"hello"[..]));
    }

    /// Random bytes never make the decoders panic.
    #[test]
    fn random_input_is_rejected_or_read_without_panic() {
        let mut state = 0x9E37_79B9_7F4A_7C15u64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for round in 0..20_000 {
            let length = (next() % 600) as usize;
            let mut bytes: Vec<u8> = (0..length).map(|_| next() as u8).collect();
            if round % 2 == 0 && !bytes.is_empty() {
                bytes[0] = 1; // a container signature, so decompression goes further
                if bytes.len() > 2 {
                    bytes[2] = (bytes[2] & 0x8F) | 0x30; // and a valid chunk signature
                }
            }
            if let Ok(plain) = decompress(&bytes, 64 * 1024) {
                let _ = parse_dir(&plain);
            }
            let _ = parse_dir(&bytes);
            let _ = module_source(&bytes, (next() % 700) as u32, 1252);
            let _ = parse_protection(&bytes);
            let hex: String = bytes.iter().take(40).map(|b| format!("{b:02X}")).collect();
            let _ = decrypt(&hex);
        }
    }

    #[test]
    fn reads_a_project_and_its_modules() {
        let mut project = MemoryProject::new(
            &[],
            &[("Module1", "Attribute VB_Name = \"Module1\"\r\nSub Main()\r\nEnd Sub\r\n", true)],
            "Name=\"VBAProject\"\r\n",
        );
        let read = read_project(&mut project).unwrap();
        assert_eq!(read.modules.len(), 1);
        assert!(read.modules[0].source.as_ref().unwrap().contains("Sub Main()"));
        // A wrong text offset: the source is found by searching the stream.
        let stream = project.0.get_mut("VBA/MODULE1").unwrap();
        stream.splice(0..0, [0u8; 11]);
        let read = read_project(&mut project).unwrap();
        assert!(read.modules[0].source.as_ref().unwrap().contains("Sub Main()"));
        // A module stream without source code.
        project.0.insert("VBA/MODULE1".into(), vec![0u8; 64]);
        let read = read_project(&mut project).unwrap();
        assert!(read.modules[0].source.is_err());
        project.0.remove("VBA/DIR");
        assert!(read_project(&mut project).is_err());
    }
}
