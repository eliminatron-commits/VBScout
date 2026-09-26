//! Scheduled tasks that run VBScript: the task definitions in `%SystemRoot%\System32\Tasks`
//! (XML, one file per task), read as files. Batch and PowerShell files a task starts are
//! checked one level deep.

use std::path::Path;

use vbs_core::model::{Activation, Detail, FindingKind, Location, LocationKind, NotCheckableReason, SourceStatus};
use vbs_core::module::{CREDENTIAL_RULE, Module, ModuleInfo, Report};
use vbs_core::views::{SystemView, ViewError};

use super::{Rules, check_command, report_command};
use crate::analysis::{markup, text};

pub struct ScheduledTasks;

static INFO: ModuleInfo = ModuleInfo {
    id: "scheduled-task",
    system_source: Some("tasks.scheduled"),
    rules: &["VBS-301", "VBS-302", CREDENTIAL_RULE],
    fallback_rule: "VBS-300",
    needs_admin: true,
};

const RULES: Rules =
    Rules { breaks: "VBS-301", review: "VBS-302", not_checkable: "VBS-300", kind: FindingKind::ScheduledTask };

/// Task definitions are small; a bigger file is not one.
const MAX_TASK_BYTES: u64 = 4 * 1024 * 1024;
/// Folder depth limit (the task tree is shallow; this guards against odd loops).
const MAX_DEPTH: usize = 16;

impl Module for ScheduledTasks {
    fn info(&self) -> &'static ModuleInfo {
        &INFO
    }

    fn scan_system(&self, system: &SystemView<'_>, report: &mut Report) {
        let Some(root) = system.env.system_root.as_ref().map(|root| root.join("System32").join("Tasks")) else {
            report.set_source_status(SourceStatus::Unavailable, "noSystemRoot");
            return;
        };
        report.add_root(root.display().to_string());
        match system.files.list_dir(&root) {
            Ok(_) => walk(system, report, &root, &root, 0),
            Err(ViewError::NotFound) => report.set_source_status(SourceStatus::Unavailable, "notFound"),
            Err(error) => report.set_source_status(SourceStatus::Failed, error.reason_code()),
        }
    }
}

fn walk(system: &SystemView<'_>, report: &mut Report, root: &Path, dir: &Path, depth: usize) {
    let entries = match system.files.list_dir(dir) {
        Ok(entries) => entries,
        Err(error) => {
            report.record_error(format!("{} ({error})", dir.display()));
            return;
        }
    };
    for entry in entries {
        let path = dir.join(&entry.name);
        if entry.is_dir {
            if depth < MAX_DEPTH {
                walk(system, report, root, &path, depth + 1);
            }
            continue;
        }
        report.count_entries(1);
        let task_path = format!("\\{}", path.strip_prefix(root).unwrap_or(&path).to_string_lossy().replace('/', "\\"));
        let location = Location { kind: LocationKind::ScheduledTask, path: task_path, item: None };
        match system.files.read(&path, MAX_TASK_BYTES) {
            Ok(bytes) => inspect_task(system, report, location, &bytes),
            Err(ViewError::AccessDenied) => report.record_error(format!("{} (access denied)", path.display())),
            Err(ViewError::NotFound) => {}
            Err(error) => {
                report
                    .not_checkable("VBS-300", location, super::view_reason(&error))
                    .kind(FindingKind::ScheduledTask)
                    .emit();
            }
        }
    }
}

/// One task definition.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct TaskDefinition {
    pub enabled: bool,
    pub triggers: Vec<String>,
    /// (command, arguments, working directory) of every `Exec` action.
    pub actions: Vec<(String, String, Option<String>)>,
}

/// Parses a task definition (Task Scheduler schema); `None` if it is not one.
pub(crate) fn parse(content: &str) -> Option<TaskDefinition> {
    let tags = markup::tags(content);
    if !tags.iter().any(|tag| tag.is("Task") && !tag.closing) {
        return None;
    }
    let mut task = TaskDefinition { enabled: true, ..TaskDefinition::default() };
    let mut in_triggers = false;
    let mut in_settings = false;
    let mut current: Option<(String, String, Option<String>)> = None;
    for (index, tag) in tags.iter().enumerate() {
        let next = tags.get(index + 1);
        let text = || markup::text_after(content, tag, next).into_owned();
        match (tag.closing, tag.name.rsplit(':').next().unwrap_or(tag.name)) {
            (false, "Triggers") => in_triggers = true,
            (true, "Triggers") => in_triggers = false,
            (false, "Settings") => in_settings = true,
            (true, "Settings") => in_settings = false,
            (false, "Exec") => current = Some(Default::default()),
            (true, "Exec") => task.actions.extend(current.take()),
            (false, "Command") => {
                if let Some(action) = current.as_mut() {
                    action.0 = text();
                }
            }
            (false, "Arguments") => {
                if let Some(action) = current.as_mut() {
                    action.1 = text();
                }
            }
            (false, "WorkingDirectory") => {
                if let Some(action) = current.as_mut() {
                    action.2 = Some(text()).filter(|t| !t.is_empty());
                }
            }
            (false, "Enabled") if in_settings && !in_triggers => task.enabled = !text().eq_ignore_ascii_case("false"),
            (false, name) if in_triggers && name.ends_with("Trigger") => {
                let kind = name.trim_end_matches("Trigger").to_ascii_lowercase();
                if !task.triggers.contains(&kind) {
                    task.triggers.push(kind);
                }
            }
            _ => {}
        }
    }
    Some(task)
}

