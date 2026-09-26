//! Office documents that can carry VBA projects, recognised by their content – never by
//! their extension alone, and never opened with Office:
//!
//! * compound files (`.xls`, `.xla`, `.xlt`, `.doc`, `.dot`; also `vbaProject.bin`): every
//!   storage with a `VBA` storage and a `dir` stream in it is a project – the document's own
//!   (`_VBA_PROJECT_CUR`, `Macros`) and those of embedded objects (`ObjectPool/…`);
//! * Office Open XML packages (`.xlsm`, `.xlsb`, `.xlam`, `.docm`, `.dotm`, `.pptm`, …): the
//!   `vbaProject.bin` parts, plus embedded objects and packages;
//! * Access databases (`.mdb`, `.accdb`), see [`super::jet`].
//!
//! A package encrypted with a password to open (or with rights management) is a compound file
//! with `EncryptionInfo` and `EncryptedPackage`: its macros cannot be read. Binary documents
//! encrypted with a password keep their VBA storage unencrypted, so it is read as usual.
//! Files with a macro extension that cannot hold VBA (RTF, HTML or CSV exports, lock files of
//! open documents, Excel 2–4 worksheets, Windows' own ESE databases named `.mdb`, Windows
//! servicing data – see [`super::servicing`]) have no project and are no finding.

use std::io::{Cursor, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use cfb::CompoundFile;

use super::ovba::{self, ProjectStreams};
use super::{jet, markup, servicing};

/// Largest part of a package, embedded object or storage table read into memory.
pub const MAX_PART: u64 = 64 * 1024 * 1024;
/// Embedded objects are followed this deep.
const MAX_DEPTH: usize = 2;
/// More projects than this in one file are not examined (the rest is reported).
const MAX_PROJECTS: usize = 32;
/// How much of a file that is neither compound file, package nor database is searched for
/// macros of the web and XML formats (`ActiveMime`).
const MAX_TEXT_SCAN: u64 = 16 * 1024 * 1024;

/// Why (part of) a document could not be examined.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Obstacle {
    /// Encrypted with a password to open.
    PasswordProtected,
    /// Protected with rights management (IRM/RMS).
    RightsManagement,
    Corrupt,
    /// Recognised, but a format the collector does not read (e.g. Excel 5.0/95 modules).
    Unsupported,
    TooLarge,
}

/// Something that may contain macros but could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unreadable {
    /// Where in the file (`None` = the whole document).
    pub location: Option<String>,
    pub obstacle: Obstacle,
    /// Short technical reason (no file content).
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Container {
    /// OLE compound file.
    #[default]
    Compound,
    /// Office Open XML package (ZIP).
    Package,
    /// Access database (Jet/ACE).
    Access,
    /// A file that cannot hold VBA (text, RTF, lock file, old worksheet format, ESE database,
    /// Windows servicing data).
    Other,
}

impl Container {
    pub fn as_str(self) -> &'static str {
        match self {
            Container::Compound => "compoundFile",
            Container::Package => "openXml",
            Container::Access => "accessDatabase",
            Container::Other => "other",
        }
    }
}

/// A VBA project found in a document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoundProject {
    /// Where the project is stored, e.g. `_VBA_PROJECT_CUR`, `Macros`, `xl/vbaProject.bin`,
    /// `MSysAccessStorage/VBA/VBAProject`.
    pub storage: String,
    /// The embedded object holding the project (`ObjectPool/_1234`, `xl/embeddings/oleObject1.bin`);
    /// `None` for the document's own project.
    pub embedded: Option<String>,
    pub project: Result<ovba::Project, String>,
}

/// What the examination of a document found.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Document {
    pub container: Container,
    pub projects: Vec<FoundProject>,
    pub unreadable: Vec<Unreadable>,
    /// A binary document encrypted with a password to open; its VBA storage is not encrypted.
    pub content_encrypted: bool,
    /// Encryption of the whole package: `agile`, `standard`, `extensible` or `rightsManagement`.
    pub encryption: Option<&'static str>,
}

