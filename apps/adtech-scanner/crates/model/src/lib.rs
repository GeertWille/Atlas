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

pub mod network;
pub use network::*;

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

    /// What the scanner does on the page while it records.
    #[derive(Copy, Eq, Hash)]
    pub enum VisitorBehavior {
        /// Load the page and watch; no input at all.
        None,
        /// Scroll through the page.
        Scroll,
        /// Move, scroll and follow same-site links like a reader would.
        Browse,
        /// A person drives the browser; the scanner only records.
        Manual,
    }

    #[derive(Copy, Eq, Hash)]
    pub enum ProxyScheme {
        Http,
        Https,
        Socks5,
    }

    /// A named proxy the user saved, e.g. "Belgium". Its password is never
    /// part of this type: it lives in the OS keychain, one entry per profile.
    pub struct ProxyProfile {
        pub id: String,
        /// Unique among the profiles, compared without case.
        pub name: String,
        pub scheme: ProxyScheme,
        /// Host name or IP address, without scheme, port or credentials.
        pub host: String,
        pub port: u16,
        /// Not supported with `socks5` (Chrome cannot authenticate there).
        pub username: Option<String>,
        /// Read-only: whether a password for it is stored in the keychain.
        pub has_password: bool,
    }

    /// What `save_proxy_profile` takes: a new profile, or with `id` the new
    /// values of an existing one.
    pub struct ProxyProfileDraft {
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub id: Option<String>,
        pub name: String,
        pub scheme: ProxyScheme,
        pub host: String,
        pub port: u16,
        pub username: Option<String>,
    }

    /// The one proxy a scan's browser and its request for the site's ads.txt
    /// go through. The exchanges' sellers.json are not fetched through it.
    #[serde(tag = "kind")]
    pub enum ProxyChoice {
        /// Connect directly, ignoring the system's proxy.
        None,
        /// Whatever the operating system is configured with.
        System,
        /// One of `Settings.proxies`.
        #[serde(rename_all = "camelCase")]
        Profile {
            id: String,
            /// Name and host of the profile when the scan started, filled in
            /// by the backend, so a stored scan still reads after the
            /// profile is deleted. Ignored on input.
            #[ts(optional)]
            #[serde(default, skip_serializing_if = "Option::is_none")]
            name: Option<String>,
            #[ts(optional)]
            #[serde(default, skip_serializing_if = "Option::is_none")]
            host: Option<String>,
        },
        /// A residential exit that Browserbase provides, best effort, in a
        /// country and optionally a state and city. Only a scan whose browser
        /// runs on Browserbase can use it.
        #[serde(rename_all = "camelCase")]
        BrowserbaseLocation {
            /// ISO 3166-1 alpha-2, upper case: "BE".
            country: String,
            /// US state code, upper case: "NY".
            #[ts(optional)]
            #[serde(default, skip_serializing_if = "Option::is_none")]
            state: Option<String>,
            /// City name in upper case with underscores, as Browserbase
            /// takes it: "NEW_YORK".
            #[ts(optional)]
            #[serde(default, skip_serializing_if = "Option::is_none")]
            city: Option<String>,
        },
    }

    /// Where a scan's browser can run.
    #[derive(Copy, Eq, Hash)]
    pub enum BrowserKind {
        /// Chrome on this machine.
        Local,
        /// A browser in Browserbase's cloud, driven over a WebSocket.
        Browserbase,
    }

    /// The cloud regions Browserbase starts browsers in. Serialized as
    /// Browserbase names them.
    #[derive(Copy, Eq, Hash)]
    pub enum BrowserbaseRegion {
        #[serde(rename = "us-west-2")]
        UsWest2,
        #[serde(rename = "us-east-1")]
        UsEast1,
        #[serde(rename = "eu-central-1")]
        EuCentral1,
        #[serde(rename = "ap-southeast-1")]
        ApSoutheast1,
    }

    /// Where the browser of one scan runs.
    #[serde(tag = "kind")]
    pub enum BrowserChoice {
        Local,
        #[serde(rename_all = "camelCase")]
        Browserbase {
            /// Absent: `Settings.browserbase.region`.
            #[ts(optional)]
            #[serde(default, skip_serializing_if = "Option::is_none")]
            region: Option<BrowserbaseRegion>,
        },
    }

    /// The Browserbase part of the settings. The API key and the project id
    /// are never part of it: they live in the OS keychain.
    pub struct BrowserbaseSettings {
        /// The region a Browserbase scan runs in unless its options say
        /// otherwise.
        pub region: BrowserbaseRegion,
        /// Where a Browserbase scan leaves from unless its options say
        /// otherwise: `none` (the cloud region), one of `Settings.proxies`,
        /// or a Browserbase location. Never `system`.
        pub default_proxy: ProxyChoice,
        /// Whether Browserbase scans may keep a browser profile (one
        /// Browserbase context per device). Off unless the user turns it on,
        /// whatever `Settings.persistent_profile` says.
        pub persistent_profile: bool,
        /// Read-only: an API key and a project id are stored.
        pub connected: bool,
        /// Read-only: name of the stored project, as Browserbase reported it
        /// when the credentials were saved or last tested.
        pub project_name: Option<String>,
        /// Read-only: browsers the project may run at once, as last reported.
        pub concurrency: Option<u32>,
    }

    /// Partial update of [`BrowserbaseSettings`]; omitted fields keep their
    /// value.
    #[derive(Default)]
    pub struct BrowserbaseSettingsPatch {
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub region: Option<BrowserbaseRegion>,
        /// `system` is stored as `none`: a cloud browser has no use for this
        /// machine's proxy.
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub default_proxy: Option<ProxyChoice>,
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub persistent_profile: Option<bool>,
    }

    /// User preferences. Persisted locally; API keys and the proxy password
    /// are never part of this struct, they live in the OS keychain and are
    /// only reported via `has_api_key`, `has_typesafe_key` and each proxy
    /// profile's `has_password`.
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
        /// exchanges its ads.txt names. Contacts those third-party hosts
        /// (at most twelve per scan, never for a page on this machine); on
        /// unless the user turns it off.
        pub check_sellers_json: bool,
        /// What a new scan does on the page unless its options say otherwise.
        pub default_visitor: VisitorBehavior,
        /// Further pages a browse visit follows links to, 0-5.
        pub default_extra_pages: u32,
        /// Keep one browser profile (cookies, consent, cache) between scans
        /// instead of a throwaway one per scan.
        pub persistent_profile: bool,
        /// Saved proxies, by name. Changed through `save_proxy_profile` and
        /// `delete_proxy_profile`, not through `SettingsPatch`.
        pub proxies: Vec<ProxyProfile>,
        /// The proxy a scan uses unless its options say otherwise.
        pub default_proxy: ProxyChoice,
        /// Keep each new scan's network data (requests, headers, scripts,
        /// documents, JSON and bid payloads) in a local archive.
        pub keep_network_data: bool,
        /// Together all archives stay under this many bytes; the oldest
        /// scans lose their network data first.
        pub archive_limit_bytes: f64,
        /// Store cookie and authorization header values as sent instead of
        /// their lengths. Applies to new scans.
        pub store_full_headers: bool,
        /// Where a scan's browser runs unless its options say otherwise.
        /// `browserbase` only while Browserbase credentials are stored.
        pub default_browser: BrowserKind,
        pub browserbase: BrowserbaseSettings,
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
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub default_visitor: Option<VisitorBehavior>,
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub default_extra_pages: Option<u32>,
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub persistent_profile: Option<bool>,
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub default_proxy: Option<ProxyChoice>,
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub keep_network_data: Option<bool>,
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub archive_limit_bytes: Option<f64>,
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub store_full_headers: Option<bool>,
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub default_browser: Option<BrowserKind>,
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub browserbase: Option<BrowserbaseSettingsPatch>,
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

    /// The browser profile that persistent-profile scans share. Both device
    /// profiles together.
    pub struct ScanProfileInfo {
        pub exists: bool,
        pub size_bytes: f64,
        /// Unix epoch ms of the last scan that used it.
        pub last_used_at: Option<f64>,
        /// A scan is running in it right now; it cannot be cleared.
        pub in_use: bool,
    }

    /// One Browserbase project of an API key. The id is an identifier, not
    /// a secret.
    pub struct BrowserbaseProject {
        pub id: String,
        pub name: String,
        /// Browsers the project may run at once.
        pub concurrency: u32,
    }

    /// What `set_browserbase_credentials` answers.
    pub struct BrowserbaseCredentialsStatus {
        /// The key was accepted, a project is settled, and both are stored.
        pub valid: bool,
        /// Human-readable reason when `valid` is false.
        pub message: Option<String>,
        /// The key's projects when it has several and none was named: the
        /// caller picks one and sends the credentials again with its id.
        /// Empty otherwise.
        pub projects: Vec<BrowserbaseProject>,
    }

    /// What `test_browserbase` reads from the account. Costs nothing.
    pub struct BrowserbaseStatus {
        pub project_name: String,
        pub concurrency: u32,
        /// Running totals of the project since its start, all its users
        /// together. Browserbase reports no allowance and no period.
        pub browser_minutes: f64,
        pub proxy_bytes: f64,
        /// Browsers running in the project right now, whoever started them.
        pub running_sessions: u32,
    }

    /// What a page in a Browserbase browser can read, measured by
    /// `probe_browserbase` on a blank page of one short session.
    pub struct BrowserbaseProbe {
        pub region: BrowserbaseRegion,
        /// `Browser.getVersion` product, e.g. "Chrome/157.0.8103.79".
        pub browser_version: Option<String>,
        pub user_agent: Option<String>,
        /// `navigator.webdriver`; `None` when it could not be read.
        pub webdriver: Option<bool>,
        pub timezone: Option<String>,
        /// `navigator.language`.
        pub language: Option<String>,
        /// Median round trip of one command to the browser.
        pub round_trip_ms: Option<f64>,
        /// How long the session ran. Browserbase bills at least one minute.
        pub session_seconds: Option<f64>,
    }

    /// The browser profile Browserbase scans keep: one Browserbase context
    /// per device profile, both together.
    pub struct BrowserbaseProfileInfo {
        pub exists: bool,
        /// Unix epoch ms of the last scan that used it.
        pub last_used_at: Option<f64>,
        /// A scan is running in it right now; it cannot be cleared.
        pub in_use: bool,
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
            check_sellers_json: true,
            default_visitor: VisitorBehavior::Scroll,
            default_extra_pages: 2,
            persistent_profile: false,
            proxies: Vec::new(),
            default_proxy: ProxyChoice::System,
            keep_network_data: true,
            archive_limit_bytes: DEFAULT_ARCHIVE_LIMIT_BYTES,
            store_full_headers: false,
            default_browser: BrowserKind::Local,
            browserbase: BrowserbaseSettings::default(),
        }
    }
}

