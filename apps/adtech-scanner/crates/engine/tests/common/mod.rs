//! Canned captures and test doubles for the engine's outer ports. Storage,
//! the pipeline and the Anthropic client under test are always the real ones.
#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use adscan_engine::application::archive::Archive;
use adscan_engine::application::archive::network::NetworkView;
use adscan_engine::application::archive::source::ScriptSources;
use adscan_engine::application::configurations::ConfigurationService;
use adscan_engine::application::pipeline::PipelineDeps;
use adscan_engine::application::ports::{
    CaptureError, CaptureSession, CaptureSource, EventSink, FetchError, PublicFile,
    PublicFileFetcher, PublicFileRequest, SecretStore, SourceFormatter,
};
use adscan_engine::application::scans::ScanService;
use adscan_engine::application::settings::SettingsService;
use adscan_engine::infrastructure::anthropic::{AnthropicClient, RetryPolicy};
use adscan_engine::infrastructure::beautify::OxcFormatter;
use adscan_engine::infrastructure::keychain::Keychain;
use adscan_engine::infrastructure::pack_store::LocalPackStore;
use adscan_engine::infrastructure::secrets::{MemorySecretStore, VaultSecretStore};
use adscan_engine::infrastructure::sqlite::SqliteStore;
use adscan_engine::infrastructure::system::{SystemClock, UuidGenerator};
use adscan_engine::infrastructure::typesafe::TypesafeClient;
use adscan_model::{
    BodyAdmission, BodyOffer, BrowserInfo, Capture, CaptureActivity, CaptureRequest, CapturedBody,
    CapturedPage, CapturedScript, DeviceProfile, NetworkRecord, PageReach, ScanDetail, ScanEvent,
    ScanOptions, ScanProfileInfo, ScanStatus, TrackedAdUnit, TrackerEvent, TrackerEventKind as Ev,
    Viewport,
};
use async_trait::async_trait;
use serde_json::json;
use tokio_util::sync::CancellationToken;

pub const WRAPPER_SOURCE: &str = r#"(function(){var pbjs=window.pbjs=window.pbjs||{};pbjs.que=pbjs.que||[];var googletag=window.googletag=window.googletag||{cmd:[]};var adUnits=[{code:"top",mediaTypes:{banner:{sizes:[[728,90]]}},bids:[{bidder:"rubicon",params:{accountId:1}}]}];function isMobile(){return window.innerWidth>768}function startAuction(){pbjs.que.push(function(){pbjs.addAdUnits(adUnits);pbjs.requestBids({timeout:1500,bidsBackHandler:function(){pbjs.setTargetingForGPTAsync();googletag.pubads().refresh()}})})}googletag.cmd.push(function(){googletag.defineSlot("/123/top",[728,90],"top").addService(googletag.pubads());googletag.pubads().disableInitialLoad();googletag.enableServices()});if(isMobile()){window.addEventListener("scroll",startAuction,{once:true})}else{startAuction()}})();"#;

pub fn ev(t: f64, kind: Ev) -> TrackerEvent {
    TrackerEvent { page: 0, t, kind }
}

pub fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| item.to_string()).collect()
}