impl Document {
    fn add_unreadable(&mut self, location: Option<String>, obstacle: Obstacle, message: impl Into<String>) {
        self.unreadable.push(Unreadable { location, obstacle, message: message.into() });
    }
}

const CFB_SIGNATURE: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
/// Extensible Storage Engine database header signature (0x89ABCDEF at offset 4).
const ESE_SIGNATURE: [u8; 4] = [0xEF, 0xCD, 0xAB, 0x89];

/// Examines a document. `name` is the file name (lock files of open documents start with `~$`).
pub fn analyze<R: Read + Seek>(mut reader: R, name: &str) -> Document {
    let mut head = [0u8; 32];
    let read = read_head(&mut reader, &mut head);
    let head = &head[..read];
    let mut document = Document::default();
    if let Err(error) = reader.seek(SeekFrom::Start(0)) {
        document.add_unreadable(None, Obstacle::Corrupt, format!("unreadable: {error}"));
        return document;
    }
    if head.starts_with(&CFB_SIGNATURE) {
        document.container = Container::Compound;
        match CompoundFile::open(reader) {
            Ok(mut file) => scan_compound(&mut file, "", 0, &mut document),
            Err(error) => {
                document.add_unreadable(None, Obstacle::Corrupt, format!("not a valid compound file: {error}"))
            }
        }
    } else if head.starts_with(b"PK\x03\x04") || head.starts_with(b"PK\x05\x06") {
        document.container = Container::Package;
        scan_package(reader, "", 0, &mut document);
    } else if jet::is_database(head) {
        document.container = Container::Access;
        jet::scan(reader, &mut document);
    } else {
        document.container = Container::Other;
        other_format(reader, head, name, &mut document);
    }
    document.projects.truncate(MAX_PROJECTS);
    document
}

fn read_head<R: Read>(reader: &mut R, head: &mut [u8]) -> usize {
    let mut filled = 0;
    while filled < head.len() {
        match reader.read(&mut head[filled..]) {
            Ok(0) | Err(_) => break,
            Ok(count) => filled += count,
        }
    }
    filled
}

// ---- compound files ------------------------------------------------------------------------

/// Finds the projects of a compound file (and of the packages embedded in it).
pub(crate) fn scan_compound<F: Read + Seek>(
    file: &mut CompoundFile<F>,
    prefix: &str,
    depth: usize,
    document: &mut Document,
) {
    if file.is_stream("/EncryptionInfo") && file.is_stream("/EncryptedPackage") {
        let kind = if is_rights_managed(file) { "rightsManagement" } else { encryption_kind(file) };
        document.encryption.get_or_insert(kind);
        let obstacle =
            if kind == "rightsManagement" { Obstacle::RightsManagement } else { Obstacle::PasswordProtected };
        let location = (!prefix.is_empty()).then(|| prefix.trim_end_matches('/').to_owned());
        document.add_unreadable(location, obstacle, format!("encrypted package ({kind})"));
        return;
    }
    if file.is_stream("/\u{9}DRMContent") {
        let location = (!prefix.is_empty()).then(|| prefix.trim_end_matches('/').to_owned());
        document.add_unreadable(location, Obstacle::RightsManagement, "document protected with rights management");
        return;
    }
    if depth == 0 {
        document.content_encrypted = binary_document_encrypted(file);
    }
    let mut storages: Vec<PathBuf> = Vec::new();
    let mut packages: Vec<PathBuf> = Vec::new();
    for entry in file.walk() {
        if entry.is_storage() || entry.is_root() {
            storages.push(entry.path().to_path_buf());
        } else if entry.name().eq_ignore_ascii_case("Package") && depth < MAX_DEPTH {
            packages.push(entry.path().to_path_buf());
        }
    }
    for storage in storages {
        if document.projects.len() >= MAX_PROJECTS {
            document.add_unreadable(None, Obstacle::TooLarge, format!("more than {MAX_PROJECTS} VBA projects"));
            return;
        }
        if !file.is_stream(storage.join("VBA").join("dir")) {
            continue;
        }
        let relative = relative_path(&storage);
        let project = ovba::read_project(&mut CompoundProject { file, base: storage.clone() }).map_err(|e| e.0);
        let (storage_name, embedded) = place(prefix, &relative);
        document.projects.push(FoundProject { storage: storage_name, embedded, project });
    }
    if depth == 0 && document.projects.is_empty() && file.is_stream("/Book") && !file.is_stream("/Workbook") {
        excel95_modules(file, document);
    }
    for package in packages {
        let relative = relative_path(&package);
        match read_stream(file, &package, MAX_PART) {
            Ok(bytes) => {
                if bytes.starts_with(b"PK\x03\x04") {
                    scan_package(Cursor::new(bytes), &format!("{prefix}{relative}/"), depth + 1, document);
                }
            }
            Err(message) => document.add_unreadable(Some(format!("{prefix}{relative}")), Obstacle::Corrupt, message),
        }
    }
}

