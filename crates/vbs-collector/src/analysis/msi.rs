//! Windows Installer packages (`.msi`), read as files – never through msi.dll, which could run
//! or change something. A package is an OLE compound file whose tables are streams:
//! `_StringPool`/`_StringData` hold all strings, `_Columns` the table schemas, and every table
//! is stored column by column. Only the tables needed to find script custom actions are read:
//! `CustomAction`, `Binary` streams, `Property`, the sequence tables and `ControlEvent`.
//! Nested packages (custom action type 7, stored as sub-storages) are examined as well.

use std::collections::{HashMap, HashSet};
use std::io::{Read, Seek};
use std::path::{Path, PathBuf};

use cfb::CompoundFile;

use super::text;

/// A script custom action (VBScript) of a package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptAction {
    /// Action name; `Storage/Action` for actions of a nested package.
    pub name: String,
    /// Numeric custom action type (including option flags).
    pub action_type: i32,
    pub source: SourceKind,
    /// `Source` column: Binary key, File key or Property name.
    pub source_ref: Option<String>,
    /// `Target` column: function name, or the script itself for inline actions.
    pub target: Option<String>,
    /// `msidbCustomActionTypeContinue`: errors are ignored.
    pub continue_on_error: bool,
    /// Referenced by a sequence table or a `DoAction` control event.
    pub scheduled: bool,
    /// Condition of the first sequence entry, if any.
    pub condition: Option<String>,
    /// The script text, where the package contains it (binary stream, inline or property).
    pub script: Option<String>,
}

/// Where a script custom action takes its code from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    /// Type 6: a stream of the `Binary` table.
    Binary,
    /// Type 22: a file installed with the product.
    InstalledFile,
    /// Type 38: the `Target` column.
    Inline,
    /// Type 54: a property value.
    Property,
}

impl SourceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            SourceKind::Binary => "binaryTable",
            SourceKind::InstalledFile => "installedFile",
            SourceKind::Inline => "inline",
            SourceKind::Property => "property",
        }
    }
}

/// What the analysis of a package found.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Package {
    pub product_name: Option<String>,
    pub product_version: Option<String>,
    pub product_code: Option<String>,
    /// VBScript custom actions (JScript and native actions are not listed).
    pub vbscript_actions: Vec<ScriptAction>,
}

/// Why a package could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MsiError(pub String);

/// Largest table or script stream read into memory.
const MAX_STREAM: u64 = 32 * 1024 * 1024;
/// Nested packages are followed this deep.
const MAX_NESTING: usize = 2;

const TYPE_MASK: i32 = 0x07;
const SOURCE_MASK: i32 = 0x30;
const VBSCRIPT: i32 = 0x06;
const INSTALL: i32 = 0x07;
const CONTINUE: i32 = 0x40;

/// Reads a package and lists its VBScript custom actions.
pub fn analyze<F: Read + Seek>(inner: F) -> Result<Package, MsiError> {
    let mut file = CompoundFile::open(inner).map_err(|error| MsiError(format!("not a valid package: {error}")))?;
    let mut package = Package::default();
    analyze_storage(&mut file, Path::new("/"), "", 0, &mut package)?;
    Ok(package)
}

