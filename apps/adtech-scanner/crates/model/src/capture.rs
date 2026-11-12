//! Raw, in-memory output of one page capture.
//!
//! This is the hand-off between the browser crate (which produces it) and the
//! engine (which derives auctions, slots, metrics and findings from it). The
//! [`Capture`] itself is never persisted as a whole. What a scan keeps of
//! the raw data goes through [`NetworkSink`] while the capture runs, and
//! from there into the scan's archive.

use serde::{Deserialize, Serialize};

use crate::{
    DeviceProfile, FollowedLink, InitiatorKind, PageReach, ProxyScheme, RequestContext,
    ResourceKind, ServedFrom, VisitorBehavior,
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureRequest {
    pub url: String,
    pub device: DeviceProfile,
    /// Seconds to observe each page view. Does not apply to manual visits.
    pub observe_seconds: u32,
    pub visitor: VisitorBehavior,
    /// Further pages to follow links to, 0-5. Zero unless `visitor` is browse.
    pub extra_pages: u32,
    /// Seed of the scripted visit: the same seed on the same page plans the
    /// same actions.
    pub seed: u32,
    pub accept_consent: bool,
    pub headless: bool,
    /// Use the scan profile that survives the scan instead of a throwaway one.
    pub persistent_profile: bool,
    pub proxy: CaptureProxy,
    /// Explicit browser executable; `None` means auto-detect.
    pub chrome_path: Option<String>,
}

/// The proxy the browser connects through. The password of a custom proxy is
/// handed over outside this type, so it can never be serialised with it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum CaptureProxy {
    /// Connect directly, ignoring the system's proxy.
    None,
    /// The browser's default: the operating system's proxy settings.
    System,
    Custom {
        scheme: ProxyScheme,
        host: String,
        port: u16,
        username: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Capture {
    /// URL after redirects.
    pub final_url: String,
    pub page_title: Option<String>,
    pub device: DeviceProfile,
    pub viewport: Viewport,
    pub user_agent: String,
    /// Unix epoch ms of navigation start; every `t` below is relative to it.
    pub navigation_start: f64,
    /// ms from navigation start to the load event.
    pub page_load_ms: Option<f64>,
    pub scripts: Vec<CapturedScript>,
    /// Tracker events in emission order.
    pub events: Vec<TrackerEvent>,
    pub console: Vec<ConsoleLine>,
    /// Final scroll depth reached, 0-1.
    pub max_scroll_depth: f64,
    pub consent_accepted: bool,
    /// What the scanner itself saw and did about consent dialogs, in time
    /// order. What the consent platform reports is in `events` (`TcfState`).
    #[serde(default)]
    pub consent_notes: Vec<ConsentNote>,
    /// What the bidders' endpoints answered, reduced to a status and a short
    /// reason. The response bodies themselves are dropped during capture.
    #[serde(default)]
    pub bid_responses: Vec<ObservedBidResponse>,
    /// The page views of the visit, in order; never empty. Every `t` and
    /// `*_at_ms` of a record counts from the start of the page view its
    /// `page` names.
    pub pages: Vec<CapturedPage>,
    pub environment: CaptureEnvironment,
}

impl Capture {
    /// ms since `navigation_start` of something that happened `t` ms into
    /// page view `page`.
    pub fn absolute_ms(&self, page: u32, t: f64) -> f64 {
        self.pages
            .get(page as usize)
            .map_or(t, |view| view.started_at_ms + t)
    }

    /// A desktop capture of one page view that loaded and recorded nothing:
    /// the values a test changes from.
    pub fn single_page(url: impl Into<String>, events: Vec<TrackerEvent>) -> Self {
        let url = url.into();
        Self {
            final_url: url.clone(),
            page_title: None,
            device: DeviceProfile::Desktop,
            viewport: Viewport {
                width: 1440,
                height: 900,
                device_scale_factor: 1.0,
                mobile: false,
            },
            user_agent: String::new(),
            navigation_start: 0.0,
            page_load_ms: None,
            scripts: Vec::new(),
            events,
            console: Vec::new(),
            max_scroll_depth: 0.0,
            consent_accepted: false,
            consent_notes: Vec::new(),
            bid_responses: Vec::new(),
            pages: vec![CapturedPage::entry(url)],
            environment: CaptureEnvironment::default(),
        }
    }
}

/// Something the scanner noticed or did about a consent dialog.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConsentNote {
    pub page: u32,
    /// ms since the start of the page view.
    pub t: f64,
    pub kind: ConsentNoteKind,
    /// The consent platform, or the label of the control: "Sourcepoint",
    /// `button "alle akzeptieren"`.
    pub banner: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConsentNoteKind {
    /// A consent dialog is on the page and the scanner found nothing in it
    /// that plainly means "accept all". Noted once per page view.
    DialogWithoutAccept,
    /// The scanner clicked the dialog's accept-all control.
    Accepted,
}

/// One page view of the visit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapturedPage {
    pub url: String,
    pub title: Option<String>,
    /// ms since `Capture.navigation_start`; the entry page starts at 0.
    pub started_at_ms: f64,
    pub ended_at_ms: f64,
    pub reached_by: PageReach,
    pub link: Option<FollowedLink>,
    /// ms from the start of the page view to its load event. `None` for
    /// route changes.
    pub load_ms: Option<f64>,
    /// 0-1.
    pub max_scroll_depth: f64,
}

impl CapturedPage {
    /// The page view a visit starts with, before anything is known about it.
    pub fn entry(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            title: None,
            started_at_ms: 0.0,
            ended_at_ms: 0.0,
            reached_by: PageReach::Entry,
            link: None,
            load_ms: None,
            max_scroll_depth: 0.0,
        }
    }
}

