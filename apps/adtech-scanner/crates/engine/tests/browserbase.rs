//! Browserbase as the place a scan's browser runs: credentials, settings,
//! the pipeline around a capture there, the profile kept there, and the
//! sessions an earlier run left behind. The capture and Browserbase's API
//! are doubles; storage and the pipeline are the real ones.

mod common;

use std::time::Duration;

use adscan_engine::application::AppError;
use adscan_engine::application::ports::{
    BrowserbaseError, BrowserbaseLedger, CaptureError, SettingsRepository,
};
use adscan_model::{
    ActivityKind, BrowserChoice, BrowserKind, BrowserUse, BrowserbaseProxyKind, BrowserbaseRegion,
    BrowserbaseSessionFacts, BrowserbaseSettingsPatch, Capture, CaptureActivity, CaptureBrowser,
    CaptureEnvironment, CaptureProxy, DeviceProfile, ErrorCode, ExportFormat, LogKind, LogLevel,
    PipelineNode, ProxyChoice, ProxyKind, ProxyProfileDraft, ProxyScheme, RemoteUsage,
    ScanConfigurationDraft, ScanEvent, ScanOptions, ScanStatus, SettingsPatch, StepStatus,
    VisitorBehavior,
};
use common::*;
use serde_json::json;

const OFFLINE: &str = "http://127.0.0.1:1";

fn offline(behavior: CaptureBehavior) -> Harness {
    harness(behavior, OFFLINE, None)
}

/// A page view as a Browserbase browser hands it over.
fn cloud_capture() -> Capture {
    let mut capture = healthy_capture(DeviceProfile::Desktop);
    capture.user_agent = "Mozilla/5.0 (X11; Linux x86_64) Chrome/157.0.0.0 Safari/537.36".into();
    capture.environment = CaptureEnvironment {
        headless: true,
        webdriver: Some(false),
        viewport_overridden: true,
        browserbase: Some(BrowserbaseSessionFacts {
            session_id: REMOTE_SESSION_ID.into(),
            browser_version: Some("Chrome/157.0.8103.79".into()),
            original_timezone: Some("America/Los_Angeles".into()),
            applied_timezone: Some("Europe/Berlin".into()),
            language: Some("en-US".into()),
            round_trip_ms: Some(118.6),
            clock_offset_ms: Some(-53.0),
        }),
        ..CaptureEnvironment::default()
    };
    capture
}

fn returning_a_cloud_capture() -> Harness {
    offline(CaptureBehavior::Return(Box::new(cloud_capture())))
}

async fn connect(harness: &Harness) {
    let status = harness
        .browserbase
        .set_credentials(BROWSERBASE_KEY, None)
        .await
        .unwrap();
    assert!(status.valid, "{:?}", status.message);
}

fn in_frankfurt() -> ScanOptions {
    ScanOptions {
        browser: Some(BrowserChoice::Browserbase {
            region: Some(BrowserbaseRegion::EuCentral1),
        }),
        ..scan_options(false)
    }
}

fn belgium() -> ProxyProfileDraft {
    ProxyProfileDraft {
        id: None,
        name: "Belgium".into(),
        scheme: ProxyScheme::Http,
        host: "be.proxy.example".into(),
        port: 3128,
        username: Some("scanner".into()),
    }
}

fn location(country: &str) -> ProxyChoice {
    ProxyChoice::BrowserbaseLocation {
        country: country.into(),
        state: None,
        city: None,
    }
}

