//! Tauri shell of the AdTech Scanner: composition root and command layer.

mod adapters;
pub mod commands;
pub mod state;

use std::sync::Arc;

use tauri::{Manager, RunEvent, Runtime};

pub use crate::adapters::{BrowserbaseAdapter, ChromeCaptureSource, OpenUrl, TauriEventSink};
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
            commands::finish_capture,
            commands::get_scan_profile,
            commands::clear_scan_profile,
            commands::save_proxy_profile,
            commands::delete_proxy_profile,
            commands::set_proxy_password,
            commands::clear_proxy_password,
            commands::list_scan_configurations,
            commands::save_scan_configuration,
            commands::delete_scan_configuration,
            commands::list_network_requests,
            commands::get_network_summary,
            commands::get_network_request,
            commands::get_network_body,
            commands::get_script_source,
            commands::get_archive_storage,
            commands::clear_archive,
            commands::get_sellers_json_cache,
            commands::clear_sellers_json_cache,
            commands::set_browserbase_credentials,
            commands::clear_browserbase_credentials,
            commands::test_browserbase,
            commands::probe_browserbase,
            commands::get_browserbase_profile,
            commands::clear_browserbase_profile,
            commands::open_browserbase_live_view,
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
            // Throwaway profiles that a crashed run left behind. Off the
            // startup path: it only frees disk space.
            tauri::async_runtime::spawn_blocking(adscan_browser::sweep_stale_profiles);
            let browserbase = adscan_browser::Browserbase::new(config.browserbase.clone())
                .map_err(|error| error.to_string())?;
            let capture = Arc::new(ChromeCaptureSource::new(
                &config.data_dir,
                browserbase.clone(),
            ));
            // A live view is shown in the user's own browser, opened from
            // here: its address never passes through the webview.
            let open_url: OpenUrl = Arc::new(|address| {
                tauri_plugin_opener::open_url(address, None::<&str>)
                    .map_err(|error| error.to_string())
            });
            let browserbase = Arc::new(BrowserbaseAdapter::new(browserbase, open_url));
            let state = tauri::async_runtime::block_on(AppState::build(
                config,
                capture,
                browserbase,
                events,
            ))?;
            app.manage(state);
            #[cfg(unix)]
            exit_on_termination_signals(app.handle().clone());
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

/// Ctrl+C in a terminal and `kill` would end the process on the spot. Going
/// through the event loop's exit instead cancels the running scans, so they
/// are recorded as cancelled and their browsers are closed in the usual way.
/// A second signal ends the process at once, should that exit ever hang.
#[cfg(unix)]
fn exit_on_termination_signals<R: Runtime>(app: tauri::AppHandle<R>) {
    use tokio::signal::unix::{SignalKind, signal};

    tauri::async_runtime::spawn(async move {
        let (Ok(mut interrupt), Ok(mut terminate)) = (
            signal(SignalKind::interrupt()),
            signal(SignalKind::terminate()),
        ) else {
            return;
        };
        let mut signalled = async || {
            tokio::select! {
                _ = interrupt.recv() => {}
                _ = terminate.recv() => {}
            }
        };
        signalled().await;
        app.exit(0);
        signalled().await;
        std::process::exit(1);
    });
}