impl Default for BrowserbaseSettings {
    fn default() -> Self {
        Self {
            region: BrowserbaseRegion::UsWest2,
            default_proxy: ProxyChoice::None,
            persistent_profile: false,
            connected: false,
            project_name: None,
            concurrency: None,
        }
    }
}

impl BrowserbaseRegion {
    pub const ALL: [Self; 4] = [
        Self::UsWest2,
        Self::UsEast1,
        Self::EuCentral1,
        Self::ApSoutheast1,
    ];

    /// The name Browserbase's API takes.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UsWest2 => "us-west-2",
            Self::UsEast1 => "us-east-1",
            Self::EuCentral1 => "eu-central-1",
            Self::ApSoutheast1 => "ap-southeast-1",
        }
    }

    /// The timezone the scanner gives the browser when the visit leaves
    /// from this region without a Browserbase proxy: without it every
    /// region reports America/Los_Angeles.
    pub fn timezone(self) -> &'static str {
        match self {
            Self::UsWest2 => "America/Los_Angeles",
            Self::UsEast1 => "America/New_York",
            Self::EuCentral1 => "Europe/Berlin",
            Self::ApSoutheast1 => "Asia/Singapore",
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
        /// Superseded by `visitor`; kept for stored scans and old callers. The
        /// backend stores it as `visitor != none`.
        pub auto_scroll: bool,
        /// Try to accept the consent banner (CMP) so bidding is not blocked.
        pub accept_consent: bool,
        /// Run the two LLM phases. When false (or no API key) only the
        /// deterministic checks run.
        pub ai_enabled: bool,
        /// Absent: derived from `auto_scroll` (true is scroll, false is none).
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub visitor: Option<VisitorBehavior>,
        /// Further pages to visit by following links, 0-5. Browse only.
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub extra_pages: Option<u32>,
        /// Seed of the scripted visit; the backend fills it in when absent.
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub seed: Option<u32>,
        /// Absent: `Settings.headless` for none and scroll, false for browse.
        /// Manual is always headed.
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub headless: Option<bool>,
        /// Absent: `Settings.persistent_profile`.
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub persistent_profile: Option<bool>,
        /// Absent: `Settings.default_proxy`.
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub proxy: Option<ProxyChoice>,
        /// Cap on LLM spend for this scan in USD. Absent: `Settings.budget_usd`.
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub budget_usd: Option<f64>,
        /// Where the browser runs. Absent: `Settings.default_browser`, except
        /// that an address only this machine can reach is scanned locally.
        /// With Browserbase, an absent `proxy` is
        /// `Settings.browserbase.default_proxy` and an absent
        /// `persistent_profile` is `Settings.browserbase.persistent_profile`.
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub browser: Option<BrowserChoice>,
    }

    /// The saved configuration a scan was started from, as it was named then.
    pub struct ScanConfigurationRef {
        pub id: String,
        pub name: String,
    }

    /// A saved setup to scan one site with again and again.
    pub struct ScanConfiguration {
        pub id: String,
        /// Unique among the configurations, compared without case.
        pub name: String,
        /// The URL and everything else a scan of it starts with. Fields left
        /// absent follow the settings at the time of each run. Never holds a
        /// seed: every run gets its own.
        pub options: ScanOptions,
        pub created_at: f64,
        pub updated_at: f64,
        /// The last scan started from it, if that scan still exists.
        pub last_scan_id: Option<String>,
        pub last_run_at: Option<f64>,
    }

    /// What `save_scan_configuration` takes: a new configuration, or with
    /// `id` the new name and options of an existing one.
    pub struct ScanConfigurationDraft {
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub id: Option<String>,
        pub name: String,
        pub options: ScanOptions,
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
        /// The saved configuration the scan was started from, if any.
        #[serde(default)]
        pub configuration: Option<ScanConfigurationRef>,
        /// The proxy the scan runs through. `None` for scans stored before
        /// it was recorded.
        #[serde(default)]
        pub proxy: Option<ProxyUse>,
        /// What the scan used of a browser that is billed by use. `None`
        /// for local scans, and until the remote browser has ended.
        #[serde(default)]
        pub remote: Option<RemoteUsage>,
    }

    /// Browserbase usage of one scan. Not part of the LLM budget.
    pub struct RemoteUsage {
        pub region: BrowserbaseRegion,
        /// From the browser's start to its end. Billed per started minute.
        pub session_seconds: Option<f64>,
        /// Traffic through a proxy, as Browserbase counted it. It is read
        /// when the browser ends and once more half a minute later, because
        /// the count lags; `None` until the first read.
        pub proxy_bytes: Option<f64>,
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
        /// The page views of the visit, in order. Empty for scans stored
        /// before multi-page visits existed: those have one implicit page.
        #[serde(default)]
        pub pages: Vec<PageView>,
        /// `None` for scans stored before it was recorded, and for scans that
        /// ended before the capture did.
        #[serde(default)]
        pub environment: Option<ScanEnvironment>,
        /// The scan's network record. `None` reads as `notRecorded`: scans
        /// stored before the archive existed.
        #[serde(default)]
        pub network: Option<NetworkArchiveInfo>,
    }

    /// Counts are sums over all page views. `page_load_ms`,
    /// `time_to_first_auction_ms` and `time_to_first_ad_ms` are the entry
    /// page's.
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
        /// `None` without a winning bid, and when the winning bids are in
        /// more than one currency: such prices cannot be averaged.
        pub avg_winning_cpm: Option<f64>,
        /// 0 where `avg_winning_cpm` is `None`.
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
// The visit: page views and the environment the pages saw
// ---------------------------------------------------------------------------

