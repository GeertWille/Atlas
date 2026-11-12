//! The bidder-checks node inside real scans: which public files a scan
//! requests, what it stores of them, and how it ends when they misbehave.
//! The web is a test double; no test here contacts a real host.

mod common;

use std::time::Duration;

use adscan_engine::application::ports::FetchError;
use adscan_model::{
    AdsTxtStatus, Capture, DeviceProfile, DiagnosticArea, DiagnosticStatus, PipelineNode,
    ScanDetail, ScanStatus, SettingsPatch, StepStatus,
};
use common::*;
use serde_json::json;

/// Nothing listens here; these scans run without AI.
const NO_API: &str = "http://127.0.0.1:1";

fn harness_for(capture: Capture) -> Harness {
    harness(CaptureBehavior::Return(Box::new(capture)), NO_API, None)
}

fn healthy() -> Capture {
    healthy_capture(DeviceProfile::Desktop)
}

fn checks_step(detail: &ScanDetail) -> &adscan_model::PipelineStep {
    detail
        .steps
        .iter()
        .find(|step| step.node == PipelineNode::BidderDiagnostics)
        .unwrap()
}

fn rules(detail: &ScanDetail) -> Vec<&str> {
    detail
        .findings
        .iter()
        .map(|finding| finding.title.as_str())
        .collect()
}

async fn turn_on_sellers_json(harness: &Harness) {
    let patch = SettingsPatch {
        check_sellers_json: Some(true),
        ..SettingsPatch::default()
    };
    assert!(
        harness
            .settings
            .update(patch)
            .await
            .unwrap()
            .check_sellers_json
    );
}

#[tokio::test]
async fn a_scan_reads_the_sites_ads_txt_and_stores_what_it_concluded() {
    let harness = harness_for(healthy());

    let detail = run_scan(&harness, scan_options(false)).await;

    assert_eq!(detail.summary.status, ScanStatus::Completed);
    let step = checks_step(&detail);
    assert_eq!(step.status, StepStatus::Completed);
    assert_eq!(
        step.summary.as_deref(),
        Some(
            "2 bidders checked: 0 problems, 0 notices; ads.txt has 2 records; sellers.json not checked"
        )
    );
    assert_eq!(
        rules(&detail),
        Vec::<&str>::new(),
        "a healthy page stays silent"
    );

    // One request, to the scanned site's own root domain, and nothing else.
    let requests = harness.files.requests.lock().unwrap().clone();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        (
            requests[0].url.as_str(),
            requests[0].max_bytes,
            requests[0].timeout,
            requests[0].loopback_origin.as_deref(),
        ),
        (SITE_ADS_TXT_URL, 1024 * 1024, Duration::from_secs(10), None)
    );

    let diagnostics = detail.bidder_diagnostics.as_ref().expect("the checks ran");
    assert_eq!(
        (
            diagnostics.ads_txt.url.as_str(),
            diagnostics.ads_txt.status,
            diagnostics.ads_txt.record_count,
        ),
        (SITE_ADS_TXT_URL, AdsTxtStatus::Found, 2)
    );
    assert!(!diagnostics.sellers_json_checked);
    assert_eq!(
        diagnostics
            .bidders
            .iter()
            .map(|bidder| (bidder.bidder.as_str(), bidder.requests, bidder.bids))
            .collect::<Vec<_>>(),
        [("appnexus", 3, 2), ("rubicon", 3, 3)]
    );
    // It survives a reload from storage unchanged.
    let reloaded = harness.scans.get(&detail.summary.id).await.unwrap();
    assert_eq!(reloaded.bidder_diagnostics, detail.bidder_diagnostics);

    let logs = harness
        .scans
        .logs(&detail.summary.id, Some(&step.id))
        .await
        .unwrap();
    let messages: Vec<&str> = logs.iter().map(|entry| entry.message.as_str()).collect();
    assert_eq!(
        messages,
        [
            "Requesting https://pub.example/ads.txt",
            "https://pub.example/ads.txt: 2 seller records, 0 invalid lines",
            "Bidder checks: 2 bidders checked: 0 problems, 0 notices; ads.txt has 2 records; sellers.json not checked",
        ]
    );
    assert_eq!(
        logs[2].data.as_ref().unwrap()["diagnostics"]["adsTxt"]["recordCount"],
        2
    );
}

