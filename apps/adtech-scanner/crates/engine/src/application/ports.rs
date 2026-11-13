//! Everything the application needs from the outside world.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use adscan_model::{
    AdSlot, ApiProvider, ArchiveStorage, Auction, BidderDiagnostics, BrowserInfo,
    BrowserbaseCredentials, BrowserbaseProbe, BrowserbaseProject, BrowserbaseRegion, Capture,
    CaptureActivity, CaptureProxy, CaptureRequest, DashboardStats, DeviceProfile, Finding,
    LogEntry, LogSearchHit, NetworkArchiveInfo, NetworkAvailability, NetworkSink, PageView,
    PipelineStep, RemoteSessionEvent, ScanConfiguration, ScanDetail, ScanEnvironment, ScanEvent,
    ScanList, ScanListQuery, ScanMetrics, ScanOptions, ScanProfileInfo, ScanSummary, ScriptRecord,
    SellersJsonCacheInfo, Settings,
};
use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

use super::judge::{JudgeRequest, JudgeResponse};
use super::llm::{LlmError, LlmRequest, LlmResponse};
use crate::domain::archive::Location;
pub use crate::domain::archive::index::{BodyIndexRow, RequestEndKind, RequestIndexRow};
pub use crate::domain::network::exchange::RequestAdLink;

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("{0}")]
pub struct RepositoryError(pub String);

pub type RepoResult<T> = Result<T, RepositoryError>;

#[async_trait]
pub trait ScanRepository: Send + Sync {
    async fn create(&self, summary: &ScanSummary, options: &ScanOptions) -> RepoResult<()>;
    async fn save_summary(&self, summary: &ScanSummary) -> RepoResult<()>;
    async fn save_telemetry(
        &self,
        scan_id: &str,
        metrics: &ScanMetrics,
        pages: &[PageView],
        auctions: &[Auction],
        slots: &[AdSlot],
    ) -> RepoResult<()>;
    async fn save_environment(
        &self,
        scan_id: &str,
        environment: &ScanEnvironment,
    ) -> RepoResult<()>;
    async fn save_scripts(&self, scan_id: &str, scripts: &[ScriptRecord]) -> RepoResult<()>;
    async fn save_diagnostics(
        &self,
        scan_id: &str,
        diagnostics: &BidderDiagnostics,
    ) -> RepoResult<()>;
    /// Replaces the scan's findings with `findings`, in order.
    async fn save_findings(&self, scan_id: &str, findings: &[Finding]) -> RepoResult<()>;
    async fn save_overview(&self, scan_id: &str, overview: &str) -> RepoResult<()>;
    async fn save_step(&self, step: &PipelineStep) -> RepoResult<()>;
    async fn summary(&self, scan_id: &str) -> RepoResult<Option<ScanSummary>>;
    async fn detail(&self, scan_id: &str) -> RepoResult<Option<ScanDetail>>;
    /// Newest first.
    async fn list(&self, query: &ScanListQuery) -> RepoResult<ScanList>;
    /// Removes the scan with everything that belongs to it. False when unknown.
    async fn delete(&self, scan_id: &str) -> RepoResult<bool>;
    async fn delete_all(&self) -> RepoResult<()>;
    async fn dashboard_stats(&self) -> RepoResult<DashboardStats>;
    /// Marks scans left queued or running by a previous process as failed.
    async fn fail_interrupted(&self, message: &str, now_ms: f64) -> RepoResult<u32>;
}

/// Saved scan configurations. Uniqueness of names is the use case's rule,
/// not the store's.
#[async_trait]
pub trait ScanConfigurationRepository: Send + Sync {
    /// By name.
    async fn list(&self) -> RepoResult<Vec<ScanConfiguration>>;
    async fn get(&self, id: &str) -> RepoResult<Option<ScanConfiguration>>;
    /// Inserts, or replaces the name, options and `updated_at` of the stored
    /// configuration with that id. The last-run fields are left alone.
    async fn save(&self, configuration: &ScanConfiguration) -> RepoResult<()>;
    /// False when unknown. Scans started from it stay.
    async fn delete(&self, id: &str) -> RepoResult<bool>;
    async fn record_run(&self, id: &str, scan_id: &str, at_ms: f64) -> RepoResult<()>;
}

