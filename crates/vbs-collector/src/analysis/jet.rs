//! Access databases (`.mdb`, `.accdb`), read as files for their VBA project – never through the
//! database engine, which could change the file. A database is a sequence of pages: page 0
//! describes the database, table definitions (TDEF) list the columns and point to a usage map
//! of their data pages, rows are stored from the end of a data page backwards, and long values
//! (OLE, memo) live in rows of their own (LVAL). Formats: Jet 3 (Access 97, 2 KiB pages),
//! Jet 4 (Access 2000–2003) and ACE (Access 2007 and later), both with 4 KiB pages.
//!
//! Where the VBA project lives:
//!
//! * Access 2002 and later: the system table `MSysAccessStorage` holds a tree of storages
//!   (type 1) and streams (type 2, content in the long value `Lv`) –
//!   `MSysAccessStorage_ROOT/VBA/VBAProject/VBA/dir`, like a compound file;
//! * Access 2000: `MSysAccessObjects` holds a compound file in 3992-byte chunks (row 0 is a
//!   header with the length);
//! * Access 97: `MSysModules2` in a format of its own – reported as not supported when it lists
//!   code modules.
//!
//! Databases encoded by Jet ("Encrypt/Decrypt Database") use RC4 per page with a key stored in
//! the header and are read; ACE databases encrypted with a password cannot be read.
//! Formats: mdbtools (`HACKING.md`) and Jackcess, see `docs/research-notes.md`.

use std::collections::{HashMap, HashSet};
use std::io::{Cursor, Read, Seek, SeekFrom};

use cfb::CompoundFile;

use super::office::{self, Document, FoundProject, Obstacle};
use super::ovba::{self, ProjectStreams};

/// Page 0 names the engine at offset 4.
pub fn is_database(head: &[u8]) -> bool {
    head.len() >= 19 && head[0] == 0 && matches!(&head[4..19], b"Standard Jet DB" | b"Standard ACE DB")
}

/// Largest long value read (a stream of the VBA project).
const MAX_LONG_VALUE: usize = ovba::MAX_STREAM as usize;
/// Most rows read from one table.
const MAX_ROWS: usize = 1_000_000;
/// Pages kept decoded in memory.
const PAGE_CACHE: usize = 512;

/// Examines a database for VBA projects.
pub fn scan<R: Read + Seek>(reader: R, document: &mut Document) {
    let mut db = match Database::open(reader) {
        Ok(db) => db,
        Err(JetError::Encrypted) => {
            document.encryption = Some("database");
            document.unreadable.push(office::Unreadable {
                location: None,
                obstacle: Obstacle::PasswordProtected,
                message: "database encrypted with a password".into(),
            });
            return;
        }
        Err(JetError::Invalid(message)) => {
            document.unreadable.push(office::Unreadable { location: None, obstacle: Obstacle::Corrupt, message });
            return;
        }
    };
    if let Err(message) = scan_database(&mut db, document) {
        document.unreadable.push(office::Unreadable { location: None, obstacle: Obstacle::Corrupt, message });
    }
}

fn scan_database<R: Read + Seek>(db: &mut Database<R>, document: &mut Document) -> Result<(), String> {
    let objects = db.system_tables()?;
    if db.format.jet3 {
        // Access 97 keeps VBA in MSysModules2 in a format of its own. Every database has system
        // rows there (`MSysDb`, `Lock`); code modules have a type with the high bit set
        // (form 0x8000, report 0x8004, standard module 0x8007).
        if let Some(&page) = objects.get("msysmodules2") {
            let table = db.table(page)?;
            let modules = db
                .rows(&table, &["Type"])?
                .iter()
                .filter(|row| row[0].as_int().is_some_and(|t| t & 0x8000 != 0))
                .count();
            if modules > 0 {
                document.unreadable.push(office::Unreadable {
                    location: Some("MSysModules2".into()),
                    obstacle: Obstacle::Unsupported,
                    message: format!("Access 97 VBA ({modules} modules in MSysModules2)"),
                });
            }
        }
        return Ok(());
    }
    if let Some(&page) = objects.get("msysaccessstorage") {
        return storage_projects(db, page, document);
    }
    if let Some(&page) = objects.get("msysaccessobjects") {
        return object_chunks(db, page, document);
    }
    Ok(())
}

