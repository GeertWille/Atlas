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

/// The lookup is on unless the user switches it off.
async fn check_sellers_json(harness: &Harness, on: bool) {
    let patch = SettingsPatch {
        check_sellers_json: Some(on),
        ..SettingsPatch::default()
    };
    assert_eq!(
        harness
            .settings
            .update(patch)
            .await
            .unwrap()
            .check_sellers_json,
        on
    );
}

fn sellers_of_the_site() -> Result<adscan_engine::application::ports::PublicFile, FetchError> {
    text_file(
        200,
        &json!({ "sellers": [
            { "seller_id": "958", "seller_type": "PUBLISHER", "domain": "pub.example" },
            { "seller_id": "1001", "seller_type": "PUBLISHER", "domain": "pub.example" }
        ]})
        .to_string(),
    )
}

#[tokio::test]
async fn a_scan_reads_the_sites_ads_txt_and_stores_what_it_concluded() {
    let harness = harness_for(healthy());
    check_sellers_json(&harness, false).await;

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
    // Asked for as the scan's browser, not under the app's own name.
    assert_eq!(
        (
            requests[0].identity.user_agent.as_str(),
            requests[0].identity.accept_language.as_str()
        ),
        ("Mozilla/5.0 (Macintosh)", "en-US,en;q=0.9")
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
            final_url: "https://appnexus.com/sellers.json".into(),
            version: Default::default(),
        }),
    );
    harness.files.answer(
        "https://rubiconproject.com/sellers.json",
        text_file(503, ""),
    );

    check_sellers_json(&harness, false).await;
    let off = run_scan(&harness, scan_options(false)).await;
    assert_eq!(harness.files.requested_urls(), [SITE_ADS_TXT_URL]);
    assert!(!off.bidder_diagnostics.unwrap().sellers_json_checked);

    check_sellers_json(&harness, true).await;
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

/// Scanning a site again, or another site that sells through the same
/// exchanges, asks no exchange a second time within a day.
#[tokio::test]
async fn a_second_scan_reads_the_exchanges_files_from_the_copies_the_first_one_kept() {
    let harness = harness_for(healthy());
    for exchange in ["appnexus.com", "rubiconproject.com"] {
        harness.files.answer(
            &format!("https://{exchange}/sellers.json"),
            sellers_of_the_site(),
        );
    }
    // Requests of a scan: all of them, and those for a sellers.json.
    let requests_made = |harness: &Harness| {
        let asked = std::mem::take(&mut *harness.files.requests.lock().unwrap());
        let of_exchanges = asked
            .iter()
            .filter(|request| request.url.ends_with("/sellers.json"))
            .count();
        (asked.len(), of_exchanges)
    };

    // The lookup is on without anyone switching it on.
    let first = run_scan(&harness, scan_options(false)).await;
    assert_eq!(requests_made(&harness), (3, 2));
    assert_eq!(
        lookup_lines(&harness, &first).await,
        [
            "sellers.json of appnexus.com: 1 of the seller ids found (downloaded)",
            "sellers.json of rubiconproject.com: 1 of the seller ids found (downloaded)",
        ]
    );

    let second = run_scan(&harness, scan_options(false)).await;
    assert_eq!(
        requests_made(&harness),
        (1, 0),
        "only the site's own ads.txt is requested again"
    );
    assert_eq!(
        lookup_lines(&harness, &second).await,
        [
            "sellers.json of appnexus.com: 1 of the seller ids found (kept copy, fetched less than a minute ago)",
            "sellers.json of rubiconproject.com: 1 of the seller ids found (kept copy, fetched less than a minute ago)",
        ]
    );
    // The verdicts are the same either way.
    assert!(
        first
            .bidder_diagnostics
            .as_ref()
            .unwrap()
            .sellers_json_checked
    );
    assert_eq!(first.bidder_diagnostics, second.bidder_diagnostics);
    assert_eq!(
        checks_step(&second).summary.as_deref(),
        Some(
            "2 bidders checked: 0 problems, 0 notices; ads.txt has 2 records; sellers.json read at 2 exchanges"
        )
    );
    assert_eq!(harness.sellers_json.cache_info().await.entries, 2);

    // Clearing the kept files makes the next scan ask the exchanges again.
    harness.sellers_json.clear_cache().await.unwrap();
    run_scan(&harness, scan_options(false)).await;
    assert_eq!(requests_made(&harness), (3, 2));
}

