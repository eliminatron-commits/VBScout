//! The collector: a portable, strictly read-only command-line program that
//! scans one machine for VBScript dependencies and writes exactly one result
//! file (`.vbscout`).
//!
//! * [`cli`] – command-line options.
//! * [`engine`] – runs the modules over the file roots and system sources and
//!   assembles the result with its coverage.
//! * [`walk`] – parallel, read-only directory walk (never follows links, never
//!   touches online-only cloud files).
//! * [`read_only`] – the only way the collector opens files.
//! * [`output`] – the **only** place that writes to disk: the result file.
//! * [`platform`] – machine facts and read-only system views per platform.
//! * [`modules`] – the finding-type modules.

pub mod cli;
pub mod console;
pub mod engine;
pub mod modules;
pub mod output;
pub mod platform;
pub mod read_only;
pub mod walk;
