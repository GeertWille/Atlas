//! The real pipeline: a real Chromium captures the built-in demo site and the
//! AI phases talk to a mock of the Messages API.
//!
//! Ignored by default because it needs a local browser:
//! `cargo test -p adtech-scanner --test real_browser -- --ignored`
//! (set `ADSCAN_CHROME_PATH` if Chrome is not in a standard location).

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use adscan_engine::application::ports::EventSink;
use adscan_model::{
    AdsTxtStatus, DeviceProfile, DiagnosticStatus, FindingCategory, LogEntry, LogKind, PageReach,
    PipelineNode, ProxyKind, ScanDetail, ScanEvent, ScanOptions, ScanStatus, SettingsPatch,
    StepStatus, VisitorBehavior,
};
use adtech_scanner_lib::ChromeCaptureSource;
use adtech_scanner_lib::state::{AppConfig, AppState};
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

const CI_CHROMIUM: &str = "/opt/pw-browsers/chromium-1243/chrome-linux64/chrome";

#[derive(Default)]
struct CollectedEvents(Mutex<Vec<ScanEvent>>);

impl EventSink for CollectedEvents {
    fn emit(&self, event: ScanEvent) {
        self.0.lock().unwrap().push(event);
    }
}

fn message(content: Value, stop_reason: &str) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({
        "content": content,
        "stop_reason": stop_reason,
        "usage": { "input_tokens": 1500, "output_tokens": 150 }
    }))
}

/// Phase 1 classifies nothing; Phase 2 searches once, then finishes.
fn scripted_model(request: &Request) -> ResponseTemplate {
    let body: Value = serde_json::from_slice(&request.body).unwrap();
    if body["model"] == "claude-haiku-5-5" {
        return message(
            json!([{ "type": "text", "text": "{\"scripts\":[]}" }]),
            "end_turn",
        );
    }
    let turns = body["messages"].as_array().map_or(0, Vec::len);
    if turns == 1 {
        let search = json!({ "pattern": "requestBids|addEventListener", "script_id": null });
        message(
            json!([{ "type": "tool_use", "id": "toolu_1", "name": "search_scripts", "input": search }]),
            "tool_use",
        )
    } else {
        let finish = json!({ "overview": "Mock analysis finished." });
        message(
            json!([{ "type": "tool_use", "id": "toolu_2", "name": "finish", "input": finish }]),
            "tool_use",
        )
    }
}

async fn scan_demo_site(device: DeviceProfile) -> (ScanDetail, MockServer) {
    let (detail, _logs, server) = scan_demo_site_with(|url| ScanOptions {
        device,
        observe_seconds: 6,
        ai_enabled: true,
        ..ScanOptions::for_url(url)
    })
    .await;
    (detail, server)
}

/// Scans the demo site with the options `options_for` builds for its URL.
/// Returns the scan, its audit trail and the mock of the Messages API.
async fn scan_demo_site_with(
    options_for: impl FnOnce(String) -> ScanOptions,
) -> (ScanDetail, Vec<LogEntry>, MockServer) {
    let scan = run_demo_scan(options_for).await;
    let logs = scan
        .state
        .scans
        .logs(&scan.detail.summary.id, None)
        .await
        .unwrap();
    scan.state.scans.shutdown().await;
    (scan.detail, logs, scan.server)
}

/// A finished scan of the demo site with the app that made it still up.
struct DemoScan {
    state: AppState,
    data_dir: tempfile::TempDir,
    detail: ScanDetail,
    server: MockServer,
}

