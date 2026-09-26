//! Office macros (VBA) that use VBScript: Excel, Word and PowerPoint documents, templates and
//! add-ins, and Access databases. Documents are recognised by their content and read as files
//! (`analysis::office`, `analysis::jet`), the projects' source code is decompressed
//! (`analysis::ovba`) and examined (`analysis::vba`). Office is never started.
//!
//! A document that needs a password to open, is protected with rights management, is damaged
//! or holds macros in a format the collector does not read is reported as "not checkable"
//! (`VBS-600`). A project that is only "locked for viewing" is read like any other – the lock
//! hides the code in the VBA editor, it does not encrypt it; findings mention the lock.

use std::io::BufReader;
use std::path::Path;

use vbs_core::model::{Activation, Detail, FileFacts, Location, NotCheckableReason};
use vbs_core::module::{CREDENTIAL_RULE, CandidateFile, Module, ModuleInfo, Report};

use super::read_error;
use crate::analysis::office::{self, Document, FoundProject, Obstacle};
use crate::analysis::ovba::Project;
use crate::analysis::vba::{self, Kind, Use};

pub struct OfficeMacros;

static INFO: ModuleInfo = ModuleInfo {
    id: "office-macro",
    system_source: None,
    rules: &["VBS-601", "VBS-611", "VBS-612", "VBS-621", "VBS-622", "VBS-632", CREDENTIAL_RULE],
    fallback_rule: "VBS-600",
    needs_admin: false,
};

/// Formats that can carry VBA: Excel (workbooks, templates, add-ins), Word (documents,
/// templates), PowerPoint (Open XML presentations, templates, shows, add-ins), Access.
const EXTENSIONS: &[&str] = &[
    "xls", "xla", "xlt", "xlsm", "xlsb", "xlam", "xltm", "doc", "dot", "docm", "dotm", "pptm", "potm", "ppsm", "ppam",
    "mdb", "accdb",
];

/// Files up to this size get a content hash when they have a finding (de-duplication across
/// machines); larger ones are only read in the parts the analysis needs.
const MAX_HASHED_BYTES: u64 = 64 * 1024 * 1024;

/// Procedures that Office runs by itself when a document is opened or created.
const AUTO_MACROS: &[&str] = &[
    "autoopen",
    "auto_open",
    "autoexec",
    "autonew",
    "document_open",
    "document_new",
    "workbook_open",
    "workbook_activate",
    "workbook_addininstall",
];

impl Module for OfficeMacros {
    fn info(&self) -> &'static ModuleInfo {
        &INFO
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn inspect_file(&self, file: &dyn CandidateFile, report: &mut Report) {
        let reader = match file.open() {
            Ok(reader) => reader,
            Err(error) => {
                if let Some(reason) = error.reason() {
                    report.not_checkable("VBS-600", file.location(), reason).file(file.facts()).emit();
                }
                return;
            }
        };
        let name = file.path().file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
        let document = office::analyze(BufReader::with_capacity(64 * 1024, reader), &name);
        report_document(report, file, &document);
    }
}

fn report_document(report: &mut Report, file: &dyn CandidateFile, document: &Document) {
    let analyses: Vec<(&FoundProject, Vec<Use>)> = document
        .projects
        .iter()
        .map(|found| (found, found.project.as_ref().map(vba::analyze).unwrap_or_default()))
        .collect();
    let has_findings = !document.unreadable.is_empty()
        || analyses.iter().any(|(found, uses)| match &found.project {
            Ok(project) => !uses.is_empty() || project.modules.iter().any(|module| module.source.is_err()),
            Err(_) => true,
        });
    if !has_findings {
        return;
    }
    // A content hash for files with findings (read once more, completely, if not too large).
    if file.size() <= MAX_HASHED_BYTES {
        let _ = file.contents(MAX_HASHED_BYTES);
    }
    let facts = file.facts();
    for unreadable in &document.unreadable {
        let mut builder = report
            .not_checkable("VBS-600", file.location(), reason(unreadable.obstacle))
            .file(facts.clone())
            .activation(Activation::Macro)
            .detail("container", document.container.as_str())
            .detail("readError", read_error(&unreadable.message));
        if let Some(location) = &unreadable.location {
            builder = builder.item(location.clone());
        }
        if let Some(encryption) = document.encryption {
            builder = builder.detail("encryption", encryption);
        }
        builder.emit();
    }
    let startup = loads_at_startup(file.path());
    for (found, uses) in &analyses {
        report_project(report, file, &facts, document, found, uses, startup);
    }
}

fn reason(obstacle: Obstacle) -> NotCheckableReason {
    match obstacle {
        Obstacle::PasswordProtected => NotCheckableReason::PasswordProtected,
        Obstacle::RightsManagement => NotCheckableReason::Encrypted,
        Obstacle::Corrupt => NotCheckableReason::Corrupt,
        Obstacle::Unsupported => NotCheckableReason::UnsupportedFormat,
        Obstacle::TooLarge => NotCheckableReason::TooLarge,
    }
}