fn analyze_storage<F: Read + Seek>(
    file: &mut CompoundFile<F>,
    storage: &Path,
    prefix: &str,
    depth: usize,
    package: &mut Package,
) -> Result<(), MsiError> {
    let db = Database::open(file, storage)?;
    let properties = db.properties(file)?;
    if depth == 0 {
        package.product_name = properties.get("ProductName").cloned();
        package.product_version = properties.get("ProductVersion").cloned();
        package.product_code = properties.get("ProductCode").cloned();
    }
    let Some(actions) = db.table(file, "CustomAction")? else { return Ok(()) };
    let (scheduled, conditions) = db.scheduled_actions(file)?;
    for row in &actions.rows {
        let name = actions.text(row, "Action").unwrap_or_default();
        let Some(action_type) = actions.int(row, "Type") else { continue };
        let source_ref = actions.text(row, "Source");
        let target = actions.text(row, "Target");
        match action_type & TYPE_MASK {
            VBSCRIPT => {
                let source = match action_type & SOURCE_MASK {
                    0x00 => SourceKind::Binary,
                    0x10 => SourceKind::InstalledFile,
                    0x20 => SourceKind::Inline,
                    _ => SourceKind::Property,
                };
                let script = match source {
                    SourceKind::Binary => match source_ref.as_deref() {
                        Some(key) => db.binary_stream(file, key)?.map(|bytes| text::decode(&bytes)),
                        None => None,
                    },
                    SourceKind::Inline => target.clone(),
                    SourceKind::Property => source_ref.as_deref().and_then(|key| properties.get(key).cloned()),
                    SourceKind::InstalledFile => None,
                };
                package.vbscript_actions.push(ScriptAction {
                    name: format!("{prefix}{name}"),
                    action_type,
                    source,
                    source_ref,
                    target: if source == SourceKind::Inline { None } else { target },
                    continue_on_error: action_type & CONTINUE != 0,
                    scheduled: scheduled.contains(&name),
                    condition: conditions.get(&name).cloned(),
                    script,
                });
            }
            // Nested installation of a package stored in a sub-storage.
            INSTALL if action_type & SOURCE_MASK == 0 && depth < MAX_NESTING => {
                if let Some(sub) = source_ref.as_deref().and_then(|key| db.storage(key)) {
                    let nested_prefix = format!("{prefix}{}/", source_ref.as_deref().unwrap_or_default());
                    analyze_storage(file, &sub, &nested_prefix, depth + 1, package)?;
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// Decodes a stream name: MSI packs `[0-9A-Za-z._]` two to a character; tables start with U+4840.
pub fn decode_stream_name(name: &str) -> (String, bool) {
    fn mime(value: u32) -> char {
        let value = value & 0x3F;
        let byte = match value {
            0..=9 => b'0' + value as u8,
            10..=35 => b'A' + (value - 10) as u8,
            36..=61 => b'a' + (value - 36) as u8,
            62 => b'.',
            _ => b'_',
        };
        char::from(byte)
    }
    let mut out = String::with_capacity(name.len() * 2);
    let mut table = false;
    for (index, c) in name.chars().enumerate() {
        let code = c as u32;
        if index == 0 && code == 0x4840 {
            table = true;
        } else if (0x3800..0x4800).contains(&code) {
            let value = code - 0x3800;
            out.push(mime(value));
            out.push(mime(value >> 6));
        } else if (0x4800..0x4840).contains(&code) {
            out.push(mime(code - 0x4800));
        } else {
            out.push(c);
        }
    }
    (out, table)
}

#[derive(Debug, Clone, Copy)]
enum ColumnKind {
    Int16,
    Int32,
    String,
    Binary,
}

#[derive(Debug, Clone)]
struct Column {
    name: String,
    kind: ColumnKind,
}

#[derive(Debug, Clone, Copy)]
enum Value {
    Null,
    Int(i32),
    Str(u32),
}

struct Table<'s> {
    columns: Vec<Column>,
    rows: Vec<Vec<Value>>,
    strings: &'s [Option<String>],
}

impl Table<'_> {
    fn index(&self, column: &str) -> Option<usize> {
        self.columns.iter().position(|c| c.name.eq_ignore_ascii_case(column))
    }

    fn text(&self, row: &[Value], column: &str) -> Option<String> {
        match row.get(self.index(column)?)? {
            Value::Str(id) => self.strings.get(*id as usize).cloned().flatten(),
            Value::Int(value) => Some(value.to_string()),
            Value::Null => None,
        }
    }

    fn int(&self, row: &[Value], column: &str) -> Option<i32> {
        match row.get(self.index(column)?)? {
            Value::Int(value) => Some(*value),
            _ => None,
        }
    }
}

/// The string pool and the stream names of one (possibly nested) package.
struct Database {
    strings: Vec<Option<String>>,
    long_refs: bool,
    /// Decoded table name → stream path.
    tables: HashMap<String, PathBuf>,
    /// Decoded name of other streams (e.g. `Binary.Script1`) → stream path.
    streams: HashMap<String, PathBuf>,
    /// Decoded name of sub-storages → path.
    storages: HashMap<String, PathBuf>,
    schemas: HashMap<String, Vec<Column>>,
}

impl Database {
    fn open<F: Read + Seek>(file: &mut CompoundFile<F>, storage: &Path) -> Result<Self, MsiError> {
        let mut tables = HashMap::new();
        let mut streams = HashMap::new();
        let mut storages = HashMap::new();
        let entries = file.read_storage(storage).map_err(|e| MsiError(format!("unreadable storage: {e}")))?;
        for entry in entries {
            let (name, table) = decode_stream_name(entry.name());
            let path = entry.path().to_path_buf();
            if entry.is_storage() {
                storages.insert(name, path);
            } else if table {
                tables.insert(name, path);
            } else {
                streams.insert(name, path);
            }
        }
        let pool = read_stream(file, tables.get("_StringPool"))?.ok_or_else(|| MsiError("no string pool".into()))?;
        let data = read_stream(file, tables.get("_StringData"))?.unwrap_or_default();
        let (strings, long_refs) = string_pool(&pool, &data)?;
        let mut db = Database { strings, long_refs, tables, streams, storages, schemas: HashMap::new() };
        db.load_schemas(file)?;
        Ok(db)
    }

    fn load_schemas<F: Read + Seek>(&mut self, file: &mut CompoundFile<F>) -> Result<(), MsiError> {
        let columns = vec![
            Column { name: "Table".into(), kind: ColumnKind::String },
            Column { name: "Number".into(), kind: ColumnKind::Int16 },
            Column { name: "Name".into(), kind: ColumnKind::String },
            Column { name: "Type".into(), kind: ColumnKind::Int16 },
        ];
        let Some(bytes) = read_stream(file, self.tables.get("_Columns"))? else {
            return Err(MsiError("no column catalog".into()));
        };
        let rows = self.rows(&columns, &bytes);
        let mut schemas: HashMap<String, Vec<(i32, Column)>> = HashMap::new();
        for row in rows {
            let (Value::Str(table), Value::Int(number), Value::Str(name), Value::Int(bits)) =
                (row[0], row[1], row[2], row[3])
            else {
                continue;
            };
            let (Some(Some(table)), Some(Some(name))) =
                (self.strings.get(table as usize), self.strings.get(name as usize))
            else {
                continue;
            };
            let bits = bits as u32 & 0xFFFF;
            let kind = if bits & 0x0800 != 0 {
                if bits & !0x1000 == 0x0900 { ColumnKind::Binary } else { ColumnKind::String }
            } else if bits & 0xFF <= 2 {
                ColumnKind::Int16
            } else {
                ColumnKind::Int32
            };
            schemas.entry(table.clone()).or_default().push((number, Column { name: name.clone(), kind }));
        }
        for (table, mut columns) in schemas {
            columns.sort_by_key(|(number, _)| *number);
            self.schemas.insert(table, columns.into_iter().map(|(_, column)| column).collect());
        }
        Ok(())
    }

    fn column_size(&self, kind: ColumnKind) -> usize {
        match kind {
            ColumnKind::Int16 | ColumnKind::Binary => 2,
            ColumnKind::Int32 => 4,
            ColumnKind::String if self.long_refs => 3,
            ColumnKind::String => 2,
        }
    }

    /// Rows of a table stream: stored column by column.
    fn rows(&self, columns: &[Column], bytes: &[u8]) -> Vec<Vec<Value>> {
        let row_size: usize = columns.iter().map(|c| self.column_size(c.kind)).sum();
        if row_size == 0 {
            return Vec::new();
        }
        let count = bytes.len() / row_size;
        let mut rows = vec![Vec::with_capacity(columns.len()); count];
        let mut offset = 0;
        for column in columns {
            let size = self.column_size(column.kind);
            for (index, row) in rows.iter_mut().enumerate() {
                let at = offset + index * size;
                let raw = &bytes[at..at + size];
                let value = match column.kind {
                    ColumnKind::Int16 => match u16::from_le_bytes([raw[0], raw[1]]) {
                        0 => Value::Null,
                        stored => Value::Int(i32::from((stored ^ 0x8000) as i16)),
                    },
                    ColumnKind::Int32 => match u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]) {
                        0 => Value::Null,
                        stored => Value::Int((stored ^ 0x8000_0000) as i32),
                    },
                    ColumnKind::String => {
                        let id = u32::from(u16::from_le_bytes([raw[0], raw[1]]))
                            | raw.get(2).map_or(0, |&high| u32::from(high) << 16);
                        if id == 0 { Value::Null } else { Value::Str(id) }
                    }
                    ColumnKind::Binary => Value::Null,
                };
                row.push(value);
            }
            offset += size * count;
        }
        rows
    }

    fn table<F: Read + Seek>(&self, file: &mut CompoundFile<F>, name: &str) -> Result<Option<Table<'_>>, MsiError> {
        let Some(columns) = self.schemas.get(name) else { return Ok(None) };
        let Some(bytes) = read_stream(file, self.tables.get(name))? else {
            return Ok(Some(Table { columns: columns.clone(), rows: Vec::new(), strings: &self.strings }));
        };
        let rows = self.rows(columns, &bytes);
        Ok(Some(Table { columns: columns.clone(), rows, strings: &self.strings }))
    }

    fn properties<F: Read + Seek>(&self, file: &mut CompoundFile<F>) -> Result<HashMap<String, String>, MsiError> {
        let mut properties = HashMap::new();
        if let Some(table) = self.table(file, "Property")? {
            for row in &table.rows {
                if let (Some(name), Some(value)) = (table.text(row, "Property"), table.text(row, "Value")) {
                    properties.insert(name, value);
                }
            }
        }
        Ok(properties)
    }

    /// Actions referenced by a sequence table or a `DoAction` control event, with the
    /// condition of their first sequence entry.
    fn scheduled_actions<F: Read + Seek>(
        &self,
        file: &mut CompoundFile<F>,
    ) -> Result<(HashSet<String>, HashMap<String, String>), MsiError> {
        let mut scheduled = HashSet::new();
        let mut conditions = HashMap::new();
        for name in [
            "InstallExecuteSequence",
            "InstallUISequence",
            "AdminExecuteSequence",
            "AdminUISequence",
            "AdvtExecuteSequence",
        ] {
            let Some(table) = self.table(file, name)? else { continue };
            for row in &table.rows {
                let Some(action) = table.text(row, "Action") else { continue };
                if let Some(condition) = table.text(row, "Condition") {
                    conditions.entry(action.clone()).or_insert(condition);
                }
                scheduled.insert(action);
            }
        }
        if let Some(table) = self.table(file, "ControlEvent")? {
            for row in &table.rows {
                if table.text(row, "Event").is_some_and(|event| event.eq_ignore_ascii_case("DoAction"))
                    && let Some(action) = table.text(row, "Argument")
                {
                    scheduled.insert(action);
                }
            }
        }
        Ok((scheduled, conditions))
    }

    fn binary_stream<F: Read + Seek>(
        &self,
        file: &mut CompoundFile<F>,
        key: &str,
    ) -> Result<Option<Vec<u8>>, MsiError> {
        read_stream(file, self.streams.get(&format!("Binary.{key}")))
    }

    fn storage(&self, key: &str) -> Option<PathBuf> {
        self.storages.get(key).cloned()
    }
}