/// Access 2002 and later: the storage tree of `MSysAccessStorage`.
fn storage_projects<R: Read + Seek>(db: &mut Database<R>, page: u32, document: &mut Document) -> Result<(), String> {
    let table = db.table(page)?;
    let rows = db.rows(&table, &["Id", "ParentId", "Name", "Type", "Lv"])?;
    let mut nodes: HashMap<i64, Node> = HashMap::new();
    for row in rows {
        let (Some(id), Some(parent), Some(name), Some(kind)) =
            (row[0].as_int(), row[1].as_int(), row[2].as_text(), row[3].as_int())
        else {
            continue;
        };
        let value = match &row[4] {
            Value::Bytes(bytes) => Some(bytes.clone()),
            _ => None,
        };
        nodes.insert(id, Node { parent, name: name.to_owned(), storage: kind == 1, value });
    }
    let mut children: HashMap<i64, Vec<i64>> = HashMap::new();
    for (&id, node) in &nodes {
        if node.parent != id {
            children.entry(node.parent).or_default().push(id);
        }
    }
    for list in children.values_mut() {
        list.sort_unstable();
    }
    let tree = Tree { nodes, children };
    let mut storages: Vec<i64> = tree.nodes.iter().filter(|(_, node)| node.storage).map(|(&id, _)| id).collect();
    storages.sort_unstable();
    for id in storages {
        let Some(vba) = tree.child(id, "VBA") else { continue };
        let is_project = tree.child(vba, "dir").is_some_and(|dir| !tree.nodes[&dir].storage);
        if !is_project || !tree.nodes[&vba].storage {
            continue;
        }
        let storage = format!("MSysAccessStorage/{}", tree.path(id));
        let project = ovba::read_project(&mut StorageProject { db, tree: &tree, base: id }).map_err(|e| e.0);
        document.projects.push(FoundProject { storage, embedded: None, project });
    }
    Ok(())
}

/// Access 2000: a compound file in chunks of `MSysAccessObjects`.
fn object_chunks<R: Read + Seek>(db: &mut Database<R>, page: u32, document: &mut Document) -> Result<(), String> {
    let table = db.table(page)?;
    let mut rows: Vec<(i64, Vec<u8>)> = Vec::new();
    for row in db.rows(&table, &["ID", "Data"])? {
        if let (Some(id), Value::Bytes(data)) = (row[0].as_int(), &row[1]) {
            rows.push((id, db.long_or_plain(&table, "Data", data)?));
        }
    }
    rows.sort_by_key(|(id, _)| *id);
    let Some(((_, header), chunks)) = rows.split_first() else { return Ok(()) };
    // The header names the length of the compound file; its allocation table may still
    // cover sectors beyond that, so whole sectors of the chunks are kept.
    let length = header.get(4..8).map_or(0, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize);
    let mut bytes: Vec<u8> = chunks.iter().flat_map(|(_, chunk)| chunk.iter().copied()).collect();
    let whole_sectors = bytes.len() / 512 * 512;
    if length > 0 && length <= whole_sectors {
        bytes.truncate(whole_sectors);
    }
    if bytes.is_empty() {
        return Ok(());
    }
    match CompoundFile::open(Cursor::new(bytes)) {
        Ok(mut file) => office::scan_compound(&mut file, "MSysAccessObjects/", 1, document),
        Err(error) => document.unreadable.push(office::Unreadable {
            location: Some("MSysAccessObjects".into()),
            obstacle: Obstacle::Corrupt,
            message: format!("not a valid compound file: {error}"),
        }),
    }
    // Projects in the chunked compound file are the database's own.
    for project in &mut document.projects {
        project.embedded = None;
    }
    Ok(())
}

struct Node {
    parent: i64,
    name: String,
    storage: bool,
    /// The raw column value of `Lv` (a long value reference).
    value: Option<Vec<u8>>,
}

