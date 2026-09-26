//! Commands callable from the frontend. All rules (edition limits, what may be
//! imported) are enforced here or deeper in Rust – the UI only reflects them.

use std::path::PathBuf;

use serde::Serialize;
use tauri::{AppHandle, State, WebviewWindow};
use vbs_i18n::{Lang, t_args};

use crate::state::AppState;
use crate::summary::{ImportSummary, MachineSummary};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditionInfo {
    /// `free`, `organization` or `msp` (licenses arrive in phase 5).
    kind: &'static str,
    /// Machine limit of the free edition; `None` = unlimited.
    max_machines: Option<u32>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    product_name: String,
    version: &'static str,
    platform: &'static str,
    edition: EditionInfo,
    language: LanguageInfo,
    /// Explicit choice; `None` follows the operating system.
    ui_language_setting: Option<String>,
    translations_url: String,
    rules_as_of: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LanguageInfo {
    /// Effective UI language tag.
    tag: &'static str,
    /// Reviewed by a native speaker (English and German); otherwise the UI shows a correction hint.
    reviewed: bool,
}

impl From<Lang> for LanguageInfo {
    fn from(lang: Lang) -> Self {
        Self { tag: lang.tag(), reviewed: lang.is_reviewed() }
    }
}

fn os_locale() -> Option<String> {
    sys_locale::get_locale()
}

fn ui_lang(state: &AppState) -> Lang {
    state.settings.lock().map(|settings| settings.ui_lang(os_locale().as_deref())).unwrap_or(Lang::En)
}

#[tauri::command]
pub fn app_info(state: State<'_, AppState>) -> Result<AppInfo, String> {
    let settings = state.settings.lock().map_err(|_| "settings unavailable")?.clone();
    let product = vbs_config::product();
    Ok(AppInfo {
        product_name: product.name.clone(),
        version: env!("CARGO_PKG_VERSION"),
        platform: std::env::consts::OS,
        edition: EditionInfo { kind: "free", max_machines: Some(product.editions.free.max_machines) },
        language: settings.ui_lang(os_locale().as_deref()).into(),
        ui_language_setting: settings.ui_language.clone(),
        translations_url: product.translations_url.clone(),
        rules_as_of: vbs_core::rules::catalog().as_of_text(),
    })
}

/// Sets (or with `None` resets to the system default) the UI language.
#[tauri::command]
pub fn set_ui_language(state: State<'_, AppState>, language: Option<String>) -> Result<LanguageInfo, String> {
    let language = match language {
        Some(tag) => {
            Some(Lang::from_tag(&tag).ok_or_else(|| format!("unsupported language {tag:?}"))?.tag().to_owned())
        }
        None => None,
    };
    let mut settings = state.settings.lock().map_err(|_| "settings unavailable")?;
    settings.ui_language = language;
    settings.save(&state.settings_path).map_err(|e| e.to_string())?;
    Ok(settings.ui_lang(os_locale().as_deref()).into())
}

fn machines(state: &AppState) -> Vec<MachineSummary> {
    state.results.lock().map(|results| results.iter().map(MachineSummary::from).collect()).unwrap_or_default()
}

fn import(state: &AppState, paths: Vec<PathBuf>) -> Result<ImportSummary, String> {
    let batch = vbs_evaluation::import::import(&paths, &state.loaded_scan_ids());
    let mut results = state.results.lock().map_err(|_| "results unavailable")?;
    results.extend(batch.files.iter().cloned());
    results.sort_by_key(|file| file.result.machine.hostname.to_lowercase());
    let machines = results.iter().map(MachineSummary::from).collect();
    Ok(ImportSummary::from_batch(&batch, machines))
}

fn dialog(window: &WebviewWindow, lang: Lang) -> rfd::AsyncFileDialog {
    let filter = t_args(lang, "import.dialogFilter", &[]);
    rfd::AsyncFileDialog::new().set_parent(window).add_filter(filter, &[vbs_core::file_extension()])
}

/// Lets the user pick result files and imports them.
#[tauri::command]
pub async fn open_result_files(window: WebviewWindow, state: State<'_, AppState>) -> Result<ImportSummary, String> {
    let lang = ui_lang(&state);
    let title = t_args(lang, "home.openFiles", &[]);
    match dialog(&window, lang).set_title(title).pick_files().await {
        Some(files) => import(&state, files.into_iter().map(|file| file.path().to_path_buf()).collect()),
        None => Ok(ImportSummary::cancelled(machines(&state))),
    }
}

/// Lets the user pick a folder and imports every result file below it.
#[tauri::command]
pub async fn open_result_folder(window: WebviewWindow, state: State<'_, AppState>) -> Result<ImportSummary, String> {
    let lang = ui_lang(&state);
    let title = t_args(lang, "home.openFolder", &[]);
    match dialog(&window, lang).set_title(title).pick_folder().await {
        Some(folder) => import(&state, vec![folder.path().to_path_buf()]),
        None => Ok(ImportSummary::cancelled(machines(&state))),
    }
}

/// Imports files and folders dropped onto the window.
#[tauri::command]
pub async fn import_paths(state: State<'_, AppState>, paths: Vec<String>) -> Result<ImportSummary, String> {
    import(&state, paths.into_iter().map(PathBuf::from).collect())
}

#[tauri::command]
pub fn loaded_machines(state: State<'_, AppState>) -> Vec<MachineSummary> {
    machines(&state)
}

#[tauri::command]
pub fn clear_results(state: State<'_, AppState>) {
    if let Ok(mut results) = state.results.lock() {
        results.clear();
    }
}

/// Called once by the frontend after its first render.
#[tauri::command]
pub fn frontend_ready(app: AppHandle, state: State<'_, AppState>) {
    if state.smoke.enabled() {
        crate::smoke::run(&app);
    }
}
