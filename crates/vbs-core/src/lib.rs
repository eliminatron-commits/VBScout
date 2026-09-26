//! Shared core of the collector and the evaluation.
//!
//! * [`model`] – the schema-versioned scan result (`result.json`).
//! * [`read`] / [`write`] – the `.vbscout` container (see `docs/result-format.md`).
//! * [`rules`] – the rule catalog with classifications and sources.
//! * [`module`] / [`views`] – the uniform interface of finding-type modules and
//!   the read-only system views they examine.
//! * [`secrets`] – masking of passwords and credentials in evidence.

mod container;
mod error;
mod migrate;
pub mod model;
pub mod module;
pub mod rules;
pub mod secrets;
pub mod validate;
pub mod views;

pub use container::{Loaded, MIMETYPE_ENTRY, RESULT_ENTRY, count_unknown, read, read_file, to_bytes, write};
pub use error::FormatError;
pub use model::*;

/// Media type written into new result files (from the central product configuration).
pub fn media_type() -> &'static str {
    &vbs_config::product().result_file.mime_type
}

/// File extension of result files, without the dot.
pub fn file_extension() -> &'static str {
    &vbs_config::product().result_file.extension
}