struct Tree {
    nodes: HashMap<i64, Node>,
    children: HashMap<i64, Vec<i64>>,
}

impl Tree {
    fn child(&self, parent: i64, name: &str) -> Option<i64> {
        self.children.get(&parent)?.iter().copied().find(|id| self.nodes[id].name.eq_ignore_ascii_case(name))
    }

    /// Path of a node below the root (`VBA/VBAProject`).
    fn path(&self, mut id: i64) -> String {
        let mut parts = Vec::new();
        let mut seen = HashSet::new();
        while let Some(node) = self.nodes.get(&id) {
            if node.parent == id || !seen.insert(id) || node.name.eq_ignore_ascii_case("MSysAccessStorage_ROOT") {
                break;
            }
            parts.push(node.name.clone());
            id = node.parent;
        }
        parts.reverse();
        parts.join("/")
    }
}

struct StorageProject<'a, R> {
    db: &'a mut Database<R>,
    tree: &'a Tree,
    base: i64,
}

impl<R: Read + Seek> ProjectStreams for StorageProject<'_, R> {
    fn stream(&mut self, path: &[&str], limit: u64) -> Result<Option<Vec<u8>>, String> {
        let mut id = self.base;
        for part in path {
            match self.tree.child(id, part) {
                Some(child) => id = child,
                None => return Ok(None),
            }
        }
        let node = &self.tree.nodes[&id];
        if node.storage {
            return Ok(None);
        }
        match &node.value {
            None => Ok(Some(Vec::new())),
            Some(value) => self.db.long_value(value, limit.min(MAX_LONG_VALUE as u64) as usize).map(Some),
        }
    }
}

// ---- pages, tables and rows ------------------------------------------------------------------

enum JetError {
    /// ACE database encrypted with a password.
    Encrypted,
    Invalid(String),
}

#[derive(Debug, Clone, Copy)]
struct Format {
    page_size: usize,
    jet3: bool,
}

impl Format {
    /// Offset of the row count on a data page; the row offsets follow it.
    fn row_count_offset(self) -> usize {
        if self.jet3 { 8 } else { 12 }
    }
}

struct Database<R> {
    reader: R,
    format: Format,
    pages: u32,
    /// RC4 key of databases encoded by Jet (per page: key XOR page number).
    key: Option<u32>,
    cache: HashMap<u32, Vec<u8>>,
}

/// A table definition: its columns and where its data pages are listed.
struct Table {
    page: u32,
    var_columns: u16,
    columns: Vec<Column>,
    usage_map: u32,
}

#[derive(Debug, Clone)]
struct Column {
    name: String,
    kind: u8,
    number: u16,
    var_index: u16,
    fixed: bool,
    fixed_offset: u16,
    length: u16,
}

#[derive(Debug, Clone)]
enum Value {
    Null,
    Int(i64),
    Text(String),
    Bytes(Vec<u8>),
}

impl Value {
    fn as_int(&self) -> Option<i64> {
        match self {
            Value::Int(value) => Some(*value),
            _ => None,
        }
    }

    fn as_text(&self) -> Option<&str> {
        match self {
            Value::Text(text) => Some(text),
            _ => None,
        }
    }
}

const COLUMN_BOOL: u8 = 0x01;
const COLUMN_BYTE: u8 = 0x02;
const COLUMN_INT: u8 = 0x03;
const COLUMN_LONG: u8 = 0x04;
const COLUMN_TEXT: u8 = 0x0A;
const COLUMN_OLE: u8 = 0x0B;
const COLUMN_MEMO: u8 = 0x0C;

