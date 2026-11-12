//! Script triage: classifies the scripts the knowledge base could not decide.
//! Jev System One does it when the scan uses Jev; the filter model does it
//! otherwise, and takes over whatever Jev could not answer.

mod claude;
mod jev;

use std::collections::HashMap;

use adscan_model::{FindingSource, LogKind, LogLevel, ScriptCategory};
use serde_json::json;

use crate::application::llm::LlmError;
use crate::application::pipeline::audit::{AuditExcerpts, StepRun};
use crate::application::pipeline::state::{NodeError, NodeResult, ScanState};
use crate::domain::pricing::JEV_SYSTEM_ONE;

/// Script source the audit trail keeps per classified script. The models see
/// more; the log must stay a short excerpt.
const AUDIT_EXCERPT_CHARS: usize = 300;
/// And per scan, so a page with hundreds of undecided scripts cannot copy a
/// large part of its source into the log excerpt by excerpt.
const AUDIT_EXCERPT_CHARS_PER_SCAN: usize = 24_000;

/// An undecided script, with the scripts whose body is identical to it.
struct Candidate {
    /// The script that is asked about.
    script_index: usize,
    /// Scripts with the same body. They get the same verdict without a second question.
    duplicates: Vec<usize>,
    /// The identifiers that made the knowledge base hesitate.
    hints: Vec<String>,
}

impl Candidate {
    fn script_count(&self) -> usize {
        1 + self.duplicates.len()
    }
}

struct ScriptVerdict {
    category: ScriptCategory,
    ad_related: bool,
    confidence: f64,
    reason: String,
    /// Only a decision model reports one.
    ad_probability: Option<f64>,
    model: String,
}

#[derive(Default)]
struct Tally {
    classified: usize,
    out_of_budget: usize,
}

pub async fn classify_scripts(run: &mut StepRun<'_>) -> NodeResult {
    let mut pending = candidates(run.state);
    let asked: usize = pending.iter().map(Candidate::script_count).sum();
    let mut tally = Tally::default();
    let outcome = classify(run, &mut pending, &mut tally).await;
    // Whatever was classified before a failure is kept.
    let ad_related = persist(run).await?;
    outcome?;

    let classified = tally.classified;
    let mut summary = format!(
        "Classified {classified} of {asked} undecided scripts; {ad_related} scripts are ad-related"
    );
    if tally.out_of_budget > 0 {
        let caveat = format!(
            "{} scripts were not classified by AI because the scan budget was reached",
            tally.out_of_budget
        );
        run.warn(caveat.clone()).await?;
        summary.push_str(&format!(" ({caveat})"));
        run.state.caveats.push(caveat);
    }
    Ok(summary)
}

async fn classify(
    run: &mut StepRun<'_>,
    pending: &mut Vec<Candidate>,
    tally: &mut Tally,
) -> Result<(), NodeError> {
    let api_key = run.job.api_key.clone();
    let mut excerpts = AuditExcerpts::new(AUDIT_EXCERPT_CHARS_PER_SCAN);
    if let Some(jev_key) = run.job.jev_key.clone()
        && let Some(error) = jev::classify(run, &jev_key, pending, tally, &mut excerpts).await?
    {
        report_jev_failure(run, &error, pending, api_key.is_some()).await?;
    }
    if let Some(api_key) = api_key
        && !pending.is_empty()
    {
        claude::classify(run, &api_key, pending, tally, &mut excerpts).await?;
    }
    Ok(())
}

/// One candidate per distinct body among the undecided scripts.
fn candidates(state: &ScanState) -> Vec<Candidate> {
    let mut by_hash: HashMap<&str, usize> = HashMap::new();
    let mut candidates: Vec<Candidate> = Vec::new();
    for (script_index, script) in state.scripts.iter().enumerate() {
        let Some(hints) = &script.ambiguous_hints else {
            continue;
        };
        match by_hash.get(script.record.hash.as_str()) {
            Some(&first) => candidates[first].duplicates.push(script_index),
            None => {
                by_hash.insert(&script.record.hash, candidates.len());
                candidates.push(Candidate {
                    script_index,
                    duplicates: Vec::new(),
                    hints: hints.clone(),
                });
            }
        }
    }
    candidates
}

/// Records the verdict on the candidate's script and on every script with
/// the same body. Returns how many scripts that classified.
fn apply_verdict(state: &mut ScanState, candidate: &Candidate, verdict: ScriptVerdict) -> usize {
    for &index in std::iter::once(&candidate.script_index).chain(&candidate.duplicates) {
        let script = &mut state.scripts[index];
        script.ambiguous_hints = None;
        let record = &mut script.record;
        record.category = verdict.category;
        record.ad_related = verdict.ad_related;
        record.classification_confidence = Some(verdict.confidence);
        record.classification_reason = Some(verdict.reason.clone());
        record.classified_by = FindingSource::Ai;
        record.classifier_model = Some(verdict.model.clone());
        record.ad_probability = verdict.ad_probability;
    }
    candidate.script_count()
}

async fn report_jev_failure(
    run: &mut StepRun<'_>,
    error: &LlmError,
    pending: &[Candidate],
    claude_available: bool,
) -> Result<(), NodeError> {
    let remaining: usize = pending.iter().map(Candidate::script_count).sum();
    let fallback = if claude_available {
        run.job.settings.filter_model.clone()
    } else {
        "the knowledge base".to_string()
    };
    let outcome = match (remaining, claude_available) {
        (0, _) => "every script already had its verdict".to_string(),
        (_, true) => format!("{remaining} scripts fall back to {fallback}"),
        (_, false) => format!("{remaining} scripts stay as the knowledge base left them"),
    };
    let message = format!("Jev System One failed during script triage ({error}); {outcome}");
    run.log(
        LogLevel::Warn,
        LogKind::Message,
        message.clone(),
        Some(json!({
            "from": JEV_SYSTEM_ONE,
            "to": fallback,
            "reason": error.to_string(),
            "remainingScripts": remaining,
        })),
    )
    .await?;
    run.state.caveats.push(message);
    run.state.jev_failed = true;
    Ok(())
}

/// Saves the script records and the ad-script count. Returns that count.
async fn persist(run: &mut StepRun<'_>) -> Result<u32, NodeError> {
    let records = run.state.script_records();
    let ad_related = records.iter().filter(|record| record.ad_related).count() as u32;
    let scan_id = run.state.summary.id.clone();
    run.deps.scans.save_scripts(&scan_id, &records).await?;
    if let Some(telemetry) = run.state.telemetry.as_mut() {
        telemetry.total.metrics.ad_script_count = ad_related;
        run.deps
            .scans
            .save_telemetry(
                &scan_id,
                &telemetry.total.metrics,
                &telemetry.pages,
                &telemetry.total.auctions,
                &telemetry.total.slots,
            )
            .await?;
    }
    Ok(ad_related)
}