fn read_stream<F: Read + Seek>(
    file: &mut CompoundFile<F>,
    path: Option<&PathBuf>,
) -> Result<Option<Vec<u8>>, MsiError> {
    let Some(path) = path else { return Ok(None) };
    let mut stream = file.open_stream(path).map_err(|e| MsiError(format!("unreadable stream: {e}")))?;
    if stream.len() > MAX_STREAM {
        return Err(MsiError(format!("stream larger than {MAX_STREAM} bytes")));
    }
    let mut bytes = Vec::with_capacity(stream.len() as usize);
    stream.read_to_end(&mut bytes).map_err(|e| MsiError(format!("unreadable stream: {e}")))?;
    Ok(Some(bytes))
}

/// The string pool: a code page (bit 31 = three-byte string references), then (length,
/// reference count) pairs, one per string id; (0, 0) is an unused id. A string of 64 KiB or
/// more takes two pairs: (0, reference count), then its 32-bit length as (low, high) word – the
/// layout of Windows Installer's packages, read and written the same way by Wine
/// (`dlls/msi/string.c`); msitools writes it differently and cannot read it back.
fn string_pool(pool: &[u8], data: &[u8]) -> Result<(Vec<Option<String>>, bool), MsiError> {
    if pool.len() < 4 {
        return Err(MsiError("string pool too short".into()));
    }
    let header = u32::from_le_bytes([pool[0], pool[1], pool[2], pool[3]]);
    let long_refs = header & 0x8000_0000 != 0;
    let codepage = header & 0x7FFF_FFFF;
    let words: Vec<u16> = pool[4..].as_chunks::<2>().0.iter().map(|pair| u16::from_le_bytes(*pair)).collect();
    let mut strings = vec![None]; // string id 0 is NULL
    let mut offset = 0usize;
    let mut i = 0;
    while i + 1 < words.len() {
        let (short, refs) = (words[i], words[i + 1]);
        i += 2;
        let length = if short == 0 && refs > 0 {
            let (Some(&low), Some(&high)) = (words.get(i), words.get(i + 1)) else { break };
            i += 2;
            (u32::from(high) << 16) | u32::from(low)
        } else {
            u32::from(short)
        };
        let length = length as usize;
        let bytes = data.get(offset..offset + length).ok_or_else(|| MsiError("string data too short".into()))?;
        offset += length;
        strings.push((length > 0).then(|| decode_codepage(bytes, codepage)));
    }
    Ok((strings, long_refs))
}