fn report_project(
    report: &mut Report,
    file: &dyn CandidateFile,
    facts: &FileFacts,
    document: &Document,
    found: &FoundProject,
    uses: &[Use],
    startup: bool,
) {
    let project = match &found.project {
        Ok(project) => project,
        Err(message) => {
            // An encrypted binary document whose project cannot be read: most likely encrypted too.
            let reason = if document.content_encrypted {
                NotCheckableReason::PasswordProtected
            } else {
                NotCheckableReason::Corrupt
            };
            let mut builder = report
                .not_checkable("VBS-600", file.location(), reason)
                .item(found.storage.clone())
                .file(facts.clone())
                .activation(Activation::Macro)
                .detail("container", document.container.as_str())
                .detail("readError", read_error(message));
            if let Some(embedded) = &found.embedded {
                builder = builder.detail("embedded", embedded.clone());
            }
            builder.emit();
            return;
        }
    };
    let context = ProjectContext::new(document, found, project, startup);
    // Modules whose source could not be read: one finding for the project.
    let unreadable: Vec<&crate::analysis::ovba::Module> =
        project.modules.iter().filter(|m| m.source.is_err()).collect();
    if let Some(first) = unreadable.first() {
        let protected = project.protection.locked || project.protection.hidden || project.protection.unreadable;
        let reason = if protected || document.content_encrypted {
            NotCheckableReason::PasswordProtected
        } else {
            NotCheckableReason::Corrupt
        };
        let names: Vec<&str> = unreadable.iter().map(|module| module.name.as_str()).collect();
        let error = first.source.as_ref().err().cloned().unwrap_or_default();
        let builder = report
            .not_checkable("VBS-600", file.location(), reason)
            .item(found.storage.clone())
            .file(facts.clone())
            .activation(Activation::Macro)
            .detail("modulesUnreadable", i64::try_from(unreadable.len()).unwrap_or(i64::MAX))
            .detail("modules", read_error(&names.join(",")))
            .detail("readError", read_error(&error));
        context.details(builder).emit();
    }
    let mut modules_with_findings: Vec<&str> = Vec::new();
    for found_use in uses {
        let rule = rule(found_use.kind);
        let item = context.item(found_use);
        let mut builder = report.finding(rule, Location { item: Some(item), ..file.location() }).file(facts.clone());
        builder = builder.activation(Activation::Macro);
        if let Some(target) = &found_use.target {
            builder = builder.target(target.clone());
        }
        if let Some(via) = found_use.via {
            builder = builder.detail("via", via);
        }
        for (line, text) in &found_use.lines {
            builder = builder.evidence(*line, text);
        }
        context.details(builder).emit();
        if let Some(module) = &found_use.module
            && !modules_with_findings.contains(&module.as_str())
        {
            modules_with_findings.push(module);
        }
    }
    // Secrets in the code of modules with findings (masked; values never stored).
    for module in project.modules.iter().filter(|m| modules_with_findings.contains(&m.name.as_str())) {
        let Ok(source) = &module.source else { continue };
        let lines = vba::logical_lines(source);
        let location = Location { item: Some(context.module_item(&module.name)), ..file.location() };
        report.credentials(
            location,
            Some(facts.clone()),
            Activation::Macro,
            lines.iter().map(|l| (Some(l.number), l.text.as_str())),
        );
    }
}

fn rule(kind: Kind) -> &'static str {
    match kind {
        Kind::RegExp => "VBS-601",
        Kind::ScriptEngine => "VBS-611",
        Kind::ScriptEngineUnknown => "VBS-612",
        Kind::StartsVbScript => "VBS-621",
        Kind::StartsUnknown => "VBS-622",
        Kind::WshObject => "VBS-632",
    }
}

/// What every finding of one project says about the project.
struct ProjectContext<'a> {
    document: &'a Document,
    found: &'a FoundProject,
    project: &'a Project,
    startup: bool,
    auto_macro: Option<String>,
}

impl<'a> ProjectContext<'a> {
    fn new(document: &'a Document, found: &'a FoundProject, project: &'a Project, startup: bool) -> Self {
        let auto_macro = project
            .modules
            .iter()
            .filter_map(|module| module.source.as_ref().ok())
            .find_map(|source| vba::logical_lines(source).iter().find_map(|line| auto_macro(&line.text)));
        Self { document, found, project, startup, auto_macro }
    }

    /// `Module1`, `References/VBScript_RegExp_55`; prefixed with the embedded object.
    fn item(&self, found_use: &Use) -> String {
        match (&found_use.module, &found_use.reference) {
            (Some(module), _) => self.module_item(module),
            (None, Some(reference)) => self.prefixed(&format!("References/{reference}")),
            (None, None) => self.found.storage.clone(),
        }
    }