/// The cache's own clock is the system's, so the kept copy is aged on disk.
#[tokio::test]
async fn an_exchange_that_is_down_is_judged_from_the_kept_copy_and_the_check_says_so() {
    let harness = harness_for(healthy());
    for exchange in ["appnexus.com", "rubiconproject.com"] {
        harness.files.answer(
            &format!("https://{exchange}/sellers.json"),
            sellers_of_the_site(),
        );
    }
    run_scan(&harness, scan_options(false)).await;

    // Two days pass, and appnexus stops answering.
    let cache = harness.data_dir.path().join("cache/sellers-json");
    let two_days_ago = 1_000.0
        * (std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs_f64()
            - 2.0 * 24.0 * 3_600.0);
    for entry in std::fs::read_dir(&cache).unwrap() {
        let path = entry.unwrap().path();
        if path.to_str().unwrap().ends_with(".meta.json") {
            let mut meta: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            meta["fetchedAt"] = json!(two_days_ago);
            std::fs::write(&path, meta.to_string()).unwrap();
        }
    }
    harness.files.answer(
        "https://appnexus.com/sellers.json",
        Err(FetchError::TimedOut),
    );
    let fetched_on = adscan_engine::domain::diagnostics::sellers_cache::utc_day(two_days_ago);

    let detail = run_scan(&harness, scan_options(false)).await;

    let diagnostics = detail.bidder_diagnostics.as_ref().unwrap();
    let sellers_checks = |name: &str| -> Vec<(DiagnosticStatus, String)> {
        diagnostics
            .bidders
            .iter()
            .find(|bidder| bidder.bidder == name)
            .unwrap()
            .checks
            .iter()
            .filter(|check| check.area == DiagnosticArea::SellersJson)
            .map(|check| (check.status, check.summary.clone()))
            .collect()
    };
    assert_eq!(
        sellers_checks("appnexus"),
        [(
            DiagnosticStatus::Pass,
            format!(
                "Seller id 958 is listed in the sellers.json of appnexus.com (from a copy fetched {fetched_on}: the exchange could not be reached during this scan)"
            )
        )]
    );
    // The exchange that answered is read anew and carries no such note.
    assert_eq!(
        sellers_checks("rubicon"),
        [(
            DiagnosticStatus::Pass,
            "Seller id 1001 is listed in the sellers.json of rubiconproject.com".to_string()
        )]
    );
    assert_eq!(
        lookup_lines(&harness, &detail).await,
        [
            format!(
                "sellers.json of appnexus.com: 1 of the seller ids found (the exchange could not be reached: kept copy, fetched {fetched_on})"
            ),
            "sellers.json of rubiconproject.com: 1 of the seller ids found (downloaded)"
                .to_string(),
        ]
    );
}

/// An exchange that answers, with something that is not a sellers.json,
/// was reached: the check read from the kept copy says it answered.
#[tokio::test]
async fn an_exchange_that_answers_with_a_web_page_is_judged_from_the_kept_copy_and_the_check_says_it_answered()
 {
    const PAGE: &str = "<!DOCTYPE html><html><body>Checking your browser</body></html>";
    let harness = harness_for(healthy());
    for exchange in ["appnexus.com", "rubiconproject.com"] {
        harness.files.answer(
            &format!("https://{exchange}/sellers.json"),
            sellers_of_the_site(),
        );
    }
    run_scan(&harness, scan_options(false)).await;

    // Two days pass, and appnexus answers with a challenge page.
    let cache = harness.data_dir.path().join("cache/sellers-json");
    let two_days_ago = 1_000.0
        * (std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs_f64()
            - 2.0 * 24.0 * 3_600.0);
    for entry in std::fs::read_dir(&cache).unwrap() {
        let path = entry.unwrap().path();
        if path.to_str().unwrap().ends_with(".meta.json") {
            let mut meta: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            meta["fetchedAt"] = json!(two_days_ago);
            std::fs::write(&path, meta.to_string()).unwrap();
        }
    }
    harness
        .files
        .answer("https://appnexus.com/sellers.json", text_file(200, PAGE));
    let fetched_on = adscan_engine::domain::diagnostics::sellers_cache::utc_day(two_days_ago);

    let detail = run_scan(&harness, scan_options(false)).await;

    let appnexus: Vec<(DiagnosticStatus, &str)> = detail
        .bidder_diagnostics
        .as_ref()
        .unwrap()
        .bidders
        .iter()
        .find(|bidder| bidder.bidder == "appnexus")
        .unwrap()
        .checks
        .iter()
        .filter(|check| check.area == DiagnosticArea::SellersJson)
        .map(|check| (check.status, check.summary.as_str()))
        .collect();
    assert_eq!(
        appnexus,
        [(
            DiagnosticStatus::Pass,
            format!(
                "Seller id 958 is listed in the sellers.json of appnexus.com (from a copy fetched {fetched_on}: the exchange answered during this scan, but not with a valid sellers.json)"
            )
            .as_str()
        )]
    );
    assert!(
        !appnexus[0].1.contains("could not be reached"),
        "{appnexus:?}"
    );
    assert_eq!(
        lookup_lines(&harness, &detail).await,
        [
            format!(
                "sellers.json of appnexus.com: 1 of the seller ids found (the exchange's answer was not a valid sellers.json: kept copy, fetched {fetched_on})"
            ),
            "sellers.json of rubiconproject.com: 1 of the seller ids found (downloaded)"
                .to_string(),
        ]
    );
}