async fn wait_for_capture(harness: &Harness, count: usize) {
    for _ in 0..1000 {
        if harness.capture.requests.lock().unwrap().len() >= count {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("capture {count} never started");
}

async fn wait_for_call(harness: &Harness, call: &str, times: usize) {
    for _ in 0..1000 {
        let seen = harness
            .browserbase_api
            .calls()
            .iter()
            .filter(|made| *made == call)
            .count();
        if seen >= times {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!(
        "`{call}` was not called {times} time(s): {:?}",
        harness.browserbase_api.calls()
    );
}

fn step_summary(detail: &adscan_model::ScanDetail, node: PipelineNode) -> &str {
    detail
        .steps
        .iter()
        .find(|step| step.node == node)
        .and_then(|step| step.summary.as_deref())
        .unwrap_or_default()
}

fn visit_of(detail: &adscan_model::ScanDetail) -> adscan_model::BrowserbaseVisit {
    match detail.environment.clone().unwrap().browser {
        BrowserUse::Browserbase(visit) => *visit,
        BrowserUse::Local => panic!("the scan ran on Browserbase"),
    }
}

// -- credentials ------------------------------------------------------------------

#[tokio::test]
async fn credentials_are_stored_only_when_browserbase_accepts_them() {
    let harness = returning_a_cloud_capture();
    let before = harness.settings.get().await.unwrap();
    assert!(!before.browserbase.connected);
    assert_eq!(before.default_browser, BrowserKind::Local);

    let empty = harness
        .browserbase
        .set_credentials("  ", None)
        .await
        .unwrap();
    assert_eq!(
        (empty.valid, empty.message.as_deref()),
        (false, Some("Enter a Browserbase API key."))
    );
    let refused = harness
        .browserbase
        .set_credentials("bb_wrong", None)
        .await
        .unwrap();
    assert_eq!(
        (refused.valid, refused.message.as_deref()),
        (false, Some("Browserbase did not accept the API key."))
    );
    *harness.browserbase_api.projects.lock().unwrap() = Err(BrowserbaseError::Failed(
        "Browserbase could not be reached: the request timed out".into(),
    ));
    let unverified = harness
        .browserbase
        .set_credentials(BROWSERBASE_KEY, None)
        .await
        .unwrap();
    assert_eq!(
        unverified.message.as_deref(),
        Some(
            "The key could not be verified (Browserbase could not be reached: the request timed out). It was not saved."
        )
    );
    assert_eq!(
        harness.secrets.browserbase_credentials().await.unwrap(),
        None
    );
    assert!(!harness.settings.get().await.unwrap().browserbase.connected);

    // The key's only project is adopted.
    *harness.browserbase_api.projects.lock().unwrap() =
        Ok(vec![browserbase_project("project-1", "Local")]);
    let accepted = harness
        .browserbase
        .set_credentials(&format!("  {BROWSERBASE_KEY} "), None)
        .await
        .unwrap();
    assert!(accepted.valid && accepted.message.is_none() && accepted.projects.is_empty());
    assert_eq!(
        harness.secrets.browserbase_credentials().await.unwrap(),
        Some(browserbase_credentials())
    );
    let settings = harness.settings.get().await.unwrap();
    assert!(settings.browserbase.connected);
    assert_eq!(settings.browserbase.project_name.as_deref(), Some("Local"));
    assert_eq!(settings.browserbase.concurrency, Some(3));

    // Neither the key nor the project id is in the settings, as returned or
    // as stored; only the project's name is.
    let stored = harness.store.load().await.unwrap().unwrap();
    for document in [
        serde_json::to_string(&settings).unwrap(),
        serde_json::to_string(&stored).unwrap(),
    ] {
        assert!(!document.contains(BROWSERBASE_KEY));
        assert!(!document.contains("project-1"));
    }
    assert!(!stored.browserbase.connected, "what exists is never stored");
}

#[tokio::test]
async fn a_key_with_several_projects_needs_one_to_be_chosen() {
    let harness = returning_a_cloud_capture();
    *harness.browserbase_api.projects.lock().unwrap() = Ok(vec![
        browserbase_project("project-1", "Local"),
        browserbase_project("project-2", "Production"),
    ]);

    let undecided = harness
        .browserbase
        .set_credentials(BROWSERBASE_KEY, None)
        .await
        .unwrap();
    assert!(!undecided.valid);
    assert_eq!(
        undecided.message.as_deref(),
        Some("This key has more than one Browserbase project. Choose the one to use.")
    );
    assert_eq!(
        undecided
            .projects
            .iter()
            .map(|project| (project.id.as_str(), project.name.as_str()))
            .collect::<Vec<_>>(),
        [("project-1", "Local"), ("project-2", "Production")]
    );
    assert_eq!(
        harness.secrets.browserbase_credentials().await.unwrap(),
        None
    );

    let unknown = harness
        .browserbase
        .set_credentials(BROWSERBASE_KEY, Some("project-9"))
        .await
        .unwrap();
    assert_eq!(
        (unknown.valid, unknown.message.as_deref()),
        (
            false,
            Some("This Browserbase project was not found for this key.")
        )
    );

    let chosen = harness
        .browserbase
        .set_credentials(BROWSERBASE_KEY, Some("project-2"))
        .await
        .unwrap();
    assert!(chosen.valid);
    assert_eq!(
        harness
            .secrets
            .browserbase_credentials()
            .await
            .unwrap()
            .unwrap()
            .project_id,
        "project-2"
    );
    assert_eq!(
        harness
            .settings
            .get()
            .await
            .unwrap()
            .browserbase
            .project_name
            .as_deref(),
        Some("Production")
    );

    *harness.browserbase_api.projects.lock().unwrap() = Ok(Vec::new());
    let none = harness
        .browserbase
        .set_credentials(BROWSERBASE_KEY, None)
        .await
        .unwrap();
    assert_eq!(
        none.message.as_deref(),
        Some("This key has no Browserbase project.")
    );
}

#[tokio::test]
async fn browserbase_is_the_default_only_while_its_credentials_are_stored() {
    let harness = returning_a_cloud_capture();
    let by_default = SettingsPatch {
        default_browser: Some(BrowserKind::Browserbase),
        ..SettingsPatch::default()
    };
    assert_eq!(
        harness.settings.update(by_default.clone()).await,
        Err(AppError::InvalidInput(
            "Add a Browserbase API key before making Browserbase the default.".into()
        ))
    );

    connect(&harness).await;
    let settings = harness.settings.update(by_default).await.unwrap();
    assert_eq!(settings.default_browser, BrowserKind::Browserbase);
    // A scan that names no browser now runs there, and one of a page on
    // this machine still runs here.
    let cloud = run_scan(&harness, scan_options(false)).await;
    assert!(matches!(
        cloud.options.browser,
        Some(BrowserChoice::Browserbase { .. })
    ));
    let local = run_scan(
        &harness,
        ScanOptions {
            url: "http://127.0.0.1:4100/".into(),
            ..scan_options(false)
        },
    )
    .await;
    assert_eq!(local.options.browser, Some(BrowserChoice::Local));

    let cleared = harness.browserbase.clear_credentials().await.unwrap();
    assert!(!cleared.browserbase.connected);
    assert_eq!(cleared.default_browser, BrowserKind::Local);
    assert_eq!(cleared.browserbase.project_name, None);
    assert_eq!(
        harness.secrets.browserbase_credentials().await.unwrap(),
        None
    );
    assert_eq!(
        harness.store.load().await.unwrap().unwrap().default_browser,
        BrowserKind::Local,
        "the stored default is reset, not only masked"
    );
    assert_eq!(
        harness
            .scans
            .start(in_frankfurt())
            .await
            .map(|summary| summary.id),
        Err(AppError::BrowserbaseAuth(
            "Browserbase is not set up. Add the API key in Settings.".into()
        ))
    );
}

#[tokio::test]
async fn the_browserbase_settings_are_patched_validated_and_follow_the_proxy_profiles() {
    let harness = returning_a_cloud_capture();
    let patch = |cloud: BrowserbaseSettingsPatch| SettingsPatch {
        browserbase: Some(cloud),
        ..SettingsPatch::default()
    };
    let defaults = harness.settings.get().await.unwrap().browserbase;
    assert_eq!(defaults.region, BrowserbaseRegion::UsWest2);
    assert_eq!(defaults.default_proxy, ProxyChoice::None);
    assert!(!defaults.persistent_profile);

    let settings = harness
        .settings
        .update(patch(BrowserbaseSettingsPatch {
            region: Some(BrowserbaseRegion::EuCentral1),
            // This machine's proxy means nothing to a cloud browser.
            default_proxy: Some(ProxyChoice::System),
            persistent_profile: Some(true),
        }))
        .await
        .unwrap();
    assert_eq!(settings.browserbase.region, BrowserbaseRegion::EuCentral1);
    assert_eq!(settings.browserbase.default_proxy, ProxyChoice::None);
    assert!(settings.browserbase.persistent_profile);
    assert!(
        !settings.persistent_profile,
        "the local profile switch is another one"
    );

    let located = harness
        .settings
        .update(patch(BrowserbaseSettingsPatch {
            default_proxy: Some(location("be")),
            ..BrowserbaseSettingsPatch::default()
        }))
        .await
        .unwrap();
    assert_eq!(located.browserbase.default_proxy, location("BE"));
    assert_eq!(
        located.browserbase.region,
        BrowserbaseRegion::EuCentral1,
        "an omitted field keeps its value"
    );
    assert_eq!(
        harness
            .settings
            .update(patch(BrowserbaseSettingsPatch {
                default_proxy: Some(location("Belgium")),
                ..BrowserbaseSettingsPatch::default()
            }))
            .await
            .map(|_| ()),
        Err(AppError::InvalidInput(
            "Enter the country of the Browserbase location as a two-letter code, for example BE"
                .into()
        ))
    );
    // A location is no proxy for a browser on this machine.
    assert_eq!(
        harness
            .settings
            .update(SettingsPatch {
                default_proxy: Some(location("BE")),
                ..SettingsPatch::default()
            })
            .await
            .map(|_| ()),
        Err(AppError::InvalidInput(
            "A Browserbase location can only be used when the browser runs on Browserbase.".into()
        ))
    );

    let with_proxy = harness
        .settings
        .save_proxy_profile(belgium())
        .await
        .unwrap();
    let id = with_proxy.proxies[0].id.clone();
    let named = harness
        .settings
        .update(patch(BrowserbaseSettingsPatch {
            default_proxy: Some(ProxyChoice::Profile {
                id: id.clone(),
                name: None,
                host: None,
            }),
            ..BrowserbaseSettingsPatch::default()
        }))
        .await
        .unwrap();
    assert_eq!(
        named.browserbase.default_proxy,
        ProxyChoice::Profile {
            id: id.clone(),
            name: Some("Belgium".into()),
            host: Some("be.proxy.example".into()),
        }
    );
    // Renaming the profile renames the snapshot; deleting it leaves the
    // cloud region as the exit.
    let renamed = harness
        .settings
        .save_proxy_profile(ProxyProfileDraft {
            id: Some(id.clone()),
            name: "Brussels".into(),
            ..belgium()
        })
        .await
        .unwrap();
    assert!(matches!(
        &renamed.browserbase.default_proxy,
        ProxyChoice::Profile { name: Some(name), .. } if name == "Brussels"
    ));
    // Moved to an address only this machine can reach, the profile is
    // still the user's to save, for scans on this machine. It stops being
    // what a Browserbase browser leaves through.
    harness
        .settings
        .update(SettingsPatch {
            default_proxy: Some(ProxyChoice::Profile {
                id: id.clone(),
                name: None,
                host: None,
            }),
            ..SettingsPatch::default()
        })
        .await
        .unwrap();
    let moved = harness
        .settings
        .save_proxy_profile(ProxyProfileDraft {
            id: Some(id.clone()),
            name: "Brussels".into(),
            host: "192.168.1.20".into(),
            ..belgium()
        })
        .await
        .unwrap();
    assert_eq!(moved.proxies[0].host, "192.168.1.20");
    assert_eq!(moved.browserbase.default_proxy, ProxyChoice::None);
    assert_eq!(
        moved.default_proxy,
        ProxyChoice::Profile {
            id: id.clone(),
            name: Some("Brussels".into()),
            host: Some("192.168.1.20".into()),
        },
        "the default for this machine's browser follows the profile"
    );
    // It cannot be made the Browserbase default again while it is there.
    assert_eq!(
        harness
            .settings
            .update(patch(BrowserbaseSettingsPatch {
                default_proxy: Some(ProxyChoice::Profile {
                    id: id.clone(),
                    name: None,
                    host: None,
                }),
                ..BrowserbaseSettingsPatch::default()
            }))
            .await
            .map(|_| ()),
        Err(AppError::InvalidInput(
            "Browserbase cannot reach the proxy \"Brussels\": its address is on this Mac or your network. Use a proxy that is reachable from the internet.".into()
        ))
    );

    // Back on a public address it can be, and deleting it then leaves the
    // cloud region as the exit.
    harness
        .settings
        .save_proxy_profile(ProxyProfileDraft {
            id: Some(id.clone()),
            name: "Brussels".into(),
            ..belgium()
        })
        .await
        .unwrap();
    let named_again = harness
        .settings
        .update(patch(BrowserbaseSettingsPatch {
            default_proxy: Some(ProxyChoice::Profile {
                id: id.clone(),
                name: None,
                host: None,
            }),
            ..BrowserbaseSettingsPatch::default()
        }))
        .await
        .unwrap();
    assert!(matches!(
        &named_again.browserbase.default_proxy,
        ProxyChoice::Profile { name: Some(name), .. } if name == "Brussels"
    ));
    let deleted = harness.settings.delete_proxy_profile(&id).await.unwrap();
    assert_eq!(deleted.browserbase.default_proxy, ProxyChoice::None);
}

#[tokio::test]
async fn testing_the_connection_reads_the_account_and_starts_no_browser() {
    let harness = returning_a_cloud_capture();
    let not_set_up = harness.browserbase.status().await.unwrap_err();
    assert_eq!(not_set_up.code(), ErrorCode::BrowserbaseAuth);
    assert_eq!(
        not_set_up.to_string(),
        "Browserbase is not set up. Add the API key in Settings."
    );
    assert!(harness.browserbase_api.calls().is_empty());

    connect(&harness).await;
    *harness.browserbase_api.projects.lock().unwrap() =
        Ok(vec![adscan_model::BrowserbaseProject {
            concurrency: 10,
            ..browserbase_project("project-1", "Shared")
        }]);
    let status = harness.browserbase.status().await.unwrap();
    assert_eq!(status.project_name, "Shared");
    assert_eq!(status.concurrency, 10);
    assert_eq!(status.browser_minutes, 483.0);
    assert_eq!(status.proxy_bytes, 1_998_349_077.0);
    assert_eq!(status.running_sessions, 1);
    assert_eq!(
        harness.browserbase_api.calls(),
        ["project", "projectUsage", "runningSessionCount"]
    );
    // What the settings show of the project follows what was just read.
    let settings = harness.settings.get().await.unwrap();
    assert_eq!(settings.browserbase.project_name.as_deref(), Some("Shared"));
    assert_eq!(settings.browserbase.concurrency, Some(10));

    *harness.browserbase_api.failure.lock().unwrap() = Some(BrowserbaseError::Auth(
        "Browserbase did not accept the API key. Check it in Settings.".into(),
    ));
    assert_eq!(
        harness.browserbase.status().await.unwrap_err().code(),
        ErrorCode::BrowserbaseAuth
    );
}

#[tokio::test]
async fn the_probe_runs_in_the_asked_region_or_the_settings_one_tagged_with_the_install_id() {
    let harness = returning_a_cloud_capture();
    assert_eq!(
        harness.browserbase.probe(None).await.unwrap_err().code(),
        ErrorCode::BrowserbaseAuth
    );
    connect(&harness).await;
    let install_id = harness.store.install_id().await.unwrap();

    let probe = harness.browserbase.probe(None).await.unwrap();
    assert_eq!(probe.region, BrowserbaseRegion::UsWest2);
    assert_eq!(probe.webdriver, Some(false));
    let asked = harness
        .browserbase
        .probe(Some(BrowserbaseRegion::ApSoutheast1))
        .await
        .unwrap();
    assert_eq!(asked.region, BrowserbaseRegion::ApSoutheast1);
    assert_eq!(
        harness.browserbase_api.calls(),
        [
            format!("probe:us-west-2:{install_id}"),
            format!("probe:ap-southeast-1:{install_id}"),
        ]
    );

    *harness.browserbase_api.failure.lock().unwrap() = Some(BrowserbaseError::Limit(
        "Browserbase is at its limit of browsers running at once.".into(),
    ));
    let limited = harness.browserbase.probe(None).await.unwrap_err();
    assert_eq!(limited.code(), ErrorCode::BrowserbaseLimit);
    assert_eq!(
        limited.to_string(),
        "Browserbase is at its limit of browsers running at once."
    );
}

// -- a scan on Browserbase --------------------------------------------------------

#[tokio::test]
async fn a_browserbase_scan_records_where_it_ran_what_it_used_and_what_the_page_saw() {
    let harness = returning_a_cloud_capture();
    connect(&harness).await;
    harness.capture.activities.lock().unwrap().extend([
        CaptureActivity {
            kind: ActivityKind::RemoteBrowser,
            message: "Browserbase started a browser in eu-central-1".into(),
            page: None,
            data: Some(json!({ "event": "sessionCreated", "sessionId": REMOTE_SESSION_ID })),
        },
        CaptureActivity {
            kind: ActivityKind::RemoteBrowser,
            message: "The connection to the Browserbase browser was lost".into(),
            page: None,
            data: Some(json!({ "event": "connectionLost" })),
        },
        CaptureActivity {
            kind: ActivityKind::RemoteBrowser,
            message: "Browserbase did not confirm that the browser has ended".into(),
            page: None,
            data: Some(json!({ "event": "releaseUnconfirmed" })),
        },
        // An event this version does not know is kept as it came.
        CaptureActivity {
            kind: ActivityKind::RemoteBrowser,
            message: "Browserbase ran 5 scripts of its own in the page".into(),
            page: None,
            data: Some(json!({ "event": "serviceScripts", "count": 5 })),
        },
    ]);
    let install_id = harness.store.install_id().await.unwrap();

    let detail = run_scan(&harness, in_frankfurt()).await;

    assert_eq!(detail.summary.status, ScanStatus::Completed);
    // No browser on this machine was looked for.
    assert_eq!(
        step_summary(&detail, PipelineNode::LaunchBrowser),
        "Browserbase, eu-central-1"
    );
    let logs = harness.scans.logs(&detail.summary.id, None).await.unwrap();
    assert!(
        logs.iter()
            .any(|entry| entry.message == "Using Browserbase in eu-central-1 (project Local)")
    );
    let start = logs
        .iter()
        .find(|entry| entry.message.starts_with("Loading https://pub.example/"))
        .unwrap();
    assert!(
        start.message.contains(
            "Browserbase browser in eu-central-1, throwaway profile, accept consent true, \
             no proxy: leaves from the cloud region)"
        ),
        "{}",
        start.message
    );

    // The request says which browser; the credentials travel beside it.
    let request = harness.capture.requests.lock().unwrap()[0].clone();
    let CaptureBrowser::Browserbase(cloud) = &request.browser else {
        panic!("the capture was asked for a local browser");
    };
    assert_eq!(cloud.region, BrowserbaseRegion::EuCentral1);
    assert_eq!(cloud.install_id, install_id);
    assert_eq!(cloud.scan_id, detail.summary.id);
    assert_eq!((&cloud.location, &cloud.context_id), (&None, &None));
    assert!(cloud.settings.solve_captchas && cloud.settings.record_session);
    assert!(cloud.settings.log_session && !cloud.settings.block_ads && !cloud.settings.keep_alive);
    assert_eq!(request.proxy, CaptureProxy::None);
    assert!(!request.persistent_profile);
    assert_eq!(
        *harness.capture.browserbase_credentials.lock().unwrap(),
        [Some(browserbase_credentials())]
    );
    let serialised = serde_json::to_string(&request).unwrap();
    assert!(!serialised.contains(BROWSERBASE_KEY) && !serialised.contains("project-1"));

    // What is stored: settled options, usage on the summary, the visit in
    // the environment next to what the page read.
    assert_eq!(
        detail.options.browser,
        Some(BrowserChoice::Browserbase {
            region: Some(BrowserbaseRegion::EuCentral1)
        })
    );
    assert_eq!(detail.options.headless, Some(true));
    assert_eq!(detail.options.proxy, Some(ProxyChoice::None));
    assert_eq!(
        detail.summary.remote,
        Some(RemoteUsage {
            region: BrowserbaseRegion::EuCentral1,
            session_seconds: Some(41.0),
            proxy_bytes: Some(0.0),
        })
    );
    let environment = detail.environment.clone().unwrap();
    assert_eq!(environment.webdriver, Some(false));
    assert!(environment.headless && environment.viewport_overridden);
    let visit = visit_of(&detail);
    assert_eq!(visit.session_id, REMOTE_SESSION_ID);
    assert_eq!(visit.region, BrowserbaseRegion::EuCentral1);
    assert_eq!(visit.proxy_kind, BrowserbaseProxyKind::None);
    assert_eq!(
        visit.browser_version.as_deref(),
        Some("Chrome/157.0.8103.79")
    );
    assert_eq!(
        visit.user_agent.as_deref(),
        Some("Mozilla/5.0 (X11; Linux x86_64) Chrome/157.0.0.0 Safari/537.36")
    );
    assert_eq!(
        (
            visit.original_timezone.as_deref(),
            visit.applied_timezone.as_deref()
        ),
        (Some("America/Los_Angeles"), Some("Europe/Berlin"))
    );
    assert_eq!(visit.round_trip_ms, Some(118.6));
    assert_eq!(
        (visit.session_seconds, visit.proxy_bytes),
        (Some(41.0), Some(0.0))
    );

    // What the service did is in the audit trail as a visitor entry.
    let browser_lines: Vec<_> = logs
        .iter()
        .filter(|entry| entry.kind == LogKind::Visitor)
        .filter(|entry| {
            entry
                .data
                .as_ref()
                .is_some_and(|data| data["activity"] == "browser")
        })
        .collect();
    assert_eq!(browser_lines.len(), 4);
    assert_eq!(
        browser_lines[0].data.as_ref().unwrap()["detail"]["event"],
        "sessionCreated"
    );
    assert_eq!(
        browser_lines
            .iter()
            .map(|entry| entry.level)
            .collect::<Vec<_>>(),
        [
            LogLevel::Info,
            LogLevel::Warn,
            LogLevel::Warn,
            LogLevel::Info
        ]
    );
    assert_eq!(
        browser_lines[3].data.as_ref().unwrap()["detail"],
        json!({ "event": "serviceScripts", "count": 5 })
    );
    assert_eq!(
        browser_lines[3].message,
        "Browserbase ran 5 scripts of its own in the page"
    );

    // The session is closed on the record, and no credential reached the
    // audit trail or the stored scan.
    assert!(harness.store.open_sessions().await.unwrap().is_empty());
    let everything = format!(
        "{}{}",
        serde_json::to_string(&logs).unwrap(),
        serde_json::to_string(&detail).unwrap()
    );
    assert!(!everything.contains(BROWSERBASE_KEY) && !everything.contains("project-1"));

    // Nothing asked Browserbase for anything beside the capture: no proxy,
    // so nothing to count again.
    tokio::time::sleep(PROXY_COUNT_DELAY * 3).await;
    assert!(harness.browserbase_api.calls().is_empty());
}

#[tokio::test]
async fn a_local_scan_is_given_no_browserbase_credentials_even_when_they_are_stored() {
    let harness = offline(CaptureBehavior::Return(Box::new(healthy_capture(
        DeviceProfile::Desktop,
    ))));
    connect(&harness).await;

    let detail = run_scan(&harness, scan_options(false)).await;

    assert_eq!(detail.summary.status, ScanStatus::Completed);
    assert_eq!(detail.options.browser, Some(BrowserChoice::Local));
    assert_eq!(detail.summary.remote, None);
    assert_eq!(
        detail.environment.clone().unwrap().browser,
        BrowserUse::Local
    );
    assert_eq!(
        harness.capture.requests.lock().unwrap()[0].browser,
        CaptureBrowser::Local
    );
    assert_eq!(
        *harness.capture.browserbase_credentials.lock().unwrap(),
        [None]
    );
    assert_eq!(
        step_summary(&detail, PipelineNode::LaunchBrowser),
        "Chromium 141.0.0.0 found"
    );
}

#[tokio::test]
async fn a_browserbase_scan_does_not_need_a_browser_on_this_machine() {
    // `NoBrowser` is a machine without Chrome: a local scan stops at its
    // first step there.
    let harness = offline(CaptureBehavior::NoBrowser);
    connect(&harness).await;
    let local = run_scan(&harness, scan_options(false)).await;
    assert_eq!(
        local.steps[0].status,
        StepStatus::Failed,
        "a local scan needs Chrome"
    );

    let cloud = run_scan(&harness, in_frankfurt()).await;
    assert_eq!(cloud.steps[0].status, StepStatus::Completed);
    assert_eq!(
        cloud.steps[0].summary.as_deref(),
        Some("Browserbase, eu-central-1")
    );
}

#[tokio::test]
async fn a_failed_browserbase_capture_keeps_the_kind_of_its_failure_and_what_it_used() {
    for (failure, code, message) in [
        (
            CaptureError::RemoteLimit(
                "Browserbase is at its limit of browsers running at once (3 of 3, shared by everything that uses this project). Try again when one has finished.".into(),
            ),
            ErrorCode::BrowserbaseLimit,
            "Browserbase is at its limit of browsers running at once (3 of 3, shared by everything that uses this project). Try again when one has finished.",
        ),
        (
            CaptureError::RemoteAuth(
                "Browserbase did not accept the API key. Check it in Settings.".into(),
            ),
            ErrorCode::BrowserbaseAuth,
            "Browserbase did not accept the API key. Check it in Settings.",
        ),
        (
            CaptureError::Remote(
                "The connection to the Browserbase browser was lost. The browser has ended. Nothing was kept.".into(),
            ),
            ErrorCode::BrowserbaseFailed,
            "The connection to the Browserbase browser was lost. The browser has ended. Nothing was kept.",
        ),
    ] {
        assert_eq!(AppError::from(failure.clone()).code(), code);
        let harness = offline(CaptureBehavior::Fail(failure));
        connect(&harness).await;

        let detail = run_scan(&harness, in_frankfurt()).await;

        assert_eq!(detail.summary.status, ScanStatus::Failed);
        assert_eq!(detail.summary.error.as_deref(), Some(message));
        // The browser ran and is billed, whatever became of the scan.
        assert_eq!(
            detail.summary.remote.and_then(|usage| usage.session_seconds),
            Some(41.0)
        );
        assert_eq!(detail.environment, None);
        assert!(harness.store.open_sessions().await.unwrap().is_empty());
    }
}

#[tokio::test]
async fn the_system_proxy_is_dropped_for_a_browserbase_scan_with_a_word_in_the_audit_trail() {
    let harness = returning_a_cloud_capture();
    connect(&harness).await;
    let detail = run_scan(
        &harness,
        ScanOptions {
            proxy: Some(ProxyChoice::System),
            ..in_frankfurt()
        },
    )
    .await;
    assert_eq!(detail.options.proxy, Some(ProxyChoice::None));
    assert_eq!(
        detail.summary.proxy.as_ref().map(|proxy| proxy.kind),
        Some(ProxyKind::None)
    );
    let logs = harness.scans.logs(&detail.summary.id, None).await.unwrap();
    assert!(logs.iter().any(|entry| entry.message
        == "The system proxy of this Mac does not apply to a Browserbase browser: the visit leaves from Browserbase's eu-central-1 region."));
}

#[tokio::test]
async fn a_named_proxy_is_given_to_browserbase_with_its_password_outside_the_request() {
    let harness = returning_a_cloud_capture();
    connect(&harness).await;
    let settings = harness
        .settings
        .save_proxy_profile(belgium())
        .await
        .unwrap();
    let id = settings.proxies[0].id.clone();
    harness
        .settings
        .set_proxy_password(&id, "proxy-secret")
        .await
        .unwrap();

    let detail = run_scan(
        &harness,
        ScanOptions {
            proxy: Some(ProxyChoice::Profile {
                id,
                name: None,
                host: None,
            }),
            ..in_frankfurt()
        },
    )
    .await;

    assert_eq!(detail.summary.status, ScanStatus::Completed);
    let request = harness.capture.requests.lock().unwrap()[0].clone();
    assert!(matches!(
        &request.proxy,
        CaptureProxy::Custom { host, .. } if host == "be.proxy.example"
    ));
    assert_eq!(
        *harness.capture.proxy_passwords.lock().unwrap(),
        [Some("proxy-secret".to_string())]
    );
    assert!(
        !serde_json::to_string(&request)
            .unwrap()
            .contains("proxy-secret")
    );
    assert_eq!(visit_of(&detail).proxy_kind, BrowserbaseProxyKind::External);
    let logs = harness.scans.logs(&detail.summary.id, None).await.unwrap();
    assert!(logs.iter().any(|entry| {
        entry
            .message
            .ends_with("proxy \"Belgium\" (be.proxy.example), given to Browserbase)")
    }));
}

#[tokio::test]
async fn ads_txt_of_a_browserbase_scan_is_fetched_from_this_machine_and_the_step_says_so() {
    let harness = returning_a_cloud_capture();
    connect(&harness).await;
    let settings = harness
        .settings
        .save_proxy_profile(belgium())
        .await
        .unwrap();
    let id = settings.proxies[0].id.clone();

    let through_location = run_scan(
        &harness,
        ScanOptions {
            proxy: Some(location("BE")),
            ..in_frankfurt()
        },
    )
    .await;
    let through_own_proxy = run_scan(
        &harness,
        ScanOptions {
            proxy: Some(ProxyChoice::Profile {
                id,
                name: None,
                host: None,
            }),
            ..in_frankfurt()
        },
    )
    .await;

    let routes: Vec<CaptureProxy> = harness
        .files
        .requests
        .lock()
        .unwrap()
        .iter()
        .filter(|request| request.url == SITE_ADS_TXT_URL)
        .map(|request| request.proxy.route.clone())
        .collect();
    // A Browserbase exit cannot carry the app's own request.
    assert_eq!(routes[0], CaptureProxy::System);
    assert!(matches!(&routes[1], CaptureProxy::Custom { host, .. } if host == "be.proxy.example"));

    let said = |detail: &adscan_model::ScanDetail, text: &'static str| {
        let scan_id = detail.summary.id.clone();
        let scans = &harness.scans;
        async move {
            scans
                .logs(&scan_id, None)
                .await
                .unwrap()
                .iter()
                .any(|entry| {
                    entry.message.starts_with("Requesting") && entry.message.contains(text)
                })
        }
    };
    assert!(
        said(
            &through_location,
            "from this Mac, not from the Browserbase browser"
        )
        .await
    );
    assert!(
        said(
            &through_own_proxy,
            "through the scan's proxy, from this Mac"
        )
        .await
    );
}

#[tokio::test]
async fn proxy_traffic_is_counted_again_after_the_scan_without_holding_it_up() {
    let harness = returning_a_cloud_capture();
    connect(&harness).await;
    // Right after the browser ended Browserbase still counts zero.
    *harness.capture.remote_release.lock().unwrap() = Some((Some(41.0), Some(0.0)));
    *harness.browserbase_api.proxy_bytes.lock().unwrap() = Some(82_715.0);

    let detail = run_scan(
        &harness,
        ScanOptions {
            proxy: Some(location("BE")),
            ..in_frankfurt()
        },
    )
    .await;

    // The scan is done with the first reading.
    assert_eq!(detail.summary.status, ScanStatus::Completed);
    let visit = visit_of(&detail);
    assert_eq!(visit.proxy_kind, BrowserbaseProxyKind::Browserbase);
    assert_eq!(visit.proxy_country.as_deref(), Some("BE"));
    assert_eq!(
        detail
            .summary
            .proxy
            .as_ref()
            .and_then(|proxy| proxy.name.as_deref()),
        Some("BE")
    );

    // The UI hears of the second reading like of any other change of the
    // scan, once it is stored.
    let mut announced = false;
    for _ in 0..400 {
        announced = harness.events.events().iter().any(|event| {
            matches!(
                event,
                ScanEvent::ScanUpdated { summary }
                    if summary.remote.as_ref().and_then(|usage| usage.proxy_bytes) == Some(82_715.0)
            )
        });
        if announced {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(announced, "the second reading was never announced");
    assert_eq!(
        harness.browserbase_api.calls(),
        [format!("sessionUsage:{REMOTE_SESSION_ID}")]
    );
    let counted = harness.scans.get(&detail.summary.id).await.unwrap();
    assert_eq!(
        counted
            .summary
            .remote
            .as_ref()
            .and_then(|usage| usage.proxy_bytes),
        Some(82_715.0)
    );
    assert_eq!(visit_of(&counted).proxy_bytes, Some(82_715.0));
    assert_eq!(counted.summary.status, ScanStatus::Completed);
    assert_eq!(counted.findings.len(), detail.findings.len());
}

#[tokio::test]
async fn the_export_names_the_browserbase_visit() {
    let harness = returning_a_cloud_capture();
    connect(&harness).await;
    let detail = run_scan(
        &harness,
        ScanOptions {
            proxy: Some(location("BE")),
            ..in_frankfurt()
        },
    )
    .await;

    let markdown = harness
        .scans
        .export(&detail.summary.id, ExportFormat::Markdown)
        .await
        .unwrap();
    for line in [
        "- **Browser:** Browserbase, eu-central-1 (headless, not shown in its user agent), throwaway profile",
        "- **Proxy:** Browserbase residential exit, asked for BE",
        "- **Browserbase session:** session-1, Chrome/157.0.8103.79",
        "- **Browserbase settings sent:** CAPTCHA solving on, session video on, session log on, ad blocking off",
        "- **Timezone and language:** Europe/Berlin (set by the scanner to match the region; the browser reported America/Los_Angeles); language en-US",
        "- **Round trip to the browser:** 119 ms",
        "- **Browserbase usage:** 41 s of browser time, 0.0 MB through the proxy (billed by Browserbase, not part of the LLM cost)",
        "- **Automation signals the pages could see:** navigator.webdriver false",
    ] {
        assert!(markdown.contains(line), "missing `{line}` in:\n{markdown}");
    }
    assert!(!markdown.contains(BROWSERBASE_KEY));

    let exported: adscan_model::ScanDetail = serde_json::from_str(
        &harness
            .scans
            .export(&detail.summary.id, ExportFormat::Json)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(visit_of(&exported).session_id, REMOTE_SESSION_ID);
}

#[tokio::test]
async fn a_saved_configuration_carries_its_browser_choice() {
    let harness = returning_a_cloud_capture();
    // It must be able to start a scan today.
    let draft = |options: ScanOptions| ScanConfigurationDraft {
        id: None,
        name: "Pub in Frankfurt".into(),
        options,
    };
    assert_eq!(
        harness
            .configurations
            .save(draft(in_frankfurt()))
            .await
            .map(|saved| saved.id),
        Err(AppError::BrowserbaseAuth(
            "Browserbase is not set up. Add the API key in Settings.".into()
        ))
    );
    connect(&harness).await;
    assert_eq!(
        harness
            .configurations
            .save(draft(ScanOptions {
                url: "http://localhost:3000/".into(),
                ..in_frankfurt()
            }))
            .await
            .map(|saved| saved.id),
        Err(AppError::InvalidInput(
            "This address is on this Mac or your network; a cloud browser cannot reach it. Scan it with this Mac's Chrome."
                .into()
        ))
    );

    let saved = harness
        .configurations
        .save(draft(ScanOptions {
            proxy: Some(location("be")),
            ..in_frankfurt()
        }))
        .await
        .unwrap();
    assert_eq!(saved.options.browser, in_frankfurt().browser);
    assert_eq!(saved.options.proxy, Some(location("BE")));

    let started = harness.scans.start_configuration(&saved.id).await.unwrap();
    let detail = wait_for_end(&harness, &started.id).await;
    assert_eq!(detail.summary.status, ScanStatus::Completed);
    assert_eq!(visit_of(&detail).region, BrowserbaseRegion::EuCentral1);
    assert_eq!(
        detail
            .summary
            .configuration
            .map(|from| from.name)
            .as_deref(),
        Some("Pub in Frankfurt")
    );

    // A configuration that names no browser follows the settings of the
    // moment, like its other open choices.
    let open = harness
        .configurations
        .save(ScanConfigurationDraft {
            id: None,
            name: "Pub".into(),
            options: scan_options(false),
        })
        .await
        .unwrap();
    assert_eq!(open.options.browser, None);
}

// -- manual visits ----------------------------------------------------------------

fn manual_in_frankfurt() -> ScanOptions {
    ScanOptions {
        visitor: Some(VisitorBehavior::Manual),
        ..in_frankfurt()
    }
}

#[tokio::test]
async fn a_manual_browserbase_visit_opens_the_live_view_by_itself_and_again_on_request() {
    let harness = offline(CaptureBehavior::RecordUntilFinished(Box::new(
        cloud_capture(),
    )));
    connect(&harness).await;
    let live_view = format!("openLiveView:{REMOTE_SESSION_ID}");

    let started = harness.scans.start(manual_in_frankfurt()).await.unwrap();
    wait_for_capture(&harness, 1).await;
    {
        let request = &harness.capture.requests.lock().unwrap()[0];
        assert_eq!(request.visitor, VisitorBehavior::Manual);
        assert!(
            !request.accept_consent,
            "the consent choice is the person's"
        );
        assert!(matches!(request.browser, CaptureBrowser::Browserbase(_)));
    }
    // Opened once, when the visit was ready for a person.
    wait_for_call(&harness, &live_view, 1).await;
    // While it runs the session is on record as open.
    assert_eq!(
        harness
            .store
            .open_sessions()
            .await
            .unwrap()
            .into_iter()
            .map(|session| session.session_id)
            .collect::<Vec<_>>(),
        [REMOTE_SESSION_ID]
    );

    // "Open it again": by scan id, and nothing comes back but success.
    harness.scans.open_live_view(&started.id).await.unwrap();
    wait_for_call(&harness, &live_view, 2).await;

    harness.scans.finish_capture(&started.id).await.unwrap();
    let detail = wait_for_end(&harness, &started.id).await;
    assert_eq!(detail.summary.status, ScanStatus::Completed);
    assert_eq!(detail.environment.unwrap().visitor, VisitorBehavior::Manual);
    let logs = harness.scans.logs(&started.id, None).await.unwrap();
    assert!(logs.iter().any(|entry| entry.message
        == "Opened Browserbase's live view in your browser: browse there, then finish the visit here"));

    // Once the browser is gone there is nothing to show.
    let nothing_to_show =
        AppError::InvalidInput("This scan has no Browserbase browser to show right now.".into());
    assert_eq!(
        harness.scans.open_live_view(&started.id).await,
        Err(nothing_to_show.clone())
    );
    assert_eq!(
        harness.scans.open_live_view("nope").await,
        Err(AppError::NotFound("Scan nope".into()))
    );
    assert_eq!(
        harness
            .browserbase_api
            .calls()
            .iter()
            .filter(|call| **call == live_view)
            .count(),
        2
    );
}

#[tokio::test]
async fn a_live_view_that_cannot_be_opened_does_not_stop_the_visit_and_a_local_scan_has_none() {
    let harness = offline(CaptureBehavior::RecordUntilFinished(Box::new(
        cloud_capture(),
    )));
    connect(&harness).await;
    *harness.browserbase_api.failure.lock().unwrap() = Some(BrowserbaseError::Failed(
        "The live view could not be opened in your browser.".into(),
    ));

    let started = harness.scans.start(manual_in_frankfurt()).await.unwrap();
    wait_for_capture(&harness, 1).await;
    let warning = "Browserbase's live view could not be opened (The live view could not be opened in your browser.). Use \"Open live view\" to try again.";
    let mut warned = false;
    for _ in 0..400 {
        let logs = harness.scans.logs(&started.id, None).await.unwrap();
        if logs
            .iter()
            .any(|entry| entry.level == LogLevel::Warn && entry.message == warning)
        {
            warned = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(warned, "the failure was not reported");
    // Asked again, the failure is the caller's to see.
    assert_eq!(
        harness.scans.open_live_view(&started.id).await,
        Err(AppError::BrowserbaseFailed(
            "The live view could not be opened in your browser.".into()
        ))
    );
    harness.scans.finish_capture(&started.id).await.unwrap();
    assert_eq!(
        wait_for_end(&harness, &started.id).await.summary.status,
        ScanStatus::Completed
    );

    // A manual visit in this machine's Chrome has no live view.
    *harness.browserbase_api.failure.lock().unwrap() = None;
    let local = harness
        .scans
        .start(ScanOptions {
            visitor: Some(VisitorBehavior::Manual),
            ..scan_options(false)
        })
        .await
        .unwrap();
    wait_for_capture(&harness, 2).await;
    assert_eq!(
        harness.scans.open_live_view(&local.id).await,
        Err(AppError::InvalidInput(
            "This scan has no Browserbase browser to show right now.".into()
        ))
    );
    harness.scans.cancel(&local.id).await.unwrap();
    wait_for_end(&harness, &local.id).await;
}

/// The browser of a running scan belongs to the credentials the scan
/// started with. Removing them from the settings in the middle of a manual
/// visit does not take the live view away from it.
#[tokio::test]
async fn the_live_view_of_a_running_visit_opens_with_the_credentials_the_scan_started_with() {
    let harness = offline(CaptureBehavior::RecordUntilFinished(Box::new(
        cloud_capture(),
    )));
    connect(&harness).await;
    let live_view = format!("openLiveView:{REMOTE_SESSION_ID}");
    let started = harness.scans.start(manual_in_frankfurt()).await.unwrap();
    wait_for_capture(&harness, 1).await;
    wait_for_call(&harness, &live_view, 1).await;

    let settings = harness.browserbase.clear_credentials().await.unwrap();
    assert!(!settings.browserbase.connected);
    assert_eq!(
        harness.secrets.browserbase_credentials().await.unwrap(),
        None
    );

    assert_eq!(harness.scans.open_live_view(&started.id).await, Ok(()));
    wait_for_call(&harness, &live_view, 2).await;

    harness.scans.finish_capture(&started.id).await.unwrap();
    assert_eq!(
        wait_for_end(&harness, &started.id).await.summary.status,
        ScanStatus::Completed
    );
    // With the browser gone there is nothing to show, credentials or not.
    assert_eq!(
        harness.scans.open_live_view(&started.id).await,
        Err(AppError::InvalidInput(
            "This scan has no Browserbase browser to show right now.".into()
        ))
    );
}

/// What the capture reports last is that its browser has ended. A capture
/// whose reports can no longer be stored is stopped and still heard out:
/// the scan fails, and the record of its browser is closed with what the
/// browser used, instead of staying open until the next start.
#[tokio::test]
async fn a_capture_whose_reports_cannot_be_stored_is_still_heard_out_to_the_end_of_its_browser() {
    let harness = offline(CaptureBehavior::RecordUntilFinished(Box::new(
        cloud_capture(),
    )));
    connect(&harness).await;
    *harness.capture.last_activities.lock().unwrap() = vec![CaptureActivity {
        kind: ActivityKind::RemoteBrowser,
        message: "Browserbase ran 5 scripts of its own in the page".into(),
        page: None,
        data: Some(json!({ "event": "serviceScripts", "count": 5 })),
    }];
    let started = harness.scans.start(manual_in_frankfurt()).await.unwrap();
    wait_for_capture(&harness, 1).await;
    wait_for_call(&harness, &format!("openLiveView:{REMOTE_SESSION_ID}"), 1).await;
    // Everything the capture said so far is stored, the opened live view
    // included.
    for _ in 0..400 {
        let logs = harness.scans.logs(&started.id, None).await.unwrap();
        if logs
            .iter()
            .any(|entry| entry.message.starts_with("Opened Browserbase's live view"))
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert_eq!(harness.store.open_sessions().await.unwrap().len(), 1);

    harness
        .logs
        .refuse_appends
        .store(true, std::sync::atomic::Ordering::SeqCst);
    harness.scans.finish_capture(&started.id).await.unwrap();
    let detail = wait_for_end(&harness, &started.id).await;

    assert_eq!(detail.summary.status, ScanStatus::Failed);
    assert!(
        detail
            .summary
            .error
            .as_ref()
            .is_some_and(|error| error.contains("database or disk is full")),
        "{:?}",
        detail.summary.error
    );
    assert!(
        harness.store.open_sessions().await.unwrap().is_empty(),
        "the end of the browser was not put on record"
    );
    assert_eq!(
        detail.summary.remote,
        Some(RemoteUsage {
            region: BrowserbaseRegion::EuCentral1,
            session_seconds: Some(41.0),
            proxy_bytes: Some(0.0),
        })
    );
}

#[tokio::test]
async fn cancelling_a_manual_browserbase_visit_discards_it_and_still_books_the_browser_time() {
    let harness = offline(CaptureBehavior::RecordUntilFinished(Box::new(
        cloud_capture(),
    )));
    connect(&harness).await;
    let started = harness.scans.start(manual_in_frankfurt()).await.unwrap();
    wait_for_capture(&harness, 1).await;

    harness.scans.cancel(&started.id).await.unwrap();
    let detail = wait_for_end(&harness, &started.id).await;

    assert_eq!(detail.summary.status, ScanStatus::Cancelled);
    assert!(detail.pages.is_empty());
    assert_eq!(
        detail
            .summary
            .remote
            .and_then(|usage| usage.session_seconds),
        Some(41.0)
    );
    assert!(harness.store.open_sessions().await.unwrap().is_empty());
}

// -- the profile kept at Browserbase ----------------------------------------------

async fn keep_profiles_at_browserbase(harness: &Harness) {
    harness
        .settings
        .update(SettingsPatch {
            browserbase: Some(BrowserbaseSettingsPatch {
                persistent_profile: Some(true),
                ..BrowserbaseSettingsPatch::default()
            }),
            ..SettingsPatch::default()
        })
        .await
        .unwrap();
}

fn context_of(harness: &Harness, capture: usize) -> Option<String> {
    match &harness.capture.requests.lock().unwrap()[capture].browser {
        CaptureBrowser::Browserbase(cloud) => cloud.context_id.clone(),
        CaptureBrowser::Local => panic!("capture {capture} was local"),
    }
}

#[tokio::test]
async fn the_profile_kept_at_browserbase_is_one_context_per_device_made_on_first_use() {
    let harness = returning_a_cloud_capture();
    connect(&harness).await;
    let nothing = harness.browserbase.profile().await.unwrap();
    assert!(!nothing.exists && !nothing.in_use && nothing.last_used_at.is_none());

    // Off by default, even with the local profile switched on.
    harness
        .settings
        .update(SettingsPatch {
            persistent_profile: Some(true),
            ..SettingsPatch::default()
        })
        .await
        .unwrap();
    let throwaway = run_scan(&harness, in_frankfurt()).await;
    assert_eq!(throwaway.options.persistent_profile, Some(false));
    assert_eq!(context_of(&harness, 0), None);
    assert!(harness.browserbase_api.calls().is_empty());

    keep_profiles_at_browserbase(&harness).await;
    let first = run_scan(&harness, in_frankfurt()).await;
    assert_eq!(first.options.persistent_profile, Some(true));
    assert!(first.environment.is_some());
    assert_eq!(context_of(&harness, 1).as_deref(), Some("context-1"));
    assert!(harness.capture.requests.lock().unwrap()[1].persistent_profile);
    // The next desktop scan uses the same context; a phone gets its own.
    run_scan(&harness, in_frankfurt()).await;
    assert_eq!(context_of(&harness, 2).as_deref(), Some("context-1"));
    run_scan(
        &harness,
        ScanOptions {
            device: DeviceProfile::Mobile,
            ..in_frankfurt()
        },
    )
    .await;
    assert_eq!(context_of(&harness, 3).as_deref(), Some("context-2"));
    // A scan that asks for none gets none.
    run_scan(
        &harness,
        ScanOptions {
            persistent_profile: Some(false),
            ..in_frankfurt()
        },
    )
    .await;
    assert_eq!(context_of(&harness, 4), None);
    assert_eq!(
        harness.browserbase_api.calls(),
        ["createContext", "createContext"]
    );

    let kept = harness.browserbase.profile().await.unwrap();
    assert!(kept.exists && !kept.in_use && kept.last_used_at.is_some());

    let cleared = harness.browserbase.clear_profile().await.unwrap();
    assert!(!cleared.exists && cleared.last_used_at.is_none());
    assert_eq!(
        harness.browserbase_api.calls()[2..],
        ["deleteContext:context-1", "deleteContext:context-2"]
    );
    // Clearing what is not there asks Browserbase nothing.
    harness.browserbase.clear_profile().await.unwrap();
    assert_eq!(harness.browserbase_api.calls().len(), 4);
}

#[tokio::test]
async fn scans_in_the_browserbase_profile_run_one_at_a_time_apart_from_the_local_profile() {
    let harness = offline(CaptureBehavior::RecordUntilFinished(Box::new(
        cloud_capture(),
    )));
    connect(&harness).await;
    keep_profiles_at_browserbase(&harness).await;

    let first = harness.scans.start(manual_in_frankfurt()).await.unwrap();
    wait_for_capture(&harness, 1).await;
    assert!(harness.browserbase.profile().await.unwrap().in_use);
    // The profile cannot be cleared from under a scan.
    assert_eq!(
        harness.browserbase.clear_profile().await,
        Err(AppError::ProfileBusy)
    );

    // A scan in this machine's kept profile is not held up by it: two
    // different profiles.
    let local = harness
        .scans
        .start(ScanOptions {
            visitor: Some(VisitorBehavior::Manual),
            persistent_profile: Some(true),
            ..scan_options(false)
        })
        .await
        .unwrap();
    wait_for_capture(&harness, 2).await;
    harness.scans.finish_capture(&local.id).await.unwrap();
    wait_for_end(&harness, &local.id).await;

    // The second Browserbase scan waits at its capture step.
    let second = harness.scans.start(manual_in_frankfurt()).await.unwrap();
    let waiting = "Waiting for another scan to release the browser profile";
    let mut waited = false;
    for _ in 0..400 {
        let logs = harness.scans.logs(&second.id, None).await.unwrap();
        if logs.iter().any(|entry| entry.message == waiting) {
            waited = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(waited, "the second scan never reached the profile queue");
    assert_eq!(harness.capture.requests.lock().unwrap().len(), 2);

    harness.scans.finish_capture(&first.id).await.unwrap();
    wait_for_capture(&harness, 3).await;
    harness.scans.finish_capture(&second.id).await.unwrap();
    for id in [&first.id, &second.id] {
        assert_eq!(
            wait_for_end(&harness, id).await.summary.status,
            ScanStatus::Completed
        );
    }
    assert_eq!(context_of(&harness, 0), context_of(&harness, 2));
    assert!(!harness.browserbase.profile().await.unwrap().in_use);
}

#[tokio::test]
async fn removing_the_credentials_takes_the_kept_profile_along() {
    let harness = returning_a_cloud_capture();
    connect(&harness).await;
    keep_profiles_at_browserbase(&harness).await;
    run_scan(&harness, in_frankfurt()).await;
    assert!(harness.browserbase.profile().await.unwrap().exists);

    harness.browserbase.clear_credentials().await.unwrap();

    // Deleted at Browserbase while the key could still do it.
    assert!(
        harness
            .browserbase_api
            .calls()
            .contains(&"deleteContext:context-1".to_string())
    );
    assert!(!harness.browserbase.profile().await.unwrap().exists);
    assert_eq!(
        harness
            .browserbase
            .clear_profile()
            .await
            .map(|info| info.exists),
        Ok(false)
    );

    // Unreachable Browserbase: the key still goes, and the record with it.
    connect(&harness).await;
    run_scan(&harness, in_frankfurt()).await;
    *harness.browserbase_api.failure.lock().unwrap() = Some(BrowserbaseError::Failed(
        "Browserbase could not be reached".into(),
    ));
    let cleared = harness.browserbase.clear_credentials().await.unwrap();
    assert!(!cleared.browserbase.connected);
    assert!(!harness.browserbase.profile().await.unwrap().exists);
}

/// A scan waits for its turn in the kept profile with the credentials it
/// started with. When those are removed, or exchanged for another
/// project's, while it waits, a context made with them could never be
/// deleted by the app again, and a context on record by then is another
/// project's. The scan runs, in a browser that keeps nothing, and says so.
#[tokio::test]
async fn a_scan_that_waited_while_the_credentials_went_runs_without_the_kept_profile() {
    let harness = offline(CaptureBehavior::RecordUntilFinished(Box::new(
        cloud_capture(),
    )));
    connect(&harness).await;
    keep_profiles_at_browserbase(&harness).await;
    let contexts_made = |harness: &Harness| {
        harness
            .browserbase_api
            .calls()
            .iter()
            .filter(|call| *call == "createContext")
            .count()
    };
    let waiting = "Waiting for another scan to release the browser profile";
    let wait_for_log = |scan_id: String| {
        let harness = &harness;
        async move {
            for _ in 0..400 {
                let logs = harness.scans.logs(&scan_id, None).await.unwrap();
                if logs.iter().any(|entry| entry.message == waiting) {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            panic!("the scan never reached the profile queue");
        }
    };

    let first = harness.scans.start(manual_in_frankfurt()).await.unwrap();
    wait_for_capture(&harness, 1).await;
    assert_eq!(context_of(&harness, 0).as_deref(), Some("context-1"));
    let second = harness.scans.start(manual_in_frankfurt()).await.unwrap();
    wait_for_log(second.id.clone()).await;

    // Disconnected while the second scan waits for its turn.
    harness.browserbase.clear_credentials().await.unwrap();
    assert!(!harness.browserbase.profile().await.unwrap().exists);
    harness.scans.finish_capture(&first.id).await.unwrap();
    wait_for_capture(&harness, 2).await;

    assert_eq!(context_of(&harness, 1), None);
    assert!(
        !harness.capture.requests.lock().unwrap()[1].persistent_profile,
        "a browser without a context keeps no profile"
    );
    assert_eq!(
        contexts_made(&harness),
        1,
        "{:?}",
        harness.browserbase_api.calls()
    );
    assert!(
        !harness.browserbase.profile().await.unwrap().exists,
        "no profile is on record while Browserbase is disconnected"
    );
    harness.scans.finish_capture(&second.id).await.unwrap();
    let detail = wait_for_end(&harness, &second.id).await;
    assert_eq!(detail.summary.status, ScanStatus::Completed);
    assert!(!detail.environment.unwrap().persistent_profile);
    let logs = harness.scans.logs(&second.id, None).await.unwrap();
    assert!(
        logs.iter().any(|entry| entry.message
            == "Browserbase was disconnected, or connected to another project, after this scan was started: it runs without the browser profile kept there"),
        "{:#?}",
        logs.iter().map(|entry| &entry.message).collect::<Vec<_>>()
    );

    // Connected again, a new scan makes a profile as on first use.
    connect(&harness).await;
    let third = harness.scans.start(manual_in_frankfurt()).await.unwrap();
    wait_for_capture(&harness, 3).await;
    assert_eq!(context_of(&harness, 2).as_deref(), Some("context-2"));
    harness.scans.finish_capture(&third.id).await.unwrap();
    wait_for_end(&harness, &third.id).await;
}

#[tokio::test]
async fn a_profile_cannot_be_cleared_without_credentials_or_when_browserbase_refuses() {
    let harness = returning_a_cloud_capture();
    connect(&harness).await;
    keep_profiles_at_browserbase(&harness).await;
    run_scan(&harness, in_frankfurt()).await;

    *harness.browserbase_api.failure.lock().unwrap() = Some(BrowserbaseError::Failed(
        "Browserbase is not available right now (HTTP 503).".into(),
    ));
    assert_eq!(
        harness.browserbase.clear_profile().await,
        Err(AppError::BrowserbaseFailed(
            "Browserbase is not available right now (HTTP 503).".into()
        ))
    );
    // Nothing was forgotten: it can be tried again.
    assert!(harness.browserbase.profile().await.unwrap().exists);
    *harness.browserbase_api.failure.lock().unwrap() = None;
    assert!(!harness.browserbase.clear_profile().await.unwrap().exists);
}

// -- sessions an earlier run left behind ------------------------------------------

#[tokio::test]
async fn a_session_whose_end_was_never_confirmed_is_released_at_the_next_start() {
    let harness = offline(CaptureBehavior::Fail(CaptureError::Remote(
        "The connection to the Browserbase browser was lost. The browser has ended. Nothing was kept.".into(),
    )));
    connect(&harness).await;
    // The capture could not confirm that its browser ended.
    *harness.capture.remote_release.lock().unwrap() = None;

    let detail = run_scan(&harness, in_frankfurt()).await;
    assert_eq!(detail.summary.status, ScanStatus::Failed);
    assert_eq!(
        detail.summary.remote, None,
        "nothing is known about its use"
    );
    let open = harness.store.open_sessions().await.unwrap();
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].session_id, REMOTE_SESSION_ID);

    // The next start: Browserbase still runs it, under this installation's id.
    let install_id = harness.store.install_id().await.unwrap();
    *harness.browserbase_api.running.lock().unwrap() = vec![REMOTE_SESSION_ID.to_string()];
    assert_eq!(
        harness
            .browserbase
            .release_abandoned_sessions(f64::INFINITY)
            .await,
        Ok(1)
    );
    // Asked about by its own id, with this installation's id to check it
    // against, and released on that answer.
    assert_eq!(
        harness.browserbase_api.calls(),
        [
            format!("sessionStanding:{REMOTE_SESSION_ID}:{install_id}"),
            format!("release:{REMOTE_SESSION_ID}"),
        ]
    );
    assert!(harness.store.open_sessions().await.unwrap().is_empty());

    // And the start after that has nothing to do.
    assert_eq!(
        harness
            .browserbase
            .release_abandoned_sessions(f64::INFINITY)
            .await,
        Ok(0)
    );
    assert_eq!(harness.browserbase_api.calls().len(), 2);
}

#[tokio::test]
async fn the_sweep_only_releases_sessions_that_are_on_record_and_reported_as_this_installations() {
    let harness = returning_a_cloud_capture();
    connect(&harness).await;
    let now = adscan_engine::application::ports::Clock::now_ms(
        &adscan_engine::infrastructure::system::SystemClock,
    );
    harness
        .store
        .session_created("ours-running", now)
        .await
        .unwrap();
    harness
        .store
        .session_created("ours-ended", now)
        .await
        .unwrap();
    // On record here, but at Browserbase it carries someone else's tags:
    // the record is no licence to end it.
    harness
        .store
        .session_created("tagged-as-someone-elses", now)
        .await
        .unwrap();
    // On record, and unknown to Browserbase under this key.
    harness
        .store
        .session_created("unknown-to-this-key", now)
        .await
        .unwrap();
    // Browserbase reports one of ours and one this record never saw start:
    // that one is not this app's to end, whatever its metadata says.
    *harness.browserbase_api.running.lock().unwrap() =
        vec!["ours-running".to_string(), "not-on-record".to_string()];
    *harness.browserbase_api.running_for_others.lock().unwrap() =
        vec!["tagged-as-someone-elses".to_string()];

    assert_eq!(
        harness
            .browserbase
            .release_abandoned_sessions(f64::INFINITY)
            .await,
        Ok(1)
    );

    let calls = harness.browserbase_api.calls();
    let releases: Vec<&String> = calls
        .iter()
        .filter(|call| call.starts_with("release:"))
        .collect();
    assert_eq!(releases, ["release:ours-running"]);
    // Only what is on record was asked about, each by its own id.
    let install_id = harness.store.install_id().await.unwrap();
    let mut asked_about: Vec<&String> = calls
        .iter()
        .filter(|call| call.starts_with("sessionStanding:"))
        .collect();
    asked_about.sort();
    assert_eq!(
        asked_about,
        [
            &format!("sessionStanding:ours-ended:{install_id}"),
            &format!("sessionStanding:ours-running:{install_id}"),
            &format!("sessionStanding:tagged-as-someone-elses:{install_id}"),
            &format!("sessionStanding:unknown-to-this-key:{install_id}"),
        ]
    );
    assert_eq!(
        *harness.browserbase_api.running.lock().unwrap(),
        ["not-on-record"]
    );
    assert!(harness.store.open_sessions().await.unwrap().is_empty());
}

/// A session leaves the record when Browserbase has said what became of
/// it, not before: one it could not be asked about stays for the next
/// start, and does not keep the others from being seen to.
#[tokio::test]
async fn a_session_the_sweep_could_not_ask_about_stays_on_record_while_the_others_are_settled() {
    let harness = returning_a_cloud_capture();
    connect(&harness).await;
    let now = adscan_engine::application::ports::Clock::now_ms(
        &adscan_engine::infrastructure::system::SystemClock,
    );
    for (session, at) in [
        ("unreadable", now - 3.0),
        ("running", now - 2.0),
        ("ended", now - 1.0),
    ] {
        harness.store.session_created(session, at).await.unwrap();
    }
    *harness.browserbase_api.running.lock().unwrap() =
        vec!["unreadable".to_string(), "running".to_string()];
    *harness.browserbase_api.unreadable.lock().unwrap() = vec!["unreadable".to_string()];
    let open = |harness: &Harness| {
        let store = harness.store.clone();
        async move {
            store
                .open_sessions()
                .await
                .unwrap()
                .into_iter()
                .map(|session| session.session_id)
                .collect::<Vec<_>>()
        }
    };

    assert_eq!(
        harness
            .browserbase
            .release_abandoned_sessions(f64::INFINITY)
            .await,
        Err(AppError::BrowserbaseFailed(
            "Browserbase is not available right now (HTTP 503).".into()
        ))
    );
    assert_eq!(open(&harness).await, ["unreadable"]);
    assert!(
        harness
            .browserbase_api
            .calls()
            .contains(&"release:running".to_string())
    );

    // The next start asks again, and this time gets an answer.
    harness.browserbase_api.unreadable.lock().unwrap().clear();
    assert_eq!(
        harness
            .browserbase
            .release_abandoned_sessions(f64::INFINITY)
            .await,
        Ok(1)
    );
    assert!(open(&harness).await.is_empty());
    assert!(
        harness
            .browserbase_api
            .calls()
            .contains(&"release:unreadable".to_string())
    );
}

#[tokio::test]
async fn the_sweep_asks_browserbase_nothing_without_open_sessions_credentials_or_a_live_session() {
    let hour = 3_600_000.0;
    let now = adscan_engine::application::ports::Clock::now_ms(
        &adscan_engine::infrastructure::system::SystemClock,
    );

    // Nothing on record: nothing happens.
    let clean = returning_a_cloud_capture();
    connect(&clean).await;
    assert_eq!(
        clean
            .browserbase
            .release_abandoned_sessions(f64::INFINITY)
            .await,
        Ok(0)
    );
    assert!(clean.browserbase_api.calls().is_empty());

    // A session older than Browserbase lets any session live has ended by
    // itself: its row goes without a question.
    let expired = returning_a_cloud_capture();
    connect(&expired).await;
    expired
        .store
        .session_created("ancient", now - 7.0 * hour)
        .await
        .unwrap();
    assert_eq!(
        expired
            .browserbase
            .release_abandoned_sessions(f64::INFINITY)
            .await,
        Ok(0)
    );
    assert!(expired.browserbase_api.calls().is_empty());
    assert!(expired.store.open_sessions().await.unwrap().is_empty());

    // Without the key nothing can be done; the rows go, so the next start
    // does not look for the key again.
    let disconnected = returning_a_cloud_capture();
    disconnected
        .store
        .session_created("orphan", now)
        .await
        .unwrap();
    assert_eq!(
        disconnected
            .browserbase
            .release_abandoned_sessions(f64::INFINITY)
            .await,
        Ok(0)
    );
    assert!(disconnected.browserbase_api.calls().is_empty());
    assert!(disconnected.store.open_sessions().await.unwrap().is_empty());
}

#[tokio::test]
async fn the_sweep_leaves_the_browser_of_a_scan_that_started_since_the_app_came_up() {
    let harness = returning_a_cloud_capture();
    connect(&harness).await;
    let app_started_at = 1_000_000.0;
    harness
        .store
        .session_created("from-the-run-before", app_started_at - 60_000.0)
        .await
        .unwrap();
    harness
        .store
        .session_created("of-a-scan-running-now", app_started_at + 5.0)
        .await
        .unwrap();
    *harness.browserbase_api.running.lock().unwrap() = vec![
        "from-the-run-before".to_string(),
        "of-a-scan-running-now".to_string(),
    ];

    // The earlier one is older than any session can be; the later one is
    // not from before this start. Neither is asked about.
    assert_eq!(
        harness
            .browserbase
            .release_abandoned_sessions(app_started_at)
            .await,
        Ok(0)
    );
    assert!(harness.browserbase_api.calls().is_empty());
    assert_eq!(
        harness
            .store
            .open_sessions()
            .await
            .unwrap()
            .into_iter()
            .map(|session| session.session_id)
            .collect::<Vec<_>>(),
        ["of-a-scan-running-now"]
    );
}

#[tokio::test]
async fn a_sweep_that_cannot_reach_browserbase_keeps_its_record_for_the_next_start() {
    let harness = returning_a_cloud_capture();
    connect(&harness).await;
    let now = adscan_engine::application::ports::Clock::now_ms(
        &adscan_engine::infrastructure::system::SystemClock,
    );
    harness.store.session_created("ours", now).await.unwrap();
    *harness.browserbase_api.running.lock().unwrap() = vec!["ours".to_string()];
    *harness.browserbase_api.failure.lock().unwrap() = Some(BrowserbaseError::Failed(
        "Browserbase could not be reached".into(),
    ));

    assert_eq!(
        harness
            .browserbase
            .release_abandoned_sessions(f64::INFINITY)
            .await,
        Err(AppError::BrowserbaseFailed(
            "Browserbase could not be reached".into()
        ))
    );
    assert_eq!(harness.store.open_sessions().await.unwrap().len(), 1);

    *harness.browserbase_api.failure.lock().unwrap() = None;
    assert_eq!(
        harness
            .browserbase
            .release_abandoned_sessions(f64::INFINITY)
            .await,
        Ok(1)
    );
    assert!(harness.store.open_sessions().await.unwrap().is_empty());
}
