//! Runs many decision-model requests for one step with bounded concurrency,
//! charging each against the scan budget.

use std::sync::Arc;

use adscan_model::ApiProvider;
use tokio::task::JoinSet;

use super::audit::{ModelCall, StepRun};
use super::state::NodeError;
use crate::application::judge::{JudgeRequest, JudgeResponse};
use crate::application::llm::LlmError;
use crate::domain::pricing::{TokenUsage, cost_of};

/// Requests in flight at once. TypeSafe publishes no rate limits; this is the
/// low end of what its own examples use and has not been tuned.
const CONCURRENCY: usize = 4;

/// The requests of one batch and what to do with their answers.
pub trait JudgeWork: Send {
    /// The request for `job`, or `None` when it is no longer needed. Asked
    /// once per job, in order, just before the request would be sent.
    fn request(&mut self, job: usize) -> Option<JudgeRequest>;
    fn answered(&mut self, job: usize, response: &JudgeResponse);
}

#[derive(Debug, PartialEq)]
pub enum BatchEnd {
    /// Every job was answered or declined by the work itself.
    Completed,
    /// The scan budget cannot pay for the next request.
    BudgetReached,
    /// The provider failed. Nothing more was sent after the first failure.
    Failed(LlmError),
}

struct Finished {
    job: usize,
    model: String,
    estimate: TokenUsage,
    /// What was held back from the budget while the request was under way.
    cost: f64,
    answer: Result<JudgeResponse, LlmError>,
}

/// Sends the requests of jobs `0..jobs`. Requests already under way when the
/// batch ends early are still awaited, so what they cost is booked. The first
/// reason to stop sending is the one reported: a request that fails after the
/// budget ran out does not turn a budget stop into a provider failure.
pub async fn judge_batch(
    run: &mut StepRun<'_>,
    api_key: &str,
    jobs: usize,
    work: &mut dyn JudgeWork,
) -> Result<BatchEnd, NodeError> {
    let mut in_flight: JoinSet<Finished> = JoinSet::new();
    let mut reserved_usd = 0.0;
    let mut next = 0;
    let mut end = BatchEnd::Completed;
    loop {
        run.check_cancelled()?;
        while end == BatchEnd::Completed && in_flight.len() < CONCURRENCY && next < jobs {
            let job = next;
            next += 1;
            let Some(request) = work.request(job) else {
                continue;
            };
            let estimate = request.estimated_usage();
            let cost = cost_of(&request.model, estimate).usd;
            if !run.state.budget.can_afford(reserved_usd + cost) {
                end = BatchEnd::BudgetReached;
                break;
            }
            reserved_usd += cost;
            let judge = Arc::clone(&run.deps.judge);
            let api_key = api_key.to_string();
            let cancel = run.job.cancel.clone();
            in_flight.spawn(async move {
                let answer = judge.judge(&api_key, &request, &cancel).await;
                Finished {
                    job,
                    model: request.model,
                    estimate,
                    cost,
                    answer,
                }
            });
        }
        let Some(joined) = in_flight.join_next().await else {
            return Ok(end);
        };
        let finished = match joined {
            Ok(finished) => finished,
            // A request that panicked is one more way for the provider call
            // to fail; the step's fallback handles it like any other.
            Err(error) => {
                if end == BatchEnd::Completed {
                    end = BatchEnd::Failed(LlmError::InvalidResponse(format!(
                        "a request stopped unexpectedly: {error}"
                    )));
                }
                continue;
            }
        };
        reserved_usd -= finished.cost;
        match finished.answer {
            Ok(response) => {
                run.record_llm_call(ModelCall {
                    provider: ApiProvider::Typesafe,
                    model: &finished.model,
                    usage: if response.usage_reported {
                        response.usage
                    } else {
                        finished.estimate
                    },
                    usage_estimated: !response.usage_reported,
                })
                .await?;
                work.answered(finished.job, &response);
            }
            Err(LlmError::Cancelled) => return Err(NodeError::Cancelled),
            Err(error) => {
                if end == BatchEnd::Completed {
                    end = BatchEnd::Failed(error);
                }
            }
        }
    }
}
