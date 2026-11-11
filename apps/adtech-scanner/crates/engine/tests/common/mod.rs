//! Canned captures and test doubles for the engine's outer ports. Storage,
//! the pipeline and the Anthropic client under test are always the real ones.
#![allow(dead_code)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use adscan_engine::application::pipeline::PipelineDeps;
use adscan_engine::application::ports::{CaptureError, CaptureSource, EventSink, ProgressFn};
use adscan_engine::application::scans::ScanService;
use adscan_engine::application::settings::SettingsService;
use adscan_engine::infrastructure::anthropic::{AnthropicClient, RetryPolicy};
use adscan_engine::infrastructure::beautify::OxcFormatter;
use adscan_engine::infrastructure::secrets::MemorySecretStore;
use adscan_engine::infrastructure::sqlite::SqliteStore;
use adscan_engine::infrastructure::system::{SystemClock, UuidGenerator};
use adscan_model::{
    BrowserInfo, Capture, CaptureRequest, CapturedScript, DeviceProfile, ScanDetail, ScanEvent,
    ScanOptions, ScanStatus, TrackedAdUnit, TrackerEvent, TrackerEventKind as Ev, Viewport,
};
use async_trait::async_trait;
use serde_json::json;
use tokio_util::sync::CancellationToken;

pub const WRAPPER_SOURCE: &str = r#"(function(){var pbjs=window.pbjs=window.pbjs||{};pbjs.que=pbjs.que||[];var googletag=window.googletag=window.googletag||{cmd:[]};var adUnits=[{code:"top",mediaTypes:{banner:{sizes:[[728,90]]}},bids:[{bidder:"rubicon",params:{accountId:1}}]}];function isMobile(){return window.innerWidth>768}function startAuction(){pbjs.que.push(function(){pbjs.addAdUnits(adUnits);pbjs.requestBids({timeout:1500,bidsBackHandler:function(){pbjs.setTargetingForGPTAsync();googletag.pubads().refresh()}})})}googletag.cmd.push(function(){googletag.defineSlot("/123/top",[728,90],"top").addService(googletag.pubads());googletag.pubads().disableInitialLoad();googletag.enableServices()});if(isMobile()){window.addEventListener("scroll",startAuction,{once:true})}else{startAuction()}})();"#;

pub fn ev(t: f64, kind: Ev) -> TrackerEvent {
    TrackerEvent { t, kind }
}

pub fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| item.to_string()).collect()
}

pub fn script(url: &str, body: &str) -> CapturedScript {
    CapturedScript {
        url: url.into(),
        inline: false,
        body: body.into(),
        loaded_at_ms: Some(100.0),
        initiator: Some("https://pub.example/".into()),
    }
}

pub fn bid_response(t: f64, auction: &str, bidder: &str, code: &str, cpm: f64) -> TrackerEvent {
    ev(
        t,
        Ev::BidResponse {
            auction_id: auction.into(),
            bidder: bidder.into(),
            ad_unit_code: code.into(),
            cpm,
            currency: Some("USD".into()),
            size: Some("300x250".into()),
            time_to_respond: None,
            deal_id: None,
        },
    )
}

pub fn slot_defined(t: f64, id: &str, in_viewport: bool) -> TrackerEvent {
    ev(
        t,
        Ev::SlotDefined {
            element_id: id.into(),
            ad_unit_path: Some(format!("/123/{id}")),
            sizes: strings(&["300x250"]),
            in_viewport: Some(in_viewport),
        },
    )
}

pub fn slot_requested(t: f64, id: &str, with_prebid_targeting: bool) -> TrackerEvent {
    ev(
        t,
        Ev::SlotRequested {
            element_id: id.into(),
            prebid_targeting_keys: if with_prebid_targeting {
                strings(&["hb_pb", "hb_bidder"])
            } else {
                vec![]
            },
        },
    )
}

pub fn slot_rendered(t: f64, id: &str, is_empty: bool) -> TrackerEvent {
    ev(
        t,
        Ev::SlotRenderEnded {
            element_id: id.into(),
            is_empty,
            size: (!is_empty).then(|| "300x250".to_string()),
            advertiser_id: None,
            line_item_id: None,
        },
    )
}

