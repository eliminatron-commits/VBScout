//! Serializable views of loaded results for the frontend.

use serde::Serialize;
use vbs_core::model::FindingStatus;
use vbs_evaluation::import::{ImportBatch, ImportProblem, ImportedFile};

/// One loaded machine (one result file).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MachineSummary {
    pub scan_id: String,
    pub file_name: String,
    pub path: String,
    pub hostname: String,
    pub domain: Option<String>,
    /// e.g. "Windows 11 Pro 24H2".
    pub os: Option<String>,
    /// RFC 3339; formatted by the frontend in the UI language.
    pub scanned_at: String,
    /// `full` or `limited`.
    pub coverage: String,
    /// Limitation codes (`limitation.<code>` translation keys).
    pub limitations: Vec<String>,
    pub findings: usize,
    pub not_checkable: usize,
    pub collector_version: String,
}

impl From<&ImportedFile> for MachineSummary {
    fn from(file: &ImportedFile) -> Self {
        let result = &file.result;
        let os = &result.machine.os;
        let os_name = match (&os.name, &os.display_version) {
            (Some(name), Some(version)) => Some(format!("{name} {version}")),
            (Some(name), None) => Some(name.clone()),
            (None, _) => os.version.clone(),
        };
        let not_checkable = result.findings.iter().filter(|f| f.status == FindingStatus::NotCheckable).count();
        MachineSummary {
            scan_id: result.scan_id.to_string(),
            file_name: file.path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default(),
            path: file.path.display().to_string(),
            hostname: result.machine.hostname.clone(),
            domain: result.machine.domain.clone(),
            os: os_name,
            scanned_at: result.started_at.format(&time::format_description::well_known::Rfc3339).unwrap_or_default(),
            coverage: result.coverage.mode.as_str().to_owned(),
            limitations: result.coverage.limitations.iter().map(|l| l.code.as_str().to_owned()).collect(),
            findings: result.findings.len() - not_checkable,
            not_checkable,
            collector_version: result.generator.version.clone(),
        }
    }
}

/// A file that could not be imported.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportErrorView {
    pub file: String,
    /// `import.error.<code>` translation key.
    pub code: &'static str,
    pub message: Option<String>,
    pub found: Option<u32>,
    pub supported: Option<u32>,
}

/// Outcome of an import action plus the machines loaded afterwards.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportSummary {
    pub cancelled: bool,
    pub loaded: usize,
    pub duplicates: usize,
    pub errors: Vec<ImportErrorView>,
    /// Some files contain values from a newer version (shown simplified).
    pub newer_values: bool,
    pub machines: Vec<MachineSummary>,
}

impl ImportSummary {
    pub fn cancelled(machines: Vec<MachineSummary>) -> Self {
        Self { cancelled: true, loaded: 0, duplicates: 0, errors: Vec::new(), newer_values: false, machines }
    }

    pub fn from_batch(batch: &ImportBatch, machines: Vec<MachineSummary>) -> Self {
        let errors = batch
            .errors
            .iter()
            .map(|error| {
                let (message, found, supported) = match &error.problem {
                    ImportProblem::NewerSchema { found, supported } => (None, Some(*found), Some(*supported)),
                    ImportProblem::WrongFormat(text)
                    | ImportProblem::Invalid(text)
                    | ImportProblem::TooLarge(text)
                    | ImportProblem::Io(text) => (Some(text.clone()), None, None),
                    ImportProblem::NotAResultFile => (None, None, None),
                };
                ImportErrorView {
                    file: error.path.display().to_string(),
                    code: error.problem.code(),
                    message,
                    found,
                    supported,
                }
            })
            .collect();
        Self {
            cancelled: false,
            loaded: batch.files.len(),
            duplicates: batch.duplicates.len(),
            errors,
            newer_values: batch.files.iter().any(|file| file.unknown_values > 0),
            machines,
        }
    }
}
