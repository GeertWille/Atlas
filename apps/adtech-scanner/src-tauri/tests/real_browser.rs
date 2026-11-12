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
    AdsTxtStatus, DeviceProfile, DiagnosticStatus, FindingCategory, PipelineNode, ScanDetail,
    ScanEvent, ScanOptions, ScanStatus, SettingsPatch, StepStatus,
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
        },
        Arc::new(ChromeCaptureSource),
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

    let started = state
        .scans
        .start(ScanOptions {
            visitor: None,
            extra_pages: None,
            seed: None,
            headless: None,
            persistent_profile: None,
            proxy: None,
            budget_usd: None,
            url: state.demo_site.url.clone(),
            device,
            observe_seconds: 6,
            auto_scroll: true,
            accept_consent: true,
            ai_enabled: true,
        })
        .await
        .unwrap();
    let detail = tokio::time::timeout(Duration::from_secs(120), async {
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
    state.scans.shutdown().await;
    (detail, server)
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