contract! {
    /// How the visit got to a page view.
    #[derive(Copy, Eq, Hash)]
    pub enum PageReach {
        /// The URL the scan was started with.
        Entry,
        /// The scanner followed a link.
        LinkClick,
        /// The page changed its URL without loading a new document.
        RouteChange,
        /// The person browsing navigated (manual visits).
        UserNavigation,
        Redirect,
        Reload,
    }

    /// The link the scanner chose to follow, and why.
    pub struct FollowedLink {
        /// Link text, at most 80 characters.
        pub text: String,
        /// Origin and path; query values are removed.
        pub href: String,
        /// e.g. "content link in <article>, article-like path; best of 5".
        pub reason: String,
    }

    #[derive(Default)]
    pub struct PageMetrics {
        pub auction_count: u32,
        pub bid_request_count: u32,
        pub bid_response_count: u32,
        pub slot_count: u32,
        pub empty_slot_count: u32,
        /// Slots that rendered a creative / slots that requested one, 0-1.
        pub fill_rate: f64,
        /// ms since this page view started.
        pub time_to_first_auction_ms: Option<f64>,
        pub time_to_first_ad_ms: Option<f64>,
    }

    /// One page of the visit. Every `*_at_ms` of an auction, slot or script
    /// counts from the start of the page view it belongs to.
    pub struct PageView {
        /// Position in `ScanDetail.pages`; what the `page` fields refer to.
        pub index: u32,
        pub url: String,
        pub title: Option<String>,
        /// ms since the scan's navigation start (the entry page starts at 0).
        pub started_at_ms: f64,
        pub dwell_ms: f64,
        pub reached_by: PageReach,
        pub link: Option<FollowedLink>,
        /// `None` for route changes: no document was loaded.
        pub load_ms: Option<f64>,
        /// 0-1.
        pub max_scroll_depth: f64,
        pub metrics: PageMetrics,
    }

    #[derive(Copy, Eq, Hash)]
    pub enum DetectorKind {
        /// Ad verification that measures invalid traffic.
        InvalidTrafficMeasurement,
        /// Bot protection or a challenge in front of the site.
        BotProtection,
    }

    /// A vendor on the page that is known to tell automated visits apart.
    pub struct AutomationDetector {
        pub vendor: String,
        pub kind: DetectorKind,
        /// The requested host that identified it.
        pub host: String,
    }

    #[derive(Copy, Eq, Hash)]
    pub enum ProxyKind {
        None,
        System,
        Profile,
        /// A residential exit of Browserbase.
        BrowserbaseLocation,
    }

    /// The proxy a scan ran through. Never carries credentials.
    pub struct ProxyUse {
        pub kind: ProxyKind,
        /// Name of the proxy profile for `profile`; the requested place for
        /// `browserbaseLocation`, e.g. "BE" or "US, NY, NEW_YORK".
        pub name: Option<String>,
        /// Host of the proxy profile, for `profile`.
        pub host: Option<String>,
    }

    /// How a Browserbase browser reached the web.
    #[derive(Copy, Eq, Hash)]
    pub enum BrowserbaseProxyKind {
        /// Straight from the cloud region.
        None,
        /// Through one of the user's own proxies.
        External,
        /// Through a residential exit of Browserbase.
        Browserbase,
    }

    /// The session settings the scanner sends to Browserbase with every
    /// browser it starts, stated explicitly so that a changed default at
    /// Browserbase cannot change a scan.
    #[derive(Copy, Eq)]
    pub struct BrowserbaseSessionSettings {
        pub solve_captchas: bool,
        pub record_session: bool,
        pub log_session: bool,
        pub block_ads: bool,
        pub keep_alive: bool,
    }

    /// A visit that ran in a Browserbase browser: what was asked of
    /// Browserbase and what the browser and the service reported.
    pub struct BrowserbaseVisit {
        pub region: BrowserbaseRegion,
        /// Browserbase's id of the session. An identifier, not a credential.
        pub session_id: String,
        /// `Browser.getVersion` product, e.g. "Chrome/157.0.8103.79".
        pub browser_version: Option<String>,
        pub proxy_kind: BrowserbaseProxyKind,
        /// The country asked for, with a Browserbase proxy. Best effort on
        /// Browserbase's side.
        pub proxy_country: Option<String>,
        pub settings: BrowserbaseSessionSettings,
        /// The timezone the page reported before the scanner changed
        /// anything.
        pub original_timezone: Option<String>,
        /// The timezone the scanner set to match the region. `None` when it
        /// set none: with a Browserbase proxy the service aligns it itself.
        pub applied_timezone: Option<String>,
        /// `navigator.language`, as reported; the scanner never changes it.
        pub language: Option<String>,
        /// Round trip of one command between the app and the browser. Every
        /// frame or request the scanner holds for a moment waits this long.
        pub round_trip_ms: Option<f64>,
        pub session_seconds: Option<f64>,
        pub proxy_bytes: Option<f64>,
    }

    /// Where the scan's browser ran.
    #[derive(Default)]
    #[serde(tag = "kind")]
    pub enum BrowserUse {
        /// Chrome on this machine. Also what scans stored before the
        /// browser was recorded read as.
        #[default]
        Local,
        Browserbase(BrowserbaseVisit),
    }

    /// What the scanned pages could tell about the visit, reported as it
    /// was. The scanner adds no disguise of its own; a Browserbase browser
    /// does not tell the page it is automated, and `webdriver` says so.
    pub struct ScanEnvironment {
        pub visitor: VisitorBehavior,
        pub seed: u32,
        pub headless: bool,
        pub persistent_profile: bool,
        /// `navigator.webdriver` as the page saw it; `None` when it could not
        /// be read.
        pub webdriver: Option<bool>,
        pub user_agent_overridden: bool,
        pub viewport_overridden: bool,
        pub emulated_mobile: bool,
        pub detectors: Vec<AutomationDetector>,
        pub proxy: ProxyUse,
        #[serde(default)]
        pub browser: BrowserUse,
    }
}