/// What the pages could observe about the browser, read from the browser
/// itself. Reported as it was.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureEnvironment {
    pub headless: bool,
    pub persistent_profile: bool,
    /// `navigator.webdriver` as the page saw it; `None` when it could not be
    /// read.
    pub webdriver: Option<bool>,
    pub user_agent_overridden: bool,
    pub viewport_overridden: bool,
    /// Third-party host names the pages requested: names only, at most 500.
    pub request_hosts: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ActivityKind {
    /// A progress line of the capture itself.
    Progress,
    /// A page view opened.
    PageView,
    /// Something the visitor did: a scroll, a pointer move, a click.
    Action,
    /// Which link the visitor follows next, and what it passed over.
    LinkChoice,
    /// Something the scanner refused to do or to record.
    Blocked,
    /// What the page can tell about the browser.
    Environment,
}

/// One thing that happened during a capture, reported while it runs. Every
/// one becomes an audit-trail entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureActivity {
    pub kind: ActivityKind,
    /// One readable line.
    pub message: String,
    /// The page view it happened on, when it belongs to one.
    pub page: Option<u32>,
    /// Structured detail; its shape depends on `kind`.
    pub data: Option<serde_json::Value>,
}

impl CaptureActivity {
    pub fn progress(message: impl Into<String>) -> Self {
        Self {
            kind: ActivityKind::Progress,
            message: message.into(),
            page: None,
            data: None,
        }
    }
}

