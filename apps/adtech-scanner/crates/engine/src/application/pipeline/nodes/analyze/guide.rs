//! Builds the reading guide with Jev System One: chunks of the selected
//! scripts are judged on what kind of ad logic they hold, and the ranked
//! result tells the analysis model where to start. Kept in memory only.

use adscan_model::{LogKind, LogLevel};
use serde_json::{Map, Value, json};

use crate::application::judge::{JudgeRequest, JudgeResponse, questions_for_audit};
use crate::application::pipeline::audit::StepRun;
use crate::application::pipeline::judging::{BatchEnd, JudgeWork, judge_batch};
use crate::application::pipeline::state::NodeError;
use crate::domain::chunks::{LINE_CHUNK_MAX_CHARS, LineRange, split_lines};
use crate::domain::guide::{
    self, GuideEntry, MAX_SCREENED_CHUNKS, ScoredChunk, chunks_to_screen, is_publisher_ad_code,
    telemetry_literals,
};
use crate::domain::pricing::JEV_SYSTEM_ONE;
use crate::domain::questions::{QUESTION_SET_VERSION, QUESTIONS};

struct Chunk {
    /// Position of the script in the analysis selection.
    source: usize,
    lines: LineRange,
    state: Value,
}

struct GuideWork {
    chunks: Vec<Chunk>,
    scored: Vec<ScoredChunk>,
}

impl JudgeWork for GuideWork {
    fn request(&mut self, job: usize) -> Option<JudgeRequest> {
        Some(JudgeRequest {
            model: JEV_SYSTEM_ONE.to_string(),
            state: self.chunks[job].state.take(),
            questions: &QUESTIONS,
        })
    }

    fn answered(&mut self, job: usize, response: &JudgeResponse) {
        let chunk = &self.chunks[job];
        self.scored.push(ScoredChunk {
            source: chunk.source,
            lines: chunk.lines,
            probabilities: response.probabilities.clone(),
        });
    }
}

/// The guide for this scan's analysis selection, best range first. A Jev
/// failure leaves a shorter or empty guide and never fails the step.
pub(super) async fn build(
    run: &mut StepRun<'_>,
    api_key: &str,
) -> Result<Vec<GuideEntry>, NodeError> {
    let mut work = GuideWork {
        chunks: plan(run)?,
        scored: Vec::new(),
    };
    if work.chunks.is_empty() {
        return Ok(Vec::new());
    }
    run.log(
        LogLevel::Debug,
        LogKind::LlmRequest,
        format!(
            "Jev System One reading guide: {} chunks of the selected scripts, {} questions each. Each request carries one chunk; script source is not stored.",
            work.chunks.len(),
            QUESTIONS.len(),
        ),
        Some(json!({
            "model": JEV_SYSTEM_ONE,
            "questionSet": QUESTION_SET_VERSION,
            "questions": questions_for_audit(&QUESTIONS),
            "minRank": guide::MIN_RANK,
            "maxEntries": guide::MAX_ENTRIES,
        })),
    )
    .await?;

    let jobs = work.chunks.len();
    match judge_batch(run, api_key, jobs, &mut work).await? {
        BatchEnd::Completed => {}
        BatchEnd::BudgetReached => {
            run.warn("The reading guide is incomplete: the scan budget was reached")
                .await?;
        }
        BatchEnd::Failed(error) => {
            run.state.jev_failed = true;
            run.warn(format!(
                "Jev System One failed while building the reading guide ({error}); the analysis continues with the {} of {jobs} chunks that were answered",
                work.scored.len(),
            ))
            .await?;
        }
    }

    promote_publisher_code(run, &work.scored);
    let entries = guide::build(&work.scored);
    log_scores(run, &work.scored, entries.len()).await?;
    Ok(entries)
}

/// The chunks worth asking about, publisher scripts first, up to the cap.
fn plan(run: &StepRun<'_>) -> Result<Vec<Chunk>, NodeError> {
    let placements = telemetry_literals(run.state.telemetry()?);
    let mut chunks = Vec::new();
    for (source, file) in run.state.sources.iter().enumerate() {
        let record = &run.state.scripts[file.script_index].record;
        let ranges = split_lines(&file.lines);
        let vendor_library = file.default_search.is_some();
        for index in chunks_to_screen(&file.lines, &ranges, vendor_library, &placements) {
            if chunks.len() == MAX_SCREENED_CHUNKS {
                return Ok(chunks);
            }
            let lines = ranges[index];
            let code: String = lines
                .of(&file.lines)
                .join("\n")
                .chars()
                .take(LINE_CHUNK_MAX_CHARS)
                .collect();
            chunks.push(Chunk {
                source,
                lines,
                state: json!({
                    "script": {
                        "url": record.url,
                        "firstParty": record.first_party,
                        "lines": format!("{}-{} of {}", lines.start, lines.end, file.lines.len()),
                    },
                    "code": code,
                }),
            });
        }
    }
    Ok(chunks)
}

/// A stretch of a vendor library that Jev takes for the publisher's own ad
/// code joins the default search, like the stretches naming a placement.
fn promote_publisher_code(run: &mut StepRun<'_>, scored: &[ScoredChunk]) {
    for chunk in scored {
        if let Some(ranges) = run.state.sources[chunk.source].default_search.as_mut()
            && is_publisher_ad_code(&chunk.probabilities)
            && !ranges.contains(&chunk.lines)
        {
            ranges.push(chunk.lines);
            ranges.sort_by_key(|range| range.start);
        }
    }
}

/// Every answer, as line ranges and numbers only.
async fn log_scores(
    run: &mut StepRun<'_>,
    scored: &[ScoredChunk],
    listed: usize,
) -> Result<(), NodeError> {
    let chunks: Vec<Value> = scored
        .iter()
        .map(|chunk| {
            let script = run.state.sources[chunk.source].script_index;
            let answers: Map<String, Value> = QUESTIONS
                .iter()
                .zip(&chunk.probabilities)
                .map(|(question, probability)| (question.id.to_string(), json!(probability)))
                .collect();
            json!({
                "scriptId": run.state.scripts[script].record.id,
                "lines": [chunk.lines.start, chunk.lines.end],
                "rank": guide::rank(&chunk.probabilities),
                "answers": answers,
            })
        })
        .collect();
    run.log(
        LogLevel::Debug,
        LogKind::Message,
        format!(
            "Jev System One judged {} chunks; {listed} ranked high enough for the reading guide",
            scored.len()
        ),
        Some(json!({ "model": JEV_SYSTEM_ONE, "chunks": chunks })),
    )
    .await
}