#[tokio::test]
async fn exchanges_are_only_contacted_when_the_setting_is_on_and_only_those_ads_txt_names() {
    let harness = harness_for(healthy());
    harness.files.answer(
        "https://appnexus.com/sellers.json",
        Ok(adscan_engine::application::ports::PublicFile {
            status: 200,
            content_type: Some("application/json".into()),
            body: json!({ "sellers": [
                { "seller_id": "958", "seller_type": "PUBLISHER", "domain": "pub.example" },
                { "seller_id": "1", "seller_type": "PUBLISHER", "domain": "other.example" }
            ]})
            .to_string()
            .into_bytes(),
        }),
    );
    harness.files.answer(
        "https://rubiconproject.com/sellers.json",
        text_file(503, ""),
    );

    let off = run_scan(&harness, scan_options(false)).await;
    assert_eq!(harness.files.requested_urls(), [SITE_ADS_TXT_URL]);
    assert!(!off.bidder_diagnostics.unwrap().sellers_json_checked);

    turn_on_sellers_json(&harness).await;
    harness.files.requests.lock().unwrap().clear();
    let on = run_scan(&harness, scan_options(false)).await;

    assert_eq!(
        harness.files.requested_urls(),
        [
            "https://appnexus.com/sellers.json",
            SITE_ADS_TXT_URL,
            "https://rubiconproject.com/sellers.json",
        ]
    );
    for request in harness.files.requests.lock().unwrap().iter() {
        assert_eq!(request.loopback_origin, None);
        if request.url.ends_with("/sellers.json") {
            assert_eq!(
                (request.max_bytes, request.timeout),
                (24 * 1024 * 1024, Duration::from_secs(20))
            );
        }
    }
    let diagnostics = on.bidder_diagnostics.as_ref().unwrap();
    assert!(diagnostics.sellers_json_checked);
    let sellers_checks = |name: &str| -> Vec<(DiagnosticStatus, &str)> {
        diagnostics
            .bidders
            .iter()
            .find(|bidder| bidder.bidder == name)
            .unwrap()
            .checks
            .iter()
            .filter(|check| check.area == DiagnosticArea::SellersJson)
            .map(|check| (check.status, check.summary.as_str()))
            .collect()
    };
    assert_eq!(
        sellers_checks("appnexus"),
        [(
            DiagnosticStatus::Pass,
            "Seller id 958 is listed in the sellers.json of appnexus.com"
        )]
    );
    assert_eq!(
        sellers_checks("rubicon"),
        [(
            DiagnosticStatus::NotChecked,
            "The sellers.json of rubiconproject.com could not be read: the exchange answered HTTP 503"
        )]
    );
    assert_eq!(
        checks_step(&on).summary.as_deref(),
        Some(
            "2 bidders checked: 0 problems, 0 notices; ads.txt has 2 records; sellers.json read at 2 exchanges"
        )
    );
    assert_eq!(rules(&on), Vec::<&str>::new());
}

#[tokio::test]
async fn a_site_without_ads_txt_gets_one_finding_and_no_exchange_is_asked() {
    let harness = harness_for(healthy());
    harness.files.answer(SITE_ADS_TXT_URL, text_file(404, ""));
    turn_on_sellers_json(&harness).await;

    let detail = run_scan(&harness, scan_options(false)).await;

    assert_eq!(rules(&detail), ["The site has no ads.txt"]);
    assert_eq!(harness.files.requested_urls(), [SITE_ADS_TXT_URL]);
    let diagnostics = detail.bidder_diagnostics.unwrap();
    assert_eq!(diagnostics.ads_txt.status, AdsTxtStatus::Missing);
    assert!(diagnostics.sellers_json_checked);
}

