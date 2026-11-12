//! Raw, in-memory output of one page capture.
//!
//! This is the hand-off between the browser crate (which produces it) and the
//! engine (which derives auctions, slots, metrics and findings from it). It is
//! deliberately **never persisted**: script bodies and raw telemetry are dropped
//! once the scan finishes.

use serde::{Deserialize, Serialize};

use crate::{DeviceProfile, FollowedLink, PageReach, ProxyScheme, VisitorBehavior};

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

/// One node of an OpenRTB supply chain object.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SchainNode {
    /// Canonical domain of the advertising system.
    pub asi: String,
    /// Seller id within that system.
    pub sid: String,
    /// Whether the node takes part in the flow of payment.
    pub hp: Option<bool>,
}