impl BrowserbaseSessionSettings {
    /// The owner's decision of 2026-11-13: Browserbase as it is, with its
    /// defaults stated.
    pub const STANDARD: Self = Self {
        solve_captchas: true,
        record_session: true,
        log_session: true,
        block_ads: false,
        keep_alive: false,
    };
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
        /// Page views the finding was seen on; empty when it is not specific
        /// to a page.
        #[serde(default)]
        pub pages: Vec<u32>,
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
        /// Last line of the excerpt, inclusive.
        #[serde(default)]
        pub end_line: Option<u32>,
        /// The scan's script the excerpt came from (`ScriptRecord.id`).
        #[serde(default)]
        pub script_id: Option<String>,
        /// The request this evidence is about (`NetworkRow.id`).
        #[serde(default)]
        pub request_id: Option<String>,
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
        /// Page view the auction started on.
        #[serde(default)]
        pub page: u32,
        /// ms since the start of that page view. An auction that outlives a
        /// route change keeps counting from it.
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
        /// ms since the start of the auction's page view when the request
        /// went out.
        pub requested_at_ms: Option<f64>,
        /// Round-trip latency.
        pub response_time_ms: Option<f64>,
        pub deal_id: Option<String>,
        /// The bid's request on the network. `None` without an archive.
        #[serde(default)]
        pub exchange: Option<BidExchange>,
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