async fn run_demo_scan(options_for: impl FnOnce(String) -> ScanOptions) -> DemoScan {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(scripted_model)
        .mount(&server)
        .await;
    let data_dir = tempfile::tempdir().unwrap();
    let state = AppState::build(
        AppConfig {
            data_dir: data_dir.path().to_path_buf(),
            anthropic_base_url: server.uri(),
            typesafe_base_url: "http://127.0.0.1:1".into(),
            api_key_override: Some("sk-ant-real-browser-test".into()),
            typesafe_api_key_override: None,
            proxy_password_override: None,
        },
        Arc::new(ChromeCaptureSource::new(data_dir.path())),
        Arc::new(CollectedEvents::default()),
    )
    .await
    .unwrap();
    if std::env::var_os("ADSCAN_CHROME_PATH").is_none() && Path::new(CI_CHROMIUM).exists() {
        let patch = SettingsPatch {
            chrome_path: Some(Some(CI_CHROMIUM.into())),
            ..SettingsPatch::default()
        };
        state.settings.update(patch).await.unwrap();
    }
    let browser = state.scans.detect_browser().await.unwrap();
    assert!(
        browser.is_some(),
        "no Chrome/Chromium found; set ADSCAN_CHROME_PATH"
    );

    let options = options_for(state.demo_site.url.clone());
    let started = state.scans.start(options).await.unwrap();
    let detail = tokio::time::timeout(Duration::from_secs(240), async {
        loop {
            let detail = state.scans.get(&started.id).await.unwrap();
            if !matches!(
                detail.summary.status,
                ScanStatus::Queued | ScanStatus::Running
            ) {
                break detail;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    })
    .await
    .expect("the scan did not finish in time");
    DemoScan {
        state,
        data_dir,
        detail,
        server,
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a local Chrome/Chromium"]
async fn demo_site_scan_runs_the_whole_pipeline_with_a_real_browser() {
    let (detail, server) = scan_demo_site(DeviceProfile::Desktop).await;

    assert_eq!(
        detail.summary.status,
        ScanStatus::Completed,
        "{:?}",
        detail.summary.error
    );
    for step in &detail.steps {
        assert_ne!(
            step.status,
            StepStatus::Failed,
            "{:?} failed: {:?}",
            step.node,
            step.summary
        );
    }
    let step = |node: PipelineNode| detail.steps.iter().find(|step| step.node == node).unwrap();
    assert_eq!(step(PipelineNode::Capture).status, StepStatus::Completed);
    assert_eq!(
        step(PipelineNode::AnalyzeLogic).status,
        StepStatus::Completed
    );

    // The demo publisher page runs Prebid + GPT and loads real scripts.
    assert!(detail.metrics.script_count > 0);
    assert!(detail.metrics.has_prebid, "{:#?}", detail.metrics);
    assert!(detail.metrics.has_gpt, "{:#?}", detail.metrics);
    assert!(detail.scripts.iter().any(|script| script.ad_related));
    assert!(
        detail.scripts.iter().any(|script| script.analyzed),
        "ad scripts were handed to Phase 2"
    );

    // Its planted bugs are visible to the deterministic rules.
    let titles: Vec<&str> = detail
        .findings
        .iter()
        .map(|finding| finding.title.as_str())
        .collect();
    for expected in [
        "Bidding waits for the first scroll on desktop",
        "sent before the Prebid auction finished",
        "Auction timeout of 250 ms",
        "have no matching GPT slot",
        "never returned a bid",
        "adapter params the adapter rejects",
    ] {
        assert!(
            titles.iter().any(|title| title.contains(expected)),
            "missing `{expected}` in {titles:#?}"
        );
    }
    // The demo's identity call fails on purpose and Chrome logs one console
    // error per document for it (demo-site README, "On the network"). No
    // deterministic rule reads the console, so nothing the rules report may
    // present that line as a fault of the site.
    for finding in &detail.findings {
        if finding.source != adscan_model::FindingSource::Heuristic {
            continue;
        }
        let mut text = format!(
            "{} {} {}",
            finding.title, finding.description, finding.impact
        );
        for evidence in &finding.evidence {
            text.push_str(&evidence.body);
        }
        for console_noise in [
            "identity/envelope",
            "ERR_EMPTY_RESPONSE",
            "Failed to load resource",
        ] {
            assert!(
                !text.contains(console_noise),
                "`{}` reports the planned console error: {text}",
                finding.title
            );
        }
    }

    // The bidder checks read the demo site's own ads.txt over loopback and
    // name the cause of each planted bidder bug.
    assert_eq!(
        step(PipelineNode::BidderDiagnostics).status,
        StepStatus::Completed
    );
    let diagnostics = detail
        .bidder_diagnostics
        .as_ref()
        .expect("the bidder checks ran");
    assert_eq!(
        (
            diagnostics.ads_txt.url.as_str(),
            diagnostics.ads_txt.status,
            diagnostics.ads_txt.record_count,
            diagnostics.ads_txt.invalid_line_count,
        ),
        (
            format!("{}ads.txt", detail.summary.url).as_str(),
            AdsTxtStatus::Found,
            6,
            0
        )
    );
    assert_eq!(
        diagnostics.ads_txt.owner_domain.as_deref(),
        Some("meridian-media.example")
    );
    assert!(
        !diagnostics.sellers_json_checked,
        "exchanges are not contacted unless the user turns that on"
    );
    assert!(!diagnostics.consent.cmp_present);
    let problems = |bidder: &str| -> Vec<(&str, &str)> {
        diagnostics
            .bidders
            .iter()
            .filter(|diagnosis| diagnosis.bidder == bidder)
            .flat_map(|diagnosis| &diagnosis.checks)
            .filter(|check| check.status == DiagnosticStatus::Problem)
            .map(|check| (check.code.as_str(), check.summary.as_str()))
            .collect()
    };
    assert_eq!(
        problems("ix"),
        [(
            "ads-txt-not-listed",
            "ads.txt has no entry for indexexchange.com"
        )]
    );
    assert_eq!(
        problems("triplelift"),
        [(
            "params-missing",
            "`inventoryCode` missing on 2 of 3 ad units"
        )]
    );
    let openx = problems("openx");
    assert_eq!(openx.len(), 1, "{openx:?}");
    assert_eq!(openx[0].0, "response-reason");
    for healthy in ["appnexus", "rubicon", "pubmatic", "criteo"] {
        assert_eq!(problems(healthy), [], "{healthy}");
    }
    assert_eq!(
        diagnostics
            .bidders
            .iter()
            .map(|diagnosis| diagnosis.bidder.as_str())
            .collect::<Vec<_>>(),
        [
            "appnexus",
            "criteo",
            "ix",
            "openx",
            "pubmatic",
            "rubicon",
            "triplelift"
        ]
    );

    let never_bid = detail
        .findings
        .iter()
        .find(|finding| finding.title.contains("never returned a bid"))
        .unwrap();
    assert_eq!(never_bid.affected_bidders, ["ix", "openx"]);
    assert!(
        never_bid
            .description
            .contains("- `ix`: ads.txt has no entry for indexexchange.com"),
        "{}",
        never_bid.description
    );
    assert!(
        never_bid
            .description
            .contains("- `openx`: The bidder's endpoint explained why it did not bid"),
        "{}",
        never_bid.description
    );
    assert!(
        never_bid
            .evidence
            .iter()
            .any(|evidence| evidence.body == "no-bid reason 7 (blocked publisher or site)"),
        "{:#?}",
        never_bid.evidence
    );
    assert!(
        !never_bid
            .recommendation
            .contains("compare with their own reporting"),
        "the generic advice is only the fallback: {}",
        never_bid.recommendation
    );
    let rejected = detail
        .findings
        .iter()
        .find(|finding| finding.title.contains("adapter params the adapter rejects"))
        .unwrap();
    assert_eq!(rejected.affected_bidders, ["triplelift"]);
    assert_eq!(rejected.affected_slots, ["ad-inarticle-1", "ad-sidebar-1"]);
    assert!(
        !titles
            .iter()
            .any(|title| title.contains("not authorised by the site's ads.txt")),
        "the missing entry is reported once, as the cause of ix's silence: {titles:#?}"
    );

    assert!(detail.summary.health_score.unwrap() < 100);
    assert!(detail.summary.ai_used);
    assert!(detail.summary.cost_usd > 0.0);
    assert!(detail.overview.unwrap().contains("Mock analysis finished."));

    // Phase 2 really searched the captured, beautified scripts.
    let requests = server.received_requests().await.unwrap();
    let last: Value = serde_json::from_slice(&requests.last().unwrap().body).unwrap();
    let search_result = last["messages"][2]["content"][0]["content"]
        .as_str()
        .unwrap();
    assert!(search_result.contains("matching lines"), "{search_result}");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a local Chrome/Chromium"]
async fn demo_site_reference_bug_differs_between_device_profiles() {
    let (desktop, _desktop_api) = scan_demo_site(DeviceProfile::Desktop).await;
    let (mobile, _mobile_api) = scan_demo_site(DeviceProfile::Mobile).await;
    assert_eq!(
        desktop.summary.status,
        ScanStatus::Completed,
        "{:?}",
        desktop.summary.error
    );
    assert_eq!(
        mobile.summary.status,
        ScanStatus::Completed,
        "{:?}",
        mobile.summary.error
    );

    let gated = |detail: &ScanDetail| {
        detail
            .findings
            .iter()
            .filter(|finding| matches!(finding.category, FindingCategory::LazyLoad))
            .map(|finding| finding.title.clone())
            .collect::<Vec<_>>()
    };
    assert_ne!(
        gated(&desktop).is_empty(),
        gated(&mobile).is_empty(),
        "exactly one device class should hit the scroll-gated auction; desktop: {:?}, mobile: {:?}",
        gated(&desktop),
        gated(&mobile),
    );
}

/// The findings only a visit of several page views, or a refreshed slot,
/// can produce. Which of the demo site's bugs a browse visit meets depends
/// on the links it follows.
const VISIT_FINDINGS: [&str; 3] = [
    "Later page views are sold without header bidding",
    "Ad slots are not reset when the page changes route",
    "Slots are refreshed with bids that were already used",
];

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a local Chrome/Chromium"]
async fn a_browse_visit_of_the_demo_site_follows_links_and_reports_per_page_view() {
    let (detail, logs, _server) = scan_demo_site_with(|url| ScanOptions {
        visitor: Some(VisitorBehavior::Browse),
        extra_pages: Some(2),
        headless: Some(true),
        seed: Some(20_261_112),
        observe_seconds: 8,
        ..ScanOptions::for_url(url)
    })
    .await;

    assert_eq!(
        detail.summary.status,
        ScanStatus::Completed,
        "{:?}",
        detail.summary.error
    );
    // The visit left the entry page and stayed on the site.
    assert!(detail.pages.len() >= 2, "{:#?}", detail.pages);
    assert_eq!(detail.pages[0].reached_by, PageReach::Entry);
    assert_eq!(detail.pages[0].started_at_ms, 0.0);
    for (index, page) in detail.pages.iter().enumerate() {
        assert_eq!(page.index as usize, index);
        assert!(page.url.starts_with(&detail.summary.url), "{}", page.url);
        if index > 0 {
            assert!(
                matches!(
                    page.reached_by,
                    PageReach::LinkClick | PageReach::RouteChange
                ),
                "{page:#?}"
            );
            assert!(page.started_at_ms > detail.pages[index - 1].started_at_ms);
        }
    }
    assert!(
        detail
            .auctions
            .iter()
            .all(|auction| (auction.page as usize) < detail.pages.len())
            && detail
                .slots
                .iter()
                .all(|slot| (slot.page as usize) < detail.pages.len())
    );
    assert!(
        detail.slots.iter().any(|slot| slot.page > 0),
        "a later page view has slots"
    );

    // The environment is reported as it was: an automated, headless visit.
    let environment = detail
        .environment
        .as_ref()
        .expect("recorded with the telemetry");
    assert_eq!(environment.visitor, VisitorBehavior::Browse);
    assert_eq!(environment.seed, 20_261_112);
    assert!(environment.headless);
    assert_eq!(
        environment.webdriver,
        Some(true),
        "scripted visits do not hide"
    );
    assert_eq!(environment.proxy.kind, ProxyKind::System);

    // Every step of the visit is in the audit trail.
    let activities: Vec<&str> = logs
        .iter()
        .filter(|entry| entry.kind == LogKind::Visitor)
        .filter_map(|entry| entry.data.as_ref()?.get("activity")?.as_str())
        .collect();
    for expected in ["environment", "pageView", "action", "linkChoice"] {
        assert!(
            activities.contains(&expected),
            "no `{expected}` in {activities:?}"
        );
    }
    let page_views = activities
        .iter()
        .filter(|kind| **kind == "pageView")
        .count();
    assert_eq!(page_views, detail.pages.len());

    // At least one of the bugs that need more than one page load was found,
    // on the page view it happened on.
    let visit_findings: Vec<_> = detail
        .findings
        .iter()
        .filter(|finding| VISIT_FINDINGS.contains(&finding.title.as_str()))
        .collect();
    assert!(
        !visit_findings.is_empty(),
        "none of {VISIT_FINDINGS:?} in {:#?}",
        detail
            .findings
            .iter()
            .map(|finding| &finding.title)
            .collect::<Vec<_>>()
    );
    for finding in visit_findings {
        assert!(!finding.pages.is_empty(), "{finding:#?}");
        assert!(
            finding
                .pages
                .iter()
                .all(|page| (*page as usize) < detail.pages.len()),
            "{finding:#?}"
        );
    }
}

/// The archive of a real capture: what the browser crate hands over is
/// stored, and reads back as the network view and the script viewer need it.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a local Chrome/Chromium"]
async fn the_demo_scan_is_archived_and_reads_back_for_the_network_view_and_the_script_viewer() {
    use adscan_model::{
        BodySide, BodyState, ExchangeVerdict, NetworkAvailability, NetworkCategory, ResourceKind,
        SourceAvailability, SourceView,
    };
    let scan = run_demo_scan(|url| ScanOptions {
        observe_seconds: 6,
        ai_enabled: true,
        ..ScanOptions::for_url(url)
    })
    .await;
    let DemoScan {
        state,
        data_dir,
        detail,
        server,
    } = &scan;
    assert_eq!(
        detail.summary.status,
        ScanStatus::Completed,
        "{:?}",
        detail.summary.error
    );
    let scan_id = detail.summary.id.as_str();

    // The app's own calls to the model API do not go through the scanned
    // browser: the scan made them, and neither they nor the key they carry
    // are anywhere in what the scan stored.
    let model_calls = server.received_requests().await.unwrap_or_default();
    assert!(!model_calls.is_empty(), "the scan called the model API");
    let model_api = format!(":{}/", server.address().port());
    let mut stored_files = vec![data_dir.path().to_path_buf()];
    while let Some(path) = stored_files.pop() {
        if path.is_dir() {
            stored_files.extend(
                std::fs::read_dir(&path)
                    .unwrap()
                    .map(|entry| entry.unwrap().path()),
            );
            continue;
        }
        let bytes = std::fs::read(&path).unwrap();
        let text = if path.extension().is_some_and(|ext| ext == "zst") {
            String::from_utf8_lossy(&zstd::stream::decode_all(bytes.as_slice()).unwrap())
                .into_owned()
        } else {
            String::from_utf8_lossy(&bytes).into_owned()
        };
        assert!(
            !text.contains("sk-ant-real-browser-test"),
            "the API key is in {}",
            path.display()
        );
        if path.extension().is_some_and(|ext| ext == "zst") {
            assert!(!text.contains("/v1/messages") && !text.contains(&model_api));
        }
    }

    // One sealed pack, and the report says so.
    let network = detail
        .network
        .clone()
        .expect("the scan has a network record");
    assert_eq!(network.availability, NetworkAvailability::Available);
    assert!(network.headers_redacted);
    let pack = data_dir
        .path()
        .join(format!("archive/scans/{scan_id}.pack.zst"));
    assert_eq!(
        std::fs::metadata(&pack).unwrap().len() as f64,
        network.stored_bytes
    );

    let started = std::time::Instant::now();
    let chunk = state.network.list(scan_id, &[], None, None).await.unwrap();
    let list_ms = started.elapsed().as_secs_f64() * 1e3;
    let rows = &chunk.rows;
    assert_eq!(rows.len() as u32, network.request_count);
    assert!(rows.len() >= 15, "only {} requests", rows.len());
    let raw_bytes: f64 = rows.iter().map(|row| row.decoded_bytes).sum();
    eprintln!(
        "demo scan archive: {} requests, {} bodies, pack {} bytes ({} bytes of bodies before compression), list in {list_ms:.1} ms",
        network.request_count, network.body_count, network.stored_bytes, raw_bytes
    );

    // The page, its scripts, an image without a body.
    let document = rows
        .iter()
        .find(|row| row.kind == ResourceKind::Document)
        .unwrap();
    assert!(document.first_party && document.has_response_body);
    assert_eq!(document.category, NetworkCategory::Site);
    let image = rows.iter().find(|row| row.kind == ResourceKind::Image);
    if let Some(image) = image {
        assert!(!image.has_response_body);
        let body = state
            .network
            .body(scan_id, &image.id, BodySide::Response, None, None)
            .await
            .unwrap();
        assert_eq!(body.state, BodyState::NotKeptType);
    }

    // The openx exchange: the request with Prebid's ids, the answer decoded.
    let openx = rows
        .iter()
        .find(|row| row.bidder.as_deref() == Some("openx") && row.kind != ResourceKind::Preflight)
        .expect("the demo's openx bid request is in the list");
    assert_eq!(openx.category, NetworkCategory::Bid);
    let started = std::time::Instant::now();
    let opened = state.network.request(scan_id, &openx.id).await.unwrap();
    let detail_ms = started.elapsed().as_secs_f64() * 1e3;
    let decoded = opened.decoded.expect("the exchange is OpenRTB");
    assert_eq!(decoded.response.unwrap().nbr, Some(7));
    assert!(!decoded.request.unwrap().imps.is_empty());
    let link = openx
        .ad
        .clone()
        .expect("the request is tied to its auction");
    assert_eq!(link.nbr, Some(7));
    assert!(
        detail
            .auctions
            .iter()
            .any(|auction| auction.id == link.auction_id)
    );

    // The verdicts of the first auction: under the 250 ms timeout two
    // bidders answer late or never, and triplelift's bids without an
    // inventory code never leave the page.
    let verdicts: Vec<(String, String, ExchangeVerdict)> = detail
        .auctions
        .iter()
        .flat_map(|auction| &auction.ad_units)
        .flat_map(|unit| {
            unit.bids.iter().filter_map(|bid| {
                Some((
                    unit.code.clone(),
                    bid.bidder.clone(),
                    bid.exchange.as_ref()?.verdict,
                ))
            })
        })
        .collect();
    assert!(
        verdicts.iter().any(|(_, _, verdict)| matches!(
            verdict,
            ExchangeVerdict::LateOnNetwork | ExchangeVerdict::NeverAnswered
        )),
        "{verdicts:?}"
    );
    assert!(
        verdicts
            .iter()
            .any(|(_, bidder, verdict)| bidder == "triplelift"
                && *verdict == ExchangeVerdict::NotSent),
        "{verdicts:?}"
    );

    // The wrapper opens in the viewer, formatted, with the planted line.
    let wrapper = detail
        .scripts
        .iter()
        .find(|script| script.url.contains("hb-wrapper"))
        .expect("the demo wrapper was captured");
    assert!(wrapper.request_id.is_some() && wrapper.sha256.is_some());
    let started = std::time::Instant::now();
    let original = state
        .sources
        .get(scan_id, &wrapper.id, SourceView::Original, None)
        .await
        .unwrap();
    let source_ms = started.elapsed().as_secs_f64() * 1e3;
    assert_eq!(original.availability, SourceAvailability::Available);
    let started = std::time::Instant::now();
    let formatted = state
        .sources
        .get(scan_id, &wrapper.id, SourceView::Formatted, None)
        .await
        .unwrap();
    let format_ms = started.elapsed().as_secs_f64() * 1e3;
    let text = formatted.text.unwrap();
    assert!(
        text.lines().any(|line| line.contains("min-width")),
        "the planted bug is in the source"
    );
    assert!(formatted.line_count > original.line_count);
    assert!(
        formatted
            .ad_calls
            .iter()
            .any(|call| call.name == "requestBids")
    );
    // The script's own response is the same content.
    let response = state
        .network
        .request(scan_id, wrapper.request_id.as_deref().unwrap())
        .await
        .unwrap();
    assert_eq!(
        response.response_body.script_id.as_deref(),
        Some(wrapper.id.as_str())
    );
    eprintln!(
        "demo scan reads: request detail {detail_ms:.1} ms, script source {source_ms:.1} ms, formatted {format_ms:.1} ms"
    );

    // Nothing of the capture's credentials or of the app's key is on disk.
    let on_disk = std::fs::read(&pack).unwrap();
    assert!(
        !on_disk
            .windows(24)
            .any(|window| window == b"sk-ant-real-browser-test")
    );
    state.scans.shutdown().await;
}