/// A page served with status 200 where the file should be says nothing about
/// the site's seller id: the check is "could not be read", never "not listed".
#[tokio::test]
async fn an_exchange_that_serves_a_web_page_is_reported_as_unreadable_never_as_seller_not_listed() {
    const PAGE: &str = "<!DOCTYPE html><html><body>Checking your browser</body></html>";
    const TRUNCATED: &str = r#"{"sellers":[{"seller_id":"1001","seller_type":"PUBL"#;
    let harness = harness_for(healthy());
    harness
        .files
        .answer("https://appnexus.com/sellers.json", text_file(200, PAGE));
    harness.files.answer(
        "https://rubiconproject.com/sellers.json",
        text_file(200, TRUNCATED),
    );

    let detail = run_scan(&harness, scan_options(false)).await;

    let checks: Vec<(DiagnosticStatus, &str)> = detail
        .bidder_diagnostics
        .as_ref()
        .unwrap()
        .bidders
        .iter()
        .flat_map(|bidder| &bidder.checks)
        .filter(|check| check.area == DiagnosticArea::SellersJson)
        .map(|check| (check.status, check.summary.as_str()))
        .collect();
    assert_eq!(
        checks,
        [
            (
                DiagnosticStatus::NotChecked,
                "The sellers.json of appnexus.com could not be read: the site refused the request: it answered with a web page instead of the file"
            ),
            (
                DiagnosticStatus::NotChecked,
                "The sellers.json of rubiconproject.com could not be read: the file is not valid sellers.json (EOF while parsing a string at line 1 column 51)"
            ),
        ]
    );
    assert_eq!(
        lookup_lines(&harness, &detail).await,
        [
            "sellers.json of appnexus.com could not be read: the site refused the request: it answered with a web page instead of the file",
            "sellers.json of rubiconproject.com could not be read: the file is not valid sellers.json (EOF while parsing a string at line 1 column 51)",
        ]
    );
    assert_eq!(rules(&detail), Vec::<&str>::new());

    // The next scan within the hour does not ask again, and says the same.
    harness.files.requests.lock().unwrap().clear();
    let again = run_scan(&harness, scan_options(false)).await;
    assert_eq!(harness.files.requested_urls(), [SITE_ADS_TXT_URL]);
    assert_eq!(
        lookup_lines(&harness, &again).await,
        [
            "sellers.json of appnexus.com could not be read less than a minute ago: the site refused the request: it answered with a web page instead of the file",
            "sellers.json of rubiconproject.com could not be read less than a minute ago: the file is not valid sellers.json (EOF while parsing a string at line 1 column 51)",
        ]
    );
}

/// What the audit trail says about each exchange's file, in order of exchange.
async fn lookup_lines(harness: &Harness, detail: &ScanDetail) -> Vec<String> {
    let mut lines: Vec<String> = harness
        .scans
        .logs(&detail.summary.id, Some(&checks_step(detail).id))
        .await
        .unwrap()
        .into_iter()
        .map(|entry| entry.message)
        .filter(|message| message.starts_with("sellers.json of "))
        .collect();
    lines.sort();
    lines
}

#[tokio::test]
async fn a_site_without_ads_txt_gets_one_finding_and_no_exchange_is_asked() {
    let harness = harness_for(healthy());
    harness.files.answer(SITE_ADS_TXT_URL, text_file(404, ""));

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
        (
            text_file(403, "Forbidden"),
            "the site refused the request (HTTP 403)",
        ),
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
                "2 bidders checked: 0 problems, 0 notices; ads.txt could not be read; sellers.json read at 0 exchanges"
            ),
            "no ads.txt names an exchange to ask"
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
    check_sellers_json(&harness, false).await;

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
        (StepStatus::Cancelled, Some("Cancelled"))
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