pub fn prebid_config(t: f64, has_floors: bool, has_consent_management: bool) -> TrackerEvent {
    ev(
        t,
        Ev::PrebidConfig {
            bidder_timeout: Some(1500.0),
            price_granularity: Some("medium".into()),
            has_floors,
            has_user_sync: true,
            has_consent_management,
            currency: Some("USD".into()),
            raw: json!({ "bidderTimeout": 1500 }),
        },
    )
}

pub fn prebid_detected(t: f64, global: &str, version: &str) -> TrackerEvent {
    ev(
        t,
        Ev::PrebidDetected {
            global_name: global.into(),
            version: Some(version.into()),
            modules: strings(&["rubiconBidAdapter", "appnexusBidAdapter"]),
        },
    )
}

pub fn lifecycle(t: f64, phase: &str) -> TrackerEvent {
    ev(
        t,
        Ev::PageLifecycle {
            phase: phase.into(),
        },
    )
}

const SLOTS: [&str; 3] = ["top", "side", "footer"];
const BIDDERS: [&str; 2] = ["rubicon", "appnexus"];

/// One complete, healthy auction for the three slots starting at `start`:
/// both bidders answer in time, targeting is set before the ad requests and
/// every slot renders.
pub fn auction_round(start: f64, auction: &str) -> Vec<TrackerEvent> {
    let mut events = vec![ev(
        start,
        Ev::AuctionInit {
            auction_id: auction.into(),
            timeout: Some(1500.0),
            ad_units: SLOTS
                .iter()
                .map(|code| TrackedAdUnit {
                    code: code.to_string(),
                    sizes: strings(&["300x250"]),
                    bidders: strings(&BIDDERS),
                })
                .collect(),
        },
    )];
    for bidder in BIDDERS {
        events.push(ev(
            start + 5.0,
            Ev::BidRequested {
                auction_id: auction.into(),
                bidder: bidder.into(),
                ad_unit_codes: strings(&SLOTS),
            },
        ));
    }
    events.extend([
        bid_response(start + 240.0, auction, "rubicon", "top", 1.2),
        bid_response(start + 240.0, auction, "rubicon", "side", 0.8),
        bid_response(start + 240.0, auction, "rubicon", "footer", 0.5),
        bid_response(start + 300.0, auction, "appnexus", "top", 1.5),
        ev(
            start + 300.0,
            Ev::NoBid {
                auction_id: auction.into(),
                bidder: "appnexus".into(),
                ad_unit_code: "side".into(),
            },
        ),
        bid_response(start + 300.0, auction, "appnexus", "footer", 0.4),
        ev(
            start + 340.0,
            Ev::AuctionEnd {
                auction_id: auction.into(),
            },
        ),
        ev(
            start + 345.0,
            Ev::SetTargeting {
                ad_unit_codes: strings(&SLOTS),
            },
        ),
    ]);
    for slot in SLOTS {
        events.push(slot_requested(start + 350.0, slot, true));
        events.push(ev(
            start + 520.0,
            Ev::SlotResponseReceived {
                element_id: slot.into(),
            },
        ));
        events.push(slot_rendered(start + 540.0, slot, false));
    }
    events.extend([
        ev(
            start + 545.0,
            Ev::BidWon {
                auction_id: Some(auction.into()),
                bidder: "appnexus".into(),
                ad_unit_code: "top".into(),
                cpm: 1.5,
            },
        ),
        ev(
            start + 560.0,
            Ev::AdRenderSucceeded {
                ad_unit_code: Some("top".into()),
                bidder: Some("appnexus".into()),
            },
        ),
    ]);
    events
}

/// Library detection, slot definitions and config that precede any auction.
pub fn page_setup() -> Vec<TrackerEvent> {
    vec![
        prebid_detected(120.0, "pbjs", "9.20.0"),
        ev(
            150.0,
            Ev::GptDetected {
                version: Some("2026".into()),
            },
        ),
        slot_defined(200.0, "top", true),
        slot_defined(200.0, "side", true),
        slot_defined(200.0, "footer", false),
        lifecycle(300.0, "domContentLoaded"),
        prebid_config(350.0, true, true),
    ]
}