/// Storage and embedding of a project: the document's own project is stored at the top
/// level (`_VBA_PROJECT_CUR`, `Macros`, or the root of `vbaProject.bin`); deeper projects
/// belong to embedded objects (`ObjectPool/_1234/_VBA_PROJECT_CUR`).
fn place(prefix: &str, relative: &str) -> (String, Option<String>) {
    let storage = format!("{prefix}{relative}").trim_end_matches('/').to_owned();
    let embedded = match relative.rsplit_once('/') {
        Some((parent, _)) => Some(format!("{prefix}{parent}")),
        None if !prefix.is_empty() => Some(prefix.trim_end_matches('/').to_owned()),
        None => None,
    };
    (storage, embedded)
}

fn relative_path(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    text.trim_start_matches('/').to_owned()
}

struct CompoundProject<'a, F> {
    file: &'a mut CompoundFile<F>,
    base: PathBuf,
}

impl<F: Read + Seek> ProjectStreams for CompoundProject<'_, F> {
    fn stream(&mut self, path: &[&str], limit: u64) -> Result<Option<Vec<u8>>, String> {
        let mut full = self.base.clone();
        for part in path {
            if part.is_empty() || part.contains(['/', '\\']) {
                return Ok(None);
            }
            full.push(part);
        }
        if !self.file.is_stream(&full) {
            return Ok(None);
        }
        read_stream(self.file, &full, limit).map(Some)
    }
}

fn read_stream<F: Read + Seek>(file: &mut CompoundFile<F>, path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    let mut stream = file.open_stream(path).map_err(|e| format!("unreadable stream: {e}"))?;
    if stream.len() > limit {
        return Err(format!("stream larger than {limit} bytes"));
    }
    let mut bytes = Vec::with_capacity(stream.len() as usize);
    stream.read_to_end(&mut bytes).map_err(|e| format!("unreadable stream: {e}"))?;
    Ok(bytes)
}

fn is_rights_managed<F: Read + Seek>(file: &CompoundFile<F>) -> bool {
    file.exists("/\u{6}DataSpaces/TransformInfo/DRMEncryptedTransform")
        || file.exists("/\u{6}DataSpaces/DataSpaceInfo/DRMEncryptedDataSpace")
}

/// `EncryptionInfo` starts with its version: 4.4 = agile, x.2 = standard, x.3 = extensible.
fn encryption_kind<F: Read + Seek>(file: &mut CompoundFile<F>) -> &'static str {
    let mut version = [0u8; 4];
    let read = file.open_stream("/EncryptionInfo").map(|mut stream| read_head(&mut stream, &mut version));
    match (read, u16::from_le_bytes([version[0], version[1]]), u16::from_le_bytes([version[2], version[3]])) {
        (Ok(4), 4, 4) => "agile",
        (Ok(4), _, 2) => "standard",
        (Ok(4), _, 3) => "extensible",
        _ => "unknown",
    }
}

