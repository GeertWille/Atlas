//! One test against the real Browserbase. It starts real, billed browsers,
//! so it never runs unless asked for by name:
//!
//! ```sh
//! ADSCAN_BROWSERBASE_LIVE=1 cargo test -p adscan-browser --test browserbase_live -- --ignored --nocapture
//! ```
//!
//! The key is `ADSCAN_BROWSERBASE_API_KEY`. Without it the header carries
//! the placeholder `proxy-injected`, for a machine whose HTTPS proxy puts
//! the real key in. Two short sessions are started and both are released.
//! They are tagged with an installation id of their own, and nothing but
//! sessions with that tag is ever listed or released: the project may be
//! shared with other applications.

mod support;

use std::sync::{Arc, Mutex};

use adscan_browser::{Browserbase, BrowserbaseEndpoint, BrowserbaseHooks};
use adscan_model::{
    ActivityKind, BrowserbaseCapture, BrowserbaseCredentials, BrowserbaseRegion,
    BrowserbaseSessionSettings, CaptureBrowser, CaptureProxy, CaptureRequest, CapturedBody,
    DeviceProfile, RemoteSessionEvent, RequestContext, VisitorBehavior,
};
use support::{Reported, Requests};

/// The `userMetadata.install` of every session this test starts.
const INSTALL: &str = "sandbox-test";
const PLACEHOLDER_KEY: &str = "proxy-injected";

fn region() -> BrowserbaseRegion {
    match std::env::var("ADSCAN_BROWSERBASE_LIVE_REGION").as_deref() {
        Ok("us-west-2") => BrowserbaseRegion::UsWest2,
        Ok("eu-central-1") => BrowserbaseRegion::EuCentral1,
        Ok("ap-southeast-1") => BrowserbaseRegion::ApSoutheast1,
        _ => BrowserbaseRegion::UsEast1,
    }
}

