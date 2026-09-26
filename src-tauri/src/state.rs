use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Mutex;

use uuid::Uuid;
use vbs_evaluation::import::ImportedFile;

use crate::settings::Settings;
use crate::smoke::SmokeTest;

/// Application state shared by all commands.
pub struct AppState {
    pub settings_path: PathBuf,
    pub settings: Mutex<Settings>,
    /// Result files loaded in this session (nothing is persisted).
    pub results: Mutex<Vec<ImportedFile>>,
    pub smoke: SmokeTest,
}

impl AppState {
    pub fn new(settings_path: PathBuf, smoke: SmokeTest) -> Self {
        Self {
            settings: Mutex::new(Settings::load(&settings_path)),
            settings_path,
            results: Mutex::new(Vec::new()),
            smoke,
        }
    }

    /// Scan IDs of the loaded results (to recognise duplicates).
    pub fn loaded_scan_ids(&self) -> HashSet<Uuid> {
        self.results.lock().map(|results| results.iter().map(|file| file.result.scan_id).collect()).unwrap_or_default()
    }
}