impl<R: Read + Seek> Database<R> {
    fn open(mut reader: R) -> Result<Self, JetError> {
        let invalid = |message: &str| JetError::Invalid(message.to_owned());
        let length = reader.seek(SeekFrom::End(0)).map_err(|e| JetError::Invalid(format!("unreadable: {e}")))?;
        reader.seek(SeekFrom::Start(0)).map_err(|e| JetError::Invalid(format!("unreadable: {e}")))?;
        let mut header = vec![0u8; 0x100];
        reader.read_exact(&mut header).map_err(|_| invalid("database header too short"))?;
        let version = header[0x14];
        let jet3 = version == 0;
        let format = Format { page_size: if jet3 { 2048 } else { 4096 }, jet3 };
        // The header is obfuscated with RC4 and a fixed key from offset 0x18.
        let span = if jet3 { 126 } else { 128 };
        rc4(&[0xC7, 0xDA, 0x39, 0x6B], &mut header[0x18..0x18 + span]);
        let key = u32::from_le_bytes([header[0x3E], header[0x3F], header[0x40], header[0x41]]);
        let key = match (key, version) {
            (0, _) => None,
            (key, 0 | 1) => Some(key),
            _ => return Err(JetError::Encrypted),
        };
        let pages = u32::try_from(length / format.page_size as u64).unwrap_or(u32::MAX);
        if pages < 3 {
            return Err(invalid("database too short"));
        }
        Ok(Database { reader, format, pages, key, cache: HashMap::new() })
    }

    fn page(&mut self, number: u32) -> Result<Vec<u8>, String> {
        if let Some(page) = self.cache.get(&number) {
            return Ok(page.clone());
        }
        if number >= self.pages {
            return Err(format!("page {number} beyond the end of the database"));
        }
        let size = self.format.page_size;
        let mut page = vec![0u8; size];
        self.reader
            .seek(SeekFrom::Start(u64::from(number) * size as u64))
            .and_then(|_| self.reader.read_exact(&mut page))
            .map_err(|e| format!("unreadable page {number}: {e}"))?;
        if let (Some(key), true) = (self.key, number != 0) {
            rc4(&(key ^ number).to_le_bytes(), &mut page);
        }
        if self.cache.len() >= PAGE_CACHE {
            self.cache.clear();
        }
        self.cache.insert(number, page.clone());
        Ok(page)
    }