fn request(url: &str, scan: &str, visitor: VisitorBehavior, seconds: u32) -> CaptureRequest {
    CaptureRequest {
        url: url.to_string(),
        device: DeviceProfile::Desktop,
        observe_seconds: seconds,
        visitor,
        extra_pages: 0,
        seed: 1,
        accept_consent: true,
        headless: true,
        persistent_profile: false,
        proxy: CaptureProxy::None,
        chrome_path: None,
        browser: CaptureBrowser::Browserbase(BrowserbaseCapture {
            region: region(),
            settings: BrowserbaseSessionSettings::STANDARD,
            location: None,
            context_id: None,
            install_id: INSTALL.to_string(),
            scan_id: scan.to_string(),
        }),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "starts real, billed Browserbase sessions; set ADSCAN_BROWSERBASE_LIVE=1 and ask for it by name"]
async fn two_real_pages_are_captured_in_real_browserbase_browsers_and_both_are_released() {
    if std::env::var("ADSCAN_BROWSERBASE_LIVE").as_deref() != Ok("1") {
        eprintln!("skipped: ADSCAN_BROWSERBASE_LIVE is not 1");
        return;
    }
    let api_key =
        std::env::var("ADSCAN_BROWSERBASE_API_KEY").unwrap_or_else(|_| PLACEHOLDER_KEY.to_string());
    let client = Browserbase::new(BrowserbaseEndpoint::default()).unwrap();

    // Costs nothing: the key works and names a project.
    let projects = client
        .projects(&api_key)
        .await
        .expect("the key is accepted");
    let project = projects.first().expect("the key has a project");
    eprintln!(
        "project: {:?}, concurrency {}, {} project(s) on the key",
        project.name,
        project.concurrency,
        projects.len()
    );
    let credentials = BrowserbaseCredentials {
        api_key: api_key.clone(),
        project_id: project.id.clone(),
    };
    let usage_before = client.project_usage(&credentials).await.unwrap();

    let visits = [
        (
            "https://example.com/",
            "live-example",
            VisitorBehavior::None,
            5,
        ),
        (
            "https://www.theguardian.com/international",
            "live-news",
            VisitorBehavior::Scroll,
            20,
        ),
    ];
    let mut failures = Vec::new();
    for (url, scan, visitor, seconds) in visits {
        let events: Arc<Mutex<Vec<RemoteSessionEvent>>> = Arc::default();
        let heard = events.clone();
        let requests = Requests::keeping_text();
        let mut reported = Reported::new();
        reported.hooks.network = Some(requests.clone());
        reported.hooks.browserbase = Some(BrowserbaseHooks {
            client: client.clone(),
            credentials: credentials.clone(),
            on_session: Arc::new(move |event| heard.lock().unwrap().push(event)),
        });
        let started = std::time::Instant::now();
        let outcome = reported.run(request(url, scan, visitor, seconds)).await;
        eprintln!("\n=== {url} ({:.1} s) ===", started.elapsed().as_secs_f64());
        for activity in &outcome.activities {
            let shown = matches!(
                activity.kind,
                ActivityKind::RemoteBrowser | ActivityKind::Environment | ActivityKind::Blocked
            );
            if shown || activity.kind == ActivityKind::Progress {
                eprintln!("  {:?}: {}", activity.kind, activity.message);
            }
            if activity.kind == ActivityKind::Environment {
                eprintln!("    {}", activity.data.clone().unwrap_or_default());
            }
        }
        // No credential in anything that was said.
        for activity in &outcome.activities {
            let said = format!("{} {:?}", activity.message, activity.data);
            for credential in [
                "connect.",
                "wss://",
                "signingKey",
                "inspector.html",
                &project.id,
            ] {
                assert!(
                    !said.contains(credential),
                    "an activity holds `{credential}`: {said}"
                );
            }
            assert!(api_key == PLACEHOLDER_KEY || !said.contains(&api_key));
        }
        let events = events.lock().unwrap().clone();
        let created = events
            .iter()
            .filter(|event| matches!(event, RemoteSessionEvent::Created { .. }))
            .count();
        let released: Vec<_> = events
            .iter()
            .filter_map(|event| match event {
                RemoteSessionEvent::Released {
                    session_seconds,
                    proxy_bytes,
                    ..
                } => Some((*session_seconds, *proxy_bytes)),
                _ => None,
            })
            .collect();
        eprintln!("  sessions created: {created}, released: {released:?}");
        if created != released.len() {
            failures.push(format!(
                "{url}: {created} created, {} released",
                released.len()
            ));
        }
        let capture = match &outcome.result {
            Ok(capture) => capture,
            Err(error) => {
                failures.push(format!("{url}: {error}"));
                continue;
            }
        };
        let records = requests.records();
        let read: Vec<u64> = records
            .iter()
            .filter_map(|record| match &record.body {
                CapturedBody::Read { total_bytes, .. } => Some(*total_bytes),
                _ => None,
            })
            .collect();
        let unavailable = records
            .iter()
            .filter(|record| matches!(record.body, CapturedBody::Unavailable(_)))
            .count();
        let too_large = records
            .iter()
            .filter(|record| matches!(record.body, CapturedBody::TooLargeForRemote { .. }))
            .count();
        let in_frames = records
            .iter()
            .filter(|record| record.context == RequestContext::Iframe)
            .count();
        let in_child_targets = records
            .iter()
            .filter(|record| record.target_id.is_some())
            .count();
        let hosts: std::collections::BTreeSet<String> = records
            .iter()
            .filter_map(|record| {
                url::Url::parse(&record.url)
                    .ok()?
                    .host_str()
                    .map(str::to_string)
            })
            .collect();
        let tracker_kinds: std::collections::BTreeMap<String, usize> =
            capture
                .events
                .iter()
                .fold(Default::default(), |mut kinds, event| {
                    let kind = serde_json::to_value(&event.kind).unwrap()["type"]
                        .as_str()
                        .unwrap_or_default()
                        .to_string();
                    *kinds.entry(kind).or_default() += 1;
                    kinds
                });
        eprintln!(
            "  title: {:?}, final url: {}",
            capture.page_title, capture.final_url
        );
        eprintln!(
            "  requests: {} on {} hosts, {} in frames, {} in targets of their own",
            records.len(),
            hosts.len(),
            in_frames,
            in_child_targets
        );
        eprintln!(
            "  bodies read: {} ({} bytes, largest {}), unavailable: {unavailable}, too large: {too_large}",
            read.len(),
            read.iter().sum::<u64>(),
            read.iter().max().copied().unwrap_or(0)
        );
        eprintln!(
            "  scripts: {}, console lines: {}, consent accepted: {}, load: {:?} ms",
            capture.scripts.len(),
            capture.console.len(),
            capture.consent_accepted,
            capture.page_load_ms
        );
        eprintln!("  tracker events: {tracker_kinds:?}");
        eprintln!("  environment: {:?}", capture.environment.browserbase);
        eprintln!(
            "  webdriver: {:?}, user agent: {}",
            capture.environment.webdriver, capture.user_agent
        );
        let service_lines: Vec<&str> = capture
            .console
            .iter()
            .map(|line| line.text.as_str())
            .filter(|text| text.to_lowercase().contains("browserbase"))
            .collect();
        if !service_lines.is_empty() {
            failures.push(format!(
                "{url}: the service's console lines were kept: {service_lines:?}"
            ));
        }
        if capture.environment.browserbase.is_none() || records.is_empty() {
            failures.push(format!(
                "{url}: nothing was recorded of the visit or its browser"
            ));
        }
        // The browser's clock and the capture's agree: no request starts
        // long before its page view or after the visit's end.
        let page = &capture.pages[0];
        let outside = records
            .iter()
            .filter(|record| record.page == 0)
            .filter(|record| {
                record.started_at_ms < -1_000.0 || record.started_at_ms > page.ended_at_ms + 1_000.0
            })
            .count();
        eprintln!(
            "  page view lasted {:.0} ms; requests timed outside it: {outside}",
            page.ended_at_ms
        );
        if outside > 0 {
            failures.push(format!(
                "{url}: {outside} requests are timed outside their page view"
            ));
        }
    }

    // Nothing of this test is left running. Only sessions with this
    // test's tag are looked at.
    let still_running = client
        .running_sessions_of(&credentials, INSTALL)
        .await
        .unwrap();
    for session in &still_running {
        client.release_session(&credentials, session).await.unwrap();
    }
    let usage_after = client.project_usage(&credentials).await.unwrap();
    eprintln!(
        "\nproject usage (all its users): {} -> {} browser minutes, {} -> {} proxy bytes",
        usage_before.browser_minutes,
        usage_after.browser_minutes,
        usage_before.proxy_bytes,
        usage_after.proxy_bytes
    );
    assert!(
        still_running.is_empty(),
        "sessions were left running: {still_running:?}"
    );
    assert!(failures.is_empty(), "{failures:#?}");
}
