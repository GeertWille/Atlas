//! Shared data contract for the Ad-Tech Agentic Scanner.
//!
//! Every type here is serialized as camelCase JSON and exported to TypeScript
//! (`src/bindings/`) with `cargo test -p adscan-model`. The frontend, the engine,
//! the browser capture crate and the Tauri command layer all speak these types.
//!
//! Conventions: timestamps are Unix epoch milliseconds (`f64`), durations are
//! milliseconds, money is USD, CPMs are in the currency the bidder reported.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub mod capture;
pub use capture::*;

macro_rules! contract {
    ($($item:item)*) => {
        $(
            #[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
            #[serde(rename_all = "camelCase")]
            #[ts(export, export_to = "../../../src/bindings/")]
            $item
        )*
    };
}

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

contract! {
    #[derive(Copy, Eq, Hash)]
    pub enum DeviceProfile {
        Desktop,
        Mobile,
    }

    #[derive(Copy, Eq, Hash)]
    pub enum Effort {
        Low,
        Medium,
        High,
    }

    #[derive(Copy, Eq, Hash)]
    pub enum ThemePreference {
        System,
        Light,
        Dark,
    }

    /// The vendors whose API the scanner can call with a user-supplied key.
    #[derive(Copy, Eq, Hash)]
    pub enum ApiProvider {
        Anthropic,
        Typesafe,
    }

    /// User preferences. Persisted locally; API keys are never part of this
    /// struct, they live in the OS keychain and are only reported via
    /// `has_api_key` and `has_typesafe_key`.
    pub struct Settings {
        /// Script triage model, used when Jev is off and as its fallback.
        pub filter_model: String,
        /// Phase 2 logic-analysis model.
        pub analysis_model: String,
        pub analysis_effort: Effort,
        /// Hard cap on LLM spend per scan in USD. The pipeline stops calling the
        /// model once the cap is reached and reports partial results.
        pub budget_usd: f64,
        /// Explicit Chrome/Chromium executable. `None` means auto-detect.
        pub chrome_path: Option<String>,
        pub headless: bool,
        pub default_device: DeviceProfile,
        /// Seconds to keep observing the page after load.
        pub default_observe_seconds: u32,
        pub theme: ThemePreference,
        /// Read-only: whether an Anthropic API key is stored in the keychain.
        pub has_api_key: bool,
        /// Classify scripts with TypeSafe's Jev System One instead of
        /// `filter_model`, and let it build the Phase 2 reading guide. Can only
        /// be true while a TypeSafe key is stored.
        pub jev_enabled: bool,
        /// Read-only: whether a TypeSafe API key is stored in the keychain.
        pub has_typesafe_key: bool,
        /// Look up the publisher's seller ids in the sellers.json of the ad
        /// exchanges its ads.txt names. Contacts those third-party hosts, so
        /// it is off unless the user turns it on.
        pub check_sellers_json: bool,
    }

    /// Partial update; omitted fields keep their current value.
    #[derive(Default)]
    pub struct SettingsPatch {
        #[ts(optional)]
        pub filter_model: Option<String>,
        #[ts(optional)]
        pub analysis_model: Option<String>,
        #[ts(optional)]
        pub analysis_effort: Option<Effort>,
        #[ts(optional)]
        pub budget_usd: Option<f64>,
        /// `Some(None)` clears the explicit path and returns to auto-detect.
        #[ts(optional, type = "string | null")]
        #[serde(default, skip_serializing_if = "Option::is_none", with = "double_option")]
        pub chrome_path: Option<Option<String>>,
        #[ts(optional)]
        pub headless: Option<bool>,
        #[ts(optional)]
        pub default_device: Option<DeviceProfile>,
        #[ts(optional)]
        pub default_observe_seconds: Option<u32>,
        #[ts(optional)]
        pub theme: Option<ThemePreference>,
        #[ts(optional)]
        pub jev_enabled: Option<bool>,
        #[ts(optional)]
        pub check_sellers_json: Option<bool>,
    }

    pub struct BrowserInfo {
        pub path: String,
        /// e.g. "Google Chrome 141.0.7390.55"
        pub version: String,
        /// Human name of the browser family ("Google Chrome", "Chromium", "Brave", ...).
        pub name: String,
    }

    pub struct ApiKeyStatus {
        pub valid: bool,
        /// Human-readable reason when `valid` is false.
        pub message: Option<String>,
    }
}

