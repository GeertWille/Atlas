//! Raw, in-memory output of one page capture.
//!
//! This is the hand-off between the browser crate (which produces it) and the
//! engine (which derives auctions, slots, metrics and findings from it). It is
//! deliberately **never persisted**: script bodies and raw telemetry are dropped
//! once the scan finishes.

use serde::{Deserialize, Serialize};

use crate::DeviceProfile;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureRequest {
    pub url: String,
    pub device: DeviceProfile,
    pub observe_seconds: u32,
    pub auto_scroll: bool,
    pub accept_consent: bool,
    pub headless: bool,
    /// Explicit browser executable; `None` means auto-detect.
    pub chrome_path: Option<String>,
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
    /// ms since navigation start when loading finished.
    pub loaded_at_ms: Option<f64>,
    /// URL of the script or document that initiated the request, if known.
    pub initiator: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConsoleLine {
    /// ms since navigation start.
    pub t: f64,
    /// "log" | "info" | "warning" | "error" | "debug"
    pub level: String,
    pub text: String,
}

/// One event emitted by the injected tracker script.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackerEvent {
    /// ms since navigation start.
    pub t: f64,
    #[serde(flatten)]
    pub kind: TrackerEventKind,
}

/// Tracker event payloads. The injected script emits JSON objects of the shape
/// `{ "t": 1234.5, "type": "bidResponse", ...fields }`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
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
    AuctionInit {
        auction_id: String,
        timeout: Option<f64>,
        ad_units: Vec<TrackedAdUnit>,
    },
    BidRequested {
        auction_id: String,
        bidder: String,
        ad_unit_codes: Vec<String>,
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
        /// "domContentLoaded" | "load" | "firstScroll" | "firstInteraction"
        phase: String,
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
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackedAdUnit {
    pub code: String,
    pub sizes: Vec<String>,
    pub bidders: Vec<String>,
}
