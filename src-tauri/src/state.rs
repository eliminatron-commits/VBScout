use std::path::PathBuf;
use std::sync::Mutex;

use vbs_evaluation::assessment::Assessment;
use vbs_evaluation::edition::Edition;
use vbs_evaluation::import::{ImportedFile, LoadedState};
use vbs_evaluation::report::{Branding, Logo};
use vbs_license::License;

use crate::settings::Settings;
use crate::smoke::SmokeTest;

/// Application state shared by all commands.
pub struct AppState {
    pub settings_path: PathBuf,
    /// The report logo, a copy of the chosen image next to the settings.
    pub logo_path: PathBuf,
    pub settings: Mutex<Settings>,
    /// Result files loaded in this session (nothing is persisted).
    pub results: Mutex<Vec<ImportedFile>>,
    /// The assessment of the loaded results, rebuilt after every change.
    pub assessment: Mutex<Option<Assessment>>,
    pub logo: Mutex<Option<Logo>>,
    /// Verified license (phase 5 adds the signed keys); `None` = free edition.
    pub license: Mutex<Option<License>>,
    pub smoke: SmokeTest,
}

impl AppState {
    pub fn new(config_dir: PathBuf, smoke: SmokeTest) -> Self {
        let settings_path = config_dir.join("settings.json");
        let logo_path = config_dir.join("report-logo");
        let logo = std::fs::read(&logo_path).ok().and_then(|bytes| Logo::from_bytes(bytes).ok());
        Self {
            settings: Mutex::new(Settings::load(&settings_path)),
            settings_path,
            logo_path,
            results: Mutex::new(Vec::new()),
            assessment: Mutex::new(None),
            logo: Mutex::new(logo),
            license: Mutex::new(None),
            smoke,
        }
    }

    /// Scans and machines already loaded (duplicates, machine limit).
    pub fn loaded_state(&self) -> LoadedState {
        self.results.lock().map(|results| LoadedState::of(&results)).unwrap_or_default()
    }

    /// The edition the current license grants today (system clock).
    pub fn edition(&self) -> Edition {
        let today = time::OffsetDateTime::now_utc().date();
        let license = self.license.lock().ok().and_then(|license| license.clone());
        Edition::from_license(license.as_ref(), today)
    }

    /// Rebuilds the assessment from the loaded results.
    pub fn rebuild(&self) {
        let assessment =
            self.results.lock().ok().map(|results| {
                (!results.is_empty()).then(|| Assessment::build(&results, self.edition().machine_limit()))
            });
        if let (Some(assessment), Ok(mut slot)) = (assessment, self.assessment.lock()) {
            *slot = assessment;
        }
    }

    /// Names and logo for the reports.
    pub fn branding(&self) -> Branding {
        Branding {
            customer: self.settings.lock().ok().and_then(|settings| settings.report_customer.clone()),
            logo: self.logo.lock().ok().and_then(|logo| logo.clone()),
        }
    }
}
