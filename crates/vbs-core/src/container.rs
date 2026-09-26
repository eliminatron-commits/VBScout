//! The `.vbscout` result file: a ZIP archive with
//!
//! ```text
//! mimetype      first entry, stored – the media type (file-type magic)
//! result.json   deflated – the ScanResult (schema-versioned JSON)
//! ```
//!
//! This crate only serialises into memory or into a caller-provided writer. It
//! never creates files: in the collector, the single place that writes to disk
//! is `crates/vbs-collector/src/output.rs` (enforced by
//! `scripts/check-readonly.mjs`).

use std::fs::File;
use std::io::{BufReader, Cursor, Read, Seek, Write};
use std::path::Path;

use zip::result::ZipError;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::error::FormatError;
use crate::migrate;
use crate::model::ScanResult;
use crate::validate::{self, limits};

pub const MIMETYPE_ENTRY: &str = "mimetype";
pub const RESULT_ENTRY: &str = "result.json";

/// A result read from disk.
#[derive(Debug)]
pub struct Loaded {
    pub result: ScanResult,
    /// Values from a newer format revision that this version shows generically
    /// (unknown finding kinds, reasons, …). Non-zero means: update the app.
    pub unknown_values: usize,
}

fn accepted_format(media_type: &str) -> bool {
    let config = &vbs_config::product().result_file;
    media_type == config.mime_type || config.legacy_mime_types.iter().any(|legacy| legacy == media_type)
}

/// Reads a result file from any seekable source.
pub fn read<R: Read + Seek>(reader: R) -> Result<Loaded, FormatError> {
    let mut archive = ZipArchive::new(reader).map_err(|_| FormatError::NotAResultFile)?;
    if archive.len() > limits::MAX_ENTRIES {
        return Err(FormatError::LimitExceeded(format!("more than {} archive entries", limits::MAX_ENTRIES)));
    }

    let mimetype =
        read_entry(&mut archive, MIMETYPE_ENTRY, limits::MAX_MIMETYPE_BYTES)?.ok_or(FormatError::NotAResultFile)?;
    let mimetype = String::from_utf8(mimetype).map_err(|_| FormatError::NotAResultFile)?;
    if !accepted_format(mimetype.trim()) {
        return Err(FormatError::WrongFormat(mimetype.trim().to_owned()));
    }

    let json = read_entry(&mut archive, RESULT_ENTRY, limits::MAX_RESULT_BYTES)?
        .ok_or_else(|| FormatError::Invalid(format!("{RESULT_ENTRY} is missing")))?;
    let value = migrate::upgrade(serde_json::from_slice(&json)?)?;
    let mut result: ScanResult = serde_json::from_value(value)?;
    if !accepted_format(&result.format) {
        return Err(FormatError::WrongFormat(result.format));
    }
    // Files from earlier product names are presented in the current format.
    result.format = crate::media_type().to_owned();
    validate::check_structure(&result)?;
    let unknown_values = count_unknown(&result);
    Ok(Loaded { result, unknown_values })
}

/// Reads a result file (opened read-only).
pub fn read_file(path: impl AsRef<Path>) -> Result<Loaded, FormatError> {
    read(BufReader::new(File::open(path)?))
}

/// Reads one entry, enforcing `limit` on the actual (not the declared) size.
fn read_entry<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    name: &str,
    limit: u64,
) -> Result<Option<Vec<u8>>, FormatError> {
    let entry = match archive.by_name(name) {
        Ok(entry) => entry,
        Err(ZipError::FileNotFound) => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if entry.size() > limit {
        return Err(FormatError::LimitExceeded(format!("{name} is too large")));
    }
    let mut bytes = Vec::with_capacity(usize::try_from(entry.size()).unwrap_or(0));
    entry.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(FormatError::LimitExceeded(format!("{name} is too large")));
    }
    Ok(Some(bytes))
}

/// Serialises a result into `writer`. The writer invariants (short, masked
/// evidence; see [`validate::sanitize`]) are enforced on the written copy.
pub fn write<W: Write + Seek>(result: &ScanResult, writer: W) -> Result<W, FormatError> {
    let mut result = result.clone();
    result.format = crate::media_type().to_owned();
    validate::sanitize(&mut result);
    validate::check_structure(&result)?;

    let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    let deflated = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    let mut zip = ZipWriter::new(writer);
    zip.start_file(MIMETYPE_ENTRY, stored)?;
    zip.write_all(result.format.as_bytes())?;
    zip.start_file(RESULT_ENTRY, deflated)?;
    zip.write_all(&serde_json::to_vec_pretty(&result)?)?;
    Ok(zip.finish()?)
}

/// Serialises a result into a new in-memory container.
pub fn to_bytes(result: &ScanResult) -> Result<Vec<u8>, FormatError> {
    Ok(write(result, Cursor::new(Vec::new()))?.into_inner())
}

/// Number of enumeration values in `result` that this version does not know.
pub fn count_unknown(result: &ScanResult) -> usize {
    let coverage = &result.coverage;
    let mut unknown = usize::from(!coverage.mode.is_known())
        + coverage.limitations.iter().filter(|l| !l.code.is_known()).count()
        + coverage.sources.iter().filter(|s| !s.status.is_known()).count()
        + usize::from(result.machine.os.product_type.as_ref().is_some_and(|t| !t.is_known()));
    for finding in &result.findings {
        unknown += usize::from(!finding.kind.is_known())
            + usize::from(finding.classification.effective().as_str() != finding.classification.as_str())
            + usize::from(!finding.status.is_known())
            + usize::from(finding.reason.as_ref().is_some_and(|r| !r.is_known()))
            + usize::from(!finding.activation.is_known())
            + usize::from(!finding.location.kind.is_known());
    }
    unknown
}
