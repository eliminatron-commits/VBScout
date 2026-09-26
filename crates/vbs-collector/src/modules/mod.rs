//! The finding-type modules (see `vbs_core::module` for the interface).
//!
//! Phase 2 adds the system-level types (script files, invocations, shortcuts,
//! scheduled tasks, autostart, services, WMI, logon scripts, MSI, event logs),
//! phase 3 the Office macros. Every module registered here is covered by the
//! positive and negative collections in `tests/corpus/`.

use vbs_core::module::Module;

/// All modules, in the order their system parts run.
pub fn all() -> Vec<Box<dyn Module>> {
    Vec::new()
}
