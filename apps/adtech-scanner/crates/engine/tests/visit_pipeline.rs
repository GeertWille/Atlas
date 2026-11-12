//! Visits through the whole pipeline and the use cases around them: page
//! views and the environment in the stored scan, visitor activity in the
//! audit trail, finishing a manual visit, the shared scan profile, proxy
//! profiles with their passwords, and saved scan configurations.

mod common;

use std::time::Duration;

use adscan_engine::application::AppError;
use adscan_engine::application::ports::SecretStore;
use adscan_model::{
    ActivityKind, CaptureActivity, CaptureEnvironment, CaptureProxy, DeviceProfile, ErrorCode,
    ExportFormat, LogKind, LogLevel, PageReach, PipelineNode, ProxyChoice, ProxyKind,
    ProxyProfileDraft, ProxyScheme, ProxyUse, ScanConfigurationDraft, ScanListQuery, ScanOptions,
    ScanStatus, Settings, SettingsPatch, StepStatus, TrackerEvent, VisitorBehavior,
};
use common::*;
use serde_json::json;

const OFFLINE: &str = "http://127.0.0.1:1";

/// A browse visit of two pages: header bidding works on the first and is
/// skipped on the second.
fn two_page_capture() -> adscan_model::Capture {
    let mut first = page_setup();
    first.extend(auction_round(360.0, "auction-1"));
    first.push(lifecycle(950.0, "load"));
    first.extend(scroll_events(4000.0));
    let mut second: Vec<TrackerEvent> = page_setup();
    for slot in ["top", "side", "footer"] {
        second.push(slot_requested(600.0, slot, false));
        second.push(slot_rendered(900.0, slot, true));
    }
    let mut capture = visit(vec![
        (
            page_view("https://pub.example/", PageReach::Entry, 0.0, 12_000.0),
            first,
        ),
        (
            page_view(
                "https://pub.example/sport/final",
                PageReach::LinkClick,
                12_400.0,
                20_400.0,
            ),
            second,
        ),
    ]);
    capture.environment = CaptureEnvironment {
        headless: true,
        persistent_profile: false,
        webdriver: Some(true),
        user_agent_overridden: false,
        viewport_overridden: true,
        request_hosts: strings(&["pub.doubleverify.com", "cdn.pub.example"]),
    };
    capture
}

/// A shown browse visit whose entry page had a consent wall the scanner
/// could not accept. A person clicked it 27 s in; the ad stack loaded then,
/// and the page view ended three seconds later. The second page is healthy.
fn capture_with_consent_given_by_hand() -> adscan_model::Capture {
    let tcf = |t: f64, status: &str| {
        ev(
            t,
            adscan_model::TrackerEventKind::TcfState {
                event_status: Some(status.into()),
                gdpr_applies: Some(true),
                has_tc_string: status != "cmpuishown",
                purpose_consents: vec![],
                vendor_consents: vec![],
            },
        )
    };
    let mut first = vec![
        lifecycle(950.0, "load"),
        tcf(400.0, "cmpuishown"),
        tcf(27_000.0, "useractioncomplete"),
        prebid_detected(27_300.0, "pbjs", "10.25.0"),
    ];
    first.extend(scroll_events(28_000.0));
    let mut second = page_setup();
    second.push(tcf(100.0, "tcloaded"));
    second.extend(auction_round(360.0, "auction-1"));
    second.push(lifecycle(950.0, "load"));
    let mut capture = visit(vec![
        (
            page_view("https://pub.example/", PageReach::Entry, 0.0, 30_000.0),
            first,
        ),
        (
            page_view(
                "https://pub.example/sport/final",
                PageReach::LinkClick,
                30_400.0,
                60_400.0,
            ),
            second,
        ),
    ]);
    capture.consent_notes = vec![adscan_model::ConsentNote {
        page: 0,
        t: 900.0,
        kind: adscan_model::ConsentNoteKind::DialogWithoutAccept,
        banner: "Sourcepoint".into(),
    }];
    capture.environment = CaptureEnvironment {
        webdriver: Some(true),
        ..CaptureEnvironment::default()
    };
    capture
}

fn browse_options() -> ScanOptions {
    ScanOptions {
        visitor: Some(VisitorBehavior::Browse),
        extra_pages: Some(1),
        headless: Some(true),
        seed: Some(77),
        ..scan_options(false)
    }
}

