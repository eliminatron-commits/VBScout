//! The evaluation's logic, independent of the desktop shell so it can be
//! tested on any platform.
//!
//! * [`import`] – reads result files and folders in parallel.
//! * [`assessment`] – merges the results: newest scan per machine, de-duplication, Windows
//!   components, links between entries and scripts, risk and priority.
//! * [`effort`] – rule-of-thumb effort per item and in total.
//! * [`coverage`] – what the scans could see, including the time span of the event logs.
//! * [`edition`] – what the free edition and the licenses allow (enforced here, not in the UI).
//! * [`report`] – the management PDF and the technical Excel list in eight languages.
//! * [`sample`] – synthetic result files for tests, the performance test and self-checks.

pub mod assessment;
pub mod coverage;
pub mod edition;
pub mod effort;
pub mod import;
pub mod paths;
pub mod report;
pub mod sample;
