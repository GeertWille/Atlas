//! Script triage by Jev System One: the parts of each undecided script that
//! name an ad-tech identifier are judged one per request, and the highest
//! probability routes the script. The model gives numbers and no reasons, so
//! the audit trail records what was asked and how the numbers were used.

use adscan_model::{LogKind, LogLevel};
use serde_json::{Value, json};

use super::{AUDIT_EXCERPT_CHARS, Candidate, ScriptVerdict, Tally, apply_verdict};
use crate::application::judge::{JudgeRequest, JudgeResponse, questions_for_audit};
use crate::application::llm::LlmError;
use crate::application::pipeline::audit::{AuditExcerpts, StepRun};
use crate::application::pipeline::judging::{BatchEnd, JudgeWork, judge_batch};
use crate::application::pipeline::state::NodeError;
use crate::domain::chunks::split_text;
use crate::domain::pricing::JEV_SYSTEM_ONE;
use crate::domain::questions::{
    AD_ORCHESTRATION, PUBLISHER_SPECIFIC, QUESTION_SET_VERSION, triage_questions,
};
use crate::domain::triage::{
    ANALYSE_THRESHOLD, DROP_THRESHOLD, MAX_SCREENED_PARTS, PUBLISHER_SPECIFIC_THRESHOLD, PartScore,
    Route, TriageDecision, decide, parts_to_screen,
};

struct Part {
    /// Position among all parts of the script.
    index: usize,
    identifiers: Vec<&'static str>,
    /// Taken when the request is built, so the source is held once.
    code: String,
    /// The start of the code, in case this part is the one the audit entry shows.
    head: String,
    score: Option<PartScore>,
}

struct Screening {
    candidate: Candidate,
    url: String,
    first_party: bool,
    total_parts: usize,
    parts: Vec<Part>,
    /// A part already gave the script its best possible verdict.
    settled: bool,
    answered_by: Option<String>,
}

impl Screening {
    fn scores(&self) -> Vec<PartScore> {
        self.parts.iter().filter_map(|part| part.score).collect()
    }

    /// Whether the scores so far can only be confirmed, not overturned, by
    /// the parts that were not answered.
    fn is_conclusive(&self) -> bool {
        self.settled
            || self.parts.iter().all(|part| part.score.is_some())
            || self
                .scores()
                .iter()
                .any(|score| score.ad_orchestration >= ANALYSE_THRESHOLD)
    }
}

struct TriageWork {
    screenings: Vec<Screening>,
    /// (screening, part) per job.
    jobs: Vec<(usize, usize)>,
}

impl JudgeWork for TriageWork {
    fn request(&mut self, job: usize) -> Option<JudgeRequest> {
        let (screening, part) = self.jobs[job];
        let screening = &mut self.screenings[screening];
        if screening.settled {
            return None;
        }
        Some(JudgeRequest {
            model: JEV_SYSTEM_ONE.to_string(),
            state: json!({
                "script": { "url": screening.url, "firstParty": screening.first_party },
                "code": std::mem::take(&mut screening.parts[part].code),
            }),
            questions: triage_questions(),
        })
    }

    fn answered(&mut self, job: usize, response: &JudgeResponse) {
        let (screening, part) = self.jobs[job];
        let screening = &mut self.screenings[screening];
        let score = PartScore::from_answers(&response.probabilities);
        screening.parts[part].score = score;
        screening.settled |= score.is_some_and(|score| score.settles_the_script());
        screening.answered_by = Some(response.answered_by.clone());
    }
}

/// Classifies what it can of `pending`. Returns the provider error that ended
/// it early, in which case the scripts without a verdict are still in
/// `pending`. Scripts the budget could not pay for are removed and counted.
pub(super) async fn classify(
    run: &mut StepRun<'_>,
    api_key: &str,
    pending: &mut Vec<Candidate>,
    tally: &mut Tally,
    excerpts: &mut AuditExcerpts,
) -> Result<Option<LlmError>, NodeError> {
    let mut work = plan(run, std::mem::take(pending))?;
    run.log(
        LogLevel::Debug,
        LogKind::LlmRequest,
        format!(
            "Jev System One: {} scripts, up to {} parts to judge, {} questions each. Each request carries one part of one script; script source is not stored.",
            work.screenings.len(),
            work.jobs.len(),
            triage_questions().len(),
        ),
        Some(json!({
            "model": JEV_SYSTEM_ONE,
            "questionSet": QUESTION_SET_VERSION,
            "questions": questions_for_audit(triage_questions()),
            "thresholds": thresholds_json(),
            "maxPartsPerScript": MAX_SCREENED_PARTS,
        })),
    )
    .await?;

    let end = judge_batch(run, api_key, work.jobs.len(), &mut work).await?;

    for screening in work.screenings {
        let decision = screening
            .is_conclusive()
            .then(|| {
                decide(
                    &screening.scores(),
                    screening.total_parts,
                    screening.first_party,
                )
            })
            .flatten();
        match decision {
            Some(decision) => {
                log_decision(run, &screening, &decision, excerpts).await?;
                tally.classified += apply_verdict(
                    run.state,
                    &screening.candidate,
                    ScriptVerdict {
                        category: decision.category,
                        ad_related: decision.ad_related,
                        confidence: decision.confidence,
                        reason: decision.reason,
                        ad_probability: Some(decision.ad_probability),
                        model: JEV_SYSTEM_ONE.to_string(),
                    },
                );
            }
            // The dearer classifier must not pick up what the budget stopped.
            None if end == BatchEnd::BudgetReached => {
                tally.out_of_budget += screening.candidate.script_count();
            }
            None => pending.push(screening.candidate),
        }
    }
    Ok(match end {
        BatchEnd::Failed(error) => Some(error),
        BatchEnd::Completed | BatchEnd::BudgetReached => None,
    })
}

