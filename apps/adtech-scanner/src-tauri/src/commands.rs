//! Tauri commands: the frontend's contract, mapped one to one onto use cases.

use adscan_engine::application::AppError;
use adscan_engine::application::archive::source::OriginalPosition;
use adscan_model::{
    ApiKeyStatus, ApiProvider, ArchiveStorage, BodySide, BrowserInfo, CommandError, DashboardStats,
    ErrorCode, ExportFormat, LogEntry, LogSearchHit, NetworkBodyChunk, NetworkRequestDetail,
    NetworkRowChunk, NetworkSummary, ProxyProfileDraft, ScanConfiguration, ScanConfigurationDraft,
    ScanDetail, ScanList, ScanListQuery, ScanOptions, ScanProfileInfo, ScanSummary, ScriptSource,
    Settings, SettingsPatch, SourceView,
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

/// The scan's requests, ordered by page view and start time, a chunk at a
/// time. Filtering and sorting are the frontend's.
#[tauri::command]
pub async fn list_network_requests(
    state: State<'_, AppState>,
    scan_id: String,
    pages: Option<Vec<u32>>,
    cursor: Option<String>,
    limit: Option<u32>,
) -> CommandResult<NetworkRowChunk> {
    state
        .network
        .list(
            &scan_id,
            &pages.unwrap_or_default(),
            cursor.as_deref(),
            limit,
        )
        .await
        .map_err(command_error)
}

#[tauri::command]
pub async fn get_network_summary(
    state: State<'_, AppState>,
    scan_id: String,
) -> CommandResult<NetworkSummary> {
    state.network.summary(&scan_id).await.map_err(command_error)
}

#[tauri::command]
pub async fn get_network_request(
    state: State<'_, AppState>,
    scan_id: String,
    request_id: String,
) -> CommandResult<NetworkRequestDetail> {
    state
        .network
        .request(&scan_id, &request_id)
        .await
        .map_err(command_error)
}

#[tauri::command]
pub async fn get_network_body(
    state: State<'_, AppState>,
    scan_id: String,
    request_id: String,
    side: BodySide,
    offset: Option<f64>,
    max_bytes: Option<f64>,
) -> CommandResult<NetworkBodyChunk> {
    state
        .network
        .body(&scan_id, &request_id, side, offset, max_bytes)
        .await
        .map_err(command_error)
}

/// `original_line` and `original_column` name a place in the source as
/// captured; the reply says which line of the returned text it is on.
#[tauri::command]
pub async fn get_script_source(
    state: State<'_, AppState>,
    scan_id: String,
    script_id: String,
    view: SourceView,
    original_line: Option<u32>,
    original_column: Option<u32>,
) -> CommandResult<ScriptSource> {
    let position = original_line.map(|line| OriginalPosition {
        line,
        column: original_column.unwrap_or(1),
    });
    state
        .sources
        .get(&scan_id, &script_id, view, position)
        .await
        .map_err(command_error)
}

#[tauri::command]
pub async fn get_archive_storage(state: State<'_, AppState>) -> CommandResult<ArchiveStorage> {
    state
        .settings
        .archive_storage()
        .await
        .map_err(command_error)
}

#[tauri::command]
pub async fn clear_archive(state: State<'_, AppState>) -> CommandResult<ArchiveStorage> {
    state.sources.forget_formatted();
    state.settings.clear_archive().await.map_err(command_error)
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

/// Starts a scan from `options`, or from the saved configuration named by
/// `configuration_id`: exactly one of the two.
#[tauri::command]
pub async fn start_scan(
    state: State<'_, AppState>,
    options: Option<ScanOptions>,
    configuration_id: Option<String>,
) -> CommandResult<ScanSummary> {
    match (options, configuration_id) {
        (Some(options), None) => state.scans.start(options).await,
        (None, Some(configuration_id)) => state.scans.start_configuration(&configuration_id).await,
        (Some(_), Some(_)) | (None, None) => Err(AppError::InvalidInput(
            "Start a scan with either options or a saved configuration, not both.".into(),
        )),
    }
    .map_err(command_error)
}

/// Ends a manual visit and lets the scan go on with what was recorded.
#[tauri::command]
pub async fn finish_capture(state: State<'_, AppState>, scan_id: String) -> CommandResult<()> {
    state
        .scans
        .finish_capture(&scan_id)
        .await
        .map_err(command_error)
}

#[tauri::command]
pub async fn get_scan_profile(state: State<'_, AppState>) -> CommandResult<ScanProfileInfo> {
    Ok(state.scans.scan_profile().await)
}

#[tauri::command]
pub async fn clear_scan_profile(state: State<'_, AppState>) -> CommandResult<ScanProfileInfo> {
    state
        .scans
        .clear_scan_profile()
        .await
        .map_err(command_error)
}

#[tauri::command]
pub async fn save_proxy_profile(
    state: State<'_, AppState>,
    profile: ProxyProfileDraft,
) -> CommandResult<Settings> {
    state
        .settings
        .save_proxy_profile(profile)
        .await
        .map_err(command_error)
}

#[tauri::command]
pub async fn delete_proxy_profile(
    state: State<'_, AppState>,
    id: String,
) -> CommandResult<Settings> {
    state
        .settings
        .delete_proxy_profile(&id)
        .await
        .map_err(command_error)
}

#[tauri::command]
pub async fn set_proxy_password(
    state: State<'_, AppState>,
    id: String,
    password: String,
) -> CommandResult<Settings> {
    state
        .settings
        .set_proxy_password(&id, &password)
        .await
        .map_err(command_error)
}

#[tauri::command]
pub async fn clear_proxy_password(
    state: State<'_, AppState>,
    id: String,
) -> CommandResult<Settings> {
    state
        .settings
        .clear_proxy_password(&id)
        .await
        .map_err(command_error)
}

#[tauri::command]
pub async fn list_scan_configurations(
    state: State<'_, AppState>,
) -> CommandResult<Vec<ScanConfiguration>> {
    state.configurations.list().await.map_err(command_error)
}

#[tauri::command]
pub async fn save_scan_configuration(
    state: State<'_, AppState>,
    configuration: ScanConfigurationDraft,
) -> CommandResult<ScanConfiguration> {
    state
        .configurations
        .save(configuration)
        .await
        .map_err(command_error)
}

#[tauri::command]
pub async fn delete_scan_configuration(
    state: State<'_, AppState>,
    id: String,
) -> CommandResult<()> {
    state
        .configurations
        .delete(&id)
        .await
        .map_err(command_error)
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
    state.sources.forget_formatted();
    state.scans.delete(&scan_id).await.map_err(command_error)
}

#[tauri::command]
pub async fn delete_all_scans(state: State<'_, AppState>) -> CommandResult<()> {
    state.sources.forget_formatted();
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