#[tokio::test]
async fn an_ads_txt_that_cannot_be_read_is_reported_as_such_and_proves_nothing() {
    for (answer, reason) in [
        (Err(FetchError::TimedOut), "the request timed out"),
        (
            Err(FetchError::TooLarge(1024 * 1024)),
            "the file is larger than 1048576 bytes",
        ),
        (
            Err(FetchError::Refused(
                "the address was not contacted: only https addresses are read".into(),
            )),
            "the address was not contacted: only https addresses are read",
        ),
        (text_file(403, "Forbidden"), "the site answered HTTP 403"),
        (
            text_file(200, "<!doctype html><html>Checking your browser"),
            "the site answered with a web page instead of a text file",
        ),
    ] {
        let harness = harness_for(healthy());
        harness.files.answer(SITE_ADS_TXT_URL, answer);

        let detail = run_scan(&harness, scan_options(false)).await;

        assert_eq!(detail.summary.status, ScanStatus::Completed, "{reason}");
        assert_eq!(rules(&detail), Vec::<&str>::new(), "{reason}");
        let diagnostics = detail.bidder_diagnostics.as_ref().unwrap();
        assert_eq!(
            (
                diagnostics.ads_txt.status,
                diagnostics.ads_txt.detail.as_deref()
            ),
            (AdsTxtStatus::Unreadable, Some(reason))
        );
        assert_eq!(
            checks_step(&detail).summary.as_deref(),
            Some(
                "2 bidders checked: 0 problems, 0 notices; ads.txt could not be read; sellers.json not checked"
            )
        );
    }
}

#[tokio::test]
async fn a_subdomain_the_root_file_hands_over_is_read_from_its_own_ads_txt() {
    let mut capture = healthy();
    capture.final_url = "https://blog.pub.example/2026/11/story".into();
    let harness = harness_for(capture);
    harness.files.answer(
        SITE_ADS_TXT_URL,
        text_file(
            200,
            "subdomain=blog.pub.example\nappnexus.com, 958, DIRECT\n",
        ),
    );
    harness.files.answer(
        "https://blog.pub.example/ads.txt",
        text_file(200, "rubiconproject.com, 1001, DIRECT\n"),
    );

    let detail = run_scan(&harness, scan_options(false)).await;

    assert_eq!(
        harness.files.requested_urls(),
        ["https://blog.pub.example/ads.txt", SITE_ADS_TXT_URL]
    );
    let diagnostics = detail.bidder_diagnostics.as_ref().unwrap();
    assert_eq!(diagnostics.ads_txt.url, "https://blog.pub.example/ads.txt");
    assert_eq!(diagnostics.ads_txt.record_count, 1);
    assert_eq!(
        rules(&detail),
        ["1 bidder(s) are not authorised by the site's ads.txt"],
        "the subdomain's file does not list appnexus"
    );
    assert_eq!(detail.findings[0].affected_bidders, ["appnexus"]);
}

#[tokio::test]
async fn a_page_on_this_machine_is_asked_at_its_own_origin_and_no_exchange_is_asked_about_it() {
    let mut capture = healthy();
    capture.final_url = "http://127.0.0.1:4455/".into();
    let harness = harness_for(capture);
    harness.files.answer(
        "http://127.0.0.1:4455/ads.txt",
        text_file(200, SITE_ADS_TXT),
    );
    turn_on_sellers_json(&harness).await;

    let detail = run_scan(&harness, scan_options(false)).await;

    let requests = harness.files.requests.lock().unwrap().clone();
    let asked: Vec<(&str, Option<&str>)> = requests
        .iter()
        .map(|request| (request.url.as_str(), request.loopback_origin.as_deref()))
        .collect();
    assert_eq!(
        asked,
        [(
            "http://127.0.0.1:4455/ads.txt",
            Some("http://127.0.0.1:4455")
        )],
        "the demo site and local builds never send the scanner to a real exchange"
    );
    let diagnostics = detail.bidder_diagnostics.unwrap();
    assert_eq!(diagnostics.ads_txt.status, AdsTxtStatus::Found);
    assert!(!diagnostics.sellers_json_checked);
}