    fn module_item(&self, module: &str) -> String {
        self.prefixed(module)
    }

    fn prefixed(&self, name: &str) -> String {
        match &self.found.embedded {
            Some(embedded) => format!("{embedded}/{name}"),
            None => name.to_owned(),
        }
    }

    fn details<'r>(&self, mut builder: vbs_core::module::FindingBuilder<'r>) -> vbs_core::module::FindingBuilder<'r> {
        builder = builder.detail("container", self.document.container.as_str());
        if let Some(name) = &self.project.name {
            builder = builder.detail("project", name.clone());
        }
        if let Some(embedded) = &self.found.embedded {
            builder = builder.detail("embedded", embedded.clone());
        }
        if self.project.protection.locked {
            builder = builder.detail("projectLocked", true);
        }
        if self.project.protection.hidden || self.project.protection.unreadable {
            builder = builder.detail("projectUnviewable", true);
        }
        if self.document.content_encrypted {
            builder = builder.detail("documentEncrypted", true);
        }
        if let Some(name) = &self.auto_macro {
            builder = builder.detail("autoMacro", Detail::Text(name.clone()));
        }
        if self.startup {
            builder = builder.detail("loadsAtStartup", true);
        }
        builder
    }
}

/// `Sub Workbook_Open()`, `Private Sub Document_Open()` … – the procedure name if Office runs it by itself.
fn auto_macro(line: &str) -> Option<String> {
    let mut words = line.split(|c: char| c.is_whitespace() || c == '(').filter(|word| !word.is_empty());
    let mut word = words.next()?;
    if matches!(word.to_ascii_lowercase().as_str(), "private" | "public" | "friend" | "static") {
        word = words.next()?;
    }
    if !word.eq_ignore_ascii_case("sub") {
        return None;
    }
    let name = words.next()?;
    AUTO_MACROS.contains(&name.to_ascii_lowercase().as_str()).then(|| name.to_owned())
}

