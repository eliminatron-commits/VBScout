//! The evaluation's logic, independent of the desktop shell so it can be
//! tested on any platform.
//!
//! * [`import`] – reads result files and folders in parallel.
//!
//! Phase 4 adds merging, de-duplication, risk assessment, migration hints,
//! effort estimates and the reports.

pub mod import;