#[async_trait]
pub trait LogRepository: Send + Sync {
    async fn append(&self, entry: &LogEntry) -> RepoResult<()>;
    /// Oldest first.
    async fn list(&self, scan_id: &str, step_id: Option<&str>) -> RepoResult<Vec<LogEntry>>;
    /// Full-text search. Any user input is a valid query.
    async fn search(&self, query: &str, limit: u32) -> RepoResult<Vec<LogSearchHit>>;
}

#[async_trait]
pub trait SettingsRepository: Send + Sync {
    /// `None` until settings were saved for the first time.
    async fn load(&self) -> RepoResult<Option<Settings>>;
    async fn save(&self, settings: &Settings) -> RepoResult<()>;
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum SecretError {
    /// The user, or the system on their behalf, refused this app the secret.
    #[error("Keychain access was denied")]
    Denied,
    #[error("The keychain is unavailable: {0}")]
    Unavailable(String),
    #[error("The keychain operation failed: {0}")]
    Failed(String),
}

/// One secret the app keeps.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SecretName {
    ApiKey(ApiProvider),
    /// By proxy profile id.
    ProxyPassword(String),
    BrowserbaseApiKey,
    /// An identifier, kept with the key it belongs to.
    BrowserbaseProjectId,
}

/// Where the secrets live: one API key per provider, one password per
/// proxy profile, and the Browserbase key with its project id.
///
/// Reading a secret can make the system ask the user for permission, so a
/// failure is never the same as "nothing stored": callers must report it.
#[async_trait]
pub trait SecretStore: Send + Sync {
    /// Which of `candidates` exist. Answered from the app's own record and
    /// never by reading a secret, so it is safe on every settings fetch.
    async fn stored_among(&self, candidates: Vec<SecretName>) -> RepoResult<HashSet<SecretName>>;
    async fn api_key(&self, provider: ApiProvider) -> Result<Option<String>, SecretError>;
    async fn store_api_key(&self, provider: ApiProvider, key: &str) -> Result<(), SecretError>;
    async fn clear_api_key(&self, provider: ApiProvider) -> Result<(), SecretError>;
    async fn proxy_password(&self, proxy_id: &str) -> Result<Option<String>, SecretError>;
    async fn store_proxy_password(&self, proxy_id: &str, password: &str)
    -> Result<(), SecretError>;
    async fn clear_proxy_password(&self, proxy_id: &str) -> Result<(), SecretError>;
    /// `None` unless both the key and the project id are stored.
    async fn browserbase_credentials(&self) -> Result<Option<BrowserbaseCredentials>, SecretError>;
    /// Key and project id are stored together or not at all.
    async fn store_browserbase_credentials(
        &self,
        credentials: &BrowserbaseCredentials,
    ) -> Result<(), SecretError>;
    async fn clear_browserbase_credentials(&self) -> Result<(), SecretError>;
}

/// The record of which secrets exist, kept outside the keychain so that
/// showing it costs the user no permission prompt. Names only, never values.
#[async_trait]
pub trait SecretPresenceRepository: Send + Sync {
    async fn load_presence(&self) -> RepoResult<HashSet<SecretName>>;
    async fn save_presence(&self, names: &HashSet<SecretName>) -> RepoResult<()>;
}

#[async_trait]
pub trait LlmClient: Send + Sync {
    async fn complete(
        &self,
        api_key: &str,
        request: &LlmRequest,
        cancel: &CancellationToken,
    ) -> Result<LlmResponse, LlmError>;

    /// Cheap check that the key is accepted.
    async fn validate_key(&self, api_key: &str) -> Result<(), LlmError>;
}

/// A decision model: probabilities for yes/no questions about a state.
#[async_trait]
pub trait JudgeClient: Send + Sync {
    async fn judge(
        &self,
        api_key: &str,
        request: &JudgeRequest,
        cancel: &CancellationToken,
    ) -> Result<JudgeResponse, LlmError>;