    /// One ad request of a slot. The first one is also `requested_at_ms`.
    pub struct SlotRequest {
        /// ms since the start of the slot's page view.
        pub at_ms: f64,
        pub had_prebid_targeting: bool,
        /// `hb_adid` on the slot when the request went out.
        pub prebid_ad_id: Option<String>,
        /// The ad-server call that carried it (`NetworkRow.id`), when it
        /// could be told.
        #[serde(default)]
        pub request_id: Option<String>,
    }

    pub struct AdSlot {
        /// Page view the slot belongs to. The same container on two page
        /// views is two slots.
        #[serde(default)]
        pub page: u32,
        /// DOM element id of the slot container.
        pub element_id: String,
        /// GPT ad unit path, e.g. "/1234/site/top".
        pub ad_unit_path: Option<String>,
        pub sizes: Vec<String>,
        pub outcome: SlotOutcome,
        pub rendered_size: Option<String>,
        /// All timings are ms since the start of the slot's page view.
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
        /// Every ad request of the slot in order, refreshes included.
        #[serde(default)]
        pub requests: Vec<SlotRequest>,
    }
}

// ---------------------------------------------------------------------------
// Bidder diagnostics (derived results; ads.txt and sellers.json are never
// persisted)
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
        /// ms since the scan's navigation start when the CMP first reported a
        /// decided consent state.
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
        /// Page views the outcome was seen on; empty when it is not specific
        /// to a page.
        #[serde(default)]
        pub pages: Vec<u32>,
        /// The requests the outcome was read from (`NetworkRow.id`).
        #[serde(default)]
        pub request_ids: Vec<String>,
    }
}