fn inspect_task(system: &SystemView<'_>, report: &mut Report, location: Location, bytes: &[u8]) {
    let content = text::decode(bytes);
    let Some(task) = parse(&content) else {
        report.not_checkable("VBS-300", location, NotCheckableReason::Corrupt).kind(FindingKind::ScheduledTask).emit();
        return;
    };
    report.count_inspected(1);
    let activation = if task.enabled { Activation::Automatic } else { Activation::Dormant };
    let several = task.actions.len() > 1;
    for (index, (command, arguments, working_dir)) in task.actions.iter().enumerate() {
        let command_line = if arguments.is_empty() { command.clone() } else { format!("{command} {arguments}") };
        let program = command.trim().trim_matches('"');
        let quoted = if program.contains(' ') { format!("\"{program}\" {arguments}") } else { command_line.clone() };
        let check = check_command(system, &quoted, working_dir.as_deref());
        let item_location = Location { item: several.then(|| format!("action {}", index + 1)), ..location.clone() };
        let mut details = vec![("enabled", Detail::Flag(task.enabled))];
        if !task.triggers.is_empty() {
            details.push(("triggers", Detail::Text(task.triggers.join(","))));
        }
        report_command(report, RULES, item_location, activation.clone(), &command_line, &check, &details);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::testing::scan_system;
    use vbs_core::model::FindingStatus;
    use vbs_core::views::{MemoryEventLogs, MemoryRegistry, MemoryWmi, SystemEnvironment};

    const TASK: &str = r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.4" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <Triggers><CalendarTrigger><StartBoundary>2024-01-01T02:00:00</StartBoundary><Enabled>true</Enabled></CalendarTrigger><LogonTrigger/></Triggers>
  <Settings><Enabled>true</Enabled></Settings>
  <Actions Context="Author">
    <Exec><Command>C:\Windows\System32\wscript.exe</Command><Arguments>//B "C:\Scripts\nightly backup.vbs" /pw:Sommer2024!</Arguments></Exec>
  </Actions>
</Task>"#;

    #[test]
    fn parses_task_definitions() {
        let task = parse(TASK).unwrap();
        assert!(task.enabled);
        assert_eq!(task.triggers, ["calendar", "logon"]);
        assert_eq!(task.actions[0].0, r"C:\Windows\System32\wscript.exe");
        assert_eq!(task.actions[0].1, r#"//B "C:\Scripts\nightly backup.vbs" /pw:Sommer2024!"#);
        assert_eq!(parse("<html/>"), None);
    }

    fn scan(files: &[(&str, &[u8])]) -> Report {
        let dir = tempfile::tempdir().unwrap();
        for (name, bytes) in files {
            let path = dir.path().join("System32").join("Tasks").join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
        }
        let env = SystemEnvironment { system_root: Some(dir.path().to_path_buf()), ..SystemEnvironment::default() };
        let report =
            scan_system(&ScheduledTasks, &env, &MemoryRegistry::new(), &MemoryEventLogs::new(), &MemoryWmi::new());
        drop(dir);
        report
    }

    #[test]
    fn reports_tasks_running_vbscript() {
        let utf16: Vec<u8> = [0xFF, 0xFE].into_iter().chain(TASK.encode_utf16().flat_map(u16::to_le_bytes)).collect();
        let disabled = TASK.replace("<Settings><Enabled>true", "<Settings><Enabled>false");
        let report = scan(&[
            ("Contoso/Nightly Backup", &utf16),
            ("Old Job", disabled.as_bytes()),
            ("Other", b"<Task><Actions><Exec><Command>C:\\Tools\\app.exe</Command></Exec></Actions></Task>"),
            ("Broken", b"not a task"),
        ]);
        let findings = report.findings();
        let summary: Vec<(&str, &str, &Activation)> =
            findings.iter().map(|f| (f.location.path.as_str(), f.rule.as_str(), &f.activation)).collect();
        assert!(summary.contains(&(r"\Contoso\Nightly Backup", "VBS-301", &Activation::Automatic)), "{summary:?}");
        assert!(summary.contains(&(r"\Contoso\Nightly Backup", CREDENTIAL_RULE, &Activation::Automatic)));
        assert!(summary.contains(&(r"\Old Job", "VBS-301", &Activation::Dormant)));
        assert!(findings.iter().any(|f| f.location.path == r"\Broken" && f.status == FindingStatus::NotCheckable));
        assert!(!findings.iter().any(|f| f.location.path == r"\Other"));
        let main = findings.iter().find(|f| f.rule == "VBS-301").unwrap();
        assert_eq!(main.target.as_deref(), Some(r"C:\Scripts\nightly backup.vbs"));
        assert!(main.evidence[0].masked && !main.evidence[0].text.contains("Sommer2024!"));
    }

    #[test]
    fn follows_batch_files_one_level() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("nightly.cmd");
        std::fs::write(&script, "@echo off\r\ncscript //nologo C:\\Scripts\\cleanup.vbs\r\n").unwrap();
        let task = format!(
            "<Task><Actions><Exec><Command>cmd.exe</Command><Arguments>/c \"{}\"</Arguments></Exec></Actions></Task>",
            script.display()
        );
        let report = scan(&[("Nightly", task.as_bytes())]);
        let finding = &report.findings()[0];
        assert_eq!(finding.rule, "VBS-301");
        assert_eq!(finding.target.as_deref(), Some(script.display().to_string().as_str()));
        assert_eq!(finding.details["via"], Detail::Text("script".into()));
        assert!(finding.evidence[1].text.contains("nightly.cmd:2: cscript"));
    }
}
