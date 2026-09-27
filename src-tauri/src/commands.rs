//! Commands callable from the frontend. All rules (edition limits, what may be
//! imported, what the reports contain) are enforced here or deeper in Rust –
//! the UI only reflects them.

use std::path::{Path, PathBuf};

use serde::Serialize;
use tauri::{AppHandle, State, WebviewWindow};
use vbs_evaluation::edition::Edition;
use vbs_evaluation::report::{Logo, LogoError, LogoFormat, ReportContext, pdf, xlsx};
use vbs_i18n::{Lang, key, t_args};

use crate::state::AppState;
use crate::summary::{ImportSummary, MachineSummary};
use crate::views::{FindingDetail, FindingQuery, FindingsPage, Overview};

/// Error codes the frontend translates (`error.<code>`); anything else is shown as a message.
pub const NOT_LICENSED: &str = "notLicensed";

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditionInfo {
    /// `free`, `organization` or `msp`.
    kind: &'static str,
    /// Machine limit of the free edition; `None` = unlimited.
    max_machines: Option<u32>,
    /// Organization or service provider named in the reports.
    licensee: Option<String>,
    pdf: bool,
    hints: bool,
    effort: bool,
    logo: bool,
}

impl From<&Edition> for EditionInfo {
    fn from(edition: &Edition) -> Self {
        Self {
            kind: edition.kind(),
            max_machines: edition.machine_limit().map(|limit| u32::try_from(limit).unwrap_or(u32::MAX)),
            licensee: edition.licensee().map(str::to_owned),
            pdf: edition.allows_pdf(),
            hints: edition.allows_hints(),
            effort: edition.allows_effort(),
            logo: edition.allows_logo(),
        }
    }
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
        edition: (&state.edition()).into(),
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
    let limit = state.edition().machine_limit();
    let batch = vbs_evaluation::import::import(&paths, &state.loaded_state(), limit);
    {
        let mut results = state.results.lock().map_err(|_| "results unavailable")?;
        results.extend(batch.files.iter().cloned());
        results.sort_by_key(|file| file.result.machine.hostname.to_lowercase());
    }
    state.rebuild();
    Ok(ImportSummary::from_batch(&batch, machines(state), limit))
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
    state.rebuild();
}

/// Key figures, risks by type and coverage; `None` while nothing is loaded.
#[tauri::command]
pub fn overview(state: State<'_, AppState>) -> Option<Overview> {
    let edition = state.edition();
    state.assessment.lock().ok()?.as_ref().map(|assessment| Overview::new(assessment, &edition))
}

/// One page of the (filtered) finding list.
#[tauri::command]
pub fn findings(state: State<'_, AppState>, query: FindingQuery) -> FindingsPage {
    let edition = state.edition();
    let assessment = state.assessment.lock().ok();
    match assessment.as_ref().and_then(|slot| slot.as_ref()) {
        Some(assessment) => crate::views::findings_page(assessment, &edition, &query),
        None => FindingsPage { total: 0, offset: 0, rows: Vec::new() },
    }
}

/// Everything about one item (by its number).
#[tauri::command]
pub fn finding(state: State<'_, AppState>, number: usize) -> Option<FindingDetail> {
    let edition = state.edition();
    let assessment = state.assessment.lock().ok()?;
    crate::views::finding_detail(assessment.as_ref()?, &edition, number)
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogoView {
    /// `data:` URL for the preview (the CSP allows `data:` images only).
    data_url: String,
    width: u32,
    height: u32,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportSettings {
    customer: Option<String>,
    logo: Option<LogoView>,
}

fn report_settings_of(state: &AppState) -> ReportSettings {
    let branding = state.branding();
    ReportSettings {
        customer: branding.customer,
        logo: branding.logo.map(|logo| {
            let media = match logo.format() {
                LogoFormat::Png => "image/png",
                LogoFormat::Jpeg => "image/jpeg",
            };
            let (width, height) = logo.size();
            LogoView { data_url: format!("data:{media};base64,{}", base64(logo.bytes())), width, height }
        }),
    }
}

#[tauri::command]
pub fn report_settings(state: State<'_, AppState>) -> ReportSettings {
    report_settings_of(&state)
}

/// Sets the customer or environment named in the reports.
#[tauri::command]
pub fn set_report_customer(state: State<'_, AppState>, customer: Option<String>) -> Result<ReportSettings, String> {
    {
        let mut settings = state.settings.lock().map_err(|_| "settings unavailable")?;
        settings.report_customer =
            customer.map(|name| name.trim().chars().take(120).collect::<String>()).filter(|name| !name.is_empty());
        settings.save(&state.settings_path).map_err(|e| e.to_string())?;
    }
    Ok(report_settings_of(&state))
}

fn logo_error(error: &LogoError) -> String {
    match error {
        LogoError::UnsupportedFormat => "logoFormat".into(),
        LogoError::TooLarge => "logoTooLarge".into(),
        LogoError::Invalid(_) => "logoInvalid".into(),
    }
}

/// Lets the user pick a PNG or JPEG logo for the reports (shown with an MSP license).
#[tauri::command]
pub async fn choose_report_logo(window: WebviewWindow, state: State<'_, AppState>) -> Result<ReportSettings, String> {
    let lang = ui_lang(&state);
    let picked = rfd::AsyncFileDialog::new()
        .set_parent(&window)
        .set_title(t_args(lang, "export.logoChoose", &[]))
        .add_filter("PNG, JPEG", &["png", "jpg", "jpeg"])
        .pick_file()
        .await;
    let Some(file) = picked else { return Ok(report_settings_of(&state)) };
    let bytes = std::fs::read(file.path()).map_err(|e| e.to_string())?;
    let logo = Logo::from_bytes(bytes).map_err(|error| logo_error(&error))?;
    write_file(&state.logo_path, logo.bytes()).map_err(|e| e.to_string())?;
    if let Ok(mut slot) = state.logo.lock() {
        *slot = Some(logo);
    }
    Ok(report_settings_of(&state))
}

#[tauri::command]
pub fn clear_report_logo(state: State<'_, AppState>) -> Result<ReportSettings, String> {
    match std::fs::remove_file(&state.logo_path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.to_string()),
    }
    if let Ok(mut slot) = state.logo.lock() {
        *slot = None;
    }
    Ok(report_settings_of(&state))
}

/// Writes a file completely or not at all (temporary file, then rename).
fn write_file(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let temp = path.with_extension(format!("tmp-{}", uuid::Uuid::new_v4().simple()));
    std::fs::write(&temp, bytes)?;
    std::fs::rename(&temp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&temp);
    })
}

