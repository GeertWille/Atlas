//! The command layer through Tauri's IPC, on the mock runtime: every command
//! of the frontend contract is registered, takes camelCase arguments and
//! returns contract types or a `CommandError`.

use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use adscan_engine::application::ports::{CaptureError, CaptureSource, ProgressFn};
use adscan_model::{
    ApiKeyStatus, BrowserInfo, Capture, CaptureRequest, CommandError, DashboardStats, ErrorCode,
    LogEntry, LogSearchHit, SCAN_EVENT, ScanDetail, ScanEvent, ScanList, ScanStatus, ScanSummary,
    Settings,
};
use adtech_scanner_lib::state::{AppConfig, AppState};
use adtech_scanner_lib::{TauriEventSink, register_commands};
use async_trait::async_trait;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tauri::ipc::{CallbackFn, InvokeBody};
use tauri::test::{
    INVOKE_KEY, MockRuntime, get_ipc_response, mock_builder, mock_context, noop_assets,
};
use tauri::webview::InvokeRequest;
use tauri::{Listener, Manager, WebviewWindow, WebviewWindowBuilder};
use tauri_plugin_fs::FsExt;
use tokio_util::sync::CancellationToken;

/// The commands of the contract table in the build brief.
const CONTRACT_COMMANDS: [&str; 16] = [
    "get_settings",
    "update_settings",
    "set_api_key",
    "clear_api_key",
    "detect_browser",
    "get_demo_url",
    "start_scan",
    "cancel_scan",
    "list_scans",
    "get_scan",
    "delete_scan",
    "delete_all_scans",
    "get_logs",
    "search_logs",
    "get_dashboard_stats",
    "export_scan",
];

/// A machine without Chrome: every scan fails at the first pipeline node.
struct NoBrowser;

#[async_trait]
impl CaptureSource for NoBrowser {
    async fn locate_browser(&self, _explicit: Option<String>) -> Option<BrowserInfo> {
        None
    }

    async fn capture(
        &self,
        _request: CaptureRequest,
        _cancel: CancellationToken,
        _on_progress: ProgressFn,
    ) -> Result<Capture, CaptureError> {
        Err(CaptureError::BrowserNotFound)
    }
}

struct TestApp {
    window: WebviewWindow<MockRuntime>,
    events: Arc<Mutex<Vec<ScanEvent>>>,
    _data_dir: tempfile::TempDir,
}

fn test_app() -> TestApp {
    let data_dir = tempfile::tempdir().unwrap();
    let app = register_commands(mock_builder())
        .build(mock_context(noop_assets()))
        .unwrap();
    let events = Arc::new(Mutex::new(Vec::new()));
    let received = Arc::clone(&events);
    app.listen(SCAN_EVENT, move |event| {
        received
            .lock()
            .unwrap()
            .push(serde_json::from_str(event.payload()).unwrap());
    });
    let state = tauri::async_runtime::block_on(AppState::build(
        AppConfig {
            data_dir: data_dir.path().to_path_buf(),
            // Nothing listens here: key validation fails fast and offline.
            anthropic_base_url: "http://127.0.0.1:1".into(),
            api_key_override: Some("sk-ant-test".into()),
        },
        Arc::new(NoBrowser),
        Arc::new(TauriEventSink {
            app: app.handle().clone(),
        }),
    ))
    .unwrap();
    app.manage(state);
    let window = WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .unwrap();
    TestApp {
        window,
        events,
        _data_dir: data_dir,
    }
}

fn invoke_raw(app: &TestApp, command: &str, args: Value) -> Result<Value, Value> {
    get_ipc_response(
        &app.window,
        InvokeRequest {
            cmd: command.into(),
            callback: CallbackFn(0),
            error: CallbackFn(1),
            url: "tauri://localhost".parse().unwrap(),
            body: InvokeBody::Json(args),
            headers: Default::default(),
            invoke_key: INVOKE_KEY.to_string(),
        },
    )
    .map(|body| body.deserialize::<Value>().unwrap())
}