    /// Reads a table definition, following continuation pages.
    fn table(&mut self, page: u32) -> Result<Table, String> {
        let first = self.page(page)?;
        if first[0] != 0x02 {
            return Err(format!("page {page} is not a table definition"));
        }
        let mut tdef = first.clone();
        let mut next = u32::from_le_bytes([first[4], first[5], first[6], first[7]]);
        let mut seen = HashSet::from([page]);
        while next != 0 && seen.insert(next) && seen.len() < 64 {
            let more = self.page(next)?;
            if more[0] != 0x02 {
                return Err(format!("page {next} is not a table definition"));
            }
            tdef.extend_from_slice(&more[8..]);
            next = u32::from_le_bytes([more[4], more[5], more[6], more[7]]);
        }
        let jet3 = self.format.jet3;
        let (var_cols_at, cols_at, real_idx_at, usage_at, cols_start, real_idx_size, entry) =
            if jet3 { (23, 25, 31, 35, 43, 8, 18) } else { (43, 45, 51, 55, 63, 12, 25) };
        let read16 = |at: usize| tdef.get(at..at + 2).map(|b| u16::from_le_bytes([b[0], b[1]]));
        let read32 = |at: usize| tdef.get(at..at + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
        let short = || format!("table definition on page {page} too short");
        let var_columns = read16(var_cols_at).ok_or_else(short)?;
        let column_count = usize::from(read16(cols_at).ok_or_else(short)?);
        let real_indexes = read32(real_idx_at).ok_or_else(short)? as usize;
        let usage_map = read32(usage_at).ok_or_else(short)?;
        if real_indexes > 1000 || column_count > 4096 {
            return Err(format!("implausible table definition on page {page}"));
        }
        let mut at = cols_start + real_indexes * real_idx_size;
        let mut columns = Vec::with_capacity(column_count);
        for _ in 0..column_count {
            let raw = tdef.get(at..at + entry).ok_or_else(short)?;
            let get16 = |offset: usize| u16::from_le_bytes([raw[offset], raw[offset + 1]]);
            let column = if jet3 {
                Column {
                    name: String::new(),
                    kind: raw[0],
                    number: get16(1),
                    var_index: get16(3),
                    fixed: raw[13] & 0x01 != 0,
                    fixed_offset: get16(14),
                    length: get16(16),
                }
            } else {
                Column {
                    name: String::new(),
                    kind: raw[0],
                    number: get16(5),
                    var_index: get16(7),
                    fixed: raw[15] & 0x01 != 0,
                    fixed_offset: get16(21),
                    length: get16(23),
                }
            };
            columns.push(column);
            at += entry;
        }
        for column in &mut columns {
            if jet3 {
                let length = usize::from(*tdef.get(at).ok_or_else(short)?);
                let name = tdef.get(at + 1..at + 1 + length).ok_or_else(short)?;
                column.name = ovba::decode(name, 1252);
                at += 1 + length;
            } else {
                let length = usize::from(read16(at).ok_or_else(short)?);
                let name = tdef.get(at + 2..at + 2 + length).ok_or_else(short)?;
                column.name = utf16(name);
                at += 2 + length;
            }
        }
        Ok(Table { page, var_columns, columns, usage_map })
    }

    /// The data pages of a table, from its usage map.
    fn data_pages(&mut self, table: &Table) -> Result<Vec<u32>, String> {
        let map = self.row_at(table.usage_map)?;
        let Some((&kind, rest)) = map.split_first() else { return Err("empty usage map".into()) };
        let mut pages = Vec::new();
        match kind {
            0 => {
                let start =
                    rest.get(..4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])).ok_or("usage map too short")?;
                for (index, byte) in rest[4..].iter().enumerate() {
                    for bit in 0..8 {
                        if byte & (1 << bit) != 0 {
                            pages.push(start.saturating_add((index * 8 + bit) as u32));
                        }
                    }
                }
            }
            1 => {
                let per_map_page = (self.format.page_size - 4) * 8;
                for (index, pointer) in rest.as_chunks::<4>().0.iter().enumerate() {
                    let map_page = u32::from_le_bytes(*pointer);
                    if map_page == 0 {
                        continue;
                    }
                    let bitmap = self.page(map_page)?;
                    if bitmap[0] != 0x05 {
                        return Err(format!("page {map_page} is not a usage map"));
                    }
                    for (byte_index, byte) in bitmap[4..].iter().enumerate() {
                        for bit in 0..8 {
                            if byte & (1 << bit) != 0 {
                                pages.push((index * per_map_page + byte_index * 8 + bit) as u32);
                            }
                        }
                    }
                }
            }
            other => return Err(format!("unknown usage map type {other}")),
        }
        pages.retain(|&page| page < self.pages);
        Ok(pages)
    }

    /// The bytes of a row addressed by a row pointer (row number in the low byte, page above).
    fn row_at(&mut self, pointer: u32) -> Result<Vec<u8>, String> {
        let (page_number, row) = (pointer >> 8, (pointer & 0xFF) as usize);
        let page = self.page(page_number)?;
        let (start, end) = self.row_bounds(&page, row).ok_or_else(|| format!("no row {row} on page {page_number}"))?;
        Ok(page[start..end].to_vec())
    }

    /// Start and end of row `row` on a page (flags removed).
    fn row_bounds(&self, page: &[u8], row: usize) -> Option<(usize, usize)> {
        let at = self.format.row_count_offset();
        let count = usize::from(u16::from_le_bytes([page[at], page[at + 1]]));
        if row >= count {
            return None;
        }
        let offset = |index: usize| {
            let position = at + 2 + index * 2;
            page.get(position..position + 2).map(|b| usize::from(u16::from_le_bytes([b[0], b[1]]) & 0x1FFF))
        };
        let start = offset(row)?;
        let end = if row == 0 { self.format.page_size } else { offset(row - 1)? };
        (start < end && end <= page.len()).then_some((start, end))
    }

    /// Rows of a table with the named columns (in that order); missing columns are `Null`.
    fn rows(&mut self, table: &Table, names: &[&str]) -> Result<Vec<Vec<Value>>, String> {
        let wanted: Vec<Option<Column>> = names
            .iter()
            .map(|name| table.columns.iter().find(|c| c.name.eq_ignore_ascii_case(name)).cloned())
            .collect();
        let mut rows = Vec::new();
        for page_number in self.data_pages(table)? {
            let page = self.page(page_number)?;
            if page[0] != 0x01 || u32::from_le_bytes([page[4], page[5], page[6], page[7]]) != table.page {
                continue;
            }
            let at = self.format.row_count_offset();
            let count = usize::from(u16::from_le_bytes([page[at], page[at + 1]]));
            for row in 0..count {
                let Some(flags) = page.get(at + 2 + row * 2..at + 4 + row * 2) else { break };
                let flags = u16::from_le_bytes([flags[0], flags[1]]);
                if flags & 0x8000 != 0 {
                    continue; // deleted (overflow targets are marked deleted too)
                }
                let Some((start, end)) = self.row_bounds(&page, row) else { continue };
                let bytes = if flags & 0x4000 != 0 {
                    // Overflow: the row holds a pointer to where the row was moved.
                    let pointer = page.get(start..start + 4).ok_or("overflow row too short")?;
                    self.row_at(u32::from_le_bytes([pointer[0], pointer[1], pointer[2], pointer[3]]))?
                } else {
                    page[start..end].to_vec()
                };
                if let Some(values) = self.crack(table, &bytes, &wanted) {
                    rows.push(values);
                }
                if rows.len() >= MAX_ROWS {
                    return Err(format!("more than {MAX_ROWS} rows in a system table"));
                }
            }
        }
        Ok(rows)
    }

    /// Splits a row into the wanted column values (mdbtools `mdb_crack_row`).
    fn crack(&self, table: &Table, row: &[u8], wanted: &[Option<Column>]) -> Option<Vec<Value>> {
        let jet3 = self.format.jet3;
        let count_size = if jet3 { 1 } else { 2 };
        let column_count = if jet3 {
            usize::from(*row.first()?)
        } else {
            usize::from(u16::from_le_bytes([*row.first()?, *row.get(1)?]))
        };
        let mask_size = column_count.div_ceil(8);
        if mask_size + count_size >= row.len() {
            return None;
        }
        let null_mask = &row[row.len() - mask_size..];
        let mut var_offsets: Vec<usize> = Vec::new();
        let mut row_var_columns = 0usize;
        if table.var_columns > 0 {
            if jet3 {
                row_var_columns = usize::from(row[row.len() - mask_size - 1]);
                var_offsets = jet3_offsets(row, mask_size, row_var_columns)?;
            } else {
                let at = row.len().checked_sub(mask_size + 2)?;
                row_var_columns = usize::from(u16::from_le_bytes([row[at], row[at + 1]]));
                for i in 0..=row_var_columns {
                    let position = row.len().checked_sub(mask_size + 4 + i * 2)?;
                    var_offsets.push(usize::from(u16::from_le_bytes([row[position], row[position + 1]])));
                }
            }
        }
        let row_fixed_columns = column_count.saturating_sub(row_var_columns);
        let mut values = Vec::with_capacity(wanted.len());
        for column in wanted {
            let Some(column) = column else {
                values.push(Value::Null);
                continue;
            };
            let bit = usize::from(column.number);
            let present = null_mask.get(bit / 8).is_some_and(|byte| byte & (1 << (bit % 8)) != 0);
            if column.kind == COLUMN_BOOL {
                values.push(Value::Int(i64::from(present)));
                continue;
            }
            if !present {
                values.push(Value::Null);
                continue;
            }
            let data: &[u8] = if column.fixed {
                // Fixed columns added after the row was written are missing from it.
                let fixed_before = table.columns.iter().filter(|c| c.fixed && c.number < column.number).count();
                if fixed_before >= row_fixed_columns {
                    values.push(Value::Null);
                    continue;
                }
                let start = count_size + usize::from(column.fixed_offset);
                row.get(start..start + usize::from(column.length))?
            } else {
                let index = usize::from(column.var_index);
                if index >= row_var_columns {
                    values.push(Value::Null);
                    continue;
                }
                let (start, end) = (*var_offsets.get(index)?, *var_offsets.get(index + 1)?);
                if start > end || end > row.len() {
                    return None;
                }
                &row[start..end]
            };
            values.push(match column.kind {
                COLUMN_BYTE => Value::Int(i64::from(*data.first()?)),
                COLUMN_INT => Value::Int(i64::from(i16::from_le_bytes([*data.first()?, *data.get(1)?]))),
                COLUMN_LONG => Value::Int(i64::from(i32::from_le_bytes(data.get(..4)?.try_into().ok()?))),
                COLUMN_TEXT if !jet3 => Value::Text(jet4_text(data)),
                COLUMN_TEXT => Value::Text(ovba::decode(data, 1252)),
                _ => Value::Bytes(data.to_vec()),
            });
        }
        Some(values)
    }

    /// A long value (OLE, memo): inline, in one LVAL row, or in a chain of LVAL rows.
    fn long_value(&mut self, value: &[u8], limit: usize) -> Result<Vec<u8>, String> {
        if value.len() < 12 {
            return Ok(value.to_vec());
        }
        let length = usize::from(value[0]) | usize::from(value[1]) << 8 | usize::from(value[2]) << 16;
        let flags = value[3];
        if length > limit {
            return Err(format!("long value larger than {limit} bytes"));
        }
        let pointer = u32::from_le_bytes([value[4], value[5], value[6], value[7]]);
        if flags & 0x80 != 0 {
            return Ok(value[12..].iter().copied().take(length).collect());
        }
        if flags & 0x40 != 0 {
            let mut row = self.row_at(pointer)?;
            row.truncate(length);
            return Ok(row);
        }
        let mut out = Vec::with_capacity(length);
        let mut next = pointer;
        let mut seen = HashSet::new();
        while next >> 8 != 0 && out.len() < length {
            if !seen.insert(next) {
                return Err("long value chain loops".into());
            }
            let row = self.row_at(next)?;
            if row.len() < 4 {
                return Err("long value row too short".into());
            }
            next = u32::from_le_bytes([row[0], row[1], row[2], row[3]]);
            out.extend_from_slice(&row[4..]);
        }
        out.truncate(length);
        Ok(out)
    }

    /// Fixed binary columns (`MSysAccessObjects.Data`) hold the bytes themselves; OLE columns a long value.
    fn long_or_plain(&mut self, table: &Table, column: &str, data: &[u8]) -> Result<Vec<u8>, String> {
        let kind = table.columns.iter().find(|c| c.name.eq_ignore_ascii_case(column)).map(|c| c.kind);
        if matches!(kind, Some(COLUMN_OLE | COLUMN_MEMO)) {
            self.long_value(data, MAX_LONG_VALUE)
        } else {
            Ok(data.to_vec())
        }
    }

    /// Tables of `MSysObjects` (definition on page 2): lower-case name → definition page.
    fn system_tables(&mut self) -> Result<HashMap<String, u32>, String> {
        let objects = self.table(2)?;
        let mut tables = HashMap::new();
        for row in self.rows(&objects, &["Id", "Name", "Type"])? {
            if let (Some(id), Some(name), Some(1)) = (row[0].as_int(), row[1].as_text(), row[2].as_int()) {
                let lower = name.to_ascii_lowercase();
                if lower.starts_with("msys") {
                    tables.insert(lower, (id & 0x00FF_FFFF) as u32);
                }
            }
        }
        Ok(tables)
    }
}