    /// Cheap check that the key is accepted.
    async fn validate_key(&self, api_key: &str) -> Result<(), LlmError>;
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum CaptureError {
    #[error("No Chrome or Chromium installation was found")]
    BrowserNotFound,
    #[error("The browser failed to start: {0}")]
    Launch(String),
    #[error("The page could not be loaded: {0}")]
    Navigation(String),
    #[error("The scan was cancelled")]
    Cancelled,
    #[error("The browser profile is in use by a running scan")]
    ProfileBusy,
    /// Browserbase refused the key or the project.
    #[error("{0}")]
    RemoteAuth(String),
    /// Browserbase stayed at its limit for as long as the capture waited.
    #[error("{0}")]
    RemoteLimit(String),
    /// Browserbase could not start the browser, or the connection was lost.
    #[error("{0}")]
    Remote(String),
    #[error("{0}")]
    Other(String),
}

pub type ActivityFn = Arc<dyn Fn(CaptureActivity) + Send + Sync>;
pub type RemoteSessionFn = Arc<dyn Fn(RemoteSessionEvent) + Send + Sync>;

/// What a capture in a Browserbase browser needs besides its request.
#[derive(Clone)]
pub struct BrowserbaseAccess {
    /// Kept out of the request so they cannot be logged or serialised with
    /// it.
    pub credentials: BrowserbaseCredentials,
    /// Told when the session starts, when a person can take it over, and
    /// when it has ended.
    pub on_session: RemoteSessionFn,
}

/// Everything about one capture that is not part of the request: how to
/// stop it, where it reports, and the one secret it may need.
#[derive(Clone)]
pub struct CaptureSession {
    /// Stops the capture and discards what it recorded.
    pub cancel: CancellationToken,
    /// Ends a manual visit and keeps what it recorded.
    pub finish: CancellationToken,
    pub on_activity: ActivityFn,
    /// Password of the request's proxy, when one is stored. Kept out of the
    /// request so it cannot be logged or serialised with it.
    pub proxy_password: Option<String>,
    /// Where the capture hands over every request while it runs. `None`
    /// when the scan keeps no network data.
    pub network: Option<Arc<dyn NetworkSink>>,
    /// Set when the request's browser runs on Browserbase.
    pub browserbase: Option<BrowserbaseAccess>,
}

#[async_trait]
pub trait CaptureSource: Send + Sync {
    /// `explicit` is the user's configured browser path, if any.
    async fn locate_browser(&self, explicit: Option<String>) -> Option<BrowserInfo>;

    async fn capture(
        &self,
        request: CaptureRequest,
        session: CaptureSession,
    ) -> Result<Capture, CaptureError>;

    /// The browser profile that persistent-profile scans share.
    async fn scan_profile_info(&self) -> ScanProfileInfo;

    /// Removes that profile. [`CaptureError::ProfileBusy`] while a browser
    /// runs in it.
    async fn clear_scan_profile(&self) -> Result<(), CaptureError>;
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum BrowserbaseError {
    /// The key or the project was refused.
    #[error("{0}")]
    Auth(String),
    /// Too many browsers at once, or too many new ones per minute.
    #[error("{0}")]
    Limit(String),
    #[error("{0}")]
    Failed(String),
}

/// Running totals of a Browserbase project, all its users together.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProjectUsage {
    pub browser_minutes: f64,
    pub proxy_bytes: f64,
}

/// What Browserbase says a session used.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RemoteSessionUsage {
    pub ended: bool,
    pub session_seconds: Option<f64>,
    /// Lags behind the end of the session by up to half a minute.
    pub proxy_bytes: Option<f64>,
}

/// Browserbase's own API, apart from the browser a capture drives: the
/// account, the sessions this installation left behind, the contexts that
/// hold a kept profile, and the live view.
#[async_trait]
pub trait BrowserbaseApi: Send + Sync {
    /// The projects of a key. Costs nothing; this is how a key is checked.
    async fn projects(&self, api_key: &str) -> Result<Vec<BrowserbaseProject>, BrowserbaseError>;
    async fn project(
        &self,
        credentials: &BrowserbaseCredentials,
    ) -> Result<BrowserbaseProject, BrowserbaseError>;
    async fn project_usage(
        &self,
        credentials: &BrowserbaseCredentials,
    ) -> Result<ProjectUsage, BrowserbaseError>;
    /// Browsers running in the project, whoever started them.
    async fn running_session_count(
        &self,
        credentials: &BrowserbaseCredentials,
    ) -> Result<u32, BrowserbaseError>;
    /// Ids of the running sessions that carry this installation's id in
    /// their metadata. Never a session of anyone else: the project may be
    /// shared.
    async fn running_sessions_of(
        &self,
        credentials: &BrowserbaseCredentials,
        install_id: &str,
    ) -> Result<Vec<String>, BrowserbaseError>;
    async fn session_usage(
        &self,
        credentials: &BrowserbaseCredentials,
        session_id: &str,
    ) -> Result<RemoteSessionUsage, BrowserbaseError>;
    /// Ends the session. One that has ended already is not an error.
    async fn release_session(
        &self,
        credentials: &BrowserbaseCredentials,
        session_id: &str,
    ) -> Result<(), BrowserbaseError>;
    /// A new, empty context; returns its id.
    async fn create_context(
        &self,
        credentials: &BrowserbaseCredentials,
    ) -> Result<String, BrowserbaseError>;
    /// A context that is already gone is not an error.
    async fn delete_context(
        &self,
        credentials: &BrowserbaseCredentials,
        context_id: &str,
    ) -> Result<(), BrowserbaseError>;
    /// Starts one browser, reads what a blank page in it can see, and
    /// releases it. Billed as one browser minute.
    async fn probe(
        &self,
        credentials: &BrowserbaseCredentials,
        region: BrowserbaseRegion,
        install_id: &str,
    ) -> Result<BrowserbaseProbe, BrowserbaseError>;
    /// Shows the session's live view in the user's own browser. Its address
    /// is a credential: it is fetched, opened and forgotten here, and never
    /// returned, logged or put in an error.
    async fn open_live_view(
        &self,
        credentials: &BrowserbaseCredentials,
        session_id: &str,
    ) -> Result<(), BrowserbaseError>;
}