// ---------------------------------------------------------------------------
// Scripts (metadata; the bodies are in the scan's archive)
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
        /// ms since the start of the first page view that loaded it when the
        /// response finished.
        pub loaded_at_ms: Option<f64>,
        pub first_party: bool,
        /// Page views that loaded the script, ascending. Empty for scans
        /// stored before multi-page visits existed.
        #[serde(default)]
        pub pages: Vec<u32>,
        /// URL of the script or document that loaded it, when known.
        #[serde(default)]
        pub initiator: Option<String>,
        /// The request that delivered it (`NetworkRow.id`). `None` for
        /// inline scripts and without an archive.
        #[serde(default)]
        pub request_id: Option<String>,
        /// Full hex sha-256 of the body: the key of its source in the
        /// archive. `None` for scans stored before sources were kept.
        #[serde(default)]
        pub sha256: Option<String>,
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
        /// Running when the scan was cancelled.
        Cancelled,
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
        /// What the visitor did or met during the capture: a page view, an
        /// action, a link choice, something blocked, the environment, or
        /// what a Browserbase browser reported (`activity: "browser"`).
        /// `data` is `{ activity, page, detail }`.
        Visitor,
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
        /// Only scans started from this saved configuration.
        #[ts(optional)]
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub configuration_id: Option<String>,
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
        /// The scan profile is in use by a running scan.
        ProfileBusy,
        /// Browserbase is not set up, or refused the API key or the project.
        BrowserbaseAuth,
        /// Browserbase is at its limit of browsers at once or of new
        /// browsers per minute, and waiting did not help.
        BrowserbaseLimit,
        /// Browserbase could not start the browser, refused the request, or
        /// the connection to the browser was lost.
        BrowserbaseFailed,
    }
}

// ---------------------------------------------------------------------------
// Builders for tests
// ---------------------------------------------------------------------------

impl ScanOptions {
    /// A desktop scroll scan of `url` without AI: the values a test changes
    /// from.
    pub fn for_url(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            device: DeviceProfile::Desktop,
            observe_seconds: 10,
            auto_scroll: true,
            accept_consent: true,
            ai_enabled: false,
            visitor: None,
            extra_pages: None,
            seed: None,
            headless: None,
            persistent_profile: None,
            proxy: None,
            budget_usd: None,
            browser: None,
        }
    }
}
