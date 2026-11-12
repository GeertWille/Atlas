//! Tauri commands: the frontend's contract, mapped one to one onto use cases.

use adscan_engine::application::AppError;
use adscan_model::{
    ApiKeyStatus, ApiProvider, BrowserInfo, CommandError, DashboardStats, ErrorCode, ExportFormat,
    LogEntry, LogSearchHit, ScanDetail, ScanList, ScanListQuery, ScanOptions, ScanSummary,
    Settings, SettingsPatch,
};
use std::path::Path;

use tauri::{AppHandle, Runtime, State};
use tauri_plugin_fs::FsExt;

use crate::state::AppState;

type CommandResult<T> = Result<T, CommandError>;

fn command_error(error: AppError) -> CommandError {
    CommandError {
        code: error.code(),
        message: error.to_string(),
    }
}

#[tauri::command]
pub async fn get_settings(state: State<'_, AppState>) -> CommandResult<Settings> {
    state.settings.get().await.map_err(command_error)
}

#[tauri::command]
pub async fn update_settings(
    state: State<'_, AppState>,
    patch: SettingsPatch,
) -> CommandResult<Settings> {
    state.settings.update(patch).await.map_err(command_error)
}

/// `provider` defaults to Anthropic, the only provider older callers know.
#[tauri::command]
pub async fn set_api_key(
    state: State<'_, AppState>,
    key: String,
    provider: Option<ApiProvider>,
) -> CommandResult<ApiKeyStatus> {
    state
        .settings
        .set_api_key(provider.unwrap_or(ApiProvider::Anthropic), &key)
        .await
        .map_err(command_error)
}

#[tauri::command]
pub async fn clear_api_key(
    state: State<'_, AppState>,
    provider: Option<ApiProvider>,
) -> CommandResult<()> {
    state
        .settings
        .clear_api_key(provider.unwrap_or(ApiProvider::Anthropic))
        .await
        .map_err(command_error)
}

#[tauri::command]
pub async fn detect_browser(state: State<'_, AppState>) -> CommandResult<Option<BrowserInfo>> {
    state.scans.detect_browser().await.map_err(command_error)
}

#[tauri::command]
pub async fn get_demo_url(state: State<'_, AppState>) -> CommandResult<String> {
    Ok(state.demo_site.url.clone())
}

#[tauri::command]
pub async fn start_scan(
    state: State<'_, AppState>,
    options: ScanOptions,
) -> CommandResult<ScanSummary> {
    state.scans.start(options).await.map_err(command_error)
}

#[tauri::command]
pub async fn cancel_scan(state: State<'_, AppState>, scan_id: String) -> CommandResult<()> {
    state.scans.cancel(&scan_id).await.map_err(command_error)
}

#[tauri::command]
pub async fn list_scans(
    state: State<'_, AppState>,
    query: ScanListQuery,
) -> CommandResult<ScanList> {
    state.scans.list(query).await.map_err(command_error)
}

#[tauri::command]
pub async fn get_scan(state: State<'_, AppState>, scan_id: String) -> CommandResult<ScanDetail> {
    state.scans.get(&scan_id).await.map_err(command_error)
}

#[tauri::command]
pub async fn delete_scan(state: State<'_, AppState>, scan_id: String) -> CommandResult<()> {
    state.scans.delete(&scan_id).await.map_err(command_error)
}

#[tauri::command]
pub async fn delete_all_scans(state: State<'_, AppState>) -> CommandResult<()> {
    state.scans.delete_all().await.map_err(command_error)
}

#[tauri::command]
pub async fn get_logs(
    state: State<'_, AppState>,
    scan_id: String,
    step_id: Option<String>,
) -> CommandResult<Vec<LogEntry>> {
    state
        .scans
        .logs(&scan_id, step_id.as_deref())
        .await
        .map_err(command_error)
}

#[tauri::command]
pub async fn search_logs(
    state: State<'_, AppState>,
    query: String,
    limit: Option<u32>,
) -> CommandResult<Vec<LogSearchHit>> {
    state
        .scans
        .search_logs(&query, limit)
        .await
        .map_err(command_error)
}

#[tauri::command]
pub async fn get_dashboard_stats(state: State<'_, AppState>) -> CommandResult<DashboardStats> {
    state.scans.dashboard_stats().await.map_err(command_error)
}

/// Returns the document and, when `path` is given, also writes it there.
#[tauri::command]
pub async fn export_scan<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    scan_id: String,
    format: ExportFormat,
    path: Option<String>,
) -> CommandResult<String> {
    if let Some(path) = &path {
        check_export_target(&app, Path::new(path), format)?;
    }
    let document = state
        .scans
        .export(&scan_id, format)
        .await
        .map_err(command_error)?;
    if let Some(path) = path {
        tokio::fs::write(&path, &document)
            .await
            .map_err(|error| CommandError {
                code: ErrorCode::Internal,
                message: format!("Could not write {path}: {error}"),
            })?;
    }
    Ok(document)
}

/// `path` comes from the webview and the document contains text taken from
/// the scanned site, so an unchecked write would let script running in the
/// webview plant chosen lines in any file the user can write (a shell profile,
/// a launch agent). The backend cannot see the save dialog, but the dialog
/// plugin records every path the user picks in the fs plugin's scope, which
/// the webview cannot extend. So the target must be such a path, and it must
/// have the export's extension, which rules out paths picked for another
/// purpose (the Chrome binary).
fn check_export_target<R: Runtime>(
    app: &AppHandle<R>,
    path: &Path,
    format: ExportFormat,
) -> CommandResult<()> {
    let refuse = |message: &str| {
        Err(CommandError {
            code: ErrorCode::InvalidInput,
            message: message.to_string(),
        })
    };
    let extensions: &[&str] = match format {
        ExportFormat::Json => &["json"],
        ExportFormat::Markdown => &["md", "markdown"],
    };
    let has_export_extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extensions
                .iter()
                .any(|allowed| extension.eq_ignore_ascii_case(allowed))
        });
    if !has_export_extension {
        return refuse(&format!(
            "Save the export with the file extension .{}",
            extensions[0]
        ));
    }
    let picked_in_a_dialog = path.is_absolute()
        && app
            .try_fs_scope()
            .is_some_and(|scope| scope.is_allowed(path));
    if !picked_in_a_dialog {
        return refuse("Choose where to save the export in the save dialog first");
    }
    Ok(())
}
