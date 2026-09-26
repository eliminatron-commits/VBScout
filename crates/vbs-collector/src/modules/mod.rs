//! The finding-type modules (see `vbs_core::module` for the interface).
//!
//! File-driven modules inspect what the walk finds: script files, scripts and shortcuts that
//! start VBScript, installer packages and Group Policy script lists. System modules examine
//! scheduled tasks, autostart entries, services, WMI subscriptions, logon scripts, installed
//! packages and event logs through the read-only [`SystemView`]. Office macros follow in
//! phase 3. Every module registered here is covered by the positive and negative collections
//! in `tests/corpus/`.

mod autostart;
mod event_logs;
mod installer;
mod invocations;
mod logon_scripts;
mod script_files;
mod services;
mod shortcuts;
mod tasks;
mod wmi;

use std::path::{Path, PathBuf};

use vbs_core::model::{Activation, Detail, FindingKind, Location, NotCheckableReason};
use vbs_core::module::{Module, Report};
use vbs_core::views::{SystemView, ViewError};

use crate::analysis::command::{self, Reference, Usage};
use crate::analysis::script::{self, Language};
use crate::analysis::text;

/// All modules, in the order their system parts run.
pub fn all() -> Vec<Box<dyn Module>> {
    vec![
        Box::new(script_files::ScriptFiles),
        Box::new(invocations::Invocations),
        Box::new(shortcuts::Shortcuts),
        Box::new(tasks::ScheduledTasks),
        Box::new(autostart::Autostart),
        Box::new(services::Services),
        Box::new(wmi::WmiSubscriptions),
        Box::new(logon_scripts::LogonScripts),
        Box::new(installer::InstallerPackages::default()),
        Box::new(event_logs::DeprecationAlerts),
        Box::new(event_logs::Sysmon),
    ]
}

/// Largest script file read completely.
pub(crate) const MAX_SCRIPT_BYTES: u64 = 16 * 1024 * 1024;

/// The rules of one kind of entry.
#[derive(Debug, Clone)]
pub(crate) struct Rules {
    /// Runs VBScript.
    pub breaks: &'static str,
    /// Starts a script whose language only its content shows.
    pub review: &'static str,
    /// Could not be checked (the `VBS-x00` of the range).
    pub not_checkable: &'static str,
    pub kind: FindingKind,
}

/// Where a file lies decides how it runs: startup folders and Group Policy script folders
/// run without user action; recently used items were opened by a user.
pub(crate) fn file_activation(path: &Path) -> Activation {
    let lower = path.to_string_lossy().replace('/', "\\").to_lowercase();
    let automatic = lower.contains("\\start menu\\programs\\startup\\")
        || lower.contains("\\grouppolicy\\machine\\scripts\\")
        || lower.contains("\\grouppolicy\\user\\scripts\\")
        || (lower.contains("\\sysvol\\") && lower.contains("\\scripts\\"))
        || lower.contains("\\netlogon\\");
    if automatic { Activation::Automatic } else { Activation::Dormant }
}

/// What a command runs, looking one level into batch, PowerShell and KiXtart files it starts.
#[derive(Debug, Default)]
pub(crate) struct CommandCheck {
    pub references: Vec<Reference>,
    /// A called script that starts VBScript: (script path, line number, line, reference).
    pub via_script: Option<(String, u32, String, Reference)>,
    /// A called script that could not be read (network path, access denied, …).
    pub unreadable: Option<(String, NotCheckableReason)>,
}

impl CommandCheck {
    pub fn usage(&self) -> Option<Usage> {
        let own = command::strongest(&self.references);
        let called = self.via_script.as_ref().map(|(_, _, _, reference)| reference.usage);
        [own, called].into_iter().flatten().min()
    }
}

/// Analyses a command; batch, PowerShell and KiXtart files it names are read through the
/// file view (local files only) and checked for calls of VBScript.
pub(crate) fn check_command(system: &SystemView<'_>, command_line: &str, working_dir: Option<&str>) -> CommandCheck {
    let mut check = CommandCheck { references: command::analyze(command_line), ..CommandCheck::default() };
    if check.usage() == Some(Usage::VbScript) {
        return check;
    }
    for called in command::called_scripts(command_line).into_iter().take(3) {
        let Some(path) = resolve(system, &called, working_dir) else { continue };
        check_script_file(system, &path, &mut check);
        if check.usage() == Some(Usage::VbScript) {
            break;
        }
    }
    check
}