fn offline(behavior: CaptureBehavior) -> Harness {
    harness(behavior, OFFLINE, None)
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

fn profile_choice(id: &str) -> ProxyChoice {
    ProxyChoice::Profile {
        id: id.into(),
        name: None,
        host: None,
    }
}

// -- the stored scan -------------------------------------------------------------

#[tokio::test]
async fn consent_the_scanner_failed_to_give_is_in_the_overview_the_findings_and_the_audit_trail() {
    let harness = offline(CaptureBehavior::Return(Box::new(
        capture_with_consent_given_by_hand(),
    )));
    let options = ScanOptions {
        headless: Some(false),
        ..browse_options()
    };
    let detail = run_scan(&harness, options).await;

    let account = "On page 1 a consent dialog was showing from 0.4 s and the scanner did not accept it \
        (it found a Sourcepoint dialog with no control that plainly means \"accept all\"). Consent was \
        given by hand in the browser window 27.0 s into the page view.";
    let overview = detail.overview.as_deref().unwrap();
    assert!(
        overview.contains(&format!("**Note:** {account}")),
        "{overview}"
    );
    let waited = detail
        .findings
        .iter()
        .find(|finding| {
            finding
                .title
                .starts_with("Bidding on page 1 (/) waited for a consent choice")
        })
        .expect("the wait is a finding");
    assert_eq!(waited.pages, [0]);
    // The page view that had three seconds for its ads is not held against
    // the site, and nothing lowers the score for it.
    assert!(
        detail
            .findings
            .iter()
            .all(|finding| !finding.title.contains("no auction ever started")),
        "{:#?}",
        detail
            .findings
            .iter()
            .map(|finding| &finding.title)
            .collect::<Vec<_>>()
    );
    let logs = harness.scans.logs(&detail.summary.id, None).await.unwrap();
    assert!(
        logs.iter()
            .any(|entry| entry.level == LogLevel::Warn && entry.message.starts_with(account)),
        "the audit trail says it too"
    );
}

#[tokio::test]
async fn a_multi_page_visit_is_stored_with_its_page_views_environment_and_findings() {
    let harness = offline(CaptureBehavior::Return(Box::new(two_page_capture())));
    let detail = run_scan(&harness, browse_options()).await;

    assert_eq!(detail.summary.status, ScanStatus::Completed);
    let request = harness.capture.requests.lock().unwrap()[0].clone();
    assert_eq!(
        (
            request.visitor,
            request.extra_pages,
            request.seed,
            request.headless
        ),
        (VisitorBehavior::Browse, 1, 77, true)
    );

    // What the scan was started with is stored fully settled.
    assert_eq!(detail.options.visitor, Some(VisitorBehavior::Browse));
    assert_eq!(detail.options.extra_pages, Some(1));
    assert_eq!(detail.options.seed, Some(77));
    assert_eq!(detail.options.headless, Some(true));
    assert_eq!(detail.options.persistent_profile, Some(false));
    assert_eq!(detail.options.proxy, Some(ProxyChoice::System));
    assert_eq!(detail.options.budget_usd, Some(0.5));
    assert!(detail.options.auto_scroll);

    // Page views
    assert_eq!(detail.pages.len(), 2);
    let second = &detail.pages[1];
    assert_eq!(second.index, 1);
    assert_eq!(second.url, "https://pub.example/sport/final");
    assert_eq!(second.reached_by, PageReach::LinkClick);
    assert_eq!((second.started_at_ms, second.dwell_ms), (12_400.0, 8_000.0));
    assert_eq!(second.metrics.auction_count, 0);
    assert_eq!(second.metrics.slot_count, 3);
    assert_eq!(second.metrics.empty_slot_count, 3);
    assert_eq!(detail.pages[0].metrics.auction_count, 1);
    assert_eq!(detail.metrics.slot_count, 6);
    assert_eq!(
        detail
            .slots
            .iter()
            .map(|slot| slot.page)
            .collect::<Vec<_>>(),
        [0, 0, 0, 1, 1, 1]
    );
    assert!(detail.scripts.iter().all(|script| script.pages == [0]));

    // Environment: what the scan asked for and what the browser reported.
    let environment = detail.environment.as_ref().unwrap();
    assert_eq!(environment.visitor, VisitorBehavior::Browse);
    assert_eq!(environment.seed, 77);
    assert!(environment.headless && !environment.persistent_profile);
    assert_eq!(environment.webdriver, Some(true));
    assert_eq!(environment.detectors.len(), 1);
    assert_eq!(environment.detectors[0].vendor, "DoubleVerify");
    let system = ProxyUse {
        kind: ProxyKind::System,
        name: None,
        host: None,
    };
    assert_eq!(environment.proxy, system);
    assert_eq!(detail.summary.proxy, Some(system));

    // Findings: the multi-page rule names the page view it was seen on.
    let later = detail
        .findings
        .iter()
        .find(|finding| finding.title == "Later page views are sold without header bidding")
        .expect("the second page view lost its bids");
    assert_eq!(later.pages, [1]);
    let automated = detail
        .findings
        .iter()
        .find(|finding| finding.title == "This scan was identifiable as automated traffic")
        .expect("webdriver and a detection vendor");
    assert!(automated.pages.is_empty());
    let overview = detail.overview.as_deref().unwrap();
    assert!(
        overview.contains("The visit covered 2 page views"),
        "{overview}"
    );
    // Every bidder bid in this scan: the finding stays as information, and
    // the overview does not offer automation as an explanation for anything.
    assert!(!overview.contains("automated"), "{overview}");
    assert_eq!(
        step_summary(&detail, PipelineNode::Capture),
        "Captured 2 page views, 5 scripts and 46 tracker events from https://pub.example/"
    );
    assert!(
        step_summary(&detail, PipelineNode::Telemetry)
            .starts_with("2 page views, 1 auctions, 6 slots")
    );

    // Reopening the store gives the same scan back.
    let reloaded = harness.scans.get(&detail.summary.id).await.unwrap();
    assert_eq!(reloaded, detail);
}

fn step_summary(detail: &adscan_model::ScanDetail, node: PipelineNode) -> &str {
    detail
        .steps
        .iter()
        .find(|step| step.node == node)
        .and_then(|step| step.summary.as_deref())
        .unwrap()
}

#[tokio::test]
async fn the_export_of_a_multi_page_visit_names_page_views_and_where_findings_were_seen() {
    let harness = offline(CaptureBehavior::Return(Box::new(two_page_capture())));
    let detail = run_scan(&harness, browse_options()).await;
    let id = &detail.summary.id;

    let markdown = harness
        .scans
        .export(id, ExportFormat::Markdown)
        .await
        .unwrap();
    for expected in [
        "## Visit\n",
        "- **Visitor:** Browsed like a visitor, following links (seed 77)",
        "- **Browser:** headless, throwaway profile",
        "- **Proxy:** system setting",
        "navigator.webdriver true",
        "- **Detection vendors on the page:** DoubleVerify (pub.doubleverify.com)",
        "### Page views\n",
        "| 2 | https://pub.example/sport/final | LinkClick | 12400 | 8000 | 950 | 0 | 3 (3) | 0% |",
        "**Seen on:** page 2 (https://pub.example/sport/final)",
        "### Page 1 (https://pub.example/)",
        "### Page 2 (https://pub.example/sport/final)",
    ] {
        assert!(
            markdown.contains(expected),
            "missing `{expected}` in:\n{markdown}"
        );
    }

    let exported: serde_json::Value =
        serde_json::from_str(&harness.scans.export(id, ExportFormat::Json).await.unwrap()).unwrap();
    assert_eq!(exported["pages"].as_array().unwrap().len(), 2);
    assert_eq!(exported["pages"][1]["reachedBy"], "linkClick");
    assert_eq!(exported["environment"]["visitor"], "browse");
    assert_eq!(exported["environment"]["proxy"]["kind"], "system");
    assert_eq!(exported["slots"][3]["page"], 1);
}

// -- the audit trail of the visit ----------------------------------------------------

#[tokio::test]
async fn visitor_activity_is_kept_in_the_audit_trail_apart_from_progress() {
    let harness = offline(CaptureBehavior::Return(Box::new(two_page_capture())));
    *harness.capture.activities.lock().unwrap() = vec![
        CaptureActivity {
            kind: ActivityKind::Environment,
            message:
                "This visit is identifiable as automated: navigator.webdriver is true, headless"
                    .into(),
            page: None,
            data: Some(json!({ "webdriver": true, "headless": true })),
        },
        CaptureActivity {
            kind: ActivityKind::PageView,
            message: "Page 1: /".into(),
            page: Some(0),
            data: Some(json!({ "index": 0, "url": "https://pub.example/", "reachedBy": "entry" })),
        },
        CaptureActivity {
            kind: ActivityKind::Action,
            message: "Scrolled to 60%".into(),
            page: Some(0),
            data: Some(json!({ "action": "scroll", "scrollDepth": 0.6 })),
        },
        CaptureActivity {
            kind: ActivityKind::LinkChoice,
            message: "Following link: /sport/final (best of 2)".into(),
            page: Some(0),
            data: Some(json!({
                "chosen": { "text": "Final", "href": "https://pub.example/sport/final", "score": 6, "reasons": ["content link"] },
                "alternatives": [],
                "skipped": { "off-site": 9 }
            })),
        },
        CaptureActivity {
            kind: ActivityKind::Blocked,
            message: "Left the scanned site (partner.example): not recorded".into(),
            page: Some(1),
            data: Some(json!({ "reason": "off-site", "host": "partner.example" })),
        },
    ];
    let detail = run_scan(&harness, browse_options()).await;
    let capture_step = detail
        .steps
        .iter()
        .find(|step| step.node == PipelineNode::Capture)
        .unwrap();
    let logs = harness
        .scans
        .logs(&detail.summary.id, Some(&capture_step.id))
        .await
        .unwrap();

    assert!(
        logs[0].message.starts_with(
            "Loading https://pub.example/ as Desktop: browsing like a visitor, 10 s per page and up to 1 further page \
             (seed 77, headless, throwaway profile, accept consent true, system proxy)"
        ),
        "{}",
        logs[0].message
    );
    // Plain progress stays a message.
    assert_eq!(
        (logs[1].kind, logs[1].message.as_str()),
        (LogKind::Message, "Page loaded in 0.9s")
    );
    let visitor: Vec<_> = logs
        .iter()
        .filter(|entry| entry.kind == LogKind::Visitor)
        .collect();
    assert_eq!(
        visitor
            .iter()
            .map(|entry| entry.data.as_ref().unwrap()["activity"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["environment", "pageView", "action", "linkChoice", "blocked"]
    );
    assert_eq!(
        visitor[2].data,
        Some(json!({
            "activity": "action",
            "page": 0,
            "detail": { "action": "scroll", "scrollDepth": 0.6 }
        }))
    );
    assert_eq!(visitor[0].data.as_ref().unwrap()["page"], json!(null));
    assert_eq!(
        visitor[3].data.as_ref().unwrap()["detail"]["skipped"]["off-site"],
        9
    );
    // Only what the scanner refused is a warning.
    assert_eq!(
        visitor.iter().map(|entry| entry.level).collect::<Vec<_>>(),
        [
            LogLevel::Info,
            LogLevel::Info,
            LogLevel::Info,
            LogLevel::Info,
            LogLevel::Warn
        ]
    );
    assert_eq!(capture_step.log_count as usize, logs.len());

    // And it is searchable like the rest of the trail.
    let hits = harness
        .scans
        .search_logs("partner.example", None)
        .await
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].entry.kind, LogKind::Visitor);
}

// -- finishing a manual visit ------------------------------------------------------------

fn manual_options() -> ScanOptions {
    ScanOptions {
        visitor: Some(VisitorBehavior::Manual),
        headless: Some(true),
        ..scan_options(false)
    }
}

#[tokio::test]
async fn finishing_a_manual_visit_keeps_what_was_recorded_and_runs_the_rest() {
    let harness = offline(CaptureBehavior::RecordUntilFinished(Box::new(
        two_page_capture(),
    )));
    let started = harness.scans.start(manual_options()).await.unwrap();
    wait_for_capture(&harness, 1).await;
    {
        let request = &harness.capture.requests.lock().unwrap()[0];
        assert_eq!(request.visitor, VisitorBehavior::Manual);
        assert!(!request.headless, "a person needs a window");
        assert!(
            !request.accept_consent,
            "the consent choice is the person's"
        );
    }

    harness.scans.finish_capture(&started.id).await.unwrap();
    let detail = wait_for_end(&harness, &started.id).await;

    assert_eq!(detail.summary.status, ScanStatus::Completed);
    assert_eq!(detail.pages.len(), 2);
    assert!(
        detail
            .steps
            .iter()
            .all(|step| step.status != StepStatus::Failed)
    );
    assert_eq!(detail.environment.unwrap().visitor, VisitorBehavior::Manual);

    // Finishing a scan that already ended does nothing; an unknown one is an error.
    harness.scans.finish_capture(&started.id).await.unwrap();
    assert_eq!(
        harness.scans.finish_capture("nope").await,
        Err(AppError::NotFound("Scan nope".into()))
    );
}

#[tokio::test]
async fn cancelling_a_manual_visit_discards_it() {
    let harness = offline(CaptureBehavior::RecordUntilFinished(Box::new(
        two_page_capture(),
    )));
    let started = harness.scans.start(manual_options()).await.unwrap();
    wait_for_capture(&harness, 1).await;

    harness.scans.cancel(&started.id).await.unwrap();
    let detail = wait_for_end(&harness, &started.id).await;
    assert_eq!(detail.summary.status, ScanStatus::Cancelled);
    assert!(detail.pages.is_empty());
    assert_eq!(detail.environment, None);
}

// -- the persistent scan profile ---------------------------------------------------------

fn persistent_options() -> ScanOptions {
    ScanOptions {
        persistent_profile: Some(true),
        ..manual_options()
    }
}

#[tokio::test]
async fn scans_in_the_persistent_profile_run_one_at_a_time() {
    let harness = offline(CaptureBehavior::RecordUntilFinished(Box::new(
        two_page_capture(),
    )));
    let first = harness.scans.start(persistent_options()).await.unwrap();
    wait_for_capture(&harness, 1).await;
    assert!(harness.capture.requests.lock().unwrap()[0].persistent_profile);

    // The second scan gets as far as its capture step and waits there.
    let second = harness.scans.start(persistent_options()).await.unwrap();
    let waiting = "Waiting for another scan to release the scan profile";
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
    assert_eq!(harness.capture.requests.lock().unwrap().len(), 1);

    // The profile cannot be cleared from under a scan.
    assert_eq!(
        harness.scans.clear_scan_profile().await,
        Err(AppError::ProfileBusy)
    );
    assert_eq!(AppError::ProfileBusy.code(), ErrorCode::ProfileBusy);

    harness.scans.finish_capture(&first.id).await.unwrap();
    wait_for_capture(&harness, 2).await;
    harness.scans.finish_capture(&second.id).await.unwrap();
    for id in [&first.id, &second.id] {
        assert_eq!(
            wait_for_end(&harness, id).await.summary.status,
            ScanStatus::Completed
        );
    }

    // With no scan in it the profile can go.
    let info = harness.scans.scan_profile().await;
    assert!(info.exists && !info.in_use);
    let cleared = harness.scans.clear_scan_profile().await.unwrap();
    assert!(!cleared.exists);
    assert_eq!(*harness.capture.profile_clears.lock().unwrap(), 1);
}

#[tokio::test]
async fn finishing_a_visit_that_waits_for_the_profile_is_remembered_until_its_capture_starts() {
    let harness = offline(CaptureBehavior::RecordUntilFinished(Box::new(
        two_page_capture(),
    )));
    let holder = harness.scans.start(persistent_options()).await.unwrap();
    wait_for_capture(&harness, 1).await;
    let queued = harness.scans.start(persistent_options()).await.unwrap();
    let waiting = "Waiting for another scan to release the scan profile";
    for _ in 0..400 {
        let logs = harness.scans.logs(&queued.id, None).await.unwrap();
        if logs.iter().any(|entry| entry.message == waiting) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }

    // Finished while it waits: nothing to end yet, and not forgotten.
    harness.scans.finish_capture(&queued.id).await.unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(harness.capture.requests.lock().unwrap().len(), 1);

    harness.scans.finish_capture(&holder.id).await.unwrap();
    // No second request: the queued visit ends by itself once it has started.
    for id in [&holder.id, &queued.id] {
        assert_eq!(
            wait_for_end(&harness, id).await.summary.status,
            ScanStatus::Completed
        );
    }
    assert_eq!(harness.capture.requests.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn a_scan_waiting_for_the_profile_can_be_cancelled_and_a_throwaway_scan_does_not_wait() {
    let harness = offline(CaptureBehavior::RecordUntilFinished(Box::new(
        two_page_capture(),
    )));
    let holder = harness.scans.start(persistent_options()).await.unwrap();
    wait_for_capture(&harness, 1).await;

    // A scan with a throwaway profile starts its browser right away.
    let throwaway = harness.scans.start(manual_options()).await.unwrap();
    wait_for_capture(&harness, 2).await;
    for id in [&holder.id, &throwaway.id] {
        harness.scans.finish_capture(id).await.unwrap();
        wait_for_end(&harness, id).await;
    }

    let holder = harness.scans.start(persistent_options()).await.unwrap();
    wait_for_capture(&harness, 3).await;
    let queued = harness.scans.start(persistent_options()).await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    harness.scans.cancel(&queued.id).await.unwrap();
    assert_eq!(
        wait_for_end(&harness, &queued.id).await.summary.status,
        ScanStatus::Cancelled
    );
    assert_eq!(harness.capture.requests.lock().unwrap().len(), 3);
    harness.scans.cancel(&holder.id).await.unwrap();
    wait_for_end(&harness, &holder.id).await;
}

#[tokio::test]
async fn a_profile_the_browser_holds_cannot_be_cleared() {
    let harness = offline(CaptureBehavior::Return(Box::new(two_page_capture())));
    *harness.capture.profile_in_use.lock().unwrap() = true;
    assert!(harness.scans.scan_profile().await.in_use);
    assert_eq!(
        harness.scans.clear_scan_profile().await,
        Err(AppError::ProfileBusy)
    );
    assert_eq!(*harness.capture.profile_clears.lock().unwrap(), 0);
}

// -- settings that came with visitors ----------------------------------------------------------

#[tokio::test]
async fn visitor_defaults_are_patched_and_validated() {
    let harness = offline(CaptureBehavior::NoBrowser);
    let defaults = harness.settings.get().await.unwrap();
    assert_eq!(defaults.default_visitor, VisitorBehavior::Scroll);
    assert_eq!(defaults.default_extra_pages, 2);
    assert!(!defaults.persistent_profile);
    assert_eq!(defaults.default_proxy, ProxyChoice::System);
    assert!(defaults.proxies.is_empty());

    let patched = harness
        .settings
        .update(SettingsPatch {
            default_visitor: Some(VisitorBehavior::Browse),
            default_extra_pages: Some(5),
            persistent_profile: Some(true),
            default_proxy: Some(ProxyChoice::None),
            ..SettingsPatch::default()
        })
        .await
        .unwrap();
    assert_eq!(patched.default_visitor, VisitorBehavior::Browse);
    assert_eq!(patched.default_extra_pages, 5);
    assert!(patched.persistent_profile);
    assert_eq!(patched.default_proxy, ProxyChoice::None);
    assert_eq!(harness.settings.get().await.unwrap(), patched);

    let too_many = harness
        .settings
        .update(SettingsPatch {
            default_extra_pages: Some(6),
            ..SettingsPatch::default()
        })
        .await;
    assert_eq!(
        too_many,
        Err(AppError::InvalidInput(
            "A browse visit can follow links to at most 5 further pages".into()
        ))
    );
    let unknown_proxy = harness
        .settings
        .update(SettingsPatch {
            default_proxy: Some(profile_choice("gone")),
            ..SettingsPatch::default()
        })
        .await;
    assert_eq!(
        unknown_proxy,
        Err(AppError::InvalidInput(
            "The chosen proxy no longer exists. Pick another proxy.".into()
        ))
    );
    assert_eq!(harness.settings.get().await.unwrap(), patched);
}

// -- proxy profiles --------------------------------------------------------------------------

fn only_proxy(settings: &Settings) -> &adscan_model::ProxyProfile {
    assert_eq!(settings.proxies.len(), 1);
    &settings.proxies[0]
}

#[tokio::test]
async fn proxy_profiles_are_saved_renamed_and_listed_by_name() {
    let harness = offline(CaptureBehavior::NoBrowser);
    let saved = harness
        .settings
        .save_proxy_profile(belgium())
        .await
        .unwrap();
    let proxy = only_proxy(&saved).clone();
    assert_eq!(
        (proxy.name.as_str(), proxy.host.as_str(), proxy.port),
        ("Belgium", "be.proxy.example", 3128)
    );
    assert!(!proxy.has_password);

    let austria = harness
        .settings
        .save_proxy_profile(ProxyProfileDraft {
            name: "austria".into(),
            scheme: ProxyScheme::Socks5,
            host: "10.0.0.7".into(),
            port: 1080,
            username: None,
            ..belgium()
        })
        .await
        .unwrap();
    assert_eq!(
        austria
            .proxies
            .iter()
            .map(|proxy| proxy.name.as_str())
            .collect::<Vec<_>>(),
        ["austria", "Belgium"]
    );

    // Updating keeps the id; the name stays unique without regard to case.
    let renamed = harness
        .settings
        .save_proxy_profile(ProxyProfileDraft {
            id: Some(proxy.id.clone()),
            name: "Brussels".into(),
            port: 8080,
            ..belgium()
        })
        .await
        .unwrap();
    let brussels = renamed.proxies.iter().find(|p| p.id == proxy.id).unwrap();
    assert_eq!((brussels.name.as_str(), brussels.port), ("Brussels", 8080));
    let taken = harness
        .settings
        .save_proxy_profile(ProxyProfileDraft {
            name: "AUSTRIA".into(),
            ..belgium()
        })
        .await;
    assert_eq!(
        taken,
        Err(AppError::InvalidInput(
            "A proxy named \"AUSTRIA\" already exists".into()
        ))
    );
    let socks_with_user = harness
        .settings
        .save_proxy_profile(ProxyProfileDraft {
            name: "Socks".into(),
            scheme: ProxyScheme::Socks5,
            ..belgium()
        })
        .await;
    assert!(
        matches!(&socks_with_user, Err(AppError::InvalidInput(message)) if message.contains("SOCKS5")),
        "{socks_with_user:?}"
    );
    let unknown = harness
        .settings
        .save_proxy_profile(ProxyProfileDraft {
            id: Some("gone".into()),
            ..belgium()
        })
        .await;
    assert_eq!(unknown, Err(AppError::NotFound("Proxy gone".into())));
    assert_eq!(harness.settings.get().await.unwrap().proxies.len(), 2);
}

#[tokio::test]
async fn a_proxy_password_lives_in_the_secret_store_and_never_in_the_settings() {
    let harness = offline(CaptureBehavior::NoBrowser);
    let saved = harness
        .settings
        .save_proxy_profile(belgium())
        .await
        .unwrap();
    let id = only_proxy(&saved).id.clone();

    let with_password = harness
        .settings
        .set_proxy_password(&id, "s3cret-proxy")
        .await
        .unwrap();
    assert!(only_proxy(&with_password).has_password);
    assert_eq!(
        harness
            .secrets
            .proxy_password(&id)
            .await
            .unwrap()
            .as_deref(),
        Some("s3cret-proxy")
    );
    // Neither the returned settings nor the stored ones carry it.
    assert!(
        !serde_json::to_string(&with_password)
            .unwrap()
            .contains("s3cret-proxy")
    );
    use adscan_engine::application::ports::SettingsRepository;
    let stored = harness.store.load().await.unwrap().unwrap();
    assert!(
        !serde_json::to_string(&stored)
            .unwrap()
            .contains("s3cret-proxy")
    );
    assert!(
        !stored.proxies[0].has_password,
        "asked of the keychain, never stored"
    );

    assert_eq!(
        harness.settings.set_proxy_password(&id, "").await,
        Err(AppError::InvalidInput("Enter the proxy password.".into()))
    );
    assert_eq!(
        harness.settings.set_proxy_password("gone", "x").await,
        Err(AppError::NotFound("Proxy gone".into()))
    );

    let cleared = harness.settings.clear_proxy_password(&id).await.unwrap();
    assert!(!only_proxy(&cleared).has_password);
    assert_eq!(harness.secrets.proxy_password(&id).await.unwrap(), None);
}

#[tokio::test]
async fn deleting_a_proxy_removes_its_password_and_resets_the_default() {
    let harness = offline(CaptureBehavior::NoBrowser);
    let saved = harness
        .settings
        .save_proxy_profile(belgium())
        .await
        .unwrap();
    let id = only_proxy(&saved).id.clone();
    harness
        .settings
        .set_proxy_password(&id, "s3cret")
        .await
        .unwrap();
    let as_default = harness
        .settings
        .update(SettingsPatch {
            default_proxy: Some(profile_choice(&id)),
            ..SettingsPatch::default()
        })
        .await
        .unwrap();
    // The default keeps a snapshot of the profile it names.
    assert_eq!(
        as_default.default_proxy,
        ProxyChoice::Profile {
            id: id.clone(),
            name: Some("Belgium".into()),
            host: Some("be.proxy.example".into()),
        }
    );

    let after = harness.settings.delete_proxy_profile(&id).await.unwrap();
    assert!(after.proxies.is_empty());
    assert_eq!(after.default_proxy, ProxyChoice::System);
    assert_eq!(harness.secrets.proxy_password(&id).await.unwrap(), None);
    assert_eq!(
        harness.settings.delete_proxy_profile(&id).await,
        Err(AppError::NotFound(format!("Proxy {id}")))
    );
}

#[tokio::test]
async fn a_scan_runs_through_its_proxy_and_hands_the_password_over_outside_the_request() {
    let harness = offline(CaptureBehavior::Return(Box::new(two_page_capture())));
    harness
        .files
        .answer(SITE_ADS_TXT_URL, text_file(200, SITE_ADS_TXT));
    let saved = harness
        .settings
        .save_proxy_profile(belgium())
        .await
        .unwrap();
    let id = only_proxy(&saved).id.clone();
    harness
        .settings
        .set_proxy_password(&id, "s3cret-proxy")
        .await
        .unwrap();

    let detail = run_scan(
        &harness,
        ScanOptions {
            proxy: Some(profile_choice(&id)),
            ..browse_options()
        },
    )
    .await;
    assert_eq!(detail.summary.status, ScanStatus::Completed);

    // The browser gets the proxy in the request and the password beside it.
    let request = harness.capture.requests.lock().unwrap()[0].clone();
    assert_eq!(
        request.proxy,
        CaptureProxy::Custom {
            scheme: ProxyScheme::Http,
            host: "be.proxy.example".into(),
            port: 3128,
            username: Some("scanner".into()),
        }
    );
    assert!(
        !serde_json::to_string(&request)
            .unwrap()
            .contains("s3cret-proxy")
    );
    assert_eq!(
        *harness.capture.proxy_passwords.lock().unwrap(),
        [Some("s3cret-proxy".to_string())]
    );

    // The site's ads.txt is read through the same proxy.
    let fetches = harness.files.requests.lock().unwrap().clone();
    assert!(!fetches.is_empty());
    for fetch in &fetches {
        assert_eq!(fetch.proxy.route, request.proxy);
        assert_eq!(fetch.proxy.password.as_deref(), Some("s3cret-proxy"));
    }

    // The scan records which proxy it used, with a snapshot that outlives the profile.
    let used = ProxyUse {
        kind: ProxyKind::Profile,
        name: Some("Belgium".into()),
        host: Some("be.proxy.example".into()),
    };
    assert_eq!(detail.summary.proxy, Some(used.clone()));
    assert_eq!(detail.environment.as_ref().unwrap().proxy, used);
    harness.settings.delete_proxy_profile(&id).await.unwrap();
    let later = harness.scans.get(&detail.summary.id).await.unwrap();
    assert_eq!(
        later.options.proxy,
        Some(ProxyChoice::Profile {
            id: id.clone(),
            name: Some("Belgium".into()),
            host: Some("be.proxy.example".into()),
        })
    );

    // Nothing of the password reached storage or the audit trail.
    let logs = harness.scans.logs(&detail.summary.id, None).await.unwrap();
    let everything = format!(
        "{}{}",
        serde_json::to_string(&later).unwrap(),
        serde_json::to_string(&logs).unwrap()
    );
    assert!(!everything.contains("s3cret-proxy"));
    assert!(logs.iter().any(|entry| {
        entry
            .message
            .contains("proxy \"Belgium\" (be.proxy.example)")
    }));

    // A scan that names the deleted profile is refused before anything is created.
    let before = harness.scans.list(all_scans()).await.unwrap().total;
    let refused = harness
        .scans
        .start(ScanOptions {
            proxy: Some(profile_choice(&id)),
            ..browse_options()
        })
        .await;
    assert_eq!(
        refused,
        Err(AppError::InvalidInput(
            "The chosen proxy no longer exists. Pick another proxy.".into()
        ))
    );
    assert_eq!(harness.scans.list(all_scans()).await.unwrap().total, before);
}

#[tokio::test]
async fn a_scan_without_a_proxy_profile_has_no_password_and_the_direct_choice_is_passed_on() {
    let harness = offline(CaptureBehavior::Return(Box::new(two_page_capture())));
    harness
        .files
        .answer(SITE_ADS_TXT_URL, text_file(200, SITE_ADS_TXT));
    let detail = run_scan(
        &harness,
        ScanOptions {
            proxy: Some(ProxyChoice::None),
            ..browse_options()
        },
    )
    .await;
    assert_eq!(
        harness.capture.requests.lock().unwrap()[0].proxy,
        CaptureProxy::None
    );
    assert_eq!(*harness.capture.proxy_passwords.lock().unwrap(), [None]);
    let fetches = harness.files.requests.lock().unwrap().clone();
    assert!(
        fetches
            .iter()
            .all(|fetch| fetch.proxy.route == CaptureProxy::None)
    );
    assert_eq!(detail.summary.proxy.unwrap().kind, ProxyKind::None);
}

// -- saved scan configurations -------------------------------------------------------------------

fn draft(name: &str, options: ScanOptions) -> ScanConfigurationDraft {
    ScanConfigurationDraft {
        id: None,
        name: name.into(),
        options,
    }
}

#[tokio::test]
async fn configurations_are_saved_validated_updated_and_deleted() {
    let harness = offline(CaptureBehavior::NoBrowser);
    let saved = harness
        .configurations
        .save(draft(
            "  Publisher BE  ",
            ScanOptions {
                seed: Some(5),
                budget_usd: Some(0.2),
                ..browse_options()
            },
        ))
        .await
        .unwrap();
    assert_eq!(saved.name, "Publisher BE");
    assert_eq!(
        saved.options.url, "https://pub.example/",
        "the URL is normalised"
    );
    assert_eq!(saved.options.seed, None, "every run plans its own visit");
    assert_eq!(saved.options.budget_usd, Some(0.2));
    assert_eq!(saved.options.visitor, Some(VisitorBehavior::Browse));
    assert_eq!(saved.options.proxy, None, "left open: follows the settings");
    assert_eq!(
        (saved.last_scan_id.clone(), saved.last_run_at),
        (None, None)
    );
    assert_eq!(saved.created_at, saved.updated_at);
    assert_eq!(
        harness.configurations.list().await.unwrap(),
        std::slice::from_ref(&saved)
    );

    // Names are unique without regard to case.
    let duplicate = harness
        .configurations
        .save(draft("publisher be", browse_options()))
        .await;
    assert_eq!(
        duplicate,
        Err(AppError::InvalidInput(
            "A scan configuration named \"publisher be\" already exists".into()
        ))
    );
    // Options that could not start a scan cannot be saved.
    for (bad, expected) in [
        (
            draft("", browse_options()),
            "Give the scan configuration a name of at most 80 characters",
        ),
        (
            draft(
                "Bad URL",
                ScanOptions {
                    url: "not a url".into(),
                    ..browse_options()
                },
            ),
            "Enter a valid web address, for example https://example.com",
        ),
        (
            draft(
                "Too many pages",
                ScanOptions {
                    extra_pages: Some(9),
                    ..browse_options()
                },
            ),
            "A browse visit can follow links to at most 5 further pages",
        ),
        (
            draft(
                "Gone proxy",
                ScanOptions {
                    proxy: Some(profile_choice("gone")),
                    ..browse_options()
                },
            ),
            "The chosen proxy no longer exists. Pick another proxy.",
        ),
    ] {
        assert_eq!(
            harness.configurations.save(bad).await,
            Err(AppError::InvalidInput(expected.into()))
        );
    }

    // An update keeps the identity and the creation time.
    tokio::time::sleep(Duration::from_millis(5)).await;
    let updated = harness
        .configurations
        .save(ScanConfigurationDraft {
            id: Some(saved.id.clone()),
            name: "Publisher BE".into(),
            options: ScanOptions {
                device: DeviceProfile::Mobile,
                ..browse_options()
            },
        })
        .await
        .unwrap();
    assert_eq!(updated.id, saved.id);
    assert_eq!(updated.created_at, saved.created_at);
    assert!(updated.updated_at > saved.updated_at);
    assert_eq!(updated.options.device, DeviceProfile::Mobile);
    assert_eq!(harness.configurations.list().await.unwrap().len(), 1);
    let unknown = harness
        .configurations
        .save(ScanConfigurationDraft {
            id: Some("gone".into()),
            ..draft("Other", browse_options())
        })
        .await;
    assert_eq!(
        unknown,
        Err(AppError::NotFound("Scan configuration gone".into()))
    );

    harness.configurations.delete(&saved.id).await.unwrap();
    assert!(harness.configurations.list().await.unwrap().is_empty());
    assert_eq!(
        harness.configurations.delete(&saved.id).await,
        Err(AppError::NotFound(format!(
            "Scan configuration {}",
            saved.id
        )))
    );
}

#[tokio::test]
async fn running_a_configuration_uses_its_options_and_the_settings_of_the_moment() {
    let harness = offline(CaptureBehavior::Return(Box::new(two_page_capture())));
    let configuration = harness
        .configurations
        .save(draft(
            "Publisher BE",
            ScanOptions {
                extra_pages: None,
                headless: Some(true),
                seed: None,
                ..browse_options()
            },
        ))
        .await
        .unwrap();
    // Changed after the configuration was saved: the run follows it.
    harness
        .settings
        .update(SettingsPatch {
            default_extra_pages: Some(4),
            budget_usd: Some(0.3),
            ..SettingsPatch::default()
        })
        .await
        .unwrap();

    let started = harness
        .scans
        .start_configuration(&configuration.id)
        .await
        .unwrap();
    assert_eq!(
        started
            .configuration
            .as_ref()
            .map(|from| (from.id.as_str(), from.name.as_str())),
        Some((configuration.id.as_str(), "Publisher BE"))
    );
    let detail = wait_for_end(&harness, &started.id).await;
    assert_eq!(detail.summary.status, ScanStatus::Completed);
    let request = harness.capture.requests.lock().unwrap()[0].clone();
    assert_eq!(
        (request.visitor, request.extra_pages),
        (VisitorBehavior::Browse, 4)
    );
    assert_eq!(detail.options.budget_usd, Some(0.3));
    // Each run gets its own seed, derived from the scan.
    assert!(detail.options.seed.is_some());

    // The configuration remembers its last run.
    let after = harness.configurations.list().await.unwrap().remove(0);
    assert_eq!(after.last_scan_id.as_deref(), Some(started.id.as_str()));
    assert_eq!(after.last_run_at, Some(started.started_at));
    assert_eq!(after.options.seed, None);

    // History can be narrowed to the scans of one configuration.
    let other = run_scan(&harness, browse_options()).await;
    assert_eq!(other.summary.configuration, None);
    let of_configuration = harness
        .scans
        .list(ScanListQuery {
            configuration_id: Some(configuration.id.clone()),
            ..all_scans()
        })
        .await
        .unwrap();
    assert_eq!(of_configuration.total, 1);
    assert_eq!(of_configuration.items[0].id, started.id);
    assert_eq!(harness.scans.list(all_scans()).await.unwrap().total, 2);

    // The export names the configuration.
    let markdown = harness
        .scans
        .export(&started.id, ExportFormat::Markdown)
        .await
        .unwrap();
    assert!(
        markdown.contains("- **Scan configuration:** Publisher BE"),
        "{markdown}"
    );
    let exported: serde_json::Value = serde_json::from_str(
        &harness
            .scans
            .export(&started.id, ExportFormat::Json)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(exported["summary"]["configuration"]["name"], "Publisher BE");

    // Deleting the configuration never deletes its scans; they keep its name.
    harness
        .configurations
        .delete(&configuration.id)
        .await
        .unwrap();
    let kept = harness.scans.get(&started.id).await.unwrap();
    assert_eq!(kept.summary.configuration.unwrap().name, "Publisher BE");

    // Deleting the scan only clears the configuration's link to it.
    let again = harness
        .configurations
        .save(draft("Again", browse_options()))
        .await
        .unwrap();
    let run = harness.scans.start_configuration(&again.id).await.unwrap();
    wait_for_end(&harness, &run.id).await;
    harness.scans.delete(&run.id).await.unwrap();
    let after_delete = harness.configurations.list().await.unwrap().remove(0);
    assert_eq!(after_delete.last_scan_id, None);
    assert_eq!(after_delete.last_run_at, Some(run.started_at));

    assert_eq!(
        harness.scans.start_configuration("gone").await,
        Err(AppError::NotFound("Scan configuration gone".into()))
    );
}

#[tokio::test]
async fn a_proxy_in_use_by_a_configuration_cannot_be_deleted() {
    let harness = offline(CaptureBehavior::Return(Box::new(two_page_capture())));
    let saved = harness
        .settings
        .save_proxy_profile(belgium())
        .await
        .unwrap();
    let id = only_proxy(&saved).id.clone();
    let configuration = harness
        .configurations
        .save(draft(
            "Publisher BE",
            ScanOptions {
                proxy: Some(profile_choice(&id)),
                ..browse_options()
            },
        ))
        .await
        .unwrap();
    // The configuration stores which profile, with a snapshot for display.
    assert_eq!(
        configuration.options.proxy,
        Some(ProxyChoice::Profile {
            id: id.clone(),
            name: Some("Belgium".into()),
            host: Some("be.proxy.example".into()),
        })
    );

    assert_eq!(
        harness.settings.delete_proxy_profile(&id).await,
        Err(AppError::InvalidInput(
            "The proxy \"Belgium\" is used by the scan configuration \"Publisher BE\". Change or delete it first."
                .into()
        ))
    );
    assert_eq!(harness.settings.get().await.unwrap().proxies.len(), 1);

    harness
        .configurations
        .delete(&configuration.id)
        .await
        .unwrap();
    assert!(
        harness
            .settings
            .delete_proxy_profile(&id)
            .await
            .unwrap()
            .proxies
            .is_empty()
    );
}

#[tokio::test]
async fn a_configuration_whose_proxy_is_gone_says_so_by_name() {
    use adscan_engine::application::ports::ScanConfigurationRepository;

    let harness = offline(CaptureBehavior::Return(Box::new(two_page_capture())));
    let configuration = harness
        .configurations
        .save(draft("Publisher BE", browse_options()))
        .await
        .unwrap();
    // A profile can only vanish from under a configuration behind the app's
    // back (an edited database, a restored backup); the start still explains.
    let mut orphaned = configuration.clone();
    orphaned.options.proxy = Some(profile_choice("gone"));
    harness.store.save(&orphaned).await.unwrap();

    assert_eq!(
        harness.scans.start_configuration(&configuration.id).await,
        Err(AppError::InvalidInput(
            "The proxy of the scan configuration \"Publisher BE\" no longer exists. Edit the configuration and pick another proxy."
                .into()
        ))
    );
    assert_eq!(harness.scans.list(all_scans()).await.unwrap().total, 0);
    let untouched = harness.configurations.list().await.unwrap().remove(0);
    assert_eq!(untouched.last_scan_id, None);
}
