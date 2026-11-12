//! The command layer through Tauri's IPC, on the mock runtime: every command
//! of the frontend contract is registered, takes camelCase arguments and
//! returns contract types or a `CommandError`.

use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use adscan_engine::application::ports::{CaptureError, CaptureSession, CaptureSource};
use adscan_model::{
    ApiKeyStatus, BrowserInfo, Capture, CaptureRequest, CommandError, DashboardStats, ErrorCode,
    LogEntry, LogSearchHit, PipelineNode, ProxyChoice, SCAN_EVENT, ScanConfiguration, ScanDetail,
    ScanEvent, ScanList, ScanProfileInfo, ScanStatus, ScanSummary, Settings, StepStatus,
    VisitorBehavior,
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

/// The commands of the frontend contract.
const CONTRACT_COMMANDS: [&str; 33] = [
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
    "finish_capture",
    "get_scan_profile",
    "clear_scan_profile",
    "save_proxy_profile",
    "delete_proxy_profile",
    "set_proxy_password",
    "clear_proxy_password",
    "list_scan_configurations",
    "save_scan_configuration",
    "delete_scan_configuration",
    "list_network_requests",
    "get_network_summary",
    "get_network_request",
    "get_network_body",
    "get_script_source",
    "get_archive_storage",
    "clear_archive",
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
        _session: CaptureSession,
    ) -> Result<Capture, CaptureError> {
        Err(CaptureError::BrowserNotFound)
    }

    async fn scan_profile_info(&self) -> ScanProfileInfo {
        no_scan_profile()
    }

    async fn clear_scan_profile(&self) -> Result<(), CaptureError> {
        Ok(())
    }
}

fn no_scan_profile() -> ScanProfileInfo {
    ScanProfileInfo {
        exists: false,
        size_bytes: 0.0,
        last_used_at: None,
        in_use: false,
    }
}

/// A browser a person drives: every capture records until it is finished,
/// and its scan profile exists until it is cleared.
#[derive(Default)]
struct ManualBrowser {
    requests: Mutex<Vec<CaptureRequest>>,
    profile_cleared: Mutex<bool>,
    /// A browser of another process holds the profile.
    profile_held: Mutex<bool>,
}

#[async_trait]
impl CaptureSource for ManualBrowser {
    async fn locate_browser(&self, _explicit: Option<String>) -> Option<BrowserInfo> {
        Some(BrowserInfo {
            path: "/usr/bin/chromium".into(),
            version: "Chromium 153.0.0.0".into(),
            name: "Chromium".into(),
        })
    }

    async fn capture(
        &self,
        request: CaptureRequest,
        session: CaptureSession,
    ) -> Result<Capture, CaptureError> {
        let url = request.url.clone();
        self.requests.lock().unwrap().push(request);
        tokio::select! {
            () = session.finish.cancelled() => Ok(Capture::single_page(url, Vec::new())),
            () = session.cancel.cancelled() => Err(CaptureError::Cancelled),
        }
    }

    async fn scan_profile_info(&self) -> ScanProfileInfo {
        ScanProfileInfo {
            exists: !*self.profile_cleared.lock().unwrap(),
            size_bytes: 2048.0,
            last_used_at: Some(1_760_000_000_000.0),
            in_use: *self.profile_held.lock().unwrap(),
        }
    }

    async fn clear_scan_profile(&self) -> Result<(), CaptureError> {
        if *self.profile_held.lock().unwrap() {
            return Err(CaptureError::ProfileBusy);
        }
        *self.profile_cleared.lock().unwrap() = true;
        Ok(())
    }
}

const RECORDED_SCRIPT: &str = "function gate(){if(window.matchMedia('(min-width: 768px)').matches){pbjs.requestBids({timeout:250})}}gate();";

/// A browser whose visit made three requests: the page's script, an image,
/// and a bid request that the exchange declined.
struct RecordedBrowser;

#[async_trait]
impl CaptureSource for RecordedBrowser {
    async fn locate_browser(&self, _explicit: Option<String>) -> Option<BrowserInfo> {
        Some(BrowserInfo {
            path: "/usr/bin/chromium".into(),
            version: "Chromium 153.0.0.0".into(),
            name: "Chromium".into(),
        })
    }