/// Reads a batch, PowerShell or KiXtart file (local only) and records the strongest call of
/// VBScript in it – or why it could not be read.
pub(crate) fn check_script_file(system: &SystemView<'_>, path: &Path, check: &mut CommandCheck) {
    if crate::platform::is_network_location(path) {
        check.unreadable.get_or_insert((path.display().to_string(), NotCheckableReason::NetworkLocation));
        return;
    }
    let bytes = match system.files.read(path, MAX_SCRIPT_BYTES) {
        Ok(bytes) => bytes,
        Err(ViewError::NotFound) => return,
        Err(error) => {
            check.unreadable.get_or_insert((path.display().to_string(), view_reason(&error)));
            return;
        }
    };
    let name = path.to_string_lossy();
    let language = match text::extension(&name).as_deref() {
        Some("ps1") => Language::PowerShell,
        Some("kix") => Language::KiXtart,
        _ => Language::Batch,
    };
    let content = text::decode(&bytes);
    let mut best: Option<(u32, &str, Reference)> = None;
    for (number, line) in script::code_lines(&content, language) {
        for reference in command::analyze(line) {
            if best.as_ref().is_none_or(|(_, _, current)| reference.usage < current.usage) {
                best = Some((number, line, reference));
            }
        }
    }
    if let Some((number, line, reference)) = best {
        let better = check.via_script.as_ref().is_none_or(|(_, _, _, current)| reference.usage < current.usage);
        if better {
            check.via_script = Some((path.display().to_string(), number, line.trim().to_owned(), reference));
        }
    }
}

/// The not-checkable reason for a file view error.
pub(crate) fn view_reason(error: &ViewError) -> NotCheckableReason {
    match error {
        ViewError::AccessDenied => NotCheckableReason::AccessDenied,
        ViewError::NetworkPath => NotCheckableReason::NetworkLocation,
        ViewError::CloudPlaceholder => NotCheckableReason::CloudPlaceholder,
        ViewError::TooLarge => NotCheckableReason::TooLarge,
        ViewError::Locked => NotCheckableReason::Locked,
        _ => NotCheckableReason::Corrupt,
    }
}

/// Longest `readError` detail: the reader's message, short and without file contents.
const MAX_READ_ERROR: usize = 120;

/// The `readError` detail of a not-checkable finding: why the reader gave up (e.g. `link info`,
/// `no string pool`, `not a regular file`) – for support, without file contents or secrets.
pub(crate) fn read_error(message: &str) -> String {
    message.chars().take(MAX_READ_ERROR).collect()
}

/// A path from a command as a local absolute path: `%variables%` expanded with the machine's
/// values, relative paths joined to the working folder; `None` if it cannot be resolved.
pub(crate) fn resolve(system: &SystemView<'_>, written: &str, working_dir: Option<&str>) -> Option<PathBuf> {
    let expanded = system.env.expand(written.trim().trim_matches('"'));
    if expanded.contains('%') {
        return None;
    }
    let path = PathBuf::from(&expanded);
    // Network paths are kept (and reported as not read); anything else must be absolute here.
    if crate::platform::is_network_path(&path) || path.is_absolute() {
        return Some(path);
    }
    let base = system.env.expand(working_dir?.trim().trim_matches('"'));
    let base = PathBuf::from(base);
    (base.is_absolute() && !base.to_string_lossy().contains('%')).then(|| base.join(path))
}

/// `base` joined with a Windows-style relative path (`Microsoft\Windows\…`), one component at a time.
pub(crate) fn join_windows(base: &Path, relative: &str) -> PathBuf {
    relative.split(['\\', '/']).filter(|part| !part.is_empty()).fold(base.to_path_buf(), |path, part| path.join(part))
}

/// Reports a command of a system entry (task action, registry value, service, …): one finding
/// for VBScript or an unknown script language, or "not checkable" when a script it starts
/// could not be read; plus the credential finding if the command contains secrets.
pub(crate) fn report_command(
    report: &mut Report,
    rules: Rules,
    location: Location,
    activation: Activation,
    command_line: &str,
    check: &CommandCheck,
    details: &[(&str, Detail)],
) -> bool {
    let Some(usage) = check.usage() else {
        if let Some((path, reason)) = &check.unreadable {
            let mut builder = report
                .not_checkable(rules.not_checkable, location, reason.clone())
                .kind(rules.kind.clone())
                .activation(activation.clone())
                .target(path.clone())
                .evidence(None, command_line);
            for (key, value) in details {
                builder = builder.detail(key, value.clone());
            }
            builder.emit();
        }
        return false;
    };
    let rule = if usage == Usage::VbScript { rules.breaks } else { rules.review };
    let own = check.references.iter().filter(|r| r.usage == usage).min_by_key(|r| r.script.is_none());
    let mut builder =
        report.finding(rule, location.clone()).activation(activation.clone()).evidence(None, command_line);
    match (own, &check.via_script) {
        (Some(reference), _) => {
            builder = builder.detail("via", reference.via);
            if let Some(script) = &reference.script {
                builder = builder.target(script.clone());
            }
        }
        (None, Some((path, number, line, reference))) => {
            let name = text::file_name(path).to_owned();
            builder = builder
                .target(path.clone())
                .detail("via", "script")
                .detail("calledScriptStarts", reference.script.clone().unwrap_or_else(|| reference.via.to_owned()))
                .evidence(None, &format!("{name}:{number}: {line}"));
        }
        (None, None) => {}
    }
    for (key, value) in details {
        builder = builder.detail(key, value.clone());
    }
    builder.emit();
    report.credentials(location, None, activation, [(None, command_line)]);
    true
}