fn invoke<T: DeserializeOwned>(app: &TestApp, command: &str, args: Value) -> T {
    let value =
        invoke_raw(app, command, args).unwrap_or_else(|error| panic!("{command} failed: {error}"));
    serde_json::from_value(value)
        .unwrap_or_else(|error| panic!("{command} returned an unexpected shape: {error}"))
}

fn invoke_error(app: &TestApp, command: &str, args: Value) -> CommandError {
    let error = invoke_raw(app, command, args).expect_err("expected the command to fail");
    serde_json::from_value(error.clone())
        .unwrap_or_else(|_| panic!("{command} did not return a CommandError: {error}"))
}

/// What the dialog plugin does when the user confirms a save dialog.
fn pick_in_save_dialog(app: &TestApp, path: &std::path::Path) {
    app.window.app_handle().fs_scope().allow_file(path).unwrap();
}

fn scan_options(url: &str) -> Value {
    json!({ "options": {
        "url": url, "device": "desktop", "observeSeconds": 10,
        "autoScroll": true, "acceptConsent": true, "aiEnabled": false
    } })
}

fn wait_for_end(app: &TestApp, scan_id: &str) -> ScanDetail {
    for _ in 0..500 {
        let detail: ScanDetail = invoke(app, "get_scan", json!({ "scanId": scan_id }));
        if !matches!(
            detail.summary.status,
            ScanStatus::Queued | ScanStatus::Running
        ) {
            return detail;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("scan did not finish");
}

#[test]
fn every_contract_command_is_registered() {
    let app = test_app();
    for command in CONTRACT_COMMANDS {
        // Without arguments a registered command either succeeds or complains
        // about a missing argument; only an unregistered one is "not found".
        if let Err(error) = invoke_raw(&app, command, json!({})) {
            let message = error.to_string();
            assert!(
                !message.contains("not found"),
                "{command} is not registered: {message}"
            );
        }
    }
    let unknown = invoke_raw(&app, "no_such_command", json!({})).unwrap_err();
    assert!(unknown.to_string().contains("not found"), "{unknown}");
}

#[test]
fn settings_commands_round_trip_contract_types() {
    let app = test_app();
    let settings: Settings = invoke(&app, "get_settings", json!({}));
    assert_eq!(settings.analysis_model, "claude-sonnet-5-5");
    assert!(
        settings.has_api_key,
        "the ADSCAN_API_KEY override counts as a configured key"
    );

    let updated: Settings = invoke(
        &app,
        "update_settings",
        json!({ "patch": { "budgetUsd": 0.75, "defaultDevice": "mobile", "chromePath": null } }),
    );
    assert_eq!(updated.budget_usd, 0.75);
    assert_eq!(
        invoke::<Settings>(&app, "get_settings", json!({})).budget_usd,
        0.75
    );

    let error = invoke_error(
        &app,
        "update_settings",
        json!({ "patch": { "budgetUsd": -5 } }),
    );
    assert_eq!(error.code, ErrorCode::InvalidInput);
    assert_eq!(
        error.message,
        "The per-scan budget must be between $0 and $50"
    );

    let status: ApiKeyStatus = invoke(&app, "set_api_key", json!({ "key": "sk-ant-other" }));
    assert!(!status.valid, "an unverifiable key is reported, not stored");
    assert_eq!(
        invoke::<Value>(&app, "clear_api_key", json!({})),
        Value::Null
    );
    assert!(!invoke::<Settings>(&app, "get_settings", json!({})).has_api_key);
}

#[test]
fn environment_commands_report_the_browser_and_the_demo_site() {
    let app = test_app();
    assert_eq!(
        invoke::<Option<BrowserInfo>>(&app, "detect_browser", json!({})),
        None
    );
    let demo_url: String = invoke(&app, "get_demo_url", json!({}));
    assert!(demo_url.starts_with("http://127.0.0.1:"), "{demo_url}");
}

#[test]
fn scan_commands_cover_the_lifecycle_and_emit_scan_events() {
    let app = test_app();
    let invalid = invoke_error(&app, "start_scan", scan_options("ftp://example.com"));
    assert_eq!(
        (invalid.code, invalid.message.as_str()),
        (
            ErrorCode::InvalidInput,
            "Only http and https pages can be scanned"
        )
    );

    let started: ScanSummary = invoke(&app, "start_scan", scan_options("example.com"));
    assert_eq!(
        (started.status, started.url.as_str()),
        (ScanStatus::Running, "https://example.com/")
    );
    let detail = wait_for_end(&app, &started.id);
    assert_eq!(detail.summary.status, ScanStatus::Failed);
    assert!(
        detail
            .summary
            .error
            .unwrap()
            .starts_with("No Chrome or Chromium installation was found")
    );

    let scan_id = json!({ "scanId": started.id });
    assert_eq!(
        invoke::<Value>(&app, "cancel_scan", scan_id.clone()),
        Value::Null
    );
    let list: ScanList = invoke(
        &app,
        "list_scans",
        json!({ "query": { "search": "example", "status": "failed", "limit": 10 } }),
    );
    assert_eq!(
        (list.total, list.items[0].id.as_str()),
        (1, started.id.as_str())
    );
    assert_eq!(
        invoke::<ScanList>(
            &app,
            "list_scans",
            json!({ "query": { "device": "mobile" } })
        )
        .total,
        0
    );

    let logs: Vec<LogEntry> = invoke(&app, "get_logs", scan_id.clone());
    assert!(!logs.is_empty());
    let step_logs: Vec<LogEntry> = invoke(
        &app,
        "get_logs",
        json!({ "scanId": started.id, "stepId": logs[0].step_id }),
    );
    assert_eq!(step_logs.len(), logs.len());
    let hits: Vec<LogSearchHit> = invoke(
        &app,
        "search_logs",
        json!({ "query": "Chromium install", "limit": 5 }),
    );
    assert_eq!(hits.len(), 1);
    assert!(hits[0].snippet.contains("<mark>Chromium</mark>"));
    assert_eq!(
        invoke::<Vec<LogSearchHit>>(&app, "search_logs", json!({ "query": "\"(" })).len(),
        0
    );

    let stats: DashboardStats = invoke(&app, "get_dashboard_stats", json!({}));
    assert_eq!((stats.total_scans, stats.sites_scanned), (1, 1));

    // Events reached the frontend channel: first the running scan, last its final state.
    let (tx, rx) = mpsc::channel();
    app.window
        .app_handle()
        .listen("flush", move |_| tx.send(()).unwrap());
    tauri::Emitter::emit(app.window.app_handle(), "flush", ()).unwrap();
    rx.recv_timeout(Duration::from_secs(2)).unwrap();
    let events = app.events.lock().unwrap().clone();
    assert!(
        matches!(&events[0], ScanEvent::ScanUpdated { summary } if summary.status == ScanStatus::Running)
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event, ScanEvent::StepUpdated { .. }))
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event, ScanEvent::Log { .. }))
    );
    assert!(
        matches!(events.last().unwrap(), ScanEvent::ScanUpdated { summary } if summary.status == ScanStatus::Failed)
    );

    let missing = invoke_error(&app, "get_scan", json!({ "scanId": "nope" }));
    assert_eq!(
        (missing.code, missing.message.as_str()),
        (ErrorCode::NotFound, "Scan nope was not found")
    );
    assert_eq!(
        invoke_error(&app, "get_logs", json!({ "scanId": "nope" })).code,
        ErrorCode::NotFound
    );

    assert_eq!(
        invoke::<Value>(&app, "delete_scan", scan_id.clone()),
        Value::Null
    );
    assert_eq!(
        invoke_error(&app, "delete_scan", scan_id).code,
        ErrorCode::NotFound
    );
    invoke::<ScanSummary>(&app, "start_scan", scan_options("example.org"));
    assert_eq!(
        invoke::<Value>(&app, "delete_all_scans", json!({})),
        Value::Null
    );
    assert_eq!(
        invoke::<ScanList>(&app, "list_scans", json!({ "query": {} })).total,
        0
    );
}

