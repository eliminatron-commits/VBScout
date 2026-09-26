//! Evaluation desktop application entry point (Tauri 2).

// No console window for release builds on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod settings;
mod smoke;
mod state;
mod summary;

use tauri::Manager;

fn main() {
    let smoke = smoke::SmokeTest::from_args(std::env::args().skip(1));
    let app = tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            commands::app_info,
            commands::set_ui_language,
            commands::open_result_files,
            commands::open_result_folder,
            commands::import_paths,
            commands::loaded_machines,
            commands::clear_results,
            commands::frontend_ready,
        ])
        .setup(move |app| {
            let settings_path = app.path().app_config_dir()?.join("settings.json");
            app.manage(state::AppState::new(settings_path, smoke));
            smoke.arm_watchdog();
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("failed to initialise the application");
    app.run(|_, _| {});
}