    async fn capture(
        &self,
        request: CaptureRequest,
        session: CaptureSession,
    ) -> Result<Capture, CaptureError> {
        use adscan_model::{
            BodyAdmission, BodyOffer, CapturedBody, CapturedPostData, CapturedScript,
            InitiatorKind, NetworkInitiator, NetworkRecord, RawHeader, RequestContext, RequestEnd,
            ResourceKind, ServedFrom,
        };
        let sink = session.network.expect("the engine always records");
        let record = |seq: u32, url: &str, kind: ResourceKind, mime: &str, body: &str| {
            let admitted = sink.admit_body(&BodyOffer {
                seq,
                url,
                bidder: None,
                kind,
                mime_type: Some(mime),
                status: Some(200),
                expected_bytes: body.len() as u64,
            });
            NetworkRecord {
                seq,
                page: 0,
                context: RequestContext::Page,
                frame_id: None,
                target_id: None,
                method: "GET".into(),
                url: url.into(),
                kind,
                priority: None,
                request_headers: vec![RawHeader {
                    name: "Cookie".into(),
                    value: "sid=command-test-secret".into(),
                }],
                post_data: None,
                post_data_missing: false,
                initiator: NetworkInitiator {
                    kind: InitiatorKind::Parser,
                    url: None,
                    line: None,
                    column: None,
                    stack: Vec::new(),
                },
                redirect_from: None,
                preflight_of: None,
                status: Some(200),
                mime_type: Some(mime.into()),
                protocol: Some("h2".into()),
                remote_address: None,
                response_headers: Vec::new(),
                served_from: ServedFrom::Network,
                started_at_ms: 50.0 * f64::from(seq),
                response_at_ms: Some(50.0 * f64::from(seq) + 20.0),
                ended_at_ms: Some(50.0 * f64::from(seq) + 30.0),
                queued_ms: None,
                timing: None,
                encoded_bytes: 500,
                decoded_bytes: body.len() as u64,
                end: RequestEnd::Finished,
                bidder: None,
                body: match admitted {
                    BodyAdmission::Read { .. } => CapturedBody::Read {
                        bytes: body.as_bytes().to_vec(),
                        total_bytes: body.len() as u64,
                    },
                    BodyAdmission::Skip(why) => CapturedBody::Skipped(why),
                },
            }
        };
        let script_url = format!("{}js/gate.js", request.url);
        sink.record(record(
            0,
            &script_url,
            ResourceKind::Script,
            "text/javascript",
            RECORDED_SCRIPT,
        ));
        sink.record(record(
            1,
            &format!("{}logo.png", request.url),
            ResourceKind::Image,
            "image/png",
            "PNG",
        ));
        let mut bid = record(
            2,
            "https://rtb.openx.net/openrtbb/prebidjs",
            ResourceKind::Fetch,
            "application/json",
            r#"{"id":"1","nbr":7}"#,
        );
        bid.method = "POST".into();
        bid.bidder = Some("openx".into());
        bid.post_data = Some(CapturedPostData {
            bytes: br#"{"id":"1","imp":[{"id":"a"}]}"#.to_vec(),
            total_bytes: 29,
        });
        sink.record(bid);
        let mut capture = Capture::single_page(request.url, Vec::new());
        capture.scripts.push(CapturedScript {
            url: script_url,
            inline: false,
            body: RECORDED_SCRIPT.into(),
            loaded_at_ms: Some(30.0),
            initiator: None,
            pages: vec![0],
            request_seq: Some(0),
        });
        Ok(capture)
    }

    async fn scan_profile_info(&self) -> ScanProfileInfo {
        no_scan_profile()
    }

    async fn clear_scan_profile(&self) -> Result<(), CaptureError> {
        Ok(())
    }
}

struct TestApp {
    window: WebviewWindow<MockRuntime>,
    events: Arc<Mutex<Vec<ScanEvent>>>,
    _data_dir: tempfile::TempDir,
}

fn test_app() -> TestApp {
    test_app_with_typesafe_key(None)
}

fn test_app_with_typesafe_key(typesafe_key: Option<&str>) -> TestApp {
    build_test_app(Arc::new(NoBrowser), typesafe_key, None)
}

fn test_app_with_browser(browser: Arc<dyn CaptureSource>) -> TestApp {
    build_test_app(browser, None, None)
}

fn build_test_app(
    browser: Arc<dyn CaptureSource>,
    typesafe_key: Option<&str>,
    proxy_password: Option<&str>,
) -> TestApp {
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
            typesafe_base_url: "http://127.0.0.1:1".into(),
            api_key_override: Some("sk-ant-test".into()),
            typesafe_api_key_override: typesafe_key.map(str::to_string),
            proxy_password_override: proxy_password.map(str::to_string),
        },
        browser,
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
fn jev_cannot_be_turned_on_without_a_typesafe_key() {
    let app = test_app();
    let settings: Settings = invoke(&app, "get_settings", json!({}));
    assert!(!settings.has_typesafe_key && !settings.jev_enabled);

    let error = invoke_error(
        &app,
        "update_settings",
        json!({ "patch": { "jevEnabled": true } }),
    );
    assert_eq!(error.code, ErrorCode::InvalidInput);
    assert_eq!(
        error.message,
        "Add a TypeSafe API key before turning on Jev."
    );
    assert!(!invoke::<Settings>(&app, "get_settings", json!({})).jev_enabled);

    let status: ApiKeyStatus = invoke(
        &app,
        "set_api_key",
        json!({ "key": "typesafe-key", "provider": "typesafe" }),
    );
    assert!(!status.valid, "an unverifiable key is reported, not stored");
    let settings: Settings = invoke(&app, "get_settings", json!({}));
    assert!(!settings.has_typesafe_key);
    assert!(settings.has_api_key, "the Anthropic key is untouched");
}

