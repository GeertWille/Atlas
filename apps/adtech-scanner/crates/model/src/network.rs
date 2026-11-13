//! The scan archive as the frontend sees it: the network view, the bid
//! inspector, the script viewer and the storage settings.
//!
//! Ids: a request is `r<seq>` (`r412`), where `seq` is the capture's own
//! number for it; a script is `s<n>` as before. Every time is in ms since the
//! start of the page view the record's `page` names, like everywhere else.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{BidStatus, SchainNode};

contract! {
    /// Whether a scan has a network record, and if not, why.
    #[derive(Copy, Eq, Hash)]
    pub enum NetworkAvailability {
        /// Requests, headers and kept bodies can be read.
        Available,
        /// The scan was stored before the archive existed, or ended before
        /// its capture did.
        NotRecorded,
        /// "Keep network data" was off when the scan ran.
        Disabled,
        /// Headers and bodies were removed to stay under the storage limit;
        /// the request list (host, path, status, sizes, timing) is still there.
        Evicted,
    }

    /// The archive of one scan.
    pub struct NetworkArchiveInfo {
        pub availability: NetworkAvailability,
        pub request_count: u32,
        /// Distinct bodies kept.
        pub body_count: u32,
        /// Bytes of the scan's pack file on disk; 0 once evicted.
        pub stored_bytes: f64,
        /// Cookie and authorization values were replaced when it was written.
        pub headers_redacted: bool,
        /// Some bodies were left out because the scan reached its size cap or
        /// the writer could not keep up; those requests say so themselves.
        pub truncated: bool,
    }

    #[derive(Copy, Eq, Hash)]
    pub enum ResourceKind {
        Document,
        Script,
        Stylesheet,
        Image,
        Media,
        Font,
        Xhr,
        Fetch,
        Ping,
        WebSocket,
        EventSource,
        /// A CORS preflight (`OPTIONS`) the browser sent by itself.
        Preflight,
        Other,
    }

    /// Where a request was made.
    #[derive(Copy, Eq, Hash)]
    pub enum RequestContext {
        /// The page's main frame.
        Page,
        /// Any other frame, in the page's process or its own.
        Iframe,
        /// A dedicated or shared worker.
        Worker,
        ServiceWorker,
    }

    /// What a request is for, in ad-tech terms.
    #[derive(Copy, Eq, Hash)]
    pub enum NetworkCategory {
        /// A bidder's or Prebid Server's auction endpoint.
        Bid,
        /// The ad server's ad call (GPT, Xandr AST, Smart AdServer, ...).
        AdServer,
        /// Prebid, the publisher's wrapper and the ad libraries themselves.
        Wrapper,
        Consent,
        /// User id modules and cookie syncs.
        Identity,
        /// Analytics, viewability, verification, beacons.
        Measurement,
        /// What an ad loads once it renders.
        Creative,
        /// The scanned site itself.
        Site,
        OtherThirdParty,
    }

    #[derive(Copy, Eq, Hash)]
    pub enum RequestOutcome {
        /// Answered with a status below 400.
        Ok,
        /// Answered with a status of 400 or more.
        HttpError,
        /// A network error.
        Failed,
        /// The browser refused to send or to deliver it (CORS, CSP, mixed
        /// content, a blocker).
        Blocked,
        /// Aborted, usually because the page navigated.
        Cancelled,
        /// No answer before the visit ended.
        Pending,
    }

    #[derive(Copy, Eq, Hash)]
    pub enum ServedFrom {
        Network,
        DiskCache,
        MemoryCache,
        ServiceWorker,
        Prefetch,
    }

    /// How a bid request's answer relates to the auction it was sent for.
    #[derive(Copy, Eq, Hash)]
    pub enum ExchangeVerdict {
        /// The answer arrived before the auction closed.
        InTime,
        /// The answer arrived within the clock tolerance of the auction's
        /// close: too close to call. No number is given.
        AtDeadline,
        /// The answer arrived after the auction closed.
        LateOnNetwork,
        /// The answer arrived in time, but Prebid recorded a timeout.
        InTimeButTimedOut,
        /// No answer before the visit ended.
        NeverAnswered,
        /// The request failed or was blocked.
        Failed,
        /// Prebid announced the bid but no request left the page.
        NotSent,
    }

    /// Whether a body can be read, and if not, why.
    #[derive(Copy, Eq, Hash)]
    pub enum BodyState {
        /// Stored whole.
        Kept,
        /// Stored up to the per-body cap; `stored_bytes` of `bytes`.
        Truncated,
        /// There is no body: nothing was sent, or the answer was empty.
        Empty,
        /// Images, video, audio, fonts and other binary data are recorded as
        /// metadata only.
        NotKeptType,
        /// The scan had reached its archive size cap.
        ScanLimit,
        /// Left out because the archive writer could not keep up.
        Dropped,
        /// The browser could not hand it over: the load failed or was
        /// cancelled, it was a redirect or preflight, or Chrome had already
        /// discarded it.
        Unavailable,
        /// The request never finished.
        Pending,
        /// Removed to stay under the storage limit.
        Evicted,
        /// Larger than one message of the connection to a remote browser
        /// can carry, so it was never asked for.
        TooLargeForRemote,
    }

    #[derive(Copy, Eq, Hash)]
    pub enum InitiatorKind {
        /// The HTML parser.
        Parser,
        Script,
        Preload,
        /// A preflight: its initiator is the request it clears the way for.
        Preflight,
        Other,
    }

    /// What started a request, reduced to one place in the code.
    pub struct InitiatorRef {
        pub kind: InitiatorKind,
        /// URL of the script or document, as captured.
        pub url: Option<String>,
        /// The scan's script it belongs to, when the capture kept that script.
        pub script_id: Option<String>,
        /// 1-based, in the source as captured (not formatted).
        pub line: Option<u32>,
        /// 1-based.
        pub column: Option<u32>,
    }

    /// Durations in ms for the waterfall bar.
    pub struct RowPhases {
        /// From the request being made to work on it starting.
        pub queue: f64,
        /// DNS, TCP and TLS together.
        pub connect: f64,
        /// From the request being sent to the first byte of the answer.
        pub wait: f64,
        pub receive: f64,
    }

    /// The auction a bid or ad-server request belongs to, and how it went.
    pub struct AdLink {
        pub auction_id: String,
        /// Ad unit codes the request carried bids for.
        pub ad_units: Vec<String>,
        /// A Prebid Server request that fans out to several bidders.
        pub via_server: bool,
        /// True when an id of the auction was found in the request; false
        /// when it was matched by bidder and timing only.
        pub exact: bool,
        pub verdict: ExchangeVerdict,
        /// ms the answer arrived after the auction closed; negative when it
        /// was early. `None` when there was no answer, and inside the clock
        /// tolerance.
        pub late_by_ms: Option<f64>,
        /// The highest bid the answer carried.
        pub cpm: Option<f64>,
        /// The currency `cpm` is in, as the answer or Prebid stated it. `None`
        /// when neither did, and on scans stored before this was kept.
        #[serde(default)]
        pub currency: Option<String>,
        /// OpenRTB no-bid reason of the answer.
        pub nbr: Option<u32>,
    }

    /// One row of the request list. Carries no headers and no bodies.
    pub struct NetworkRow {
        /// `r<seq>`.
        pub id: String,
        pub seq: u32,
        /// Page view the request started on.
        pub page: u32,
        pub context: RequestContext,
        pub method: String,
        /// As captured. Without its query string once the archive is evicted.
        pub url: String,
        pub host: String,
        pub kind: ResourceKind,
        pub mime: Option<String>,
        pub status: Option<u16>,
        pub outcome: RequestOutcome,
        /// Network error or blocked reason.
        pub error: Option<String>,
        pub category: NetworkCategory,
        pub vendor: Option<String>,
        /// Prebid bidder code of a recognised bid endpoint.
        pub bidder: Option<String>,
        pub first_party: bool,
        pub initiator: Option<InitiatorRef>,
        pub started_at_ms: f64,
        /// `None` when it never finished.
        pub duration_ms: Option<f64>,
        pub phases: Option<RowPhases>,
        /// Bytes on the wire; 0 when served from a cache or a service worker.
        pub transferred_bytes: f64,
        /// Size of the decoded body.
        pub decoded_bytes: f64,
        pub served_from: ServedFrom,
        /// The previous hop, when this request follows a redirect.
        pub redirect_of: Option<String>,
        /// The request this preflight was sent for.
        pub preflight_of: Option<String>,
        pub has_request_body: bool,
        /// A body that `get_network_body` can return.
        pub has_response_body: bool,
        /// An ad-server ad call that carried no Prebid (`hb_`) targeting.
        pub no_hb_targeting: bool,
        pub ad: Option<AdLink>,
    }

    /// `list_network_requests`: rows ordered by page view, then start time.
    pub struct NetworkRowChunk {
        pub rows: Vec<NetworkRow>,
        /// Pass it back to get the next chunk; `None` after the last one.
        pub next_cursor: Option<String>,
        /// Rows of the whole selection, not of this chunk.
        pub total: u32,
    }

    pub struct NetworkCategoryCount {
        pub category: NetworkCategory,
        pub requests: u32,
        pub transferred_bytes: f64,
    }

    pub struct NetworkVendorCount {
        pub vendor: String,
        pub requests: u32,
    }

    pub struct NetworkPageCount {
        pub page: u32,
        pub requests: u32,
        pub transferred_bytes: f64,
    }

    /// `get_network_summary`: what the report's digest tab shows.
    pub struct NetworkSummary {
        pub archive: NetworkArchiveInfo,
        pub total_requests: u32,
        pub transferred_bytes: f64,
        /// Status of 400 or more, or a network error.
        pub failed: u32,
        pub blocked_or_cancelled: u32,
        pub late_bids: u32,
        pub unanswered_bids: u32,
        /// Requests not served from the network.
        pub from_cache: u32,
        pub categories: Vec<NetworkCategoryCount>,
        /// The ten vendors with most requests.
        pub vendors: Vec<NetworkVendorCount>,
        pub pages: Vec<NetworkPageCount>,
        /// At most eight rows worth opening: failed and late bid calls,
        /// blocked requests, the slowest request before the first auction.
        pub notable: Vec<NetworkRow>,
    }

    #[derive(Copy, Eq, Hash)]
    pub enum HeaderRedaction {
        /// A `Cookie` or `Set-Cookie` value: cookie names and lengths are kept.
        Cookie,
        /// An `Authorization` or `Proxy-Authorization` value: the scheme and
        /// the length are kept.
        Credential,
    }

    pub struct Header {
        /// As sent, case included.
        pub name: String,
        /// The value, or what was stored in its place when `redaction` is set,
        /// e.g. `_ga=[27 bytes]; id=[36 bytes]`.
        pub value: String,
        pub redaction: Option<HeaderRedaction>,
    }

    /// The parts of a request in ms; `None` where the browser reported none
    /// (a reused connection has no DNS, connect or TLS time).
    pub struct TimingDetail {
        pub queued_ms: Option<f64>,
        pub dns_ms: Option<f64>,
        pub connect_ms: Option<f64>,
        pub tls_ms: Option<f64>,
        pub send_ms: Option<f64>,
        pub wait_ms: Option<f64>,
        pub receive_ms: Option<f64>,
        /// ms since the start of the page view when the answer's headers
        /// arrived, and when the request ended.
        pub response_at_ms: Option<f64>,
        pub finished_at_ms: Option<f64>,
    }

    pub struct RedirectHop {
        pub request_id: String,
        pub url: String,
        pub status: Option<u16>,
    }

    pub struct StackFrame {
        pub function: Option<String>,
        pub url: String,
        pub script_id: Option<String>,
        /// 1-based, in the source as captured.
        pub line: u32,
        /// 1-based.
        pub column: u32,
    }

    /// One body of a request, without its content.
    pub struct BodyRef {
        pub state: BodyState,
        /// Size as the browser reported it.
        pub bytes: f64,
        /// What `get_network_body` can return; less than `bytes` when
        /// truncated, 0 when not kept.
        pub stored_bytes: f64,
        pub mime: Option<String>,
        /// The scan's script with this exact content, to open in the viewer.
        pub script_id: Option<String>,
    }

    /// One ad unit's part in a bid request.
    pub struct AdContextUnit {
        pub ad_unit: String,
        /// What Prebid recorded for the bid.
        pub status: BidStatus,
        pub cpm: Option<f64>,
        /// The currency Prebid recorded for the bid.
        pub currency: Option<String>,
        pub verdict: ExchangeVerdict,
    }

    /// The ad-tech side of a request: its auction, or its slot.
    pub struct AdContext {
        pub link: AdLink,
        pub bidder: Option<String>,
        pub units: Vec<AdContextUnit>,
        /// ms since the start of the page view.
        pub auction_started_at_ms: Option<f64>,
        pub auction_ended_at_ms: Option<f64>,
        pub timeout_ms: Option<f64>,
        /// Element id of the slot an ad-server call requested an ad for.
        pub slot_element_id: Option<String>,
        /// Findings and bidder check codes that concern this bidder.
        pub finding_ids: Vec<String>,
        pub check_codes: Vec<String>,
    }

    #[derive(Copy, Eq, Hash)]
    pub enum ExchangeProtocol {
        /// OpenRTB 2.x.
        OpenRtb,
        /// OpenRTB 2.x to Prebid Server, with its `ext.prebid` fields.
        PrebidServer,
    }

    pub struct DecodedImp {
        pub id: String,
        /// The Prebid ad unit the impression is for, when it can be told.
        pub ad_unit: Option<String>,
        pub tag_id: Option<String>,
        /// "banner", "video", "native", "audio".
        pub media_types: Vec<String>,
        pub sizes: Vec<String>,
        pub floor: Option<f64>,
        pub floor_currency: Option<String>,
        /// Prebid Server: the bidders asked for this impression.
        pub bidders: Vec<String>,
    }

    pub struct DecodedSchain {
        /// The chain's own claim to be complete.
        pub complete: Option<bool>,
        pub nodes: Vec<SchainNode>,
    }

    pub struct DecodedBidRequest {
        pub id: Option<String>,
        pub tmax_ms: Option<f64>,
        pub imps: Vec<DecodedImp>,
        pub site_domain: Option<String>,
        pub site_page: Option<String>,
        pub currencies: Vec<String>,
        pub schain: Option<DecodedSchain>,
        pub gdpr: Option<bool>,
        pub coppa: Option<bool>,
        /// Presence only: the strings and ids themselves are in the raw body.
        pub consent_string_present: bool,
        pub us_privacy_present: bool,
        pub gpp_present: bool,
        /// `source` of each extended id, without the ids.
        pub eid_sources: Vec<String>,
        pub test: bool,
    }

    pub struct DecodedBid {
        pub imp_id: String,
        pub ad_unit: Option<String>,
        pub price: f64,
        pub size: Option<String>,
        pub advertiser_domains: Vec<String>,
        pub creative_id: Option<String>,
        pub deal_id: Option<String>,
        /// Size of the creative markup (`adm`).
        pub markup_bytes: Option<f64>,
    }

    pub struct DecodedSeat {
        pub seat: Option<String>,
        pub bids: Vec<DecodedBid>,
    }

    pub struct ServerBidderTiming {
        pub bidder: String,
        pub ms: f64,
    }

    pub struct ServerBidderError {
        pub bidder: String,
        pub code: Option<u32>,
        pub message: String,
    }

    pub struct DecodedBidResponse {
        pub id: Option<String>,
        pub currency: Option<String>,
        /// OpenRTB no-bid reason, and what the specification calls it.
        pub nbr: Option<u32>,
        pub nbr_meaning: Option<String>,
        pub seats: Vec<DecodedSeat>,
        /// Prebid Server `ext.responsetimemillis` and `ext.errors`.
        pub server_timings: Vec<ServerBidderTiming>,
        pub server_errors: Vec<ServerBidderError>,
    }

    /// A bid exchange read as OpenRTB. Either side can be missing: a body
    /// that was not kept, or one that is not OpenRTB.
    pub struct DecodedExchange {
        pub protocol: ExchangeProtocol,
        pub request: Option<DecodedBidRequest>,
        pub response: Option<DecodedBidResponse>,
    }

    /// `get_network_request`.
    pub struct NetworkRequestDetail {
        pub row: NetworkRow,
        /// False once the archive is evicted: only `row` is filled then.
        pub available: bool,
        pub request_headers: Vec<Header>,
        pub response_headers: Vec<Header>,
        pub protocol: Option<String>,
        pub priority: Option<String>,
        pub remote_address: Option<String>,
        pub timing: Option<TimingDetail>,
        /// Every hop in order, this request included; empty without redirects.
        pub redirect_chain: Vec<RedirectHop>,
        /// Duration of the preflight that preceded this request.
        pub preflight_ms: Option<f64>,
        /// Innermost frame first; empty when no script made the request.
        pub stack: Vec<StackFrame>,
        pub request_body: BodyRef,
        pub response_body: BodyRef,
        pub ad_context: Option<AdContext>,
        /// `None` for anything that is not a bid exchange, and for adapters
        /// without a decoder.
        pub decoded: Option<DecodedExchange>,
    }

    #[derive(Copy, Eq, Hash)]
    pub enum BodySide {
        Request,
        Response,
    }

    /// `get_network_body`: a stretch of a body as text.
    pub struct NetworkBodyChunk {
        pub state: BodyState,
        /// `None` when there is nothing to read. Invalid UTF-8 is replaced.
        pub text: Option<String>,
        /// Byte offset this chunk starts at.
        pub offset: f64,
        /// Byte offset to ask for next; `None` when this was the last chunk.
        pub next_offset: Option<f64>,
        /// Stored bytes of the whole body.
        pub total_bytes: f64,
    }

    /// The network side of one bid, set by the backend when the scan has an
    /// archive.
    pub struct BidExchange {
        /// `None` when no request left the page for it.
        pub request_id: Option<String>,
        pub verdict: ExchangeVerdict,
        pub late_by_ms: Option<f64>,
        /// From the request being sent to its answer being complete.
        pub network_ms: Option<f64>,
        /// Matched by an id found in the request (true) or by timing (false).
        pub exact: bool,
    }
}

