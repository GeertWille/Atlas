use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use adscan_model::{
    BidderDiagnostics, Capture, Finding, ScanEnvironment, ScanSummary, ScriptRecord, Settings,
};
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

use crate::application::AppError;
use crate::application::archive::analysis::RecordAnalysis;
use crate::application::llm::LlmError;
use crate::application::ports::{CaptureError, RepositoryError};
use crate::domain::chunks::LineRange;
use crate::domain::pricing::Budget;
use crate::domain::scan::ValidatedScan;
use crate::domain::scripts::DescribedScript;
use crate::domain::telemetry::ScanTelemetry;

/// Immutable input of one pipeline run.
pub struct ScanJob {
    pub summary: ScanSummary,
    pub scan: ValidatedScan,
    pub settings: Settings,
    /// Anthropic key. Held in memory for the duration of the scan only.
    pub api_key: Option<String>,
    /// TypeSafe key, present only when the scan uses Jev. In memory only.
    pub jev_key: Option<String>,
    /// Password of the scan's proxy profile. In memory only.
    pub proxy_password: Option<String>,
    pub cancel: CancellationToken,
    pub finish: FinishSignal,
    /// One permit: the turn to run a browser in the persistent browser profile.
    pub scan_profile: Arc<Semaphore>,
}

/// The user's "finish capture" for a manual visit. It only counts while the
/// capture is running: pressed earlier or later it must not end a capture
/// that has not started, so the capture arms it for its own duration.
#[derive(Clone, Default)]
pub struct FinishSignal {
    token: CancellationToken,
    armed: Arc<AtomicBool>,
}

impl FinishSignal {
    /// Starts listening; the returned token is cancelled by [`Self::request`].
    pub fn arm(&self) -> CancellationToken {
        self.armed.store(true, Ordering::SeqCst);
        self.token.clone()
    }

    pub fn disarm(&self) {
        self.armed.store(false, Ordering::SeqCst);
    }

    pub fn request(&self) {
        if self.armed.load(Ordering::SeqCst) {
            self.token.cancel();
        }
    }
}

/// Beautified source of a script selected for logic analysis.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceFile {
    /// Index into `ScanState::scripts`.
    pub script_index: usize,
    pub lines: Vec<String>,
    /// The lines a search of all scripts covers. `None` is every line; a
    /// stock vendor library is only covered where it holds publisher code.
    pub default_search: Option<Vec<LineRange>>,
}

/// State shared by the nodes of one run. Raw capture data lives only here and
/// is dropped with the state when the scan ends.
pub struct ScanState {
    pub summary: ScanSummary,
    pub budget: Budget,
    pub capture: Option<Capture>,
    /// What was read out of the visit's requests while they streamed past.
    pub network: RecordAnalysis,
    /// The scan has an archive: request rows exist and can be pointed at.
    pub archived: bool,
    pub telemetry: Option<ScanTelemetry>,
    /// What the pages could tell about the visit; set with the telemetry.
    pub environment: Option<ScanEnvironment>,
    /// `None` when the page has no bidders to check.
    pub diagnostics: Option<BidderDiagnostics>,
    pub scripts: Vec<DescribedScript>,
    pub sources: Vec<SourceFile>,
    pub findings: Vec<Finding>,
    /// Overview written by the analysis model, if it finished.
    pub model_overview: Option<String>,
    /// Set when an AI node failed; later AI nodes are skipped.
    pub ai_failure: Option<String>,
    /// Set by the first Jev failure; the scan asks Jev nothing more.
    pub jev_failed: bool,
    /// Caveats to surface in the overview (budget stop, unknown model price).
    pub caveats: Vec<String>,
}

impl ScanState {
    pub fn new(summary: ScanSummary, budget: Budget) -> Self {
        Self {
            summary,
            budget,
            capture: None,
            network: RecordAnalysis::default(),
            archived: false,
            telemetry: None,
            environment: None,
            diagnostics: None,
            scripts: Vec::new(),
            sources: Vec::new(),
            findings: Vec::new(),
            model_overview: None,
            ai_failure: None,
            jev_failed: false,
            caveats: Vec::new(),
        }
    }

    pub fn capture(&self) -> Result<&Capture, NodeError> {
        self.capture.as_ref().ok_or_else(|| missing("capture"))
    }

    pub fn telemetry(&self) -> Result<&ScanTelemetry, NodeError> {
        self.telemetry.as_ref().ok_or_else(|| missing("telemetry"))
    }

    /// The script's body as captured.
    pub fn body(&self, script_index: usize) -> Result<&str, NodeError> {
        Ok(&self.capture()?.scripts[script_index].body)
    }

    pub fn script_records(&self) -> Vec<ScriptRecord> {
        self.scripts
            .iter()
            .map(|script| script.record.clone())
            .collect()
    }
}

fn missing(what: &str) -> NodeError {
    NodeError::Failed(AppError::Internal(format!(
        "pipeline node ran before the {what} was available"
    )))
}

#[derive(Debug, Clone, PartialEq)]
pub enum NodeError {
    Cancelled,
    Failed(AppError),
}

/// A node returns the one-line summary of what it did.
pub type NodeResult = Result<String, NodeError>;

impl From<AppError> for NodeError {
    fn from(error: AppError) -> Self {
        match error {
            AppError::Cancelled => Self::Cancelled,
            other => Self::Failed(other),
        }
    }
}

impl From<RepositoryError> for NodeError {
    fn from(error: RepositoryError) -> Self {
        AppError::from(error).into()
    }
}

impl From<LlmError> for NodeError {
    fn from(error: LlmError) -> Self {
        AppError::from(error).into()
    }
}

impl From<CaptureError> for NodeError {
    fn from(error: CaptureError) -> Self {
        AppError::from(error).into()
    }
}