/// A binary Excel or Word document encrypted with a password to open: a `FilePass` record
/// right after the first `BOF` of the workbook, or `fEncrypted` in Word's file information block.
fn binary_document_encrypted<F: Read + Seek>(file: &mut CompoundFile<F>) -> bool {
    let mut head = [0u8; 64];
    if let Ok(mut stream) = file.open_stream("/Workbook") {
        let read = read_head(&mut stream, &mut head);
        let head = &head[..read];
        if head.len() >= 8 && u16::from_le_bytes([head[0], head[1]]) == 0x0809 {
            let next = 4 + usize::from(u16::from_le_bytes([head[2], head[3]]));
            return head.get(next..next + 2).is_some_and(|id| u16::from_le_bytes([id[0], id[1]]) == 0x002F);
        }
        return false;
    }
    if let Ok(mut stream) = file.open_stream("/WordDocument") {
        let read = read_head(&mut stream, &mut head);
        return read >= 12 && u16::from_le_bytes([head[0], head[1]]) == 0xA5EC && head[11] & 0x01 != 0;
    }
    false
}

/// Excel 5.0/95 kept VBA in module sheets of the `Book` stream (no VBA storage): a `BoundSheet`
/// record of type 6. The collector does not read them – reported, not skipped.
fn excel95_modules<F: Read + Seek>(file: &mut CompoundFile<F>, document: &mut Document) {
    let Ok(bytes) = read_stream(file, Path::new("/Book"), MAX_PART) else { return };
    let mut pos = 0usize;
    while pos + 4 <= bytes.len() {
        let id = u16::from_le_bytes([bytes[pos], bytes[pos + 1]]);
        let size = usize::from(u16::from_le_bytes([bytes[pos + 2], bytes[pos + 3]]));
        let data = bytes.get(pos + 4..pos + 4 + size).unwrap_or_default();
        pos += 4 + size;
        match id {
            0x0085 if data.len() >= 6 && data[5] == 0x06 => {
                document.add_unreadable(Some("Book".into()), Obstacle::Unsupported, "Excel 5.0/95 VBA module sheet");
                return;
            }
            0x000A => return, // end of the workbook globals
            _ => {}
        }
    }
}

// ---- Office Open XML packages ----------------------------------------------------------------

const VBA_CONTENT_TYPE: &str = "application/vnd.ms-office.vbaproject";