// ---------------------------------------------------------------------------
// Script viewer
// ---------------------------------------------------------------------------

contract! {
    #[derive(Copy, Eq, Hash)]
    pub enum SourceView {
        /// Printed by the app's formatter: the text findings cite lines of.
        Formatted,
        /// As captured.
        Original,
    }

    #[derive(Copy, Eq, Hash)]
    pub enum SourceAvailability {
        Available,
        /// The scan was stored before sources were kept.
        NotRecorded,
        /// "Keep network data" was off, or the scan had reached its size cap.
        NotRetained,
        /// Removed to stay under the storage limit.
        Evicted,
        /// Larger than a reply may be; facts only.
        TooLarge,
    }

    #[derive(Copy, Eq, Hash)]
    pub enum SourceFormat {
        /// Parsed and printed by a JavaScript parser.
        Parser,
        /// The file did not parse; reflowed token by token.
        Reflow,
        /// Shown as captured: too large or too slow to format, or the
        /// original view.
        Unformatted,
    }

    /// Where a finding's code evidence sits in the formatted text.
    pub struct SourceAnchor {
        pub finding_id: String,
        /// Index into the finding's `evidence`.
        pub evidence_index: u32,
        /// 1-based, inclusive.
        pub start_line: u32,
        pub end_line: u32,
        /// False when the excerpt was not found again and the stored line
        /// numbers are shown as they are.
        pub located: bool,
    }

    #[derive(Copy, Eq, Hash)]
    pub enum RegionKind {
        /// Lines the analysis model read with `read_script`.
        Read,
        /// A chunk of Jev's reading guide.
        Guide,
    }

    pub struct SourceRegion {
        pub kind: RegionKind,
        /// 1-based, inclusive, in the formatted text.
        pub start_line: u32,
        pub end_line: u32,
        /// Position in the reading guide, 1 is best. Guide regions only.
        pub rank: Option<u32>,
    }

    /// A call into the ad stack (`requestBids`, `defineSlot`, ...).
    pub struct AdCall {
        pub name: String,
        /// 1-based line of the returned text.
        pub line: u32,
    }

    /// `get_script_source`.
    pub struct ScriptSource {
        pub scan_id: String,
        pub script_id: String,
        pub view: SourceView,
        pub availability: SourceAvailability,
        /// `None` unless `availability` is `available`.
        pub text: Option<String>,
        pub line_count: u32,
        pub format: SourceFormat,
        /// Formatted view only.
        pub anchors: Vec<SourceAnchor>,
        /// Formatted view only.
        pub regions: Vec<SourceRegion>,
        pub ad_calls: Vec<AdCall>,
        /// The line of the returned text that the asked-for position in the
        /// original source (`originalLine`, `originalColumn`) is on.
        pub focus_line: Option<u32>,
    }
}