pub fn scroll_events(first_scroll: f64) -> Vec<TrackerEvent> {
    vec![
        lifecycle(first_scroll, "firstScroll"),
        ev(first_scroll, Ev::Scroll { depth: 0.2 }),
        ev(first_scroll + 1500.0, Ev::Scroll { depth: 1.0 }),
    ]
}

pub fn page_scripts() -> Vec<CapturedScript> {
    vec![
        script(
            "https://cdn.pub.example/prebid9.20.0.js",
            "var auctionManager={};function registerBidAdapter(){}",
        ),
        script(
            "https://securepubads.g.doubleclick.net/tag/js/gpt.js",
            "/* gpt */",
        ),
        script("https://pub.example/js/ads.js", WRAPPER_SOURCE),
        script("https://pub.example/js/app.js", "document.title='News';"),
        script(
            "https://pub.example/js/lazy.js",
            "function lazy(el){if(el.dataset.adunit){observe(el)}}",
        ),
    ]
}

pub fn capture_with(
    device: DeviceProfile,
    mut events: Vec<TrackerEvent>,
    max_scroll_depth: f64,
) -> Capture {
    events.sort_by(|a, b| a.t.total_cmp(&b.t));
    let mobile = device == DeviceProfile::Mobile;
    Capture {
        final_url: "https://pub.example/".into(),
        page_title: Some("Publisher".into()),
        device,
        viewport: Viewport {
            width: if mobile { 390 } else { 1440 },
            height: if mobile { 844 } else { 900 },
            device_scale_factor: if mobile { 3.0 } else { 1.0 },
            mobile,
        },
        user_agent: if mobile {
            "Mozilla/5.0 (iPhone)"
        } else {
            "Mozilla/5.0 (Macintosh)"
        }
        .into(),
        navigation_start: 1_760_000_000_000.0,
        page_load_ms: Some(950.0),
        scripts: page_scripts(),
        events,
        console: vec![],
        max_scroll_depth,
        consent_accepted: false,
    }
}

/// Auction on load, everything fills, the user scrolls later.
pub fn healthy_capture(device: DeviceProfile) -> Capture {
    let mut events = page_setup();
    events.extend(auction_round(360.0, "auction-1"));
    events.push(lifecycle(950.0, "load"));
    events.extend(scroll_events(4000.0));
    capture_with(device, events, 1.0)
}

/// The reference bug as seen on the affected device class: Prebid sits idle
/// until the first scroll, then the auction starts.
pub fn scroll_gated_capture(device: DeviceProfile) -> Capture {
    let mut events = page_setup();
    events.push(lifecycle(950.0, "load"));
    events.extend(scroll_events(4000.0));
    events.extend(auction_round(4060.0, "auction-1"));
    capture_with(device, events, 1.0)
}

/// The reference bug when nobody scrolls: no auction at all.
pub fn never_started_capture(device: DeviceProfile, scrolled: bool) -> Capture {
    let mut events = page_setup();
    events.push(lifecycle(950.0, "load"));
    if scrolled {
        events.extend(scroll_events(4000.0));
    }
    capture_with(device, events, if scrolled { 1.0 } else { 0.0 })
}

// -- test doubles for the outer world ------------------------------------------

pub enum CaptureBehavior {
    Return(Box<Capture>),
    Fail(CaptureError),
    /// Blocks like a long page observation until the scan is cancelled.
    BlockUntilCancelled,
    NoBrowser,
}

pub struct FakeCaptureSource {
    behavior: CaptureBehavior,
    pub requests: Mutex<Vec<CaptureRequest>>,
}