/// Finds the projects of a package: its `vbaProject.bin` parts and those of embedded objects.
pub(crate) fn scan_package<R: Read + Seek>(reader: R, prefix: &str, depth: usize, document: &mut Document) {
    let location = (!prefix.is_empty()).then(|| prefix.trim_end_matches('/').to_owned());
    let mut archive = match zip::ZipArchive::new(reader) {
        Ok(archive) => archive,
        Err(error) => {
            document.add_unreadable(location, Obstacle::Corrupt, format!("not a valid package: {error}"));
            return;
        }
    };
    let names: Vec<String> = archive.file_names().map(str::to_owned).collect();
    let mut vba_parts: Vec<String> = Vec::new();
    if let Some(types) = names.iter().find(|name| name.eq_ignore_ascii_case("[Content_Types].xml"))
        && let Ok(bytes) = read_part(&mut archive, types, 4 * 1024 * 1024)
    {
        let text = super::text::decode(&bytes);
        for tag in markup::tags(&text) {
            let is_vba = tag.attribute("ContentType").is_some_and(|t| t.eq_ignore_ascii_case(VBA_CONTENT_TYPE));
            if tag.is("Override")
                && is_vba
                && let Some(part) = tag.attribute("PartName")
            {
                let part = part.trim_start_matches('/');
                if let Some(name) = names.iter().find(|name| name.eq_ignore_ascii_case(part)) {
                    vba_parts.push(name.clone());
                }
            }
        }
    }
    for name in &names {
        let lower = name.to_ascii_lowercase();
        if lower.ends_with("vbaproject.bin") && !vba_parts.contains(name) {
            vba_parts.push(name.clone());
        }
    }
    vba_parts.sort();
    for part in &vba_parts {
        match read_part(&mut archive, part, MAX_PART) {
            Ok(bytes) => match CompoundFile::open(Cursor::new(bytes)) {
                Ok(mut file) => {
                    if file.is_stream("/VBA/dir") {
                        let project =
                            ovba::read_project(&mut CompoundProject { file: &mut file, base: PathBuf::from("/") })
                                .map_err(|e| e.0);
                        let embedded = embedding_of(prefix, part);
                        document.projects.push(FoundProject { storage: format!("{prefix}{part}"), embedded, project });
                    } else {
                        // A project stored one level down (seen in some tools' output).
                        let mut inner = Document::default();
                        scan_compound(&mut file, &format!("{prefix}{part}/"), depth + 1, &mut inner);
                        document.projects.extend(inner.projects);
                        document.unreadable.extend(inner.unreadable);
                    }
                }
                Err(error) => document.add_unreadable(
                    Some(format!("{prefix}{part}")),
                    Obstacle::Corrupt,
                    format!("not a valid compound file: {error}"),
                ),
            },
            Err((obstacle, message)) => document.add_unreadable(Some(format!("{prefix}{part}")), obstacle, message),
        }
        if document.projects.len() >= MAX_PROJECTS {
            return;
        }
    }
    if depth >= MAX_DEPTH {
        return;
    }
    // Embedded objects (`…/embeddings/oleObject1.bin`) and embedded macro-enabled packages.
    for name in &names {
        let lower = name.to_ascii_lowercase();
        if !lower.contains("/embeddings/") {
            continue;
        }
        let package = [".xlsm", ".xlsb", ".xlam", ".xltm", ".docm", ".dotm", ".pptm", ".potm", ".ppsm", ".ppam"]
            .iter()
            .any(|extension| lower.ends_with(extension));
        if !(lower.ends_with(".bin") || package) {
            continue;
        }
        let bytes = match read_part(&mut archive, name, MAX_PART) {
            Ok(bytes) => bytes,
            Err((obstacle, message)) => {
                document.add_unreadable(Some(format!("{prefix}{name}")), obstacle, message);
                continue;
            }
        };
        let nested = format!("{prefix}{name}/");
        if bytes.starts_with(&CFB_SIGNATURE) {
            if let Ok(mut file) = CompoundFile::open(Cursor::new(bytes)) {
                scan_compound(&mut file, &nested, depth + 1, document);
            }
        } else if bytes.starts_with(b"PK\x03\x04") {
            scan_package(Cursor::new(bytes), &nested, depth + 1, document);
        }
    }
}

/// The embedding of a `vbaProject.bin` part: the document's own project lives next to the
/// main part (`xl/`, `word/`, `ppt/`); a part below `embeddings/` or in a nested package
/// belongs to an embedded object.
fn embedding_of(prefix: &str, part: &str) -> Option<String> {
    if !prefix.is_empty() {
        return Some(prefix.trim_end_matches('/').to_owned());
    }
    let lower = part.to_ascii_lowercase();
    lower.contains("/embeddings/").then(|| part.rsplit_once('/').map_or(part, |(parent, _)| parent).to_owned())
}

fn read_part<R: Read + Seek>(
    archive: &mut zip::ZipArchive<R>,
    name: &str,
    limit: u64,
) -> Result<Vec<u8>, (Obstacle, String)> {
    let entry = archive.by_name(name).map_err(|e| (Obstacle::Corrupt, format!("unreadable part: {e}")))?;
    if entry.size() > limit {
        return Err((Obstacle::TooLarge, format!("part larger than {limit} bytes")));
    }
    let mut bytes = Vec::with_capacity(entry.size() as usize);
    entry.take(limit + 1).read_to_end(&mut bytes).map_err(|e| (Obstacle::Corrupt, format!("unreadable part: {e}")))?;
    if bytes.len() as u64 > limit {
        return Err((Obstacle::TooLarge, format!("part larger than {limit} bytes")));
    }
    Ok(bytes)
}

