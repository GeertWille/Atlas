//! Tauri shell of the AdTech Scanner: composition root and command layer.

mod adapters;
pub mod commands;
pub mod state;

use std::sync::Arc;

use tauri::{Manager, RunEvent, Runtime};

pub use crate::adapters::{ChromeCaptureSource, TauriEventSink};
use crate::state::{AppConfig, AppState};

/// Every command of the frontend contract, with the plugin `export_scan`
/// depends on. Used by `run` and by the tests.
pub fn register_commands<R: Runtime>(builder: tauri::Builder<R>) -> tauri::Builder<R> {
    builder
        // The webview has no fs permission. The plugin is loaded for its scope,
        // where the dialog plugin records the paths the user picked.
        .plugin(tauri_plugin_fs::init())
        .invoke_handler(tauri::generate_handler![
            commands::get_settings,
            commands::update_settings,
            commands::set_api_key,
            commands::clear_api_key,
            commands::detect_browser,
            commands::get_demo_url,
            commands::start_scan,
            commands::cancel_scan,
            commands::list_scans,
            commands::get_scan,
            commands::delete_scan,
            commands::delete_all_scans,
            commands::get_logs,
            commands::search_logs,
            commands::get_dashboard_stats,
            commands::export_scan,
        ])
}

pub fn run() {
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let config = AppConfig::from_environment(app.path().app_data_dir()?);
            let events = Arc::new(TauriEventSink {
                app: app.handle().clone(),
            });
            let state = tauri::async_runtime::block_on(AppState::build(
                config,
                Arc::new(ChromeCaptureSource),
                events,
            ))?;
            app.manage(state);
            Ok(())
        });
    register_commands(builder)
        .build(tauri::generate_context!())
        .expect("failed to start AdTech Scanner")
        .run(|app, event| {
            if let RunEvent::Exit = event
                && let Some(state) = app.try_state::<AppState>()
            {
                // Cancels running scans, which also closes their browsers.
                tauri::async_runtime::block_on(state.scans.shutdown());
            }
        });
}