/// A Browserbase context that holds the kept browser profile of a device.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredContext {
    pub device: DeviceProfile,
    pub context_id: String,
    pub last_used_at: Option<f64>,
}

/// A session this installation started and has not seen end.
#[derive(Debug, Clone, PartialEq)]
pub struct OpenRemoteSession {
    pub session_id: String,
    pub created_at: f64,
}

/// The app's own record of its Browserbase use. Identifiers only.
#[async_trait]
pub trait BrowserbaseLedger: Send + Sync {
    /// This installation's id: what its sessions carry in their metadata.
    async fn install_id(&self) -> RepoResult<String>;
    async fn session_created(&self, session_id: &str, at_ms: f64) -> RepoResult<()>;
    async fn session_released(&self, session_id: &str, at_ms: f64) -> RepoResult<()>;
    /// Sessions without a recorded end, oldest first.
    async fn open_sessions(&self) -> RepoResult<Vec<OpenRemoteSession>>;
    /// Forgets the sessions whose end is recorded.
    async fn forget_released_sessions(&self) -> RepoResult<()>;
    async fn contexts(&self) -> RepoResult<Vec<StoredContext>>;
    /// Records the context of `device`, replacing an earlier one.
    async fn save_context(
        &self,
        device: DeviceProfile,
        context_id: &str,
        at_ms: f64,
    ) -> RepoResult<()>;
    async fn context_used(&self, device: DeviceProfile, at_ms: f64) -> RepoResult<()>;
    async fn forget_contexts(&self) -> RepoResult<()>;
}

/// A request for a public text file of a website (ads.txt, sellers.json).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicFileRequest {
    pub url: String,
    /// A larger body is an error, not a truncated file.
    pub max_bytes: usize,
    /// For the whole exchange, redirects included.
    pub timeout: Duration,
    /// `scheme://host:port` of the scanned page when it is served from this
    /// machine. It is the only origin that may be contacted although it is
    /// not public and not https.
    pub loopback_origin: Option<String>,
    /// The proxy the request leaves through: the scan's own for the scanned
    /// site's files, the system's for what is shared between scans. The
    /// loopback origin is always reached directly.
    pub proxy: FetchProxy,
    /// What an earlier answer said about its version. With it the server is
    /// asked for the file only if it changed, and answers 304 if it did not.
    pub known_version: Option<FileVersion>,
}

/// The validators of an HTTP answer, as the server stated them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileVersion {
    pub etag: Option<String>,
    pub last_modified: Option<String>,
}

impl FileVersion {
    pub fn is_stated(&self) -> bool {
        self.etag.is_some() || self.last_modified.is_some()
    }
}

/// The proxy of a public file request, with the secret it needs.
#[derive(Clone, PartialEq, Eq)]
pub struct FetchProxy {
    pub route: CaptureProxy,
    pub password: Option<String>,
}

/// Never prints the password.
impl std::fmt::Debug for FetchProxy {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FetchProxy")
            .field("route", &self.route)
            .field("has_password", &self.password.is_some())
            .finish()
    }
}