/// Variable column offsets of a Jet 3 row: one byte each, with a jump table for rows longer
/// than 256 bytes (mdbtools `mdb_crack_row3`), relative to the row start; eod is the last entry.
fn jet3_offsets(row: &[u8], mask_size: usize, var_columns: usize) -> Option<Vec<usize>> {
    let row_end = row.len() - 1;
    let mut jumps = (row.len() - 1) / 256;
    let column_pointer = row_end.checked_sub(mask_size + jumps + 1)?;
    if column_pointer.checked_sub(var_columns)? / 256 < jumps {
        jumps -= 1;
    }
    let mut used = 0usize;
    let mut offsets = Vec::with_capacity(var_columns + 1);
    for i in 0..=var_columns {
        while used < jumps && i == usize::from(*row.get(row_end.checked_sub(mask_size + used + 1)?)?) {
            used += 1;
        }
        offsets.push(usize::from(*row.get(column_pointer.checked_sub(i)?)?) + used * 256);
    }
    Some(offsets)
}

/// Jet 4 text: UTF-16 LE, or "compressed" (starting with FF FE: one byte per character,
/// a zero byte switches between compressed and UTF-16).
fn jet4_text(data: &[u8]) -> String {
    let Some(rest) = data.strip_prefix(&[0xFF, 0xFE]) else { return utf16(data) };
    let mut units: Vec<u16> = Vec::with_capacity(rest.len());
    let mut compressed = true;
    let mut pos = 0;
    while pos < rest.len() {
        if rest[pos] == 0 {
            compressed = !compressed;
            pos += 1;
        } else if compressed {
            units.push(u16::from(rest[pos]));
            pos += 1;
        } else if pos + 1 < rest.len() {
            units.push(u16::from_le_bytes([rest[pos], rest[pos + 1]]));
            pos += 2;
        } else {
            break;
        }
    }
    String::from_utf16_lossy(&units)
}