#[test]
fn key_commands_address_one_provider_and_clearing_the_typesafe_key_turns_jev_off() {
    let app = test_app_with_typesafe_key(Some("typesafe-test-key"));
    let settings: Settings = invoke(&app, "get_settings", json!({}));
    assert!(settings.has_api_key && settings.has_typesafe_key);
    assert!(!settings.jev_enabled, "a stored key does not turn Jev on");

    let enabled: Settings = invoke(
        &app,
        "update_settings",
        json!({ "patch": { "jevEnabled": true } }),
    );
    assert!(enabled.jev_enabled);

    // Without `provider` the commands mean Anthropic, as they always did.
    assert_eq!(
        invoke::<Value>(&app, "clear_api_key", json!({})),
        Value::Null
    );
    let settings: Settings = invoke(&app, "get_settings", json!({}));
    assert!(!settings.has_api_key);
    assert!(settings.has_typesafe_key && settings.jev_enabled);

    assert_eq!(
        invoke::<Value>(&app, "clear_api_key", json!({ "provider": "typesafe" })),
        Value::Null
    );
    let settings: Settings = invoke(&app, "get_settings", json!({}));
    assert!(!settings.has_typesafe_key);
    assert!(!settings.jev_enabled, "Jev is off once its key is gone");

    let unknown =
        invoke_raw(&app, "clear_api_key", json!({ "provider": "someone-else" })).unwrap_err();
    assert!(unknown.to_string().contains("provider"), "{unknown}");
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

// -- visitors, the scan profile, proxies and saved configurations -----------------

fn belgium() -> Value {
    json!({ "profile": {
        "name": "Belgium", "scheme": "http", "host": "be.proxy.example", "port": 3128, "username": "scanner"
    } })
}

fn manual_options(url: &str) -> Value {
    json!({ "options": {
        "url": url, "device": "desktop", "observeSeconds": 10,
        "autoScroll": false, "acceptConsent": true, "aiEnabled": false,
        "visitor": "manual", "extraPages": 3, "seed": 42, "headless": true,
        "persistentProfile": false, "proxy": { "kind": "none" }, "budgetUsd": 0.1
    } })
}

fn wait_for_capture(browser: &ManualBrowser, count: usize) {
    for _ in 0..500 {
        if browser.requests.lock().unwrap().len() >= count {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("capture {count} did not start");
}

#[test]
fn a_manual_visit_is_started_with_the_new_options_and_ended_by_finish_capture() {
    let browser = Arc::new(ManualBrowser::default());
    let app = test_app_with_browser(browser.clone());

    let started: ScanSummary = invoke(&app, "start_scan", manual_options("example.com"));
    assert_eq!(started.status, ScanStatus::Running);
    assert_eq!(started.configuration, None);
    wait_for_capture(&browser, 1);
    {
        let request = &browser.requests.lock().unwrap()[0];
        assert_eq!(request.visitor, VisitorBehavior::Manual);
        assert_eq!((request.extra_pages, request.seed), (0, 42));
        assert!(!request.headless && !request.accept_consent);
    }
    let running: ScanDetail = invoke(&app, "get_scan", json!({ "scanId": started.id }));
    assert_eq!(running.summary.status, ScanStatus::Running);

    assert_eq!(
        invoke::<Value>(&app, "finish_capture", json!({ "scanId": started.id })),
        Value::Null
    );
    let detail = wait_for_end(&app, &started.id);
    assert_eq!(detail.summary.status, ScanStatus::Completed);
    assert_eq!(detail.pages.len(), 1);
    assert_eq!(detail.options.visitor, Some(VisitorBehavior::Manual));
    assert_eq!(detail.options.headless, Some(false));
    assert_eq!(detail.options.budget_usd, Some(0.1));
    assert_eq!(detail.options.proxy, Some(ProxyChoice::None));
    assert_eq!(
        detail
            .environment
            .as_ref()
            .map(|environment| environment.visitor),
        Some(VisitorBehavior::Manual)
    );
    let capture = detail
        .steps
        .iter()
        .find(|step| step.node == PipelineNode::Capture)
        .unwrap();
    assert_eq!(capture.status, StepStatus::Completed);

    // Finishing again changes nothing; an unknown scan is an error.
    assert_eq!(
        invoke::<Value>(&app, "finish_capture", json!({ "scanId": started.id })),
        Value::Null
    );
    let unknown = invoke_error(&app, "finish_capture", json!({ "scanId": "nope" }));
    assert_eq!(unknown.code, ErrorCode::NotFound);

    // The serialised scan uses the contract's names.
    let raw = invoke_raw(&app, "get_scan", json!({ "scanId": started.id })).unwrap();
    assert_eq!(raw["pages"][0]["reachedBy"], "entry");
    assert_eq!(raw["environment"]["proxy"]["kind"], "none");
    assert_eq!(raw["summary"]["proxy"]["kind"], "none");
    assert_eq!(raw["summary"]["configuration"], Value::Null);
    assert_eq!(raw["options"]["visitor"], "manual");
}

#[test]
fn invalid_visit_options_are_refused_before_a_scan_exists() {
    let app = test_app();
    let mut too_many = manual_options("example.com");
    too_many["options"]["visitor"] = json!("browse");
    too_many["options"]["extraPages"] = json!(6);
    let error = invoke_error(&app, "start_scan", too_many);
    assert_eq!(
        (error.code, error.message.as_str()),
        (
            ErrorCode::InvalidInput,
            "A browse visit can follow links to at most 5 further pages"
        )
    );
    let mut unknown_proxy = manual_options("example.com");
    unknown_proxy["options"]["proxy"] = json!({ "kind": "profile", "id": "gone" });
    let error = invoke_error(&app, "start_scan", unknown_proxy);
    assert_eq!(
        (error.code, error.message.as_str()),
        (
            ErrorCode::InvalidInput,
            "The chosen proxy no longer exists. Pick another proxy."
        )
    );
    let neither = invoke_error(&app, "start_scan", json!({}));
    assert_eq!(neither.code, ErrorCode::InvalidInput);
    let mut both = manual_options("example.com");
    both["configurationId"] = json!("c1");
    assert_eq!(
        invoke_error(&app, "start_scan", both).message,
        "Start a scan with either options or a saved configuration, not both."
    );
    assert_eq!(
        invoke::<ScanList>(&app, "list_scans", json!({ "query": {} })).total,
        0
    );
}

#[test]
fn scan_profile_commands_report_and_clear_the_shared_profile() {
    let browser = Arc::new(ManualBrowser::default());
    let app = test_app_with_browser(browser.clone());

    let info: ScanProfileInfo = invoke(&app, "get_scan_profile", json!({}));
    assert!(info.exists && !info.in_use);
    assert_eq!(info.size_bytes, 2048.0);
    let raw = invoke_raw(&app, "get_scan_profile", json!({})).unwrap();
    assert_eq!(
        raw,
        json!({ "exists": true, "sizeBytes": 2048.0, "lastUsedAt": 1_760_000_000_000.0, "inUse": false })
    );

    // A browser that holds the profile keeps it.
    *browser.profile_held.lock().unwrap() = true;
    let busy = invoke_error(&app, "clear_scan_profile", json!({}));
    assert_eq!(busy.code, ErrorCode::ProfileBusy);
    assert_eq!(serde_json::to_value(&busy).unwrap()["code"], "profileBusy");
    *browser.profile_held.lock().unwrap() = false;

    // So does a running scan of this app that uses it.
    let mut options = manual_options("example.com");
    options["options"]["persistentProfile"] = json!(true);
    let started: ScanSummary = invoke(&app, "start_scan", options);
    wait_for_capture(&browser, 1);
    assert!(browser.requests.lock().unwrap()[0].persistent_profile);
    assert_eq!(
        invoke_error(&app, "clear_scan_profile", json!({})).code,
        ErrorCode::ProfileBusy
    );
    invoke::<Value>(&app, "finish_capture", json!({ "scanId": started.id }));
    wait_for_end(&app, &started.id);

    let cleared: ScanProfileInfo = invoke(&app, "clear_scan_profile", json!({}));
    assert!(!cleared.exists);
    assert!(!invoke::<ScanProfileInfo>(&app, "get_scan_profile", json!({})).exists);
}

#[test]
fn visitor_settings_are_patched_through_update_settings() {
    let app = test_app();
    let settings: Settings = invoke(&app, "get_settings", json!({}));
    assert_eq!(settings.default_visitor, VisitorBehavior::Scroll);
    assert_eq!(settings.default_proxy, ProxyChoice::System);
    let raw = invoke_raw(&app, "get_settings", json!({})).unwrap();
    assert_eq!(raw["defaultVisitor"], "scroll");
    assert_eq!(raw["defaultExtraPages"], 2);
    assert_eq!(raw["persistentProfile"], false);
    assert_eq!(raw["proxies"], json!([]));
    assert_eq!(raw["defaultProxy"], json!({ "kind": "system" }));

    let updated: Settings = invoke(
        &app,
        "update_settings",
        json!({ "patch": {
            "defaultVisitor": "browse", "defaultExtraPages": 0, "persistentProfile": true,
            "defaultProxy": { "kind": "none" }
        } }),
    );
    assert_eq!(updated.default_visitor, VisitorBehavior::Browse);
    assert_eq!(updated.default_extra_pages, 0);
    assert!(updated.persistent_profile);
    assert_eq!(updated.default_proxy, ProxyChoice::None);

    let error = invoke_error(
        &app,
        "update_settings",
        json!({ "patch": { "defaultExtraPages": 9 } }),
    );
    assert_eq!(error.code, ErrorCode::InvalidInput);
}

#[test]
fn proxy_commands_manage_profiles_and_their_passwords() {
    let app = test_app();
    let saved: Settings = invoke(&app, "save_proxy_profile", belgium());
    assert_eq!(saved.proxies.len(), 1);
    let id = saved.proxies[0].id.clone();
    assert!(!saved.proxies[0].has_password);
    let raw = invoke_raw(&app, "get_settings", json!({})).unwrap();
    assert_eq!(
        raw["proxies"][0],
        json!({
            "id": id, "name": "Belgium", "scheme": "http", "host": "be.proxy.example",
            "port": 3128, "username": "scanner", "hasPassword": false
        })
    );

    // Update by id.
    let mut update = belgium();
    update["profile"]["id"] = json!(id);
    update["profile"]["port"] = json!(8080);
    let updated: Settings = invoke(&app, "save_proxy_profile", update);
    assert_eq!((updated.proxies.len(), updated.proxies[0].port), (1, 8080));

    // What cannot work is refused in plain words.
    for (change, expected) in [
        (
            json!({ "host": "http://be.proxy.example" }),
            "host name or IP address only",
        ),
        (json!({ "port": 0 }), "between 1 and 65535"),
        (json!({ "scheme": "socks5" }), "SOCKS5"),
        (json!({ "name": "belgium" }), "already exists"),
        (json!({ "name": " " }), "Give the proxy a name"),
    ] {
        let mut draft = belgium();
        draft["profile"]["name"] = json!("Another proxy");
        for (key, value) in change.as_object().unwrap() {
            draft["profile"][key] = value.clone();
        }
        let error = invoke_error(&app, "save_proxy_profile", draft);
        assert_eq!(error.code, ErrorCode::InvalidInput, "{change}");
        assert!(
            error.message.contains(expected),
            "{change}: {}",
            error.message
        );
    }

    // The password is stored apart and only its presence is reported.
    let with_password: Settings = invoke(
        &app,
        "set_proxy_password",
        json!({ "id": id, "password": "s3cret-proxy" }),
    );
    assert!(with_password.proxies[0].has_password);
    let everything = invoke_raw(&app, "get_settings", json!({}))
        .unwrap()
        .to_string();
    assert!(!everything.contains("s3cret-proxy"));
    assert_eq!(
        invoke_error(
            &app,
            "set_proxy_password",
            json!({ "id": id, "password": "" })
        )
        .code,
        ErrorCode::InvalidInput
    );
    assert_eq!(
        invoke_error(
            &app,
            "set_proxy_password",
            json!({ "id": "gone", "password": "x" })
        )
        .code,
        ErrorCode::NotFound
    );
    let without: Settings = invoke(&app, "clear_proxy_password", json!({ "id": id }));
    assert!(!without.proxies[0].has_password);

    // As the default it is named in the settings; deleted, the default is the system's again.
    let as_default: Settings = invoke(
        &app,
        "update_settings",
        json!({ "patch": { "defaultProxy": { "kind": "profile", "id": id } } }),
    );
    assert_eq!(
        as_default.default_proxy,
        ProxyChoice::Profile {
            id: id.clone(),
            name: Some("Belgium".into()),
            host: Some("be.proxy.example".into()),
        }
    );
    let deleted: Settings = invoke(&app, "delete_proxy_profile", json!({ "id": id }));
    assert!(deleted.proxies.is_empty());
    assert_eq!(deleted.default_proxy, ProxyChoice::System);
    assert_eq!(
        invoke_error(&app, "delete_proxy_profile", json!({ "id": id })).code,
        ErrorCode::NotFound
    );
}

#[test]
fn the_proxy_password_override_answers_for_every_profile() {
    let app = build_test_app(Arc::new(NoBrowser), None, Some("from-the-environment"));
    let saved: Settings = invoke(&app, "save_proxy_profile", belgium());
    assert!(saved.proxies[0].has_password);
    assert!(
        !serde_json::to_string(&saved)
            .unwrap()
            .contains("from-the-environment")
    );
}

#[test]
fn configuration_commands_save_run_filter_and_delete() {
    let browser = Arc::new(ManualBrowser::default());
    let app = test_app_with_browser(browser.clone());
    let proxy: Settings = invoke(&app, "save_proxy_profile", belgium());
    let proxy_id = proxy.proxies[0].id.clone();

    let mut draft = manual_options("example.com");
    draft["options"]["proxy"] = json!({ "kind": "profile", "id": proxy_id });
    let saved: ScanConfiguration = invoke(
        &app,
        "save_scan_configuration",
        json!({ "configuration": { "name": "Example BE", "options": draft["options"] } }),
    );
    assert_eq!(saved.name, "Example BE");
    assert_eq!(saved.options.url, "https://example.com/");
    assert_eq!(
        saved.options.seed, None,
        "a configuration never holds a seed"
    );
    assert_eq!(
        (saved.last_scan_id.clone(), saved.last_run_at),
        (None, None)
    );
    let listed: Vec<ScanConfiguration> = invoke(&app, "list_scan_configurations", json!({}));
    assert_eq!(listed, std::slice::from_ref(&saved));
    let raw = invoke_raw(&app, "list_scan_configurations", json!({})).unwrap();
    assert_eq!(raw[0]["lastScanId"], Value::Null);
    assert_eq!(
        raw[0]["options"]["proxy"],
        json!({ "kind": "profile", "id": proxy_id, "name": "Belgium", "host": "be.proxy.example" })
    );

    let duplicate = invoke_error(
        &app,
        "save_scan_configuration",
        json!({ "configuration": { "name": "example be", "options": draft["options"] } }),
    );
    assert_eq!(
        (duplicate.code, duplicate.message.as_str()),
        (
            ErrorCode::InvalidInput,
            "A scan configuration named \"example be\" already exists"
        )
    );

    // The proxy cannot go while the configuration needs it.
    let in_use = invoke_error(&app, "delete_proxy_profile", json!({ "id": proxy_id }));
    assert_eq!(in_use.code, ErrorCode::InvalidInput);
    assert!(
        in_use.message.contains("\"Example BE\""),
        "{}",
        in_use.message
    );

    // Running it records where the scan came from.
    let started: ScanSummary = invoke(&app, "start_scan", json!({ "configurationId": saved.id }));
    assert_eq!(
        started
            .configuration
            .as_ref()
            .map(|from| (from.id.as_str(), from.name.as_str())),
        Some((saved.id.as_str(), "Example BE"))
    );
    assert_eq!(
        started.proxy.as_ref().unwrap().name.as_deref(),
        Some("Belgium")
    );
    wait_for_capture(&browser, 1);
    invoke::<Value>(&app, "finish_capture", json!({ "scanId": started.id }));
    let detail = wait_for_end(&app, &started.id);
    assert_eq!(detail.summary.status, ScanStatus::Completed);

    let ran: Vec<ScanConfiguration> = invoke(&app, "list_scan_configurations", json!({}));
    assert_eq!(ran[0].last_scan_id.as_deref(), Some(started.id.as_str()));
    assert_eq!(ran[0].last_run_at, Some(started.started_at));

    let other: ScanSummary = invoke(&app, "start_scan", manual_options("example.org"));
    wait_for_capture(&browser, 2);
    invoke::<Value>(&app, "finish_capture", json!({ "scanId": other.id }));
    wait_for_end(&app, &other.id);
    let of_configuration: ScanList = invoke(
        &app,
        "list_scans",
        json!({ "query": { "configurationId": saved.id } }),
    );
    assert_eq!(of_configuration.total, 1);
    assert_eq!(of_configuration.items[0].id, started.id);

    let markdown: String = invoke(
        &app,
        "export_scan",
        json!({ "scanId": started.id, "format": "markdown" }),
    );
    assert!(
        markdown.contains("- **Scan configuration:** Example BE"),
        "{markdown}"
    );
    assert!(
        markdown.contains("- **Proxy:** Belgium (be.proxy.example)"),
        "{markdown}"
    );

    // Deleting the configuration leaves its scans.
    assert_eq!(
        invoke::<Value>(&app, "delete_scan_configuration", json!({ "id": saved.id })),
        Value::Null
    );
    assert!(
        invoke::<Vec<ScanConfiguration>>(&app, "list_scan_configurations", json!({})).is_empty()
    );
    let kept: ScanDetail = invoke(&app, "get_scan", json!({ "scanId": started.id }));
    assert_eq!(kept.summary.configuration.unwrap().name, "Example BE");
    assert_eq!(
        invoke_error(&app, "delete_scan_configuration", json!({ "id": saved.id })).code,
        ErrorCode::NotFound
    );
    assert_eq!(
        invoke_error(&app, "start_scan", json!({ "configurationId": saved.id })).code,
        ErrorCode::NotFound
    );
    // And now the proxy is free to go.
    let freed: Settings = invoke(&app, "delete_proxy_profile", json!({ "id": proxy_id }));
    assert!(freed.proxies.is_empty());
}

// -- the scan archive: network view, script viewer, storage --------------------

fn recorded_scan(app: &TestApp) -> ScanDetail {
    let started: ScanSummary = invoke(app, "start_scan", scan_options("https://pub.example/"));
    let detail = wait_for_end(app, &started.id);
    assert_eq!(
        detail.summary.status,
        ScanStatus::Completed,
        "{:?}",
        detail.summary.error
    );
    detail
}

#[test]
fn network_commands_take_camel_case_arguments_and_return_contract_types() {
    use adscan_model::{
        BodyState, NetworkAvailability, NetworkBodyChunk, NetworkRequestDetail, NetworkRowChunk,
        NetworkSummary,
    };
    let app = test_app_with_browser(Arc::new(RecordedBrowser));
    let detail = recorded_scan(&app);
    let scan_id = detail.summary.id.as_str();

    // The report says the scan has a network record.
    let raw = invoke_raw(&app, "get_scan", json!({ "scanId": scan_id })).unwrap();
    assert_eq!(raw["network"]["availability"], "available");
    assert_eq!(raw["network"]["requestCount"], 3);
    assert_eq!(raw["network"]["headersRedacted"], true);
    assert_eq!(raw["scripts"][0]["requestId"], "r0");

    let first: NetworkRowChunk = invoke(
        &app,
        "list_network_requests",
        json!({ "scanId": scan_id, "limit": 2 }),
    );
    assert_eq!((first.rows.len(), first.total), (2, 3));
    let rest: NetworkRowChunk = invoke(
        &app,
        "list_network_requests",
        json!({ "scanId": scan_id, "pages": [0], "cursor": first.next_cursor, "limit": 2 }),
    );
    assert_eq!((rest.rows.len(), rest.next_cursor), (1, None));
    let none: NetworkRowChunk = invoke(
        &app,
        "list_network_requests",
        json!({ "scanId": scan_id, "pages": [4] }),
    );
    assert_eq!(none.total, 0);
    let raw = invoke_raw(&app, "list_network_requests", json!({ "scanId": scan_id })).unwrap();
    let row = &raw["rows"][2];
    assert_eq!(row["id"], "r2");
    assert_eq!(row["category"], "bid");
    assert_eq!(row["servedFrom"], "network");
    assert_eq!(row["hasResponseBody"], true);
    assert_eq!(row["startedAtMs"], 100.0);

    let summary: NetworkSummary = invoke(&app, "get_network_summary", json!({ "scanId": scan_id }));
    assert_eq!(summary.archive.availability, NetworkAvailability::Available);
    assert_eq!(summary.total_requests, 3);

    let request: NetworkRequestDetail = invoke(
        &app,
        "get_network_request",
        json!({ "scanId": scan_id, "requestId": "r2" }),
    );
    assert_eq!(request.row.bidder.as_deref(), Some("openx"));
    assert_eq!(request.decoded.unwrap().response.unwrap().nbr, Some(7));
    let raw = invoke_raw(
        &app,
        "get_network_request",
        json!({ "scanId": scan_id, "requestId": "r0" }),
    )
    .unwrap();
    assert_eq!(
        raw["requestHeaders"][0],
        json!({ "name": "Cookie", "value": "sid=[19 bytes]", "redaction": "cookie" })
    );
    assert_eq!(raw["responseBody"]["state"], "kept");
    assert_eq!(raw["responseBody"]["scriptId"], "s1");

    let body: NetworkBodyChunk = invoke(
        &app,
        "get_network_body",
        json!({ "scanId": scan_id, "requestId": "r0", "side": "response", "offset": 9, "maxBytes": 4 }),
    );
    assert_eq!(body.text.as_deref(), Some("gate"));
    assert_eq!((body.offset, body.next_offset), (9.0, Some(13.0)));
    let payload: NetworkBodyChunk = invoke(
        &app,
        "get_network_body",
        json!({ "scanId": scan_id, "requestId": "r2", "side": "request" }),
    );
    assert_eq!(
        payload.text.as_deref(),
        Some(r#"{"id":"1","imp":[{"id":"a"}]}"#)
    );
    let image: NetworkBodyChunk = invoke(
        &app,
        "get_network_body",
        json!({ "scanId": scan_id, "requestId": "r1", "side": "response" }),
    );
    assert_eq!((image.state, image.text), (BodyState::NotKeptType, None));

    for (command, args) in [
        (
            "get_network_request",
            json!({ "scanId": scan_id, "requestId": "r77" }),
        ),
        (
            "get_network_body",
            json!({ "scanId": scan_id, "requestId": "nope", "side": "response" }),
        ),
        ("get_network_summary", json!({ "scanId": "no-such-scan" })),
        ("list_network_requests", json!({ "scanId": "no-such-scan" })),
    ] {
        assert_eq!(
            invoke_error(&app, command, args).code,
            ErrorCode::NotFound,
            "{command}"
        );
    }
    assert_eq!(
        invoke_error(
            &app,
            "list_network_requests",
            json!({ "scanId": scan_id, "cursor": "seven" })
        )
        .code,
        ErrorCode::InvalidInput
    );
    // A side that does not exist never reaches the command.
    assert!(
        invoke_raw(
            &app,
            "get_network_body",
            json!({ "scanId": scan_id, "requestId": "r0", "side": "sideways" })
        )
        .is_err()
    );
}

#[test]
fn a_script_source_opens_through_the_command() {
    use adscan_model::{ScriptSource, SourceAvailability, SourceFormat};
    let app = test_app_with_browser(Arc::new(RecordedBrowser));
    let detail = recorded_scan(&app);
    let scan_id = detail.summary.id.as_str();

    let original: ScriptSource = invoke(
        &app,
        "get_script_source",
        json!({ "scanId": scan_id, "scriptId": "s1", "view": "original" }),
    );
    assert_eq!(original.text.as_deref(), Some(RECORDED_SCRIPT));
    assert_eq!(original.format, SourceFormat::Unformatted);

    let column = RECORDED_SCRIPT.find("pbjs.requestBids").unwrap() + 1;
    let raw = invoke_raw(
        &app,
        "get_script_source",
        json!({ "scanId": scan_id, "scriptId": "s1", "view": "formatted", "originalLine": 1, "originalColumn": column }),
    )
    .unwrap();
    assert_eq!(raw["availability"], "available");
    assert_eq!(raw["format"], "parser");
    let text = raw["text"].as_str().unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(raw["lineCount"], lines.len());
    let min_width = lines
        .iter()
        .position(|line| line.contains("min-width"))
        .unwrap()
        + 1;
    assert!(min_width > 1, "the script was put on lines of its own");
    let focus = raw["focusLine"].as_u64().unwrap() as usize;
    assert!(
        lines[focus - 1].contains("pbjs.requestBids("),
        "{}",
        lines[focus - 1]
    );
    assert_eq!(
        raw["adCalls"][0],
        json!({ "name": "requestBids", "line": focus })
    );
    assert_eq!(raw["anchors"], json!([]));

    assert_eq!(
        invoke_error(
            &app,
            "get_script_source",
            json!({ "scanId": scan_id, "scriptId": "s9", "view": "formatted" })
        )
        .code,
        ErrorCode::NotFound
    );

    // After the stored data is cleared the same call says why there is no text.
    let _: adscan_model::ArchiveStorage = invoke(&app, "clear_archive", json!({}));
    let gone: ScriptSource = invoke(
        &app,
        "get_script_source",
        json!({ "scanId": scan_id, "scriptId": "s1", "view": "formatted" }),
    );
    assert_eq!(
        (gone.availability, gone.text),
        (SourceAvailability::Evicted, None)
    );
}

#[test]
fn storage_commands_report_usage_and_the_settings_switch_storage_off() {
    use adscan_model::{ArchiveStorage, NetworkAvailability, NetworkRowChunk};
    let app = test_app_with_browser(Arc::new(RecordedBrowser));
    let empty: ArchiveStorage = invoke(&app, "get_archive_storage", json!({}));
    assert_eq!((empty.scans, empty.stored_bytes), (0, 0.0));
    assert_eq!(empty.limit_bytes, adscan_model::DEFAULT_ARCHIVE_LIMIT_BYTES);

    let settings: Settings = invoke(&app, "get_settings", json!({}));
    assert!(settings.keep_network_data && !settings.store_full_headers);

    let stored = recorded_scan(&app);
    let raw = invoke_raw(&app, "get_archive_storage", json!({})).unwrap();
    assert_eq!(raw["scans"], 1);
    assert_eq!(raw["evictedScans"], 0);
    assert!(raw["storedBytes"].as_f64().unwrap() > 100.0);
    assert_eq!(raw["oldestAt"], stored.summary.started_at);
    let pack = app
        ._data_dir
        .path()
        .join(format!("archive/scans/{}.pack.zst", stored.summary.id));
    assert!(pack.is_file());

    // Storage off: the next scan writes nothing and says so.
    let updated: Settings = invoke(
        &app,
        "update_settings",
        json!({ "patch": { "keepNetworkData": false, "storeFullHeaders": true, "archiveLimitBytes": 1073741824 } }),
    );
    assert!(!updated.keep_network_data && updated.store_full_headers);
    assert_eq!(updated.archive_limit_bytes, 1_073_741_824.0);
    let unstored = recorded_scan(&app);
    assert_eq!(
        unstored.network.unwrap().availability,
        NetworkAvailability::Disabled
    );
    let rows: NetworkRowChunk = invoke(
        &app,
        "list_network_requests",
        json!({ "scanId": unstored.summary.id }),
    );
    assert_eq!(rows.total, 0);
    let storage: ArchiveStorage = invoke(&app, "get_archive_storage", json!({}));
    assert_eq!((storage.scans, storage.limit_bytes), (1, 1_073_741_824.0));

    let too_small = invoke_error(
        &app,
        "update_settings",
        json!({ "patch": { "archiveLimitBytes": 5 } }),
    );
    assert_eq!(too_small.code, ErrorCode::InvalidInput);

    // Deleting the scan deletes its file; clearing empties the rest.
    let _: () = invoke(&app, "delete_scan", json!({ "scanId": stored.summary.id }));
    assert!(!pack.exists());
    let cleared: ArchiveStorage = invoke(&app, "clear_archive", json!({}));
    assert_eq!((cleared.scans, cleared.stored_bytes), (0, 0.0));
}