/// Workbooks, templates and add-ins in these folders are loaded whenever Excel or Word starts:
/// `XLSTART`, Word's `STARTUP` folder and the global template `Normal.dotm`.
fn loads_at_startup(path: &Path) -> bool {
    let lower = path.to_string_lossy().replace('/', "\\").to_lowercase();
    let file = lower.rsplit('\\').next().unwrap_or_default();
    lower.contains("\\xlstart\\")
        || lower.contains("\\microsoft\\word\\startup\\")
        || (lower.contains("\\microsoft office\\") && lower.contains("\\startup\\"))
        || (lower.contains("\\microsoft\\templates\\") && matches!(file, "normal.dotm" | "normal.dot"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::ovba::tests::MemoryProject;
    use crate::modules::testing::{MemoryFile, inspect};
    use std::io::{Cursor, Write};
    use vbs_core::model::FindingStatus;

    const MODULE: &str = "Attribute VB_Name = \"Module1\"\r\nPrivate Sub Workbook_Open()\r\n    Set re = CreateObject(\"VBScript.RegExp\")\r\n    conn = \"Provider=SQLOLEDB;Password=Sommer2024!\"\r\n    CreateObject(\"WScript.Shell\").Run \"wscript.exe C:\\Scripts\\sync.vbs\"\r\nEnd Sub\r\n";

    fn workbook(module: &str, project_text: &str) -> Vec<u8> {
        let project = MemoryProject::new(&[], &[("Module1", module, true)], project_text);
        let mut file = cfb::CompoundFile::create(Cursor::new(Vec::new())).unwrap();
        file.create_storage_all("/_VBA_PROJECT_CUR/VBA").unwrap();
        for (path, bytes) in &project.0 {
            let mut stream = file.create_stream(format!("/_VBA_PROJECT_CUR/{path}")).unwrap();
            stream.write_all(bytes).unwrap();
        }
        file.create_stream("/Workbook").unwrap().write_all(&[0x09, 0x08, 0x00, 0x00]).unwrap();
        file.flush().unwrap();
        file.into_inner().into_inner()
    }

    #[test]
    fn reports_each_use_with_module_and_lines() {
        let file =
            MemoryFile::new(r"C:\Users\ann\AppData\Roaming\Microsoft\Excel\XLSTART\Tools.xls", workbook(MODULE, ""));
        let findings = inspect(&OfficeMacros, &file);
        let summary: Vec<(&str, Option<&str>, Option<&str>)> =
            findings.iter().map(|f| (f.rule.as_str(), f.location.item.as_deref(), f.target.as_deref())).collect();
        assert_eq!(
            summary,
            [
                ("VBS-601", Some("Module1"), Some("VBScript.RegExp")),
                ("VBS-621", Some("Module1"), Some(r"C:\Scripts\sync.vbs")),
                ("VBS-632", Some("Module1"), Some("WScript.Shell")),
                (CREDENTIAL_RULE, Some("Module1"), None),
            ]
        );
        let regexp = &findings[0];
        assert_eq!(regexp.activation, Activation::Macro);
        assert_eq!(regexp.evidence[0].line, Some(2));
        assert_eq!(regexp.details["autoMacro"], Detail::Text("Workbook_Open".into()));
        assert_eq!(regexp.details["loadsAtStartup"], Detail::Flag(true));
        assert_eq!(regexp.details["container"], Detail::Text("compoundFile".into()));
        assert!(!serde_json::to_string(&findings).unwrap().contains("Sommer2024!"));
    }

    #[test]
    fn locked_projects_are_read_and_marked() {
        let dpb = crate::analysis::ovba::tests::encrypt(0x57, 0x2A, &[0xFF; 29]);
        let file = MemoryFile::new("C:/Finance/Locked.xls", workbook(MODULE, &format!("DPB=\"{dpb}\"\r\n")));
        let findings = inspect(&OfficeMacros, &file);
        assert_eq!(findings[0].rule, "VBS-601");
        assert_eq!(findings[0].details["projectLocked"], Detail::Flag(true));
    }

    #[test]
    fn unreadable_modules_and_documents_are_not_checkable() {
        // A module stream without source code in a locked project.
        let mut bytes = MemoryProject::new(&[], &[("Module1", MODULE, true)], "");
        bytes.0.insert("VBA/MODULE1".into(), vec![0u8; 40]);
        let dpb = crate::analysis::ovba::tests::encrypt(0x57, 0x2A, &[0xFF; 29]);
        bytes.0.insert("PROJECT".into(), format!("DPB=\"{dpb}\"\r\n").into_bytes());
        let mut file = cfb::CompoundFile::create(Cursor::new(Vec::new())).unwrap();
        file.create_storage_all("/Macros/VBA").unwrap();
        for (path, content) in &bytes.0 {
            file.create_stream(format!("/Macros/{path}")).unwrap().write_all(content).unwrap();
        }
        file.flush().unwrap();
        let findings = inspect(&OfficeMacros, &MemoryFile::new("C:/Docs/Locked.doc", file.into_inner().into_inner()));
        assert_eq!(findings.len(), 1);
        assert_eq!((findings[0].rule.as_str(), &findings[0].status), ("VBS-600", &FindingStatus::NotCheckable));
        assert_eq!(findings[0].reason, Some(NotCheckableReason::PasswordProtected));
        assert_eq!(findings[0].location.item.as_deref(), Some("Macros"));

        // A package encrypted with a password to open.
        let mut info = 4u16.to_le_bytes().to_vec();
        info.extend(4u16.to_le_bytes());
        let mut file = cfb::CompoundFile::create(Cursor::new(Vec::new())).unwrap();
        file.create_stream("/EncryptionInfo").unwrap().write_all(&info).unwrap();
        file.create_stream("/EncryptedPackage").unwrap().write_all(&[0u8; 32]).unwrap();
        file.flush().unwrap();
        let findings = inspect(&OfficeMacros, &MemoryFile::new("C:/Docs/Secret.xlsm", file.into_inner().into_inner()));
        assert_eq!(findings[0].reason, Some(NotCheckableReason::PasswordProtected));
        assert_eq!(findings[0].details["encryption"], Detail::Text("agile".into()));
    }

    #[test]
    fn documents_without_vbscript_are_no_finding() {
        let module = "Attribute VB_Name = \"Module1\"\r\nSub A()\r\n    Set fso = CreateObject(\"Scripting.FileSystemObject\")\r\nEnd Sub\r\n";
        assert!(inspect(&OfficeMacros, &MemoryFile::new("C:/x/Plain.xls", workbook(module, ""))).is_empty());
        assert!(inspect(&OfficeMacros, &MemoryFile::new("C:/x/Export.xls", b"a;b\r\n1;2\r\n".to_vec())).is_empty());
    }

    #[test]
    fn auto_macros_and_startup_folders() {
        assert_eq!(auto_macro("Private Sub Workbook_Open()").as_deref(), Some("Workbook_Open"));
        assert_eq!(auto_macro("Sub AutoOpen"), Some("AutoOpen".into()));
        assert_eq!(auto_macro("Sub Report()"), None);
        assert!(loads_at_startup(Path::new(r"C:\Users\a\AppData\Roaming\Microsoft\Templates\Normal.dotm")));
        assert!(loads_at_startup(Path::new(r"C:\Program Files\Microsoft Office\root\Office16\STARTUP\x.dotm")));
        assert!(!loads_at_startup(Path::new(r"D:\Share\Budget.xlsm")));
    }
}
