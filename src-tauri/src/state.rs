use std::path::PathBuf;
use std::sync::Mutex;

use vbs_evaluation::assessment::Assessment;
use vbs_evaluation::edition::Edition;
use vbs_evaluation::import::{ImportedFile, LoadedState};
use vbs_evaluation::report::{Branding, Logo};
use vbs_license::{License, LicenseVerifier};

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
    /// The entered license key, stored next to the settings.
    pub license_path: PathBuf,
    /// Offline verifier with the public key of this build (`product.json`).
    pub verifier: Box<dyn LicenseVerifier>,
    /// Verified license (possibly expired – the edition checks the date); `None` = free edition.
    pub license: Mutex<Option<License>>,
    pub smoke: SmokeTest,
}

impl AppState {
    pub fn new(config_dir: PathBuf, smoke: SmokeTest) -> Self {
        Self::with_verifier(config_dir, smoke, vbs_license::embedded_verifier())
    }

    /// State with a given license verifier (tests use their own key pair).
    pub fn with_verifier(config_dir: PathBuf, smoke: SmokeTest, verifier: Box<dyn LicenseVerifier>) -> Self {
        let settings_path = config_dir.join("settings.json");
        let logo_path = config_dir.join("report-logo");
        let logo = std::fs::read(&logo_path).ok().and_then(|bytes| Logo::from_bytes(bytes).ok());
        let license_path = config_dir.join("license.key");
        // A stored key is checked again at every start; a key this build cannot verify is ignored.
        let license = std::fs::read_to_string(&license_path).ok().and_then(|key| verifier.decode(&key).ok());
        Self {
            settings: Mutex::new(Settings::load(&settings_path)),
            settings_path,
            logo_path,
            results: Mutex::new(Vec::new()),
            assessment: Mutex::new(None),
            logo: Mutex::new(logo),
            license_path,
            verifier,
            license: Mutex::new(license),
            smoke,
        }
    }

    /// Scans and machines already loaded (duplicates, machine limit).
    pub fn loaded_state(&self) -> LoadedState {
        self.results.lock().map(|results| LoadedState::of(&results)).unwrap_or_default()
    }

    /// The edition the current license grants today (system clock).
    pub fn edition(&self) -> Edition {
        Edition::from_license(self.license().as_ref(), today())
    }

    pub fn license(&self) -> Option<License> {
        self.license.lock().ok().and_then(|license| license.clone())
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

/// The system date (UTC) – license expiry is checked against the system clock.
pub fn today() -> time::Date {
    time::OffsetDateTime::now_utc().date()
}