/// One answer of a known bidder endpoint, as seen on the network.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObservedBidResponse {
    /// Page view the response arrived on.
    #[serde(default)]
    pub page: u32,
    /// ms since the start of that page view when the response arrived.
    pub t: f64,
    /// Prebid bidder code the endpoint belongs to.
    pub bidder: String,
    pub host: String,
    /// `None` when the request failed before any response.
    pub status: Option<u16>,
    /// The endpoint's own explanation for not bidding, or the network error.
    /// At most 200 characters.
    pub reason: Option<String>,
    /// `NetworkRecord.seq` of the request that was answered.
    #[serde(default)]
    pub request_seq: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Viewport {
    pub width: u32,
    pub height: u32,
    pub device_scale_factor: f64,
    pub mobile: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapturedScript {
    /// Absolute URL, or `inline:<n>` for inline script blocks.
    pub url: String,
    pub inline: bool,
    /// Full source text. In memory only.
    pub body: String,
    /// ms since the start of the first page view that loaded it when
    /// loading finished.
    pub loaded_at_ms: Option<f64>,
    /// URL of the script or document that initiated the request, if known.
    pub initiator: Option<String>,
    /// Page views that loaded the script, ascending.
    #[serde(default)]
    pub pages: Vec<u32>,
    /// `NetworkRecord.seq` of the request that delivered the script (the
    /// first one, when it was loaded more than once). `None` for inline and
    /// runtime scripts.
    #[serde(default)]
    pub request_seq: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConsoleLine {
    #[serde(default)]
    pub page: u32,
    /// ms since the start of that page view.
    pub t: f64,
    /// "log" | "info" | "warning" | "error" | "debug"
    pub level: String,
    pub text: String,
}

/// One event emitted by the injected tracker script.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackerEvent {
    /// Page view the event belongs to. The tracker does not send it; the
    /// capture assigns it.
    #[serde(default)]
    pub page: u32,
    /// ms since the start of that page view.
    pub t: f64,
    #[serde(flatten)]
    pub kind: TrackerEventKind,
}

/// Tracker event payloads. The injected script emits JSON objects of the shape
/// `{ "t": 1234.5, "type": "bidResponse", ...fields }`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum TrackerEventKind {
    // -- library detection ---------------------------------------------------
    /// `window.pbjs` (or another global registered in `_pbjsGlobals`) appeared.
    PrebidDetected {
        global_name: String,
        version: Option<String>,
        /// Installed bidder adapter / module names when exposed.
        modules: Vec<String>,
    },
    /// Snapshot of `pbjs.getConfig()` (selected keys) taken at first auction.
    PrebidConfig {
        bidder_timeout: Option<f64>,
        price_granularity: Option<String>,
        has_floors: bool,
        has_user_sync: bool,
        has_consent_management: bool,
        currency: Option<String>,
        /// The selected config keys as JSON, for the analysis model.
        raw: serde_json::Value,
    },
    GptDetected {
        version: Option<String>,
    },
    AmazonDetected {},

    // -- Prebid auction lifecycle --------------------------------------------
    /// The page asked Prebid for bids (`requestBids`), whether or not an
    /// auction followed. Reported from Prebid's own hook on that call, or
    /// from its event history for calls made before the tracker subscribed
    /// (then without the details).
    BidsRequested {
        /// The ad unit codes the call named; empty when it named none or
        /// they are not known.
        #[serde(default)]
        ad_unit_codes: Vec<String>,
        /// How many ad units Prebid had to auction at that moment: those
        /// passed with the call, else those registered. `None` when unknown.
        #[serde(default)]
        ad_unit_count: Option<u32>,
        #[serde(default)]
        timeout: Option<f64>,
    },
    /// `addAdUnits` was called; `registered` is how many ad units Prebid
    /// holds afterwards (`None` when read from the event history).
    AdUnitsAdded {
        #[serde(default)]
        registered: Option<u32>,
    },
    AuctionInit {
        auction_id: String,
        timeout: Option<f64>,
        ad_units: Vec<TrackedAdUnit>,
    },
    BidRequested {
        auction_id: String,
        bidder: String,
        ad_unit_codes: Vec<String>,
        /// `gdprConsent.gdprApplies` of the bidder request. `None` when Prebid
        /// attached no consent data at all.
        #[serde(default)]
        gdpr_applies: Option<bool>,
        /// Whether the bidder request carried a TC string. The string itself
        /// is never captured.
        #[serde(default)]
        has_consent_string: Option<bool>,
        /// Supply chain nodes Prebid passed to the bidder.
        #[serde(default)]
        schain: Vec<SchainNode>,
        /// Prebid's `bidderRequestId`. Adapters put it, the auction id or
        /// the bid ids into what they send, which is how the request on the
        /// network is told apart from the bidder's other requests.
        #[serde(default)]
        bidder_request_id: Option<String>,
        /// Prebid's `bidId` per ad unit of this bidder request.
        #[serde(default)]
        bids: Vec<TrackedBidRequest>,
    },
    BidResponse {
        auction_id: String,
        bidder: String,
        ad_unit_code: String,
        cpm: f64,
        currency: Option<String>,
        size: Option<String>,
        time_to_respond: Option<f64>,
        deal_id: Option<String>,
    },
    NoBid {
        auction_id: String,
        bidder: String,
        ad_unit_code: String,
    },
    BidTimeout {
        auction_id: String,
        bidder: String,
        ad_unit_code: String,
    },
    BidderError {
        auction_id: Option<String>,
        bidder: String,
        message: Option<String>,
    },
    AuctionEnd {
        auction_id: String,
    },
    /// `setTargeting` ran: hb_* keys were written for these ad units.
    SetTargeting {
        ad_unit_codes: Vec<String>,
    },
    BidWon {
        auction_id: Option<String>,
        bidder: String,
        ad_unit_code: String,
        cpm: f64,
    },
    AdRenderFailed {
        ad_unit_code: Option<String>,
        bidder: Option<String>,
        reason: Option<String>,
    },
    AdRenderSucceeded {
        ad_unit_code: Option<String>,
        bidder: Option<String>,
    },

    // -- GPT slot lifecycle ---------------------------------------------------
    SlotDefined {
        element_id: String,
        ad_unit_path: Option<String>,
        sizes: Vec<String>,
        /// Whether the container was inside the viewport when defined.
        in_viewport: Option<bool>,
    },
    SlotRequested {
        element_id: String,
        /// hb_* targeting keys present on the slot when the request went out.
        prebid_targeting_keys: Vec<String>,
        /// Value of `hb_adid` on the slot when the request went out.
        #[serde(default)]
        prebid_ad_id: Option<String>,
    },
    /// `googletag.destroySlots` removed these slots.
    SlotsDestroyed {
        element_ids: Vec<String>,
    },
    /// `defineSlot` returned null: the container already has a slot.
    SlotDefinitionRejected {
        element_id: String,
    },
    SlotResponseReceived {
        element_id: String,
    },
    SlotRenderEnded {
        element_id: String,
        is_empty: bool,
        size: Option<String>,
        advertiser_id: Option<String>,
        line_item_id: Option<String>,
    },
    SlotOnload {
        element_id: String,
    },
    ImpressionViewable {
        element_id: String,
    },

    // -- Amazon TAM ------------------------------------------------------------
    ApsFetchBids {
        slot_ids: Vec<String>,
        timeout: Option<f64>,
    },
    ApsBidsReturned {
        slot_ids: Vec<String>,
        duration: Option<f64>,
    },

    // -- Page lifecycle ---------------------------------------------------------
    PageLifecycle {
        /// "domContentLoaded" | "load" | "firstScroll" | "firstInteraction" |
        /// "firstPointerMove"
        phase: String,
    },
    /// The page changed its URL without loading a new document.
    RouteChange {
        url: String,
        /// "push" | "replace" | "traverse" | "reload"
        navigation_type: String,
    },
    Scroll {
        /// 0-1 of the scrollable height.
        depth: f64,
    },
    /// Prebid debug / warning / error line captured from the console.
    PrebidLog {
        level: String,
        message: String,
    },

    // -- Consent -----------------------------------------------------------------
    /// The page exposes a consent API (`__tcfapi`); a stub counts.
    CmpDetected {
        /// "tcf"
        api: String,
    },
    /// What the CMP told a TCF event listener. Derived flags only: the TC
    /// string is never captured.
    TcfState {
        /// "tcloaded" | "cmpuishown" | "useractioncomplete"
        event_status: Option<String>,
        gdpr_applies: Option<bool>,
        has_tc_string: bool,
        /// Purposes (1-11) the user consented to.
        purpose_consents: Vec<u32>,
        /// IAB GVL ids of the vendors the user consented to.
        vendor_consents: Vec<u32>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackedAdUnit {
    pub code: String,
    pub sizes: Vec<String>,
    pub bidders: Vec<String>,
    /// The ad unit's bids with their adapter params.
    #[serde(default)]
    pub bids: Vec<TrackedBid>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackedBid {
    pub bidder: String,
    /// The adapter params as configured, depth and width bounded.
    pub params: serde_json::Value,
}

/// One bid Prebid asked a bidder for.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackedBidRequest {
    pub bid_id: String,
    pub ad_unit_code: String,
}

/// One node of an OpenRTB supply chain object.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../../src/bindings/")]
pub struct SchainNode {
    /// Canonical domain of the advertising system.
    pub asi: String,
    /// Seller id within that system.
    pub sid: String,
    /// Whether the node takes part in the flow of payment.
    pub hp: Option<bool>,
}

// ---------------------------------------------------------------------------
// Network hand-over: every request of the visit, streamed while it happens
// ---------------------------------------------------------------------------

/// POST bodies are handed over up to this many bytes. Also the value of
/// `maxPostDataSize` for `Network.enable`.
pub const MAX_POST_DATA_BYTES: usize = 1024 * 1024;

/// An initiator stack is handed over up to this many frames, innermost first.
pub const MAX_INITIATOR_FRAMES: usize = 32;

/// Where the capture hands over the visit's requests while it runs, so that
/// neither side has to hold all bodies in memory. The engine provides it.
///
/// Both methods are called from the capture's own task: they never block and
/// never fail. Every call happens before `capture` returns; after that the
/// sink is not used again.
pub trait NetworkSink: Send + Sync {
    /// Asked once per finished response, before its body is read from the
    /// browser. The answer is the sink's whole policy: which bodies it reads
    /// (to keep them, or only to analyse them and drop them), the per-body
    /// and per-scan caps, and back-pressure when it cannot work as fast as
    /// bodies arrive. After `Read` the capture must
    /// hand over a record for `offer.seq`, whatever becomes of the read.
    fn admit_body(&self, offer: &BodyOffer<'_>) -> BodyAdmission;

    /// One request, complete: called exactly once per `seq`, when the request
    /// has ended (finished, failed, redirected) or, for requests still open,
    /// when the capture ends. Order of calls is order of ending, not of `seq`.
    fn record(&self, record: NetworkRecord);
}

/// What the sink is told about a body before it is read.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BodyOffer<'a> {
    pub seq: u32,
    /// The request's URL, as in its record.
    pub url: &'a str,
    /// The record's `bidder`: set for a recognised bid request.
    pub bidder: Option<&'a str>,
    pub kind: ResourceKind,
    pub mime_type: Option<&'a str>,
    pub status: Option<u16>,
    /// Best guess of the decoded size: bytes received so far, else the
    /// encoded length; 0 when unknown.
    pub expected_bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyAdmission {
    /// Read the body and hand over at most `max_bytes` of it.
    Read { max_bytes: usize },
    /// Do not read it; report `CapturedBody::Skipped` with this reason.
    Skip(BodySkip),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodySkip {
    /// This kind of content is recorded as metadata only.
    Type,
    /// The scan has reached its archive size cap.
    ScanLimit,
    /// The sink holds as many unwritten bytes as it may.
    Backlog,
    /// The scan keeps no network data and nothing reads this body.
    NotStored,
}

/// The response body of a request as handed over.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapturedBody {
    /// There was nothing to read: the request failed, was cancelled, never
    /// finished, or is a redirect hop.
    Absent,
    /// The response finished with a body of zero bytes.
    Empty,
    /// The sink declined it.
    Skipped(BodySkip),
    /// The sink admitted it but the browser could not provide it (already
    /// discarded, the frame is gone). The browser's reason, at most 200
    /// characters.
    Unavailable(String),
    /// The decoded body. `bytes` holds the first `max_bytes` the sink
    /// admitted; `total_bytes` is the length before that cut.
    Read { bytes: Vec<u8>, total_bytes: u64 },
}

/// The request body (`postData`) as handed over.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturedPostData {
    /// At most [`MAX_POST_DATA_BYTES`].
    pub bytes: Vec<u8>,
    /// Length before that cut.
    pub total_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawHeader {
    /// As the browser reports it, case included.
    pub name: String,
    /// As the browser reports it. Several values of one name may be joined
    /// with a line feed, as CDP does for `Set-Cookie`.
    pub value: String,
}

/// How a request ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestEnd {
    /// `Network.loadingFinished`.
    Finished,
    /// Answered with a redirect; the next hop is its own record.
    Redirected,
    /// `Network.loadingFailed`.
    Failed {
        /// `errorText`, e.g. `net::ERR_CONNECTION_REFUSED`.
        error: String,
        cancelled: bool,
        /// `blockedReason` or `corsErrorStatus.corsError`, when there is one.
        blocked_reason: Option<String>,
    },
    /// Still open when the capture ended.
    Pending,
}

/// CDP `ResourceTiming`, each value in ms after the request's own start
/// (`requestTime`), `None` where CDP reports -1 or nothing.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ResourceTiming {
    pub dns_start: Option<f64>,
    pub dns_end: Option<f64>,
    pub connect_start: Option<f64>,
    pub connect_end: Option<f64>,
    pub ssl_start: Option<f64>,
    pub ssl_end: Option<f64>,
    pub send_start: Option<f64>,
    pub send_end: Option<f64>,
    pub receive_headers_start: Option<f64>,
    pub receive_headers_end: Option<f64>,
    pub worker_start: Option<f64>,
    pub worker_ready: Option<f64>,
}

/// One frame of the stack that made a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitiatorFrame {
    pub function: Option<String>,
    /// As CDP reports it; empty for code without a URL.
    pub url: String,
    /// `CapturedScript.url` of the script the frame runs in, when the
    /// capture kept that script: the URL itself for an external script,
    /// `inline:<n>` for an inline or runtime one.
    pub script_url: Option<String>,
    /// 0-based, as CDP reports them.
    pub line: u32,
    pub column: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkInitiator {
    pub kind: InitiatorKind,
    /// `initiator.url`: the document or script, when CDP names one.
    pub url: Option<String>,
    /// 0-based, as CDP reports it (parser initiators).
    pub line: Option<u32>,
    pub column: Option<u32>,
    /// Innermost first, async parents appended, at most
    /// [`MAX_INITIATOR_FRAMES`].
    pub stack: Vec<InitiatorFrame>,
}

/// One request of the visit, from every frame and worker of the scanned tab,
/// with everything the browser said about it. In memory only; what is stored
/// of it, and how, is the sink's decision.
#[derive(Debug, Clone, PartialEq)]
pub struct NetworkRecord {
    /// The capture's number for the request: counted from 0 in the order the
    /// requests were first seen, across all frames and workers. Each redirect
    /// hop has its own.
    pub seq: u32,
    /// Page view the request started on.
    pub page: u32,
    pub context: RequestContext,
    /// CDP frame id, when the request belongs to a frame.
    pub frame_id: Option<String>,
    /// CDP target id of the out-of-process iframe or worker that made it;
    /// `None` for the page's own target.
    pub target_id: Option<String>,
    pub method: String,
    /// Full URL with query and fragment. `data:` and `blob:` URLs without
    /// their payload.
    pub url: String,
    pub kind: ResourceKind,
    /// CDP `initialPriority`.
    pub priority: Option<String>,
    /// What was sent: `requestWillBeSentExtraInfo` where it fired, else the
    /// provisional headers of `requestWillBeSent`.
    pub request_headers: Vec<RawHeader>,
    /// `None` when the request has no body.
    pub post_data: Option<CapturedPostData>,
    /// The request has a body that could not be read.
    pub post_data_missing: bool,
    pub initiator: NetworkInitiator,
    /// The hop that redirected to this request.
    pub redirect_from: Option<u32>,
    /// For a preflight: the request it was sent for.
    pub preflight_of: Option<u32>,
    /// `None` when no response arrived.
    pub status: Option<u16>,
    pub mime_type: Option<String>,
    /// e.g. "h2", "http/1.1", "h3".
    pub protocol: Option<String>,
    /// `ip:port`.
    pub remote_address: Option<String>,
    /// What was received: `responseReceivedExtraInfo` where it fired, else
    /// the headers of `responseReceived`.
    pub response_headers: Vec<RawHeader>,
    pub served_from: ServedFrom,
    /// ms since the start of page view `page` when the request was made
    /// (`requestWillBeSent`), on the same clock as the tracker's `t`. With
    /// `Capture.navigation_start` and the page view's `started_at_ms` this
    /// gives the wall-clock time.
    pub started_at_ms: f64,
    /// Same clock: when the response's headers arrived.
    pub response_at_ms: Option<f64>,
    /// Same clock: when the request ended. `None` while `end` is `Pending`.
    pub ended_at_ms: Option<f64>,
    /// ms between the request being made and the browser starting work on
    /// it (`timing.requestTime` minus the event's timestamp).
    pub queued_ms: Option<f64>,
    pub timing: Option<ResourceTiming>,
    /// Bytes on the wire, headers included (`encodedDataLength`).
    pub encoded_bytes: u64,
    /// Decoded body bytes received (`dataReceived`).
    pub decoded_bytes: u64,
    pub end: RequestEnd,
    /// Prebid bidder code when the request is a recognised bid request.
    pub bidder: Option<String>,
    pub body: CapturedBody,
}