#[tokio::test]
async fn a_page_without_bidders_skips_the_checks_and_requests_nothing() {
    let harness = harness_for(never_started_capture(DeviceProfile::Desktop, false));
    turn_on_sellers_json(&harness).await;

    let detail = run_scan(&harness, scan_options(false)).await;

    assert_eq!(detail.summary.status, ScanStatus::Completed);
    let step = checks_step(&detail);
    assert_eq!(
        (step.status, step.summary.as_deref()),
        (
            StepStatus::Skipped,
            Some("The page has no Prebid bidders to check")
        )
    );
    assert_eq!(detail.bidder_diagnostics, None);
    assert_eq!(harness.files.requested_urls(), Vec::<String>::new());
}

#[tokio::test]
async fn cancelling_while_a_file_is_being_read_cancels_the_scan() {
    let harness = harness_for(healthy());
    harness.files.stall(SITE_ADS_TXT_URL);

    let started = harness.scans.start(scan_options(false)).await.unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        while harness.files.requests.lock().unwrap().is_empty() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the scan never asked for ads.txt");
    harness.scans.cancel(&started.id).await.unwrap();
    let detail = wait_for_end(&harness, &started.id).await;

    assert_eq!(detail.summary.status, ScanStatus::Cancelled);
    let step = checks_step(&detail);
    assert_eq!(
        (step.status, step.summary.as_deref()),
        (StepStatus::Failed, Some("Cancelled"))
    );
    assert_eq!(detail.bidder_diagnostics, None);
}

#[tokio::test]
async fn the_files_themselves_are_never_stored() {
    const ADS_TXT_MARKER: &str = "ADS-TXT-COMMENT-THAT-MUST-NOT-BE-STORED";
    const SELLERS_MARKER: &str = "SELLERS-JSON-ENTRY-THAT-MUST-NOT-BE-STORED";
    let harness = harness_for(healthy());
    harness.files.answer(
        SITE_ADS_TXT_URL,
        text_file(
            200,
            &format!(
                "# {ADS_TXT_MARKER}\n{SITE_ADS_TXT}unrelated-exchange.example, {ADS_TXT_MARKER}, RESELLER\n"
            ),
        ),
    );
    for exchange in ["appnexus.com", "rubiconproject.com"] {
        harness.files.answer(
            &format!("https://{exchange}/sellers.json"),
            text_file(
                200,
                &json!({ "sellers": [
                    { "seller_id": "958", "seller_type": "PUBLISHER", "domain": "pub.example", "name": SELLERS_MARKER },
                    { "seller_id": "1001", "seller_type": "PUBLISHER", "domain": "pub.example", "comment": SELLERS_MARKER },
                    { "seller_id": "31337", "name": SELLERS_MARKER, "domain": "someone-else.example" }
                ]})
                .to_string(),
            ),
        );
    }
    turn_on_sellers_json(&harness).await;

    let detail = run_scan(&harness, scan_options(false)).await;

    assert_eq!(
        detail
            .bidder_diagnostics
            .as_ref()
            .unwrap()
            .ads_txt
            .record_count,
        3
    );
    let logs = harness.scans.logs(&detail.summary.id, None).await.unwrap();
    let everything_stored = format!(
        "{}{}",
        serde_json::to_string(&detail).unwrap(),
        serde_json::to_string(&logs).unwrap()
    );
    assert!(!everything_stored.contains(ADS_TXT_MARKER));
    assert!(!everything_stored.contains(SELLERS_MARKER));
    assert!(!everything_stored.contains("someone-else.example"));
    // What is kept is the line that authorises the bidder.
    assert!(everything_stored.contains("appnexus.com, 958, DIRECT"));
}