pub fn script(url: &str, body: &str) -> CapturedScript {
    CapturedScript {
        pages: vec![0],
        url: url.into(),
        inline: false,
        body: body.into(),
        loaded_at_ms: Some(100.0),
        initiator: Some("https://pub.example/".into()),
        request_seq: None,
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
            prebid_ad_id: None,
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
                    bids: vec![],
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
                gdpr_applies: None,
                has_consent_string: None,
                schain: vec![],
                bidder_request_id: None,
                bids: Vec::new(),
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
    let entry_page = CapturedPage {
        title: Some("Publisher".into()),
        ended_at_ms: 12_000.0,
        load_ms: Some(950.0),
        max_scroll_depth,
        ..CapturedPage::entry("https://pub.example/")
    };
    Capture {
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
        max_scroll_depth,
        pages: vec![entry_page],
        ..Capture::single_page("https://pub.example/", events)
    }
}

/// A page view of a multi-page visit, from `started_at_ms` to `ended_at_ms`
/// into the scan.
pub fn page_view(
    url: &str,
    reached_by: PageReach,
    started_at_ms: f64,
    ended_at_ms: f64,
) -> CapturedPage {
    CapturedPage {
        title: Some("Publisher".into()),
        started_at_ms,
        ended_at_ms,
        reached_by,
        load_ms: (reached_by != PageReach::RouteChange).then_some(950.0),
        max_scroll_depth: 1.0,
        ..CapturedPage::entry(url)
    }
}

/// A desktop visit of several page views. The events of each page view
/// carry times since that page view started.
pub fn visit(pages: Vec<(CapturedPage, Vec<TrackerEvent>)>) -> Capture {
    let mut capture = capture_with(DeviceProfile::Desktop, Vec::new(), 1.0);
    capture.pages.clear();
    for (index, (page, mut events)) in (0u32..).zip(pages) {
        events.sort_by(|a, b| a.t.total_cmp(&b.t));
        for event in &mut events {
            event.page = index;
        }
        capture.events.extend(events);
        capture.pages.push(page);
    }
    capture
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
    /// A manual visit: records until the user finishes it, then returns the
    /// capture. Cancelling discards it.
    RecordUntilFinished(Box<Capture>),
    NoBrowser,
}

pub struct FakeCaptureSource {
    behavior: CaptureBehavior,
    pub requests: Mutex<Vec<CaptureRequest>>,
    /// The proxy password each capture was handed.
    pub proxy_passwords: Mutex<Vec<Option<String>>>,
    /// Reported, in order, after the first progress line of every capture.
    pub activities: Mutex<Vec<CaptureActivity>>,
    /// Whether a browser holds the scan profile, as the browser crate's own
    /// lock would say.
    pub profile_in_use: Mutex<bool>,
    pub profile_clears: Mutex<u32>,
    /// The requests every capture "sees", handed to the scan's sink the way
    /// the browser crate does: a body is offered first and only handed over
    /// when the sink admits it.
    pub network: Mutex<Vec<NetworkRecord>>,
    /// Whether the last capture was given a sink at all.
    pub had_sink: Mutex<Option<bool>>,
}

impl FakeCaptureSource {
    pub fn new(behavior: CaptureBehavior) -> Self {
        Self {
            behavior,
            requests: Mutex::new(Vec::new()),
            proxy_passwords: Mutex::new(Vec::new()),
            activities: Mutex::new(Vec::new()),
            profile_in_use: Mutex::new(false),
            profile_clears: Mutex::new(0),
            network: Mutex::new(Vec::new()),
            had_sink: Mutex::new(None),
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
        session: CaptureSession,
    ) -> Result<Capture, CaptureError> {
        self.requests.lock().unwrap().push(request);
        self.proxy_passwords
            .lock()
            .unwrap()
            .push(session.proxy_password.clone());
        (session.on_activity)(CaptureActivity::progress("Page loaded in 0.9s"));
        for activity in self.activities.lock().unwrap().iter().cloned() {
            (session.on_activity)(activity);
        }
        *self.had_sink.lock().unwrap() = Some(session.network.is_some());
        if let Some(sink) = &session.network {
            for mut record in self.network.lock().unwrap().iter().cloned() {
                if let CapturedBody::Read { bytes, total_bytes } = &record.body {
                    let admission = sink.admit_body(&BodyOffer {
                        seq: record.seq,
                        url: &record.url,
                        bidder: record.bidder.as_deref(),
                        kind: record.kind,
                        mime_type: record.mime_type.as_deref(),
                        status: record.status,
                        expected_bytes: *total_bytes,
                    });
                    record.body = match admission {
                        BodyAdmission::Read { max_bytes } => CapturedBody::Read {
                            bytes: bytes[..bytes.len().min(max_bytes)].to_vec(),
                            total_bytes: *total_bytes,
                        },
                        BodyAdmission::Skip(why) => CapturedBody::Skipped(why),
                    };
                }
                sink.record(record);
            }
        }
        match &self.behavior {
            CaptureBehavior::Return(capture) => Ok((**capture).clone()),
            CaptureBehavior::Fail(error) => Err(error.clone()),
            CaptureBehavior::BlockUntilCancelled => {
                session.cancel.cancelled().await;
                Err(CaptureError::Cancelled)
            }
            CaptureBehavior::RecordUntilFinished(capture) => tokio::select! {
                () = session.finish.cancelled() => Ok((**capture).clone()),
                () = session.cancel.cancelled() => Err(CaptureError::Cancelled),
            },
            CaptureBehavior::NoBrowser => Err(CaptureError::BrowserNotFound),
        }
    }

    async fn scan_profile_info(&self) -> ScanProfileInfo {
        let cleared = *self.profile_clears.lock().unwrap() > 0;
        ScanProfileInfo {
            exists: !cleared,
            size_bytes: if cleared { 0.0 } else { 4096.0 },
            last_used_at: None,
            in_use: *self.profile_in_use.lock().unwrap(),
        }
    }

    async fn clear_scan_profile(&self) -> Result<(), CaptureError> {
        if *self.profile_in_use.lock().unwrap() {
            return Err(CaptureError::ProfileBusy);
        }
        *self.profile_clears.lock().unwrap() += 1;
        Ok(())
    }
}

/// The ads.txt of the fixture site: it authorises both fixture bidders.
pub const SITE_ADS_TXT_URL: &str = "https://pub.example/ads.txt";
pub const SITE_ADS_TXT: &str = "appnexus.com, 958, DIRECT\nrubiconproject.com, 1001, DIRECT\n";

pub fn text_file(status: u16, body: &str) -> Result<PublicFile, FetchError> {
    Ok(PublicFile {
        status,
        content_type: Some("text/plain".into()),
        body: body.as_bytes().to_vec(),
    })
}

/// The open web as far as a scan may read it: canned answers by address.
/// An address without one cannot be connected to.
pub struct FakePublicFiles {
    answers: Mutex<HashMap<String, Result<PublicFile, FetchError>>>,
    /// Addresses that never answer; the request ends when the scan is cancelled.
    stalled: Mutex<Vec<String>>,
    pub requests: Mutex<Vec<PublicFileRequest>>,
}

impl Default for FakePublicFiles {
    fn default() -> Self {
        let files = Self {
            answers: Mutex::new(HashMap::new()),
            stalled: Mutex::new(Vec::new()),
            requests: Mutex::new(Vec::new()),
        };
        files.answer(SITE_ADS_TXT_URL, text_file(200, SITE_ADS_TXT));
        files
    }
}

impl FakePublicFiles {
    pub fn answer(&self, url: &str, answer: Result<PublicFile, FetchError>) {
        self.answers.lock().unwrap().insert(url.to_string(), answer);
    }

    pub fn stall(&self, url: &str) {
        self.stalled.lock().unwrap().push(url.to_string());
    }

    pub fn requested_urls(&self) -> Vec<String> {
        let mut urls: Vec<String> = self
            .requests
            .lock()
            .unwrap()
            .iter()
            .map(|request| request.url.clone())
            .collect();
        urls.sort();
        urls
    }
}

#[async_trait]
impl PublicFileFetcher for FakePublicFiles {
    async fn fetch(
        &self,
        request: &PublicFileRequest,
        cancel: &CancellationToken,
    ) -> Result<PublicFile, FetchError> {
        self.requests.lock().unwrap().push(request.clone());
        if self.stalled.lock().unwrap().contains(&request.url) {
            cancel.cancelled().await;
            return Err(FetchError::Cancelled);
        }
        self.answers
            .lock()
            .unwrap()
            .get(&request.url)
            .cloned()
            .unwrap_or_else(|| Err(FetchError::Network("could not connect".into())))
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
    pub configurations: ConfigurationService,
    pub secrets: Arc<dyn SecretStore>,
    pub store: SqliteStore,
    pub events: Arc<RecordingSink>,
    pub capture: Arc<FakeCaptureSource>,
    pub files: Arc<FakePublicFiles>,
    pub archive: Archive,
    pub network: NetworkView,
    pub sources: ScriptSources,
    /// Where the pack files are; removed with the harness.
    pub data_dir: Arc<tempfile::TempDir>,
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
    harness_with_formatter(
        behavior,
        anthropic_base_url,
        api_key,
        Arc::new(OxcFormatter::default()),
    )
}

pub fn harness_with_formatter(
    behavior: CaptureBehavior,
    anthropic_base_url: &str,
    api_key: Option<&str>,
    formatter: Arc<dyn SourceFormatter>,
) -> Harness {
    build_harness(
        behavior,
        Providers {
            anthropic_base_url,
            api_key,
            // Nothing listens here; these scans never use Jev.
            typesafe_base_url: "http://127.0.0.1:1",
            typesafe_key: None,
        },
        formatter,
    )
}

pub struct Providers<'a> {
    pub anthropic_base_url: &'a str,
    pub api_key: Option<&'a str>,
    pub typesafe_base_url: &'a str,
    pub typesafe_key: Option<&'a str>,
}

/// Like [`harness`], with the real TypeSafe client pointed at a mock as well.
pub fn harness_with_providers(behavior: CaptureBehavior, providers: Providers) -> Harness {
    build_harness(behavior, providers, Arc::new(OxcFormatter::default()))
}

/// Like [`harness_with_providers`], with the real secret store on `keychain`
/// and `store` as the database. A second harness over the same two is the
/// app started again. The keys of `providers` are not used.
pub fn harness_with_keychain(
    behavior: CaptureBehavior,
    providers: Providers,
    store: SqliteStore,
    keychain: Arc<dyn Keychain>,
) -> Harness {
    let secrets = Arc::new(VaultSecretStore::new(keychain, Arc::new(store.clone())));
    assemble(
        behavior,
        providers,
        Arc::new(OxcFormatter::default()),
        store,
        secrets,
    )
}

fn build_harness(
    behavior: CaptureBehavior,
    providers: Providers,
    formatter: Arc<dyn SourceFormatter>,
) -> Harness {
    let secrets = Arc::new(MemorySecretStore::with_keys(
        providers.api_key.map(str::to_string),
        providers.typesafe_key.map(str::to_string),
    ));
    let store = SqliteStore::open_in_memory().unwrap();
    assemble(behavior, providers, formatter, store, secrets)
}

/// The app over a database and a data directory that outlive the harness:
/// a second harness over the same two is the app started again.
pub fn harness_in(
    behavior: CaptureBehavior,
    store: SqliteStore,
    data_dir: Arc<tempfile::TempDir>,
) -> Harness {
    let providers = Providers {
        anthropic_base_url: "http://127.0.0.1:1",
        api_key: None,
        typesafe_base_url: "http://127.0.0.1:1",
        typesafe_key: None,
    };
    assemble_in(
        behavior,
        providers,
        Arc::new(OxcFormatter::default()),
        store,
        Arc::new(MemorySecretStore::with_keys(None, None)),
        data_dir,
    )
}

fn assemble(
    behavior: CaptureBehavior,
    providers: Providers,
    formatter: Arc<dyn SourceFormatter>,
    store: SqliteStore,
    secrets: Arc<dyn SecretStore>,
) -> Harness {
    let data_dir = Arc::new(tempfile::tempdir().unwrap());
    assemble_in(behavior, providers, formatter, store, secrets, data_dir)
}

fn assemble_in(
    behavior: CaptureBehavior,
    providers: Providers,
    formatter: Arc<dyn SourceFormatter>,
    store: SqliteStore,
    secrets: Arc<dyn SecretStore>,
    data_dir: Arc<tempfile::TempDir>,
) -> Harness {
    let archive = Archive {
        packs: Arc::new(LocalPackStore::new(data_dir.path())),
        index: Arc::new(store.clone()),
    };
    let llm = Arc::new(AnthropicClient::new(providers.anthropic_base_url, fast_retry()).unwrap());
    let judge = Arc::new(TypesafeClient::new(providers.typesafe_base_url, fast_retry()).unwrap());
    let settings = Arc::new(SettingsService::new(
        Arc::new(store.clone()),
        secrets.clone(),
        llm.clone(),
        judge.clone(),
        Arc::new(store.clone()),
        Arc::new(UuidGenerator),
        archive.clone(),
    ));
    let configurations = ConfigurationService::new(
        Arc::new(store.clone()),
        settings.clone(),
        Arc::new(SystemClock),
        Arc::new(UuidGenerator),
    );
    let events = Arc::new(RecordingSink::default());
    let capture = Arc::new(FakeCaptureSource::new(behavior));
    let files = Arc::new(FakePublicFiles::default());
    let deps = PipelineDeps {
        scans: Arc::new(store.clone()),
        logs: Arc::new(store.clone()),
        llm,
        judge,
        capture: capture.clone(),
        files: files.clone(),
        formatter: formatter.clone(),
        archive: archive.clone(),
        events: events.clone(),
        clock: Arc::new(SystemClock),
        ids: Arc::new(UuidGenerator),
    };
    Harness {
        scans: ScanService::new(deps, settings.clone(), Arc::new(store.clone())),
        settings,
        configurations,
        secrets,
        events,
        capture,
        files,
        network: NetworkView::new(archive.clone(), Arc::new(store.clone())),
        sources: ScriptSources::new(
            archive.clone(),
            Arc::new(store.clone()),
            Arc::new(store.clone()),
            formatter,
        ),
        store,
        archive,
        data_dir,
    }
}

pub fn scan_options(ai_enabled: bool) -> ScanOptions {
    ScanOptions {
        url: "pub.example".into(),
        accept_consent: true,
        ai_enabled,
        ..ScanOptions::for_url("")
    }
}

pub fn all_scans() -> adscan_model::ScanListQuery {
    adscan_model::ScanListQuery {
        configuration_id: None,
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

/// A finished GET of `url` on the entry page, 100 ms long, answered 200
/// without a body: the values a test changes from.
pub fn net(seq: u32, url: &str) -> NetworkRecord {
    use adscan_model::{
        InitiatorKind, NetworkInitiator, RequestContext, RequestEnd, ResourceKind, ResourceTiming,
        ServedFrom,
    };
    NetworkRecord {
        seq,
        page: 0,
        context: RequestContext::Page,
        frame_id: Some("MAIN".into()),
        target_id: None,
        method: "GET".into(),
        url: url.into(),
        kind: ResourceKind::Other,
        priority: Some("Low".into()),
        request_headers: Vec::new(),
        post_data: None,
        post_data_missing: false,
        initiator: NetworkInitiator {
            kind: InitiatorKind::Other,
            url: None,
            line: None,
            column: None,
            stack: Vec::new(),
        },
        redirect_from: None,
        preflight_of: None,
        status: Some(200),
        mime_type: None,
        protocol: Some("h2".into()),
        remote_address: Some("203.0.113.9:443".into()),
        response_headers: Vec::new(),
        served_from: ServedFrom::Network,
        started_at_ms: 100.0 * f64::from(seq),
        response_at_ms: Some(100.0 * f64::from(seq) + 80.0),
        ended_at_ms: Some(100.0 * f64::from(seq) + 100.0),
        queued_ms: Some(2.0),
        timing: Some(ResourceTiming {
            dns_start: Some(0.0),
            dns_end: Some(5.0),
            connect_start: Some(5.0),
            connect_end: Some(25.0),
            ssl_start: Some(10.0),
            ssl_end: Some(25.0),
            send_start: Some(26.0),
            send_end: Some(27.0),
            receive_headers_end: Some(78.0),
            ..ResourceTiming::default()
        }),
        encoded_bytes: 300,
        decoded_bytes: 0,
        end: RequestEnd::Finished,
        bidder: None,
        body: CapturedBody::Empty,
    }
}

/// `record` as a response of `kind` and `mime` with this text as its body.
pub fn with_body(
    mut record: NetworkRecord,
    kind: adscan_model::ResourceKind,
    mime: &str,
    body: &str,
) -> NetworkRecord {
    record.kind = kind;
    record.mime_type = Some(mime.into());
    record.decoded_bytes = body.len() as u64;
    record.body = CapturedBody::Read {
        bytes: body.as_bytes().to_vec(),
        total_bytes: body.len() as u64,
    };
    record
}

pub fn header(name: &str, value: &str) -> adscan_model::RawHeader {
    adscan_model::RawHeader {
        name: name.into(),
        value: value.into(),
    }
}