/// The final answer after redirects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicFile {
    /// 304 when the request named a `known_version` and it is still current.
    pub status: u16,
    pub content_type: Option<String>,
    /// In memory only; empty unless `status` is a success.
    pub body: Vec<u8>,
    /// The address that answered, after redirects.
    pub final_url: String,
    pub version: FileVersion,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FetchError {
    /// The address, or one it redirected to, must not be contacted.
    #[error("{0}")]
    Refused(String),
    #[error("the file is larger than {0} bytes")]
    TooLarge(usize),
    #[error("the request timed out")]
    TimedOut,
    #[error("{0}")]
    Network(String),
    #[error("The scan was cancelled")]
    Cancelled,
}

/// Reads public files from the open web: no cookies, no credentials, public
/// https addresses only.
#[async_trait]
pub trait PublicFileFetcher: Send + Sync {
    async fn fetch(
        &self,
        request: &PublicFileRequest,
        cancel: &CancellationToken,
    ) -> Result<PublicFile, FetchError>;
}

/// What is on record about one exchange's sellers.json.
#[derive(Debug, Clone, PartialEq)]
pub struct CachedSellersJson {
    /// When the file was downloaded, or last confirmed unchanged; epoch ms.
    pub fetched_at: f64,
    pub final_url: String,
    pub version: FileVersion,
    /// The file as it was received, or why there was none to read.
    pub content: Result<Vec<u8>, String>,
}