// ---- files that cannot hold VBA ----------------------------------------------------------------

/// Everything that is neither compound file, package nor database. Formats without VBA are
/// no finding; Word and Excel saved as web page or XML keep macros as `ActiveMime` data,
/// which the collector does not read (reported); anything else unknown is reported as corrupt.
fn other_format<R: Read + Seek>(mut reader: R, head: &[u8], name: &str, document: &mut Document) {
    if head.is_empty() {
        return; // empty file
    }
    let owner_file = name.starts_with("~$");
    let rtf = head.starts_with(b"{\\rtf");
    // Excel 2.x–4.x worksheets (BIFF2–4 without compound file), Word for Windows 1/2.
    let old_binary = matches!(u16::from_le_bytes([head[0], *head.get(1).unwrap_or(&0)]), 0x0009 | 0x0209 | 0x0409)
        || head.starts_with(&[0xDB, 0xA5]);
    // Extensible Storage Engine databases, which Windows also names `.mdb` (User Access Logging in
    // `System32\LogFiles\Sum`), and Windows servicing data (compressed component payloads).
    let ese = head.get(4..8) == Some(&ESE_SIGNATURE[..]);
    if owner_file || rtf || old_binary || ese || servicing::is_servicing_data(head) {
        return;
    }
    match contains_active_mime(&mut reader) {
        Ok(true) => document.add_unreadable(
            None,
            Obstacle::Unsupported,
            "web page or XML document with embedded macros (ActiveMime)",
        ),
        Ok(false) if looks_like_text(head) => {}
        Ok(false) => document.add_unreadable(None, Obstacle::Corrupt, "not an Office document (unknown format)"),
        Err(error) => document.add_unreadable(None, Obstacle::Corrupt, format!("unreadable: {error}")),
    }
}

/// Text formats: printable bytes, a byte order mark or UTF-16 text.
fn looks_like_text(head: &[u8]) -> bool {
    if head.starts_with(&[0xEF, 0xBB, 0xBF]) || head.starts_with(&[0xFF, 0xFE]) || head.starts_with(&[0xFE, 0xFF]) {
        return true;
    }
    head.iter().all(|&byte| byte >= 0x20 || matches!(byte, b'\t' | b'\r' | b'\n' | 0x0C))
}