mod double_option {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(
        value: &Option<Option<String>>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        match value {
            Some(inner) => inner.serialize(serializer),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<Option<String>>, D::Error> {
        Option::<String>::deserialize(deserializer).map(Some)
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            filter_model: "claude-haiku-5-5".into(),
            analysis_model: "claude-sonnet-5-5".into(),
            analysis_effort: Effort::Medium,
            budget_usd: 0.50,
            chrome_path: None,
            headless: true,
            default_device: DeviceProfile::Desktop,
            default_observe_seconds: 20,
            theme: ThemePreference::System,
            has_api_key: false,
            jev_enabled: false,
            has_typesafe_key: false,
            check_sellers_json: false,
        }
    }
}

// ---------------------------------------------------------------------------
// Scans
// ---------------------------------------------------------------------------

contract! {
    pub struct ScanOptions {
        /// Absolute http(s) URL of the page to scan.
        pub url: String,
        pub device: DeviceProfile,
        /// Seconds to observe the page after the load event.
        pub observe_seconds: u32,
        /// Scroll through the page during observation to trigger lazy ad logic.
        pub auto_scroll: bool,
        /// Try to accept the consent banner (CMP) so bidding is not blocked.
        pub accept_consent: bool,
        /// Run the two LLM phases. When false (or no API key) only the
        /// deterministic checks run.
        pub ai_enabled: bool,
    }

    #[derive(Copy, Eq, Hash)]
    pub enum ScanStatus {
        Queued,
        Running,
        Completed,
        Failed,
        Cancelled,
    }

    #[derive(Copy, Eq, Hash, PartialOrd, Ord)]
    pub enum Severity {
        Info,
        Low,
        Medium,
        High,
        Critical,
    }

    #[derive(Default, Copy, Eq)]
    pub struct SeverityCounts {
        pub critical: u32,
        pub high: u32,
        pub medium: u32,
        pub low: u32,
        pub info: u32,
    }

    /// Row in the scan history list.
    pub struct ScanSummary {
        pub id: String,
        pub url: String,
        pub hostname: String,
        pub device: DeviceProfile,
        pub status: ScanStatus,
        pub started_at: f64,
        pub finished_at: Option<f64>,
        pub duration_ms: Option<f64>,
        /// LLM spend so far in USD.
        pub cost_usd: f64,
        pub finding_counts: SeverityCounts,
        /// 0-100 ad stack health score; `None` until the scan completes.
        pub health_score: Option<u32>,
        pub prebid_version: Option<String>,
        /// Whether the LLM phases actually ran for this scan.
        pub ai_used: bool,
        /// Failure reason when `status` is `Failed`.
        pub error: Option<String>,
    }

    /// Everything the report screen needs for one scan.
    pub struct ScanDetail {
        pub summary: ScanSummary,
        pub options: ScanOptions,
        pub metrics: ScanMetrics,
        pub findings: Vec<Finding>,
        pub auctions: Vec<Auction>,
        pub slots: Vec<AdSlot>,
        pub scripts: Vec<ScriptRecord>,
        pub steps: Vec<PipelineStep>,
        /// Short executive summary written by the synthesis step.
        pub overview: Option<String>,
        /// `None` for scans stored before the bidder checks existed, and for
        /// scans that ended before they ran.
        #[serde(default)]
        pub bidder_diagnostics: Option<BidderDiagnostics>,
    }

    #[derive(Default)]
    pub struct ScanMetrics {
        pub page_load_ms: Option<f64>,
        pub auction_count: u32,
        pub ad_unit_count: u32,
        pub bidder_count: u32,
        pub bid_request_count: u32,
        pub bid_response_count: u32,
        pub bid_timeout_count: u32,
        pub no_bid_count: u32,
        /// bid_timeout_count / bid_request_count, 0-1.
        pub timeout_rate: f64,
        pub slot_count: u32,
        pub empty_slot_count: u32,
        /// Slots that rendered a creative / slots that requested one, 0-1.
        pub fill_rate: f64,
        pub avg_winning_cpm: Option<f64>,
        pub total_winning_cpm: f64,
        /// ms from navigation start to the first auction init.
        pub time_to_first_auction_ms: Option<f64>,
        /// ms from navigation start to the first ad render.
        pub time_to_first_ad_ms: Option<f64>,
        pub script_count: u32,
        pub ad_script_count: u32,
        pub total_script_bytes: f64,
        pub prebid_timeout_ms: Option<f64>,
        pub has_prebid: bool,
        pub has_gpt: bool,
        pub has_amazon_tam: bool,
    }
}

// ---------------------------------------------------------------------------
// Findings
// ---------------------------------------------------------------------------

contract! {
    #[derive(Copy, Eq, Hash)]
    pub enum FindingSource {
        /// Deterministic rule evaluated on captured telemetry.
        Heuristic,
        /// Produced by the Phase 2 logic-analysis model.
        Ai,
    }

    #[derive(Copy, Eq, Hash)]
    pub enum FindingCategory {
        /// Code path logic bug (wrong device detection, dead branch, race).
        LogicBug,
        /// Auction timing: timeouts, late bids, slow bidders.
        Timing,
        /// Wrapper / Prebid / GPT configuration problem.
        Configuration,
        /// Slots requested but left empty, or never requested.
        Fill,
        /// Lazy loading / viewport / scroll gating problems.
        LazyLoad,
        /// Price floors, currency, bid adjustments.
        Pricing,
        /// Duplicate libraries, version drift, script weight.
        Hygiene,
        /// Consent / privacy signal blocks bidding.
        Consent,
    }

    pub struct Finding {
        pub id: String,
        pub scan_id: String,
        pub severity: Severity,
        pub category: FindingCategory,
        pub source: FindingSource,
        pub title: String,
        /// What is wrong and why it loses revenue. Markdown.
        pub description: String,
        /// Concrete fix. Markdown.
        pub recommendation: String,
        /// 0-1 confidence that the finding is real.
        pub confidence: f64,
        /// Plain-language revenue impact, e.g. "Sidebar slots never bid on desktop".
        pub impact: String,
        /// Estimated share of ad revenue at risk, 0-100, when it can be estimated.
        pub estimated_revenue_impact_pct: Option<f64>,
        pub evidence: Vec<Evidence>,
        /// Ad unit codes / slot element ids this finding concerns.
        pub affected_slots: Vec<String>,
        pub affected_bidders: Vec<String>,
    }

    pub struct Evidence {
        pub kind: EvidenceKind,
        /// One-line caption shown above the body.
        pub label: String,
        /// Code excerpt, metric line or log line. Never a full raw payload.
        pub body: String,
        /// Script URL the excerpt came from, when `kind` is `Code`.
        pub script_url: Option<String>,
        /// 1-based line in the beautified source, when known.
        pub line: Option<u32>,
    }

    #[derive(Copy, Eq, Hash)]
    pub enum EvidenceKind {
        Code,
        Metric,
        Log,
        Timeline,
    }
}

// ---------------------------------------------------------------------------
// Prebid auctions and ad slots (derived, persisted analysis results)
// ---------------------------------------------------------------------------

contract! {
    pub struct Auction {
        pub id: String,
        /// ms since navigation start.
        pub started_at_ms: f64,
        pub ended_at_ms: Option<f64>,
        pub duration_ms: Option<f64>,
        /// Configured auction timeout.
        pub timeout_ms: Option<f64>,
        pub ad_units: Vec<AuctionAdUnit>,
    }

    pub struct AuctionAdUnit {
        pub code: String,
        /// Sizes requested, e.g. ["300x250", "728x90"].
        pub sizes: Vec<String>,
        pub bids: Vec<Bid>,
        pub winning_bidder: Option<String>,
        pub winning_cpm: Option<f64>,
    }

    #[derive(Copy, Eq, Hash)]
    pub enum BidStatus {
        /// Bid received in time.
        Bid,
        /// Bid received and won the Prebid auction.
        Won,
        /// Bid won and its creative was rendered.
        Rendered,
        NoBid,
        Timeout,
        Error,
        /// Requested, never answered before observation ended.
        Pending,
    }

    pub struct Bid {
        pub bidder: String,
        pub status: BidStatus,
        pub cpm: Option<f64>,
        pub currency: Option<String>,
        pub size: Option<String>,
        /// ms since navigation start when the request went out.
        pub requested_at_ms: Option<f64>,
        /// Round-trip latency.
        pub response_time_ms: Option<f64>,
        pub deal_id: Option<String>,
    }

    #[derive(Copy, Eq, Hash)]
    pub enum SlotOutcome {
        /// A creative rendered.
        Filled,
        /// Ad server answered with no creative.
        Empty,
        /// Defined but no request was ever made.
        NeverRequested,
        /// Requested but no response within the observation window.
        Pending,
    }

    pub struct AdSlot {
        /// DOM element id of the slot container.
        pub element_id: String,
        /// GPT ad unit path, e.g. "/1234/site/top".
        pub ad_unit_path: Option<String>,
        pub sizes: Vec<String>,
        pub outcome: SlotOutcome,
        pub rendered_size: Option<String>,
        /// All timings are ms since navigation start.
        pub defined_at_ms: Option<f64>,
        pub requested_at_ms: Option<f64>,
        pub response_at_ms: Option<f64>,
        pub rendered_at_ms: Option<f64>,
        pub viewable_at_ms: Option<f64>,
        /// Whether the container was inside the initial viewport.
        pub in_initial_viewport: Option<bool>,
        /// Whether Prebid targeting (hb_*) was set when the request went out.
        pub had_prebid_targeting: Option<bool>,
        /// Whether the ad request left before the Prebid auction for it ended.
        pub requested_before_auction_end: Option<bool>,
        pub advertiser_id: Option<String>,
        pub line_item_id: Option<String>,
    }
}

// ---------------------------------------------------------------------------
// Bidder diagnostics (derived results only - ads.txt, sellers.json and bid
// responses are never persisted)
// ---------------------------------------------------------------------------

contract! {
    /// What the scanner verified about each bidder's setup by itself.
    pub struct BidderDiagnostics {
        pub ads_txt: AdsTxtReport,
        /// Whether sellers.json was looked up at the ad exchanges.
        pub sellers_json_checked: bool,
        pub consent: ConsentReport,
        /// One entry per bidder of the page's ad units, by bidder code.
        pub bidders: Vec<BidderDiagnosis>,
    }

    #[derive(Copy, Eq, Hash)]
    pub enum AdsTxtStatus {
        /// Fetched and parsed.
        Found,
        /// The site answered that there is no such file.
        Missing,
        /// The request failed or the answer was not an ads.txt file; nothing
        /// can be concluded.
        Unreadable,
    }

    pub struct AdsTxtReport {
        /// The address the file was requested from.
        pub url: String,
        pub status: AdsTxtStatus,
        /// Why the file could not be used, when it could not.
        pub detail: Option<String>,
        /// Valid seller records in the file.
        pub record_count: u32,
        pub invalid_line_count: u32,
        /// The first few lines that are not valid records.
        pub invalid_lines: Vec<AdsTxtLineProblem>,
        pub owner_domain: Option<String>,
        pub manager_domains: Vec<String>,
    }

    pub struct AdsTxtLineProblem {
        /// 1-based line number in the file.
        pub line: u32,
        /// The line itself, at most 300 characters.
        pub excerpt: String,
        pub problem: String,
    }

    pub struct ConsentReport {
        /// Whether the page exposes the TCF API (`__tcfapi`).
        pub cmp_present: bool,
        /// Whether Prebid's `consentManagement` is configured. `None` when no
        /// Prebid config was captured.
        pub consent_management_configured: Option<bool>,
        /// What the CMP last reported. `None` when it reported nothing.
        pub gdpr_applies: Option<bool>,
        /// ms since navigation start when the CMP first reported a decided
        /// consent state.
        pub resolved_at_ms: Option<f64>,
        pub auction_count: u32,
        /// Auctions that started before the consent state was decided.
        pub auctions_before_consent: u32,
        /// Purposes (1-11) the user consented to, as last reported.
        pub consented_purposes: Vec<u32>,
    }

    pub struct BidderDiagnosis {
        pub bidder: String,
        /// Bid requests and bids in this scan, for context.
        pub requests: u32,
        pub bids: u32,
        /// Most certain first.
        pub checks: Vec<DiagnosticCheck>,
    }

    #[derive(Copy, Eq, Hash)]
    pub enum DiagnosticArea {
        /// Adapter params of the ad units.
        Params,
        /// What the bidder's endpoint answered.
        Response,
        AdsTxt,
        SellersJson,
        Consent,
    }

    #[derive(Copy, Eq, Hash)]
    pub enum DiagnosticStatus {
        /// Checked and fine.
        Pass,
        /// Checked and wrong: a likely reason for missing bids.
        Problem,
        /// Worth a look, not necessarily wrong.
        Notice,
        /// Could not be checked; `summary` says why.
        NotChecked,
    }

    pub struct DiagnosticCheck {
        pub area: DiagnosticArea,
        pub status: DiagnosticStatus,
        /// Stable identifier of the outcome, e.g. "params-missing".
        pub code: String,
        /// One sentence, e.g. "placementId missing on 3 of 4 ad units".
        pub summary: String,
        /// Short supporting excerpt (the matching ads.txt line, the endpoint's
        /// reason), at most 300 characters. Never a raw payload.
        pub excerpt: Option<String>,
        /// Ad unit codes the outcome concerns; empty when it concerns all.
        pub ad_units: Vec<String>,
    }
}

// ---------------------------------------------------------------------------
// Scripts (metadata only - bodies are never persisted)
// ---------------------------------------------------------------------------

contract! {
    #[derive(Copy, Eq, Hash)]
    pub enum ScriptCategory {
        /// Prebid.js itself.
        Prebid,
        /// Publisher header-bidding wrapper / ad orchestration code.
        Wrapper,
        /// Google Publisher Tag and other ad server libraries.
        AdServer,
        /// SSP / exchange / bidder adapters and sync scripts.
        Bidder,
        /// Consent management platform.
        Cmp,
        /// Ad verification, viewability, fraud, identity.
        AdQuality,
        Analytics,
        /// First-party application code not related to ads.
        Application,
        /// Third-party library unrelated to ads.
        Library,
        Unknown,
    }

    pub struct ScriptRecord {
        pub id: String,
        pub url: String,
        pub hostname: String,
        /// Inline `<script>` blocks have a synthetic URL and `inline: true`.
        pub inline: bool,
        pub size_bytes: f64,
        /// Content hash of the body (hex sha-256, first 16 chars).
        pub hash: String,
        pub category: ScriptCategory,
        pub ad_related: bool,
        /// Why script triage classified it this way.
        pub classification_reason: Option<String>,
        /// 0-1.
        pub classification_confidence: Option<f64>,
        /// Whether a deterministic rule or the model made the call.
        pub classified_by: FindingSource,
        /// Model id that made the call. `None` for rule-classified scripts and
        /// for scans stored before this field existed.
        pub classifier_model: Option<String>,
        /// Raw probability that the script is ad-related, 0-1. Only set when
        /// Jev System One classified it.
        pub ad_probability: Option<f64>,
        /// Whether the script was handed to Phase 2.
        pub analyzed: bool,
        /// Known vendor name when recognized ("Google", "Amazon", "Criteo", ...).
        pub vendor: Option<String>,
        /// ms since navigation start when the response finished.
        pub loaded_at_ms: Option<f64>,
        pub first_party: bool,
    }
}

// ---------------------------------------------------------------------------
// Pipeline + audit trail
// ---------------------------------------------------------------------------

contract! {
    /// Nodes of the scan graph, in execution order.
    #[derive(Copy, Eq, Hash)]
    pub enum PipelineNode {
        LaunchBrowser,
        /// Load the page with the tracker injected and observe it.
        Capture,
        /// Derive auctions, slots and metrics from the raw telemetry.
        Telemetry,
        /// Check each bidder's params, responses, ads.txt, sellers.json and
        /// consent.
        BidderDiagnostics,
        /// Deterministic revenue-leak rules.
        Heuristics,
        /// Script triage: classify which scripts are ad-related.
        FilterScripts,
        /// Beautify the selected scripts.
        Beautify,
        /// Phase 2: agentic logic analysis.
        AnalyzeLogic,
        /// Merge, dedupe and score findings; write the overview.
        Synthesize,
    }

    #[derive(Copy, Eq, Hash)]
    pub enum StepStatus {
        Pending,
        Running,
        Completed,
        Failed,
        /// Not run (AI disabled, no API key, budget exhausted, nothing to do).
        Skipped,
    }

    pub struct PipelineStep {
        pub id: String,
        pub scan_id: String,
        pub node: PipelineNode,
        pub status: StepStatus,
        pub started_at: Option<f64>,
        pub duration_ms: Option<f64>,
        /// One-line outcome, e.g. "Classified 42 scripts, 9 ad-related".
        pub summary: Option<String>,
        pub model: Option<String>,
        pub input_tokens: u32,
        pub output_tokens: u32,
        pub cost_usd: f64,
        pub log_count: u32,
    }

    #[derive(Copy, Eq, Hash)]
    pub enum LogLevel {
        Debug,
        Info,
        Warn,
        Error,
    }

    #[derive(Copy, Eq, Hash)]
    pub enum LogKind {
        /// Plain progress message.
        Message,
        /// Prompt sent to the model (system prompt + user turn, truncated bodies).
        LlmRequest,
        /// Model text output.
        LlmResponse,
        /// Summarized model reasoning, when the API returns it.
        Thinking,
        /// Tool the model asked to run, with its input.
        ToolCall,
        /// What the tool returned (truncated).
        ToolResult,
        /// A finding the model or a rule reported.
        Finding,
        /// How one script was classified: probabilities, thresholds, routing.
        Classification,
    }

    /// One entry of the audit trail. Stored and full-text indexed so developers
    /// can look up how the AI arrived at a conclusion.
    pub struct LogEntry {
        pub id: String,
        pub scan_id: String,
        pub step_id: String,
        pub node: PipelineNode,
        /// Unix epoch ms.
        pub ts: f64,
        pub level: LogLevel,
        pub kind: LogKind,
        pub message: String,
        /// Structured payload (tool input, token usage, ...).
        #[ts(type = "unknown")]
        pub data: Option<serde_json::Value>,
    }

    pub struct LogSearchHit {
        pub entry: LogEntry,
        pub scan_url: String,
        /// Message excerpt with the match wrapped in `<mark>` tags; all other
        /// characters are HTML-escaped.
        pub snippet: String,
    }

    pub struct ScanListQuery {
        #[ts(optional)]
        pub search: Option<String>,
        #[ts(optional)]
        pub status: Option<ScanStatus>,
        #[ts(optional)]
        pub device: Option<DeviceProfile>,
        #[ts(optional)]
        pub limit: Option<u32>,
        #[ts(optional)]
        pub offset: Option<u32>,
    }

    pub struct ScanList {
        pub items: Vec<ScanSummary>,
        pub total: u32,
    }

    /// Aggregates for the dashboard.
    #[derive(Default)]
    pub struct DashboardStats {
        pub total_scans: u32,
        pub sites_scanned: u32,
        pub open_critical_findings: u32,
        pub total_findings: u32,
        pub total_cost_usd: f64,
        pub avg_cost_usd: f64,
        pub avg_health_score: Option<f64>,
    }

    #[derive(Copy, Eq, Hash)]
    pub enum ExportFormat {
        Json,
        Markdown,
    }
}

// ---------------------------------------------------------------------------
// Events pushed from the backend to the UI
// ---------------------------------------------------------------------------

/// Tauri event name carrying a [`ScanEvent`].
pub const SCAN_EVENT: &str = "scan-event";

contract! {
    #[serde(tag = "type")]
    pub enum ScanEvent {
        /// Scan row changed (status, cost, counts).
        #[serde(rename_all = "camelCase")]
        ScanUpdated { summary: ScanSummary },
        /// A pipeline step started, finished or updated its totals.
        #[serde(rename_all = "camelCase")]
        StepUpdated { step: PipelineStep },
        /// New audit-trail entry.
        #[serde(rename_all = "camelCase")]
        Log { entry: LogEntry },
        /// A finding was recorded while the scan is still running.
        #[serde(rename_all = "camelCase")]
        FindingAdded { finding: Finding },
    }

    /// Error shape returned by every Tauri command.
    pub struct CommandError {
        pub code: ErrorCode,
        pub message: String,
    }

    #[derive(Copy, Eq, Hash)]
    pub enum ErrorCode {
        InvalidInput,
        NotFound,
        BrowserNotFound,
        BrowserFailed,
        ApiKeyMissing,
        ApiKeyInvalid,
        LlmFailed,
        Storage,
        Cancelled,
        Internal,
    }
}