// ---------------------------------------------------------------------------
// Storage
// ---------------------------------------------------------------------------

contract! {
    /// `get_archive_storage` and `clear_archive`.
    pub struct ArchiveStorage {
        /// Scans whose network data is on disk.
        pub scans: u32,
        /// Scans whose network data was removed to stay under the limit.
        pub evicted_scans: u32,
        /// Bytes of all pack files.
        pub stored_bytes: f64,
        pub limit_bytes: f64,
        /// Start of the oldest scan that still has network data, epoch ms.
        pub oldest_at: Option<f64>,
    }
}

contract! {
    /// `get_sellers_json_cache` and `clear_sellers_json_cache`: the
    /// sellers.json files of ad exchanges that are kept between scans.
    pub struct SellersJsonCacheInfo {
        /// Exchanges with an answer on record, failed lookups included.
        pub entries: u32,
        /// Bytes on disk.
        pub stored_bytes: f64,
    }
}

/// Default of `Settings.archive_limit_bytes`: 5 GiB.
pub const DEFAULT_ARCHIVE_LIMIT_BYTES: f64 = 5.0 * 1024.0 * 1024.0 * 1024.0;

/// The public id of the request the capture numbered `seq`.
pub fn request_id(seq: u32) -> String {
    format!("r{seq}")
}

