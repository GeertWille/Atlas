//! The scanner's consent step against real consent platforms on real sites.
//! It visits the open web, and on Browserbase starts real, billed browsers,
//! so it never runs unless asked for by name:
//!
//! ```sh
//! ADSCAN_CONSENT_LIVE=1 ADSCAN_CONSENT_LIVE_SITES=https://www.bild.de/ \
//!   cargo test -p adscan-browser --test consent_live -- --ignored --nocapture
//! ```
//!
//! `ADSCAN_CONSENT_LIVE_SITES` is a comma-separated list of addresses. The
//! visits run in this machine's Chromium unless
//! `ADSCAN_CONSENT_LIVE_BROWSERBASE` names a Browserbase region
//! (`eu-central-1`, ...); `ADSCAN_CONSENT_LIVE_COUNTRY` then adds a
//! Browserbase proxy location (`DE`, `NL`, ...), billed per megabyte. The
//! Browserbase key is `ADSCAN_BROWSERBASE_API_KEY`, or the placeholder
//! `proxy-injected` for a machine whose HTTPS proxy puts the real key in.
//! Every session is tagged `{app: "adscan", install: "sandbox-test"}`, and
//! nothing but sessions with that tag is ever listed or released.
//!
//! It prints, per site, what the scanner found and pressed, what the
//! consent platform reported (`__tcfapi`) and how many ad requests started
//! before and after the click. It asserts only that every session it
//! started was released: what a real site shows changes from day to day.

mod support;

use std::sync::{Arc, Mutex};

use adscan_browser::{Browserbase, BrowserbaseEndpoint, BrowserbaseHooks};
use adscan_model::{
    BodyAdmission, BodySkip, BrowserbaseCapture, BrowserbaseCredentials, BrowserbaseLocation,
    BrowserbaseRegion, BrowserbaseSessionSettings, Capture, CaptureBrowser, CaptureProxy,
    ConsentNoteKind, DeviceProfile, NetworkRecord, RemoteSessionEvent, TrackerEventKind as Kind,
};
use support::{Reported, Requests};

const INSTALL: &str = "sandbox-test";
const PLACEHOLDER_KEY: &str = "proxy-injected";
const OBSERVE_SECONDS: u32 = 15;

/// Hosts whose requests are ad serving or bidding.
const AD_HOSTS: [&str; 22] = [
    "doubleclick.net",
    "googlesyndication.com",
    "googleadservices.com",
    "adnxs.com",
    "amazon-adsystem.com",
    "criteo.com",
    "criteo.net",
    "rubiconproject.com",
    "pubmatic.com",
    "openx.net",
    "smartadserver.com",
    "yieldlab.net",
    "adform.net",
    "teads.tv",
    "casalemedia.com",
    "adsrvr.org",
    "3lift.com",
    "seedtag.com",
    "taboola.com",
    "outbrain.com",
    "improvedigital.com",
    "richaudience.com",
];

fn region(name: &str) -> BrowserbaseRegion {
    match name {
        "us-west-2" => BrowserbaseRegion::UsWest2,
        "us-east-1" => BrowserbaseRegion::UsEast1,
        "ap-southeast-1" => BrowserbaseRegion::ApSoutheast1,
        _ => BrowserbaseRegion::EuCentral1,
    }
}

fn is_ad_request(record: &NetworkRecord) -> bool {
    url::Url::parse(&record.url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_string))
        .is_some_and(|host| {
            AD_HOSTS
                .iter()
                .any(|ad| host == *ad || host.ends_with(&format!(".{ad}")))
        })
}