impl FakeCaptureSource {
    pub fn new(behavior: CaptureBehavior) -> Self {
        Self {
            behavior,
            requests: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl CaptureSource for FakeCaptureSource {
    async fn locate_browser(&self, _explicit: Option<String>) -> Option<BrowserInfo> {
        (!matches!(self.behavior, CaptureBehavior::NoBrowser)).then(|| BrowserInfo {
            path: "/usr/bin/chromium".into(),
            version: "Chromium 141.0.0.0".into(),
            name: "Chromium".into(),
        })
    }

    async fn capture(
        &self,
        request: CaptureRequest,
        cancel: CancellationToken,
        on_progress: ProgressFn,
    ) -> Result<Capture, CaptureError> {
        self.requests.lock().unwrap().push(request);
        on_progress("Page loaded in 0.9s".into());
        match &self.behavior {
            CaptureBehavior::Return(capture) => Ok((**capture).clone()),
            CaptureBehavior::Fail(error) => Err(error.clone()),
            CaptureBehavior::BlockUntilCancelled => {
                cancel.cancelled().await;
                Err(CaptureError::Cancelled)
            }
            CaptureBehavior::NoBrowser => Err(CaptureError::BrowserNotFound),
        }
    }
}

#[derive(Default)]
pub struct RecordingSink {
    events: Mutex<Vec<ScanEvent>>,
}

impl RecordingSink {
    pub fn events(&self) -> Vec<ScanEvent> {
        self.events.lock().unwrap().clone()
    }
}

impl EventSink for RecordingSink {
    fn emit(&self, event: ScanEvent) {
        self.events.lock().unwrap().push(event);
    }
}

pub struct Harness {
    pub scans: ScanService,
    pub settings: Arc<SettingsService>,
    pub store: SqliteStore,
    pub events: Arc<RecordingSink>,
    pub capture: Arc<FakeCaptureSource>,
}

pub fn fast_retry() -> RetryPolicy {
    RetryPolicy {
        max_attempts: 3,
        base_delay: Duration::from_millis(10),
        max_delay: Duration::from_millis(40),
    }
}

/// Real storage (in memory), real pipeline, real Anthropic client pointed at
/// `anthropic_base_url`; only the browser and the event sink are doubles.
pub fn harness(
    behavior: CaptureBehavior,
    anthropic_base_url: &str,
    api_key: Option<&str>,
) -> Harness {
    let store = SqliteStore::open_in_memory().unwrap();
    let llm = Arc::new(AnthropicClient::new(anthropic_base_url, fast_retry()).unwrap());
    let secrets = Arc::new(match api_key {
        Some(key) => MemorySecretStore::with_key(key),
        None => MemorySecretStore::default(),
    });
    let settings = Arc::new(SettingsService::new(
        Arc::new(store.clone()),
        secrets,
        llm.clone(),
    ));
    let events = Arc::new(RecordingSink::default());
    let capture = Arc::new(FakeCaptureSource::new(behavior));
    let deps = PipelineDeps {
        scans: Arc::new(store.clone()),
        logs: Arc::new(store.clone()),
        llm,
        capture: capture.clone(),
        formatter: Arc::new(OxcFormatter::default()),
        events: events.clone(),
        clock: Arc::new(SystemClock),
        ids: Arc::new(UuidGenerator),
    };
    Harness {
        scans: ScanService::new(deps, settings.clone()),
        settings,
        store,
        events,
        capture,
    }
}

pub fn scan_options(ai_enabled: bool) -> ScanOptions {
    ScanOptions {
        url: "pub.example".into(),
        device: DeviceProfile::Desktop,
        observe_seconds: 10,
        auto_scroll: true,
        accept_consent: true,
        ai_enabled,
    }
}

pub fn all_scans() -> adscan_model::ScanListQuery {
    adscan_model::ScanListQuery {
        search: None,
        status: None,
        device: None,
        limit: None,
        offset: None,
    }
}

/// Starts a scan and waits for its terminal status.
pub async fn run_scan(harness: &Harness, options: ScanOptions) -> ScanDetail {
    let summary = harness.scans.start(options).await.unwrap();
    wait_for_end(harness, &summary.id).await
}

pub async fn wait_for_end(harness: &Harness, scan_id: &str) -> ScanDetail {
    for _ in 0..1000 {
        let detail = harness.scans.get(scan_id).await.unwrap();
        if !matches!(
            detail.summary.status,
            ScanStatus::Queued | ScanStatus::Running
        ) {
            return detail;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("scan {scan_id} did not finish");
}