/// Loaded user hives under `HKEY_USERS` (`S-1-5-…`; the `_Classes` companions left out).
/// Hives of users who are not logged on are never loaded (that would change the system).
pub(crate) fn loaded_user_hives(system: &SystemView<'_>) -> Result<Vec<String>, ViewError> {
    let mut sids = system.registry.subkeys(vbs_core::views::Hive::Users, "", vbs_core::views::Bitness::Native)?;
    sids.retain(|sid| {
        let upper = sid.to_ascii_uppercase();
        upper.starts_with("S-1-5-") && !upper.ends_with("_CLASSES")
    });
    Ok(sids)
}

/// User profiles from `ProfileList`: (SID, profile folder); only real user accounts (`S-1-5-21-…`).
pub(crate) fn user_profiles(system: &SystemView<'_>) -> Vec<(String, PathBuf)> {
    use vbs_core::views::{Bitness, Hive};
    const PROFILE_LIST: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\ProfileList";
    let Ok(sids) = system.registry.subkeys(Hive::LocalMachine, PROFILE_LIST, Bitness::Native) else {
        return Vec::new();
    };
    sids.into_iter()
        .filter(|sid| sid.to_ascii_uppercase().starts_with("S-1-5-21-"))
        .filter_map(|sid| {
            let key = format!(r"{PROFILE_LIST}\{sid}");
            let value = system.registry.value(Hive::LocalMachine, &key, "ProfileImagePath", Bitness::Native).ok()??;
            let path = system.env.expand(value.as_text()?);
            (!path.contains('%')).then(|| (sid, PathBuf::from(path)))
        })
        .collect()
}

/// Largest user hive file read (`NTUSER.DAT` of a profile that is not logged on).
const MAX_HIVE_BYTES: u64 = 512 * 1024 * 1024;

/// The registry hive of a profile whose user is not logged on, read from `NTUSER.DAT` – the
/// file is read like any other; the hive is never loaded.
pub(crate) fn offline_hive(system: &SystemView<'_>, profile: &Path) -> Result<crate::analysis::regf::HiveFile, String> {
    let bytes = system.files.read(&profile.join("NTUSER.DAT"), MAX_HIVE_BYTES).map_err(|error| error.to_string())?;
    crate::analysis::regf::HiveFile::parse(bytes)
}

/// Reports the VBScript lines of a script file's text as credentials if they contain secrets.
pub(crate) fn file_credentials(
    report: &mut Report,
    file: &dyn vbs_core::module::CandidateFile,
    activation: Activation,
    lines: &[(u32, &str)],
) {
    report.credentials(file.location(), Some(file.facts()), activation, lines.iter().map(|(n, l)| (Some(*n), *l)));
}

#[cfg(test)]
pub(crate) mod testing {
    //! Helpers for module tests: in-memory candidate files and system views.

    use std::io::Cursor;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use time::OffsetDateTime;
    use vbs_core::model::Finding;
    use vbs_core::module::{CandidateFile, Module, ReadError, ReadSeek, Report};
    use vbs_core::views::{MemoryEventLogs, MemoryRegistry, MemoryWmi, SystemEnvironment, SystemView};

    use crate::read_only::ReadOnlyFiles;

    pub struct MemoryFile {
        pub path: PathBuf,
        pub bytes: Vec<u8>,
    }

    impl MemoryFile {
        pub fn new(path: &str, bytes: impl Into<Vec<u8>>) -> Self {
            Self { path: PathBuf::from(path), bytes: bytes.into() }
        }
    }

    impl CandidateFile for MemoryFile {
        fn path(&self) -> &Path {
            &self.path
        }
        fn extension(&self) -> &str {
            self.path.extension().and_then(|e| e.to_str()).unwrap_or_default()
        }
        fn size(&self) -> u64 {
            self.bytes.len() as u64
        }
        fn modified(&self) -> Option<OffsetDateTime> {
            None
        }
        fn network(&self) -> bool {
            false
        }
        fn contents(&self, limit: u64) -> Result<Arc<[u8]>, ReadError> {
            if self.bytes.len() as u64 > limit {
                return Err(ReadError::TooLarge { size: self.bytes.len() as u64, limit });
            }
            Ok(self.bytes.clone().into())
        }
        fn open(&self) -> Result<Box<dyn ReadSeek + '_>, ReadError> {
            Ok(Box::new(Cursor::new(&self.bytes)))
        }
        fn sha256(&self) -> Option<String> {
            None
        }
    }

    /// Runs a module's file part on one file.
    pub fn inspect(module: &dyn Module, file: &MemoryFile) -> Vec<Finding> {
        let mut report = Report::new();
        module.inspect_file(file, &mut report);
        report.into_parts().0
    }

    /// Runs a module's system part on in-memory views (files come from the real disk).
    pub fn scan_system(
        module: &dyn Module,
        env: &SystemEnvironment,
        registry: &MemoryRegistry,
        logs: &MemoryEventLogs,
        wmi: &MemoryWmi,
    ) -> Report {
        let system = SystemView { env, registry, files: &ReadOnlyFiles, event_logs: logs, wmi };
        let mut report = Report::new();
        module.scan_system(&system, &mut report);
        report
    }
}