fn plan(run: &StepRun<'_>, candidates: Vec<Candidate>) -> Result<TriageWork, NodeError> {
    let mut work = TriageWork {
        screenings: Vec::with_capacity(candidates.len()),
        jobs: Vec::new(),
    };
    for candidate in candidates {
        let record = &run.state.scripts[candidate.script_index].record;
        let texts = split_text(run.state.body(candidate.script_index)?);
        let parts: Vec<Part> = parts_to_screen(&texts)
            .into_iter()
            .map(|part| Part {
                index: part.index,
                identifiers: part.identifiers,
                code: texts[part.index].to_string(),
                head: texts[part.index]
                    .chars()
                    .take(AUDIT_EXCERPT_CHARS)
                    .collect(),
                score: None,
            })
            .collect();
        let screening = work.screenings.len();
        work.jobs
            .extend((0..parts.len()).map(|part| (screening, part)));
        work.screenings.push(Screening {
            url: record.url.clone(),
            first_party: record.first_party,
            total_parts: texts.len(),
            parts,
            settled: false,
            answered_by: None,
            candidate,
        });
    }
    Ok(work)
}

async fn log_decision(
    run: &mut StepRun<'_>,
    screening: &Screening,
    decision: &TriageDecision,
    excerpts: &mut AuditExcerpts,
) -> Result<(), NodeError> {
    let script_id = |index: usize| run.state.scripts[index].record.id.clone();
    let id = script_id(screening.candidate.script_index);
    let scored: Vec<(&Part, PartScore)> = screening
        .parts
        .iter()
        .filter_map(|part| Some((part, part.score?)))
        .collect();
    let best = scored
        .iter()
        .max_by(|a, b| a.1.ad_orchestration.total_cmp(&b.1.ad_orchestration));
    let outcome = match decision.route {
        Route::Analyse => "ad-related, Phase 2 candidate",
        Route::Unsure => "kept for analysis, ranked last",
        Route::Drop => "not ad-related",
    };
    let message = format!(
        "{id} {}: ad orchestration {:.2} (analyse >= {ANALYSE_THRESHOLD:.2}, drop < {DROP_THRESHOLD:.2}) over {} of {} parts -> {outcome} [{}]",
        screening.url,
        decision.ad_probability,
        scored.len(),
        screening.total_parts,
        decision.route.rule(),
    );
    let questions = triage_questions();
    let data = json!({
        "scriptId": id,
        "sameBodyAs": screening.candidate.duplicates.iter().map(|&index| script_id(index)).collect::<Vec<_>>(),
        "classifier": "jev",
        "model": JEV_SYSTEM_ONE,
        "modelAnswered": screening.answered_by,
        "questionSet": QUESTION_SET_VERSION,
        "answers": [
            { "questionId": questions[AD_ORCHESTRATION].id, "pYes": decision.ad_probability },
            { "questionId": questions[PUBLISHER_SPECIFIC].id, "pYes": decision.publisher_specific },
        ],
        "thresholds": thresholds_json(),
        "decision": {
            "adRelated": decision.ad_related,
            "category": decision.category,
            "route": decision.route.rule(),
        },
        "parts": {
            "total": screening.total_parts,
            "chosen": screening.parts.len(),
            "judged": scored.iter().map(|(part, score)| json!({
                "part": part.index,
                "identifiers": part.identifiers,
                "adOrchestration": score.ad_orchestration,
                "publisherSpecific": score.publisher_specific,
            })).collect::<Vec<_>>(),
        },
        "excerpt": best.map(|(part, _)| excerpts.excerpt(&part.head, AUDIT_EXCERPT_CHARS)),
    });
    run.log(LogLevel::Info, LogKind::Classification, message, Some(data))
        .await
}

fn thresholds_json() -> Value {
    json!({
        "analyse": ANALYSE_THRESHOLD,
        "drop": DROP_THRESHOLD,
        "publisherSpecific": PUBLISHER_SPECIFIC_THRESHOLD,
    })
}