/// The sellers.json files of ad exchanges, kept between scans. A file is the
/// same whatever site is scanned, so one copy per exchange serves them all.
/// `domain` is the exchange's ad system domain; an implementation must not
/// trust it to be a safe file name.
#[async_trait]
pub trait SellersJsonCache: Send + Sync {
    /// The entry of `domain`. `None` when there is none, and when what is
    /// stored is damaged, which also removes it.
    async fn load(&self, domain: &str) -> Option<CachedSellersJson>;
    /// Replaces the entry of `domain`, all of it or nothing.
    async fn store(&self, domain: &str, entry: CachedSellersJson) -> Result<(), String>;
    /// The exchange confirmed that the stored file is still current. An
    /// error when the entry of `domain` holds no file.
    async fn confirm(&self, domain: &str, fetched_at: f64) -> Result<(), String>;
    async fn info(&self) -> SellersJsonCacheInfo;
    async fn clear(&self) -> Result<(), String>;
    /// Removes what an interrupted write left behind.
    async fn sweep(&self);
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PackError {
    #[error("the pack file {0} does not exist")]
    Missing(String),
    /// The range lies outside the file: a truncated or foreign pack.
    #[error("{0}")]
    OutOfRange(String),
    #[error("{0}")]
    Io(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackInfo {
    /// Name relative to the store, e.g. `scans/<scan id>.pack.zst`.
    pub file: String,
    pub bytes: u64,
}

/// Where pack files live. Shaped like the owner's data lake: a file is
/// written once by appending frames, then only read by location or deleted.
/// The store never looks inside a frame.
#[async_trait]
pub trait PackStore: Send + Sync {
    /// Starts a new, empty pack, replacing a file of that name.
    async fn create(&self, file: &str) -> Result<Box<dyn PackWriter>, PackError>;
    /// Exactly `location.length` bytes, or an error. Never a short read.
    async fn read(&self, location: &Location) -> Result<Vec<u8>, PackError>;
    /// A file that is already gone is not an error.
    async fn delete(&self, file: &str) -> Result<(), PackError>;
    async fn list(&self) -> Result<Vec<PackInfo>, PackError>;
}

#[async_trait]
pub trait PackWriter: Send {
    /// Appends one complete frame and says where it went.
    async fn append(&mut self, frame: Vec<u8>) -> Result<Location, PackError>;
    /// Returns once everything appended so far is durable.
    async fn sync(&mut self) -> Result<(), PackError>;
    /// Makes the pack durable and closes it; nothing is appended after this.
    async fn seal(self: Box<Self>) -> Result<PackInfo, PackError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveState {
    /// The scan is still capturing into it.
    Writing,
    Sealed,
    /// The pack was removed to stay under the limit; request rows remain.
    Evicted,
    /// The scan ran with "keep network data" off: nothing was stored.
    Disabled,
}

/// The bookkeeping row of one scan's archive.
#[derive(Debug, Clone, PartialEq)]
pub struct ArchiveRecord {
    pub scan_id: String,
    pub state: ArchiveState,
    /// `None` once evicted, and for a disabled archive.
    pub pack_file: Option<String>,
    pub pack_bytes: u64,
    pub request_count: u32,
    pub body_count: u32,
    pub headers_redacted: bool,
    /// Bodies were left out because of the scan cap or back-pressure.
    pub truncated: bool,
    pub created_at: f64,
}

impl ArchiveRecord {
    /// The archive as the report shows it.
    pub fn info(&self) -> NetworkArchiveInfo {
        NetworkArchiveInfo {
            availability: match self.state {
                ArchiveState::Writing | ArchiveState::Sealed => NetworkAvailability::Available,
                ArchiveState::Evicted => NetworkAvailability::Evicted,
                ArchiveState::Disabled => NetworkAvailability::Disabled,
            },
            request_count: self.request_count,
            body_count: self.body_count,
            stored_bytes: self.pack_bytes as f64,
            headers_redacted: self.headers_redacted,
            truncated: self.truncated,
        }
    }
}

/// Totals written when a scan's pack is sealed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArchiveTotals {
    pub pack_bytes: u64,
    pub truncated: bool,
}

/// The index of all scan archives: bookkeeping per scan, body ranges and
/// request rows.
#[async_trait]
pub trait ArchiveRepository: Send + Sync {
    /// Registers the archive a scan is about to write. `pack_file: None`
    /// records that the scan keeps no network data.
    async fn begin(
        &self,
        scan_id: &str,
        pack_file: Option<&str>,
        headers_redacted: bool,
        created_at: f64,
    ) -> RepoResult<()>;
    /// Adds rows whose frames are already durable in the pack.
    async fn append(
        &self,
        scan_id: &str,
        bodies: Vec<BodyIndexRow>,
        requests: Vec<RequestIndexRow>,
    ) -> RepoResult<()>;
    async fn seal(&self, scan_id: &str, totals: ArchiveTotals) -> RepoResult<()>;
    async fn record(&self, scan_id: &str) -> RepoResult<Option<ArchiveRecord>>;
    /// Rows ordered by page view, start time and `seq`. `pages` empty means
    /// every page view. Also returns how many rows the selection has.
    async fn rows(
        &self,
        scan_id: &str,
        pages: &[u32],
        offset: u32,
        limit: u32,
    ) -> RepoResult<(Vec<RequestIndexRow>, u32)>;
    async fn rows_by_seq(&self, scan_id: &str, seqs: &[u32]) -> RepoResult<Vec<RequestIndexRow>>;
    async fn body(&self, scan_id: &str, body_no: u32) -> RepoResult<Option<BodyIndexRow>>;
    async fn body_by_sha(
        &self,
        scan_id: &str,
        sha256: [u8; 32],
    ) -> RepoResult<Option<BodyIndexRow>>;
    async fn save_ad_links(&self, scan_id: &str, links: Vec<RequestAdLink>) -> RepoResult<()>;
    /// Removes the archive's bookkeeping and all its rows. False when the
    /// scan has none.
    async fn remove(&self, scan_id: &str) -> RepoResult<bool>;
    /// Forgets the pack: body ranges go, and the request rows too unless
    /// `keep_rows`.
    async fn mark_evicted(&self, scan_id: &str, keep_rows: bool) -> RepoResult<()>;
    /// Every archive, oldest first.
    async fn all(&self) -> RepoResult<Vec<ArchiveRecord>>;
    async fn storage(&self, limit_bytes: f64) -> RepoResult<ArchiveStorage>;
    /// Returns the space of deleted rows to the file system.
    async fn compact(&self) -> RepoResult<()>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormatMethod {
    /// Parsed and printed by a real JavaScript parser.
    Parser,
    /// The parser rejected the source; reflowed token by token.
    Reflow,
    /// Too large or too slow to format; split into lines as-is.
    Unformatted,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FormattedSource {
    pub lines: Vec<String>,
    pub method: FormatMethod,
}

/// Turns minified JavaScript into readable, line-addressable source.
#[async_trait]
pub trait SourceFormatter: Send + Sync {
    async fn format(&self, source: String) -> FormattedSource;
}

pub trait EventSink: Send + Sync {
    fn emit(&self, event: ScanEvent);
}

pub trait Clock: Send + Sync {
    /// Unix epoch milliseconds.
    fn now_ms(&self) -> f64;
}

pub trait IdGenerator: Send + Sync {
    fn new_id(&self) -> String;
}