/// Searches the start of a file for `ActiveMime` data (raw, or Base64 as in MHTML and Word XML).
fn contains_active_mime<R: Read + Seek>(reader: &mut R) -> std::io::Result<bool> {
    const MARKERS: [&[u8]; 3] = [b"ActiveMime", b"QWN0aXZlTWlt", b"editdata.mso"];
    reader.seek(SeekFrom::Start(0))?;
    let mut limited = reader.take(MAX_TEXT_SCAN);
    let mut buffer = vec![0u8; 64 * 1024];
    let mut carry: Vec<u8> = Vec::new();
    loop {
        let read = limited.read(&mut buffer)?;
        if read == 0 {
            return Ok(false);
        }
        carry.extend_from_slice(&buffer[..read]);
        if MARKERS.iter().any(|marker| carry.windows(marker.len()).any(|window| window.eq_ignore_ascii_case(marker))) {
            return Ok(true);
        }
        let keep = carry.len().saturating_sub(16);
        carry.drain(..keep);
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::analysis::ovba::tests::MemoryProject;
    use std::io::Write;

    pub(crate) const MODULE: &str = "Attribute VB_Name = \"Module1\"\r\nSub Check()\r\n    Set re = CreateObject(\"VBScript.RegExp\")\r\nEnd Sub\r\n";

    /// A compound file with a VBA project below `storage` ("" = root); for tests only.
    pub(crate) fn compound_with_project(storage: &str, extra: &[(&str, &[u8])]) -> Vec<u8> {
        let project = MemoryProject::new(&[], &[("Module1", MODULE, true)], "Name=\"VBAProject\"\r\n");
        let mut file = CompoundFile::create(Cursor::new(Vec::new())).unwrap();
        let base = if storage.is_empty() { String::new() } else { format!("/{storage}") };
        if !storage.is_empty() {
            file.create_storage_all(&base).unwrap();
        }
        file.create_storage(format!("{base}/VBA")).unwrap();
        for (path, bytes) in &project.0 {
            let path = if path == "VBA/DIR" {
                "VBA/dir".to_owned()
            } else if path == "VBA/MODULE1" {
                "VBA/Module1".to_owned()
            } else {
                path.clone()
            };
            let mut stream = file.create_stream(format!("{base}/{path}")).unwrap();
            stream.write_all(bytes).unwrap();
        }
        for (path, bytes) in extra {
            let mut stream = file.create_stream(path).unwrap();
            stream.write_all(bytes).unwrap();
        }
        file.flush().unwrap();
        file.into_inner().into_inner()
    }

    fn package(parts: &[(&str, &[u8])]) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, bytes) in parts {
            writer.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            writer.write_all(bytes).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    fn sources(document: &Document) -> Vec<(String, Option<String>, String)> {
        document
            .projects
            .iter()
            .map(|p| {
                let module = &p.project.as_ref().unwrap().modules[0];
                (p.storage.clone(), p.embedded.clone(), module.source.clone().unwrap())
            })
            .collect()
    }

    #[test]
    fn finds_the_project_of_a_workbook_and_of_embedded_objects() {
        let bytes = compound_with_project("_VBA_PROJECT_CUR", &[("/Workbook", &[0x09, 0x08, 0x00, 0x00])]);
        let document = analyze(Cursor::new(bytes), "Budget.xls");
        assert_eq!(document.container, Container::Compound);
        let found = sources(&document);
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].0.as_str(), found[0].1.as_deref()), ("_VBA_PROJECT_CUR", None));
        assert!(found[0].2.contains("VBScript.RegExp"));
        assert!(!document.content_encrypted);

        let embedded = compound_with_project("ObjectPool/_1234/_VBA_PROJECT_CUR", &[]);
        let document = analyze(Cursor::new(embedded), "Letter.doc");
        let found = sources(&document);
        assert_eq!(
            (found[0].0.as_str(), found[0].1.as_deref()),
            ("ObjectPool/_1234/_VBA_PROJECT_CUR", Some("ObjectPool/_1234"))
        );
    }

    #[test]
    fn packages_hold_their_project_in_vba_project_bin() {
        let bin = compound_with_project("", &[]);
        let types = br#"<?xml version="1.0"?><Types><Override PartName="/xl/vbaProject.bin" ContentType="application/vnd.ms-office.vbaProject"/></Types>"#;
        let embedded = package(&[("word/vbaProject.bin", &bin)]);
        let bytes = package(&[
            ("[Content_Types].xml", types),
            ("xl/vbaProject.bin", &bin),
            ("xl/embeddings/Document1.docm", &embedded),
            ("xl/workbook.xml", b"<workbook/>"),
        ]);
        let document = analyze(Cursor::new(bytes), "Budget.xlsm");
        assert_eq!(document.container, Container::Package);
        let found: Vec<(String, Option<String>)> =
            sources(&document).into_iter().map(|(storage, embedded, _)| (storage, embedded)).collect();
        assert_eq!(
            found,
            [
                ("xl/vbaProject.bin".to_owned(), None),
                (
                    "xl/embeddings/Document1.docm/word/vbaProject.bin".to_owned(),
                    Some("xl/embeddings/Document1.docm".to_owned())
                ),
            ]
        );
    }

    #[test]
    fn encrypted_packages_are_unreadable() {
        let mut info = 4u16.to_le_bytes().to_vec();
        info.extend(4u16.to_le_bytes());
        info.extend([0u8; 8]);
        let mut file = CompoundFile::create(Cursor::new(Vec::new())).unwrap();
        file.create_stream("/EncryptionInfo").unwrap().write_all(&info).unwrap();
        file.create_stream("/EncryptedPackage").unwrap().write_all(&[0u8; 64]).unwrap();
        file.flush().unwrap();
        let document = analyze(Cursor::new(file.into_inner().into_inner()), "Secret.xlsm");
        assert_eq!(document.encryption, Some("agile"));
        assert_eq!(document.unreadable[0].obstacle, Obstacle::PasswordProtected);
        assert!(document.projects.is_empty());
    }

    #[test]
    fn binary_documents_encrypted_with_a_password_keep_their_macros_readable() {
        // BOF (size 4) followed by FILEPASS.
        let workbook = [0x09, 0x08, 0x04, 0x00, 0, 6, 5, 0, 0x2F, 0x00, 0x02, 0x00, 0, 0];
        let bytes = compound_with_project("_VBA_PROJECT_CUR", &[("/Workbook", &workbook)]);
        let document = analyze(Cursor::new(bytes), "Locked.xls");
        assert!(document.content_encrypted);
        assert_eq!(document.projects.len(), 1);
    }

    #[test]
    fn files_that_cannot_hold_vba_are_no_finding() {
        for (name, bytes) in [
            ("Report.xls", b"<html><table><tr><td>1</td></tr></table></html>".to_vec()),
            ("Export.xls", b"Name;Amount\r\nA;1\r\n".to_vec()),
            ("Letter.doc", b"{\\rtf1\\ansi Hello}".to_vec()),
            ("~$Budget.xlsm", {
                let mut owner = vec![5u8];
                owner.extend(b"alice");
                owner.extend([0u8; 100]);
                owner
            }),
            ("Old.xls", vec![0x09, 0x02, 0x06, 0x00, 0, 0, 0x10, 0]),
            ("Empty.xlsm", Vec::new()),
            // Extensible Storage Engine database (User Access Logging's SystemIdentity.mdb).
            ("SystemIdentity.mdb", {
                let mut ese = vec![0x3C, 0x5A, 0x1F, 0x9E, 0xEF, 0xCD, 0xAB, 0x89, 0x20, 0x06, 0, 0];
                ese.resize(4096, 0);
                ese
            }),
            // Component store: compressed payload and differential.
            ("Devices.mdb", b"DCS\x01\x01\x00\x00\x00\x00\x60\x00\x00\x0B\xE3\x2F\x10".to_vec()),
            ("Template.xls", servicing::tests::differential_bytes()),
        ] {
            let document = analyze(Cursor::new(bytes), name);
            assert!(document.projects.is_empty() && document.unreadable.is_empty(), "{name}: {document:?}");
        }
        let mht = b"MIME-Version: 1.0\r\n\r\nContent-Location: editdata.mso\r\n\r\nQWN0aXZlTWltZQAAAAAA\r\n".to_vec();
        let document = analyze(Cursor::new(mht), "Web.doc");
        assert_eq!(document.unreadable[0].obstacle, Obstacle::Unsupported);
        let garbage: Vec<u8> = (0..200u8).collect();
        let document = analyze(Cursor::new(garbage), "Broken.xlsm");
        assert_eq!(document.unreadable[0].obstacle, Obstacle::Corrupt);
    }

    #[test]
    fn damaged_projects_are_reported_with_their_reason() {
        let mut file = CompoundFile::create(Cursor::new(Vec::new())).unwrap();
        file.create_storage("/Macros").unwrap();
        file.create_storage("/Macros/VBA").unwrap();
        file.create_stream("/Macros/VBA/dir").unwrap().write_all(b"not compressed").unwrap();
        file.flush().unwrap();
        let document = analyze(Cursor::new(file.into_inner().into_inner()), "Damaged.doc");
        assert!(document.projects[0].project.is_err());
    }
}