#[derive(Debug, Clone, Copy)]
enum ReportKind {
    Excel,
    Pdf,
}

/// Creates a report in `language` (default: the UI language) and saves it where the user chooses.
/// Returns the path, or `None` when the dialog was cancelled.
async fn export(
    window: &WebviewWindow,
    state: &AppState,
    language: Option<String>,
    kind: ReportKind,
) -> Result<Option<String>, String> {
    let edition = state.edition();
    if matches!(kind, ReportKind::Pdf) && !edition.allows_pdf() {
        return Err(NOT_LICENSED.into());
    }
    let ui = ui_lang(state);
    let lang = language.as_deref().and_then(Lang::from_tag).unwrap_or(ui);
    let created = time::OffsetDateTime::now_utc();
    let date = format!("{:04}-{:02}-{:02}", created.year(), u8::from(created.month()), created.day());
    let (name_key, extension, filter) = match kind {
        ReportKind::Excel => (key("export.fileExcel"), "xlsx", "Excel"),
        ReportKind::Pdf => (key("export.filePdf"), "pdf", "PDF"),
    };
    let file_name = format!("{}.{extension}", t_args(lang, name_key, &[("date", &date)]));
    let picked = rfd::AsyncFileDialog::new()
        .set_parent(window)
        .set_file_name(file_name)
        .add_filter(filter, &[extension])
        .save_file()
        .await;
    let Some(file) = picked else { return Ok(None) };
    let bytes = {
        let branding = state.branding();
        let slot = state.assessment.lock().map_err(|_| "assessment unavailable")?;
        let assessment = slot.as_ref().ok_or("nothing loaded")?;
        let context =
            ReportContext { lang, edition: &edition, branding: &branding, created, version: env!("CARGO_PKG_VERSION") };
        match kind {
            ReportKind::Excel => xlsx::write(assessment, &context),
            ReportKind::Pdf => pdf::write(assessment, &context),
        }
        .map_err(|error| error.to_string())?
    };
    let mut path = file.path().to_path_buf();
    if path.extension().is_none_or(|existing| !existing.eq_ignore_ascii_case(extension)) {
        path.set_extension(extension);
    }
    write_file(&path, &bytes).map_err(|e| e.to_string())?;
    Ok(Some(path.display().to_string()))
}

#[tauri::command]
pub async fn export_excel(
    window: WebviewWindow,
    state: State<'_, AppState>,
    language: Option<String>,
) -> Result<Option<String>, String> {
    export(&window, &state, language, ReportKind::Excel).await
}

#[tauri::command]
pub async fn export_pdf(
    window: WebviewWindow,
    state: State<'_, AppState>,
    language: Option<String>,
) -> Result<Option<String>, String> {
    export(&window, &state, language, ReportKind::Pdf).await
}

/// Called once by the frontend after its first render.
#[tauri::command]
pub fn frontend_ready(app: AppHandle, state: State<'_, AppState>) {
    if state.smoke.enabled() {
        crate::smoke::run(&app);
    }
}

/// Standard base64 (RFC 4648) for `data:` URLs.
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let value = (u32::from(chunk[0]) << 16)
            | (u32::from(chunk.get(1).copied().unwrap_or(0)) << 8)
            | u32::from(chunk.get(2).copied().unwrap_or(0));
        for index in 0..4 {
            if index <= chunk.len() {
                out.push(ALPHABET[(value >> (18 - 6 * index)) as usize & 63] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_rfc_4648() {
        for (input, expected) in [
            (&b""[..], ""),
            (b"f", "Zg=="),
            (b"fo", "Zm8="),
            (b"foo", "Zm9v"),
            (b"foob", "Zm9vYg=="),
            (b"fooba", "Zm9vYmE="),
            (b"foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64(input), expected);
        }
    }

    #[test]
    fn files_are_written_completely() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("report.pdf");
        write_file(&path, b"%PDF-1.7").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"%PDF-1.7");
        write_file(&path, b"new").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"new");
        assert_eq!(std::fs::read_dir(path.parent().unwrap()).unwrap().count(), 1, "no temporary file left");
    }
}