fn decode_codepage(bytes: &[u8], codepage: u32) -> String {
    match codepage {
        65001 => String::from_utf8_lossy(bytes).into_owned(),
        _ if bytes.is_ascii() => String::from_utf8_lossy(bytes).into_owned(),
        // Western code pages and "neutral" (0); other code pages keep ASCII exact.
        _ => text::decode(bytes),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::{Cursor, Write};

    /// Encodes a stream name the way Windows Installer does.
    pub(crate) fn encode_stream_name(name: &str, table: bool) -> String {
        fn utf2mime(c: char) -> Option<u32> {
            Some(match c {
                '0'..='9' => c as u32 - '0' as u32,
                'A'..='Z' => c as u32 - 'A' as u32 + 10,
                'a'..='z' => c as u32 - 'a' as u32 + 36,
                '.' => 62,
                '_' => 63,
                _ => return None,
            })
        }
        let mut out = String::new();
        if table {
            out.push('\u{4840}');
        }
        let chars: Vec<char> = name.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            match (utf2mime(chars[i]), chars.get(i + 1).and_then(|&next| utf2mime(next))) {
                (Some(first), Some(second)) => {
                    out.push(char::from_u32(0x3800 + first + (second << 6)).unwrap());
                    i += 2;
                }
                (Some(first), None) => {
                    out.push(char::from_u32(0x4800 + first).unwrap());
                    i += 1;
                }
                (None, _) => {
                    out.push(chars[i]);
                    i += 1;
                }
            }
        }
        out
    }

    /// Name, (column name, column type bits) and rows of a test table.
    type TestTable = (String, Vec<(String, u16)>, Vec<Vec<Cell>>);

    /// A minimal installer database builder for tests: string pool, column catalog and tables.
    #[derive(Default)]
    pub(crate) struct Builder {
        strings: Vec<String>,
        tables: Vec<TestTable>,
        binaries: Vec<(String, Vec<u8>)>,
    }

    #[derive(Clone)]
    pub(crate) enum Cell {
        S(&'static str),
        I(i16),
        Null,
    }

    impl Builder {
        fn id(&mut self, text: &str) -> u16 {
            if let Some(i) = self.strings.iter().position(|s| s == text) {
                return (i + 1) as u16;
            }
            self.strings.push(text.to_owned());
            self.strings.len() as u16
        }

        /// Column types: 0x1D48 = nullable string(72), 0x0502 = int16 key, 0x1502 = nullable int16.
        pub(crate) fn table(mut self, name: &str, columns: &[(&str, u16)], rows: Vec<Vec<Cell>>) -> Self {
            self.tables.push((name.to_owned(), columns.iter().map(|(n, t)| ((*n).to_owned(), *t)).collect(), rows));
            self
        }

        pub(crate) fn binary(mut self, key: &str, bytes: &[u8]) -> Self {
            self.binaries.push((key.to_owned(), bytes.to_vec()));
            self
        }

        pub(crate) fn custom_actions(self, rows: Vec<Vec<Cell>>) -> Self {
            self.table(
                "CustomAction",
                &[("Action", 0x2D48), ("Type", 0x0502), ("Source", 0x1D48), ("Target", 0x1DFF)],
                rows,
            )
        }

        pub(crate) fn build(mut self) -> Vec<u8> {
            let tables = std::mem::take(&mut self.tables);
            let mut encoded_tables: Vec<(String, Vec<u8>)> = Vec::new();
            let mut column_rows: Vec<(u16, i16, u16, u16)> = Vec::new();
            for (name, columns, rows) in &tables {
                let table_id = self.id(name);
                for (number, (column, bits)) in columns.iter().enumerate() {
                    let column_id = self.id(column);
                    column_rows.push((table_id, number as i16 + 1, column_id, *bits));
                }
                let mut bytes = Vec::new();
                for (index, (_, bits)) in columns.iter().enumerate() {
                    for row in rows {
                        match (&row[index], bits & 0x0800 != 0) {
                            (Cell::S(text), true) => {
                                let id = self.id(text);
                                bytes.extend(id.to_le_bytes());
                            }
                            (Cell::I(value), false) => bytes.extend(((*value as u16) ^ 0x8000).to_le_bytes()),
                            _ => bytes.extend(0u16.to_le_bytes()),
                        }
                    }
                }
                encoded_tables.push((name.clone(), bytes));
            }
            let mut columns = Vec::new();
            for field in 0..4 {
                for (table, number, name, bits) in &column_rows {
                    let value: u16 = match field {
                        0 => *table,
                        1 => (*number as u16) ^ 0x8000,
                        2 => *name,
                        _ => bits ^ 0x8000,
                    };
                    columns.extend(value.to_le_bytes());
                }
            }
            let mut pool = Vec::new();
            pool.extend(1252u32.to_le_bytes());
            let mut data = Vec::new();
            for text in &self.strings {
                pool.extend((text.len() as u16).to_le_bytes());
                pool.extend(1u16.to_le_bytes());
                data.extend(text.as_bytes());
            }
            let mut file = CompoundFile::create(Cursor::new(Vec::new())).unwrap();
            let mut put = |name: String, bytes: &[u8]| {
                let mut stream = file.create_stream(format!("/{name}")).unwrap();
                stream.write_all(bytes).unwrap();
            };
            put(encode_stream_name("_StringPool", true), &pool);
            put(encode_stream_name("_StringData", true), &data);
            put(encode_stream_name("_Columns", true), &columns);
            for (name, bytes) in &encoded_tables {
                put(encode_stream_name(name, true), bytes);
            }
            for (key, bytes) in &self.binaries {
                put(encode_stream_name(&format!("Binary.{key}"), false), bytes);
            }
            file.flush().unwrap();
            file.into_inner().into_inner()
        }
    }

    pub(crate) fn sample() -> Vec<u8> {
        use Cell::*;
        Builder::default()
            .custom_actions(vec![
                vec![S("CheckLicense"), I(6), S("LicenseScript"), S("Main")],
                vec![S("SetShortcut"), I(38), Null, S("Set s = CreateObject(\"WScript.Shell\")")],
                vec![S("Cleanup"), I(6 | 0x40), S("LicenseScript"), S("Cleanup")],
                vec![S("JsHelper"), I(5), S("JsScript"), Null],
                vec![S("RunDll"), I(1), S("Helper"), S("Entry")],
                vec![S("Unused"), I(22), S("vbsfile"), Null],
            ])
            .table(
                "InstallExecuteSequence",
                &[("Action", 0x2D48), ("Condition", 0x1DFF), ("Sequence", 0x1502)],
                vec![
                    vec![S("CheckLicense"), S("NOT Installed"), I(1001)],
                    vec![S("SetShortcut"), Null, I(1002)],
                    vec![S("Cleanup"), S("REMOVE=\"ALL\""), I(1003)],
                ],
            )
            .table(
                "Property",
                &[("Property", 0x2D48), ("Value", 0x0DFF)],
                vec![vec![S("ProductName"), S("Contoso Inventory")], vec![S("ProductVersion"), S("2.1.0")]],
            )
            .binary(
                "LicenseScript",
                b"Function Main()\r\n  strPassword = \"Sommer2024!\"\r\n  Main = 1\r\nEnd Function\r\n",
            )
            .build()
    }

    #[test]
    fn stream_names_round_trip() {
        for (name, table) in
            [("_StringPool", true), ("CustomAction", true), ("Binary.License_Script.1", false), ("a b", false)]
        {
            assert_eq!(decode_stream_name(&encode_stream_name(name, table)), (name.to_owned(), table));
        }
    }

    #[test]
    fn finds_vbscript_custom_actions() {
        let package = analyze(Cursor::new(sample())).unwrap();
        assert_eq!(package.product_name.as_deref(), Some("Contoso Inventory"));
        let names: Vec<_> = package.vbscript_actions.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["CheckLicense", "SetShortcut", "Cleanup", "Unused"]);
        let check = &package.vbscript_actions[0];
        assert_eq!((check.source, check.scheduled, check.continue_on_error), (SourceKind::Binary, true, false));
        assert!(check.script.as_deref().unwrap().contains("strPassword"));
        assert_eq!(check.condition.as_deref(), Some("NOT Installed"));
        let inline = &package.vbscript_actions[1];
        assert_eq!(inline.source, SourceKind::Inline);
        assert!(inline.script.as_deref().unwrap().contains("WScript.Shell"));
        assert!(package.vbscript_actions[2].continue_on_error);
        let unused = &package.vbscript_actions[3];
        assert_eq!((unused.source, unused.scheduled), (SourceKind::InstalledFile, false));
    }

    #[test]
    fn rejects_other_files() {
        assert!(analyze(Cursor::new(b"MZ not a package".to_vec())).is_err());
    }

    #[test]
    fn reads_strings_of_64_kib_and_more() {
        // Pool: code page 1252; "Hi"; an unused id; a 70,000-byte string referenced 3 times
        // (length high word 1 ≠ reference count 3); "End".
        let long = "x".repeat(70_000);
        let mut pool = 1252u32.to_le_bytes().to_vec();
        for word in [2u16, 1, 0, 0, 0, 3, (70_000 & 0xFFFF) as u16, (70_000 >> 16) as u16, 3, 1] {
            pool.extend_from_slice(&word.to_le_bytes());
        }
        let data = format!("Hi{long}End");
        let (strings, long_refs) = string_pool(&pool, data.as_bytes()).unwrap();
        assert!(!long_refs);
        assert_eq!(strings.len(), 5);
        assert_eq!(strings[1].as_deref(), Some("Hi"));
        assert_eq!(strings[2], None);
        assert_eq!(strings[3].as_deref().map(str::len), Some(70_000));
        assert_eq!(strings[4].as_deref(), Some("End"));
        // A pool that promises more data than there is stays an error (not checkable).
        assert!(string_pool(&pool, b"Hi").is_err());
    }
}
