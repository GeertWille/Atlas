//! Everything the application needs from the outside world.

use std::sync::Arc;

use adscan_model::{
    AdSlot, ApiProvider, Auction, BrowserInfo, Capture, CaptureRequest, DashboardStats, Finding,
    LogEntry, LogSearchHit, PipelineStep, ScanDetail, ScanEvent, ScanList, ScanListQuery,
    ScanMetrics, ScanOptions, ScanSummary, ScriptRecord, Settings,
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
        auctions: &[Auction],
        slots: &[AdSlot],
    ) -> RepoResult<()>;
    async fn save_scripts(&self, scan_id: &str, scripts: &[ScriptRecord]) -> RepoResult<()>;
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

/// Where the API keys live, one per provider.
#[async_trait]
pub trait SecretStore: Send + Sync {
    async fn api_key(&self, provider: ApiProvider) -> Result<Option<String>, SecretError>;
    async fn store_api_key(&self, provider: ApiProvider, key: &str) -> Result<(), SecretError>;
    async fn clear_api_key(&self, provider: ApiProvider) -> Result<(), SecretError>;
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
    #[error("{0}")]
    Other(String),
}

pub type ProgressFn = Arc<dyn Fn(String) + Send + Sync>;

#[async_trait]
pub trait CaptureSource: Send + Sync {
    /// `explicit` is the user's configured browser path, if any.
    async fn locate_browser(&self, explicit: Option<String>) -> Option<BrowserInfo>;

    async fn capture(
        &self,
        request: CaptureRequest,
        cancel: CancellationToken,
        on_progress: ProgressFn,
    ) -> Result<Capture, CaptureError>;
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