#[test]
fn export_returns_the_document_and_writes_it_when_a_path_is_given() {
    let app = test_app();
    let started: ScanSummary = invoke(&app, "start_scan", scan_options("example.com"));
    wait_for_end(&app, &started.id);
    let target = app._data_dir.path().join("report.md");
    pick_in_save_dialog(&app, &target);

    let markdown: String = invoke(
        &app,
        "export_scan",
        json!({ "scanId": started.id, "format": "markdown", "path": target.to_str().unwrap() }),
    );
    assert!(markdown.starts_with("# Ad stack scan: example.com"));
    assert_eq!(std::fs::read_to_string(&target).unwrap(), markdown);

    let json_document: String = invoke(
        &app,
        "export_scan",
        json!({ "scanId": started.id, "format": "json" }),
    );
    let detail: ScanDetail = serde_json::from_str(&json_document).unwrap();
    assert_eq!(detail.summary.id, started.id);

    pick_in_save_dialog(&app, std::path::Path::new("/nonexistent-dir/report.json"));
    let unwritable = invoke_error(
        &app,
        "export_scan",
        json!({ "scanId": started.id, "format": "json", "path": "/nonexistent-dir/report.json" }),
    );
    assert_eq!(unwritable.code, ErrorCode::Internal);
    assert!(
        unwritable
            .message
            .starts_with("Could not write /nonexistent-dir/report.json")
    );
    assert_eq!(
        invoke_error(
            &app,
            "export_scan",
            json!({ "scanId": "nope", "format": "json" })
        )
        .code,
        ErrorCode::NotFound
    );
}

