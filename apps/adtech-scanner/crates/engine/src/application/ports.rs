//! Everything the application needs from the outside world.

use std::sync::Arc;
use std::time::Duration;

use adscan_model::{
    AdSlot, ApiProvider, Auction, BidderDiagnostics, BrowserInfo, Capture, CaptureActivity,
    CaptureProxy, CaptureRequest, DashboardStats, Finding, LogEntry, LogSearchHit, PageView,
    PipelineStep, ScanConfiguration, ScanDetail, ScanEnvironment, ScanEvent, ScanList,
    ScanListQuery, ScanMetrics, ScanOptions, ScanProfileInfo, ScanSummary, ScriptRecord, Settings,
};
use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

use super::judge::{JudgeRequest, JudgeResponse};
use super::llm::{LlmError, LlmRequest, LlmResponse};

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
    #[error("The system keychain is not available: {0}")]
    Unavailable(String),
    #[error("The system keychain refused the operation: {0}")]
    Failed(String),
}

/// Where the secrets live: one API key per provider and one password per
/// proxy profile.
#[async_trait]
pub trait SecretStore: Send + Sync {
    async fn api_key(&self, provider: ApiProvider) -> Result<Option<String>, SecretError>;
    async fn store_api_key(&self, provider: ApiProvider, key: &str) -> Result<(), SecretError>;
    async fn clear_api_key(&self, provider: ApiProvider) -> Result<(), SecretError>;
    async fn proxy_password(&self, proxy_id: &str) -> Result<Option<String>, SecretError>;
    async fn store_proxy_password(&self, proxy_id: &str, password: &str)
    -> Result<(), SecretError>;
    async fn clear_proxy_password(&self, proxy_id: &str) -> Result<(), SecretError>;
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
    #[error("The scan profile is in use by a running scan")]
    ProfileBusy,
    #[error("{0}")]
    Other(String),
}

pub type ActivityFn = Arc<dyn Fn(CaptureActivity) + Send + Sync>;

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
    /// The scan's proxy; the request leaves through it like the scan's
    /// browser does. The loopback origin is always reached directly.
    pub proxy: FetchProxy,
}

/// The proxy of a public file request, with the secret it needs.
#[derive(Clone, PartialEq, Eq)]
pub struct FetchProxy {
    pub route: CaptureProxy,
    pub password: Option<String>,
}

impl FetchProxy {
    pub fn system() -> Self {
        Self {
            route: CaptureProxy::System,
            password: None,
        }
    }
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
    pub status: u16,
    pub content_type: Option<String>,
    /// In memory only; empty unless `status` is a success.
    pub body: Vec<u8>,
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