fn utf16(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|pair| u16::from_le_bytes(*pair)).collect();
    String::from_utf16_lossy(&units)
}

/// RC4, used by Jet to obfuscate the header and to encode pages – decoding only.
fn rc4(key: &[u8], data: &mut [u8]) {
    let mut state: [u8; 256] = std::array::from_fn(|i| i as u8);
    let mut j = 0u8;
    for i in 0..256 {
        j = j.wrapping_add(state[i]).wrapping_add(key[i % key.len()]);
        state.swap(i, usize::from(j));
    }
    let (mut i, mut j) = (0u8, 0u8);
    for byte in data {
        i = i.wrapping_add(1);
        j = j.wrapping_add(state[usize::from(i)]);
        state.swap(usize::from(i), usize::from(j));
        *byte ^= state[usize::from(state[usize::from(i)].wrapping_add(state[usize::from(j)]))];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rc4_matches_a_known_vector() {
        // RFC 6229 test vector: key 0x0102030405, first keystream bytes b2 39 63 05.
        let mut data = [0u8; 4];
        rc4(&[1, 2, 3, 4, 5], &mut data);
        assert_eq!(data, [0xB2, 0x39, 0x63, 0x05]);
    }

    #[test]
    fn jet4_text_in_both_encodings() {
        assert_eq!(jet4_text(&[b'V', 0, b'B', 0, b'A', 0]), "VBA");
        assert_eq!(jet4_text(&[0xFF, 0xFE, b'd', b'i', b'r']), "dir");
        // Compressed, switch to UTF-16 for "€", back to compressed.
        assert_eq!(jet4_text(&[0xFF, 0xFE, b'a', 0, 0xAC, 0x20, 0, b'b']), "a€b");
    }

    #[test]
    fn jet3_offsets_without_jumps() {
        // Row: count, fixed 2 bytes, var data "ab" at 3..5, eod 5, var offsets (reverse), var count 1, mask.
        let row = [2u8, 0x11, 0x22, b'a', b'b', 5, 3, 1, 0b11];
        assert_eq!(jet3_offsets(&row, 1, 1), Some(vec![3, 5]));
    }

    #[test]
    fn random_rows_and_texts_do_not_panic() {
        let mut state = 0x2545_F491_4F6C_DD1Du64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for _ in 0..20_000 {
            let length = 1 + (next() % 700) as usize;
            let row: Vec<u8> = (0..length).map(|_| next() as u8).collect();
            let mask = (next() % 8) as usize;
            if mask < row.len() {
                let _ = jet3_offsets(&row, mask, (next() % 40) as usize);
            }
            let _ = jet4_text(&row);
        }
    }

    #[test]
    fn recognises_databases() {
        let mut head = vec![0u8, 1, 0, 0];
        head.extend(b"Standard ACE DB");
        assert!(is_database(&head));
        assert!(!is_database(b"PK\x03\x04"));
    }
}