fn report(url: &str, capture: &Capture, records: &[NetworkRecord], progress: &[String]) {
    eprintln!(
        "  title: {:?}, final url: {}",
        capture.page_title, capture.final_url
    );
    for line in progress.iter().filter(|line| {
        let line = line.to_lowercase();
        line.contains("consent") || line.contains("blocked")
    }) {
        eprintln!("  progress: {line}");
    }
    for note in &capture.consent_notes {
        eprintln!(
            "  note: page {} at {:.0} ms: {:?} {:?}",
            note.page, note.t, note.kind, note.banner
        );
    }
    for event in &capture.events {
        if let Kind::TcfState {
            event_status,
            gdpr_applies,
            has_tc_string,
            purpose_consents,
            vendor_consents,
            ..
        } = &event.kind
        {
            eprintln!(
                "  __tcfapi: page {} at {:.0} ms: {:?}, gdprApplies {:?}, TC string {}, {} purposes, {} vendors",
                event.page,
                event.t,
                event_status,
                gdpr_applies,
                has_tc_string,
                purpose_consents.len(),
                vendor_consents.len()
            );
        }
    }
    let clicked_at = capture
        .consent_notes
        .iter()
        .find(|note| note.kind == ConsentNoteKind::Accepted)
        .map(|note| capture.absolute_ms(note.page, note.t));
    let ads: Vec<f64> = records
        .iter()
        .filter(|record| is_ad_request(record))
        .map(|record| capture.absolute_ms(record.page, record.started_at_ms))
        .collect();
    let auctions = capture
        .events
        .iter()
        .filter(|event| matches!(event.kind, Kind::AuctionInit { .. }))
        .count();
    match clicked_at {
        Some(at) => eprintln!(
            "  ad requests: {} before the click at {at:.0} ms, {} after; auctions: {auctions}",
            ads.iter().filter(|t| **t < at).count(),
            ads.iter().filter(|t| **t >= at).count()
        ),
        None => eprintln!(
            "  ad requests: {} (no click); auctions: {auctions}",
            ads.len()
        ),
    }
    eprintln!(
        "  {url}: consent accepted: {}, {} requests",
        capture.consent_accepted,
        records.len()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "visits real sites, maybe in billed Browserbase browsers; set ADSCAN_CONSENT_LIVE=1 and ask for it by name"]
async fn the_consent_step_on_real_sites() {
    if std::env::var("ADSCAN_CONSENT_LIVE").as_deref() != Ok("1") {
        eprintln!("skipped: ADSCAN_CONSENT_LIVE is not 1");
        return;
    }
    let sites = std::env::var("ADSCAN_CONSENT_LIVE_SITES").unwrap_or_default();
    let remote = std::env::var("ADSCAN_CONSENT_LIVE_BROWSERBASE").ok();
    let country = std::env::var("ADSCAN_CONSENT_LIVE_COUNTRY").ok();
    let api_key =
        std::env::var("ADSCAN_BROWSERBASE_API_KEY").unwrap_or_else(|_| PLACEHOLDER_KEY.to_string());
    let client = Browserbase::new(BrowserbaseEndpoint::default()).unwrap();
    let credentials = match &remote {
        Some(_) => {
            let projects = client
                .projects(&api_key)
                .await
                .expect("the key is accepted");
            let project = projects.first().expect("the key has a project");
            Some(BrowserbaseCredentials {
                api_key: api_key.clone(),
                project_id: project.id.clone(),
            })
        }
        None => None,
    };

    let mut unreleased = Vec::new();
    let mut created = Vec::new();
    for (index, url) in sites.split(',').map(str::trim).enumerate() {
        if url.is_empty() {
            continue;
        }
        let mut request = support::request(url, DeviceProfile::Desktop);
        request.accept_consent = true;
        request.observe_seconds = OBSERVE_SECONDS;
        let events: Arc<Mutex<Vec<RemoteSessionEvent>>> = Arc::default();
        let requests = Requests::admitting(|_| BodyAdmission::Skip(BodySkip::Type));
        let mut reported = Reported::new();
        reported.hooks.network = Some(requests.clone());
        if let (Some(region_name), Some(credentials)) = (&remote, &credentials) {
            request.proxy = CaptureProxy::None;
            request.chrome_path = None;
            request.browser = CaptureBrowser::Browserbase(BrowserbaseCapture {
                region: region(region_name),
                settings: BrowserbaseSessionSettings::STANDARD,
                location: country.clone().map(|country| BrowserbaseLocation {
                    country,
                    state: None,
                    city: None,
                }),
                context_id: None,
                install_id: INSTALL.to_string(),
                scan_id: format!("consent-live-{index}"),
            });
            let heard = events.clone();
            reported.hooks.browserbase = Some(BrowserbaseHooks {
                client: client.clone(),
                credentials: credentials.clone(),
                on_session: Arc::new(move |event| {
                    match &event {
                        RemoteSessionEvent::Created { session_id } => {
                            eprintln!("  session created: {session_id}")
                        }
                        RemoteSessionEvent::Released {
                            session_id,
                            session_seconds,
                            proxy_bytes,
                        } => eprintln!(
                            "  session released: {session_id}, {session_seconds:?} s, {proxy_bytes:?} proxy bytes"
                        ),
                        RemoteSessionEvent::LiveViewReady { .. } => {}
                    }
                    heard.lock().unwrap().push(event);
                }),
            });
        }
        eprintln!("\n=== {url} ===");
        let started = std::time::Instant::now();
        let outcome = reported.run(request).await;
        eprintln!("  took {:.1} s", started.elapsed().as_secs_f64());
        match &outcome.result {
            Ok(capture) => report(url, capture, &requests.records(), &outcome.progress),
            Err(error) => eprintln!("  capture failed: {error}\n  {:#?}", outcome.progress),
        }
        let events = events.lock().unwrap();
        for event in events.iter() {
            if let RemoteSessionEvent::Created { session_id } = event {
                created.push(session_id.clone());
                let released = events.iter().any(|other| {
                    matches!(other, RemoteSessionEvent::Released { session_id: id, .. } if id == session_id)
                });
                if !released {
                    unreleased.push(session_id.clone());
                }
            }
        }
    }

    if let Some(credentials) = &credentials {
        // Only sessions with this test's tag are looked at.
        let still_running = client
            .running_sessions_of(credentials, INSTALL)
            .await
            .unwrap();
        for session in &still_running {
            eprintln!("releasing a session left running: {session}");
            client.release_session(credentials, session).await.unwrap();
        }
        // Browserbase counts proxy bytes late.
        tokio::time::sleep(std::time::Duration::from_secs(35)).await;
        for session in &created {
            match client.session_usage(credentials, session).await {
                Ok(usage) => eprintln!(
                    "usage of {session}: ended {}, {:?} s, {:?} proxy bytes",
                    usage.ended, usage.session_seconds, usage.proxy_bytes
                ),
                Err(error) => eprintln!("usage of {session}: {error}"),
            }
        }
        assert!(
            still_running.is_empty(),
            "sessions were left running: {still_running:?}"
        );
    }
    assert!(
        unreleased.is_empty(),
        "sessions without a confirmed release: {unreleased:?}"
    );
}