/// The capture's number of the request with this public id.
pub fn request_seq(id: &str) -> Option<u32> {
    id.strip_prefix('r')?.parse().ok()
}

/// The name of an OpenRTB no-bid reason code (`nbr`), from the list OpenRTB
/// 2.6 refers to (OpenRTB 3.0, "List: No-Bid Reason Codes"), which renamed
/// codes 3 and 9 of the 2.5 list and added 11 to 17. `None` for any other
/// code: from 500 on they are an exchange's own and mean nothing without
/// its documentation.
pub fn no_bid_reason_name(code: u32) -> Option<&'static str> {
    Some(match code {
        0 => "Unknown Error",
        1 => "Technical Error",
        2 => "Invalid Request",
        3 => "Known Web Crawler",
        4 => "Suspected Non-Human Traffic",
        5 => "Cloud, Data Center, or Proxy IP",
        6 => "Unsupported Device",
        7 => "Blocked Publisher or Site",
        8 => "Unmatched User",
        9 => "Daily User Cap Met",
        10 => "Daily Domain Cap Met",
        11 => "Ads.txt Authorization Unavailable",
        12 => "Ads.txt Authorization Violation",
        13 => "Ads.cert Authentication Unavailable",
        14 => "Ads.cert Authentication Violation",
        15 => "Insufficient Auction Time",
        16 => "Incomplete SupplyChain",
        17 => "Blocked SupplyChain Node",
        _ => return None,
    })
}
