use adscan_model::{Capture, Finding, ScanSummary, ScriptRecord, Settings};
use tokio_util::sync::CancellationToken;

use crate::application::AppError;
use crate::application::llm::LlmError;
use crate::application::ports::{CaptureError, RepositoryError};
use crate::domain::pricing::Budget;
use crate::domain::scan::ValidatedScan;
use crate::domain::scripts::DescribedScript;
use crate::domain::telemetry::Telemetry;

/// Immutable input of one pipeline run.
pub struct ScanJob {
    pub summary: ScanSummary,
    pub scan: ValidatedScan,
    pub settings: Settings,
    /// Held in memory for the duration of the scan only.
    pub api_key: Option<String>,
    pub cancel: CancellationToken,
}

/// Beautified source of a script selected for logic analysis.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceFile {
    /// Index into `ScanState::scripts`.
    pub script_index: usize,
    pub lines: Vec<String>,
}

/// State shared by the nodes of one run. Raw capture data lives only here and
/// is dropped with the state when the scan ends.
pub struct ScanState {
    pub summary: ScanSummary,
    pub budget: Budget,
    pub capture: Option<Capture>,
    pub telemetry: Option<Telemetry>,
    pub scripts: Vec<DescribedScript>,
    pub sources: Vec<SourceFile>,
    pub findings: Vec<Finding>,
    /// Overview written by the analysis model, if it finished.
    pub model_overview: Option<String>,
    /// Set when an AI node failed; later AI nodes are skipped.
    pub ai_failure: Option<String>,
    /// Caveats to surface in the overview (budget stop, unknown model price).
    pub caveats: Vec<String>,
}

impl ScanState {
    pub fn new(summary: ScanSummary, budget: Budget) -> Self {
        Self {
            summary,
            budget,
            capture: None,
            telemetry: None,
            scripts: Vec::new(),
            sources: Vec::new(),
            findings: Vec::new(),
            model_overview: None,
            ai_failure: None,
            caveats: Vec::new(),
        }
    }

    pub fn capture(&self) -> Result<&Capture, NodeError> {
        self.capture.as_ref().ok_or_else(|| missing("capture"))
    }

    pub fn telemetry(&self) -> Result<&Telemetry, NodeError> {
        self.telemetry.as_ref().ok_or_else(|| missing("telemetry"))
    }

    pub fn script_records(&self) -> Vec<ScriptRecord> {
        self.scripts.iter().map(|script| script.record.clone()).collect()
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