#[test]
fn export_only_writes_where_the_user_picked_and_only_export_files() {
    let app = test_app();
    let started: ScanSummary = invoke(&app, "start_scan", scan_options("example.com"));
    wait_for_end(&app, &started.id);
    let export_to = |format: &str, path: &std::path::Path| {
        invoke_error(
            &app,
            "export_scan",
            json!({ "scanId": started.id, "format": format, "path": path.to_str().unwrap() }),
        )
    };

    // A path the webview made up, never confirmed in a dialog.
    let never_picked = app._data_dir.path().join("planted.md");
    let refused = export_to("markdown", &never_picked);
    assert_eq!(
        (refused.code, refused.message.as_str()),
        (
            ErrorCode::InvalidInput,
            "Choose where to save the export in the save dialog first"
        )
    );
    assert!(!never_picked.exists());

    // A file picked in a dialog for another purpose keeps its content.
    let browser = app._data_dir.path().join("chrome");
    std::fs::write(&browser, "binary").unwrap();
    pick_in_save_dialog(&app, &browser);
    let profile = app._data_dir.path().join(".zshrc");
    pick_in_save_dialog(&app, &profile);
    for (format, target, expected) in [
        (
            "json",
            &browser,
            "Save the export with the file extension .json",
        ),
        (
            "markdown",
            &profile,
            "Save the export with the file extension .md",
        ),
        (
            "markdown",
            &browser,
            "Save the export with the file extension .md",
        ),
    ] {
        let refused = export_to(format, target);
        assert_eq!(
            (refused.code, refused.message.as_str()),
            (ErrorCode::InvalidInput, expected)
        );
    }
    assert_eq!(std::fs::read_to_string(&browser).unwrap(), "binary");
    assert!(!profile.exists());

    // A picked path that is not absolute cannot be redirected by the working directory.
    pick_in_save_dialog(&app, std::path::Path::new("report.json"));
    assert_eq!(
        export_to("json", std::path::Path::new("report.json")).code,
        ErrorCode::InvalidInput
    );

    let picked = app._data_dir.path().join("Report.JSON");
    pick_in_save_dialog(&app, &picked);
    let document: String = invoke(
        &app,
        "export_scan",
        json!({ "scanId": started.id, "format": "json", "path": picked.to_str().unwrap() }),
    );
    assert_eq!(std::fs::read_to_string(&picked).unwrap(), document);
}
