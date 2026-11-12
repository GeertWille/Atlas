//! The scan graph: node order, the conditions under which a node is skipped,
//! and what a node failure means for the rest of the run.

use adscan_model::PipelineNode::{self, *};

use super::state::{ScanJob, ScanState};
use crate::domain::scripts::select_for_analysis;

pub const NODES: [PipelineNode; 8] = [
    LaunchBrowser,
    Capture,
    Telemetry,
    Heuristics,
    FilterScripts,
    Beautify,
    AnalyzeLogic,
    Synthesize,
];

/// Phase 2 reads scripts on demand, so the cap bounds beautify time and the
/// size of the script index rather than prompt size.
const MAX_ANALYZED_SCRIPTS: usize = 12;
const MAX_ANALYZED_BYTES: f64 = 4_000_000.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnFailure {
    /// Without this node there is nothing to report.
    FailScan,
    /// The deterministic results stand; skip the remaining AI nodes.
    ContinueWithoutAi,
}

pub fn on_failure(node: PipelineNode) -> OnFailure {
    match node {
        FilterScripts | Beautify | AnalyzeLogic => OnFailure::ContinueWithoutAi,
        LaunchBrowser | Capture | Telemetry | Heuristics | Synthesize => OnFailure::FailScan,
    }
}

/// `Some(reason)` when the edge into `node` is not taken.
pub fn skip_reason(node: PipelineNode, job: &ScanJob, state: &ScanState) -> Option<String> {
    match node {
        LaunchBrowser | Capture | Telemetry | Heuristics | Synthesize => None,
        FilterScripts => ai_unavailable(job, state).or_else(|| {
            let ambiguous = state
                .scripts
                .iter()
                .filter(|script| script.ambiguous_hints.is_some())
                .count();
            (ambiguous == 0).then(|| {
                format!(
                    "All {} scripts were classified by the built-in knowledge base",
                    state.scripts.len()
                )
            })
        }),
        Beautify => ai_unavailable(job, state).or_else(|| {
            analysis_selection(state)
                .is_empty()
                .then(|| "No ad-related publisher scripts to analyze".to_string())
        }),
        AnalyzeLogic => ai_unavailable(job, state).or_else(|| {
            state
                .sources
                .is_empty()
                .then(|| "No ad-related publisher scripts to analyze".to_string())
        }),
    }
}

pub fn analysis_selection(state: &ScanState) -> Vec<usize> {
    select_for_analysis(
        &state.script_records(),
        MAX_ANALYZED_SCRIPTS,
        MAX_ANALYZED_BYTES,
    )
}

fn ai_unavailable(job: &ScanJob, state: &ScanState) -> Option<String> {
    if !job.scan.options.ai_enabled {
        return Some("AI analysis is turned off for this scan".into());
    }
    if job.api_key.is_none() {
        return Some("No Anthropic API key is configured".into());
    }
    if state.ai_failure.is_some() {
        return Some("Skipped because an earlier AI step failed".into());
    }
    if state.budget.remaining_usd() <= 0.0 {
        return Some(format!(
            "The LLM budget of ${:.2} for this scan is used up",
            state.budget.limit_usd()
        ));
    }
    None
}
