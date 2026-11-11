//! Runs one scan through the graph in [`graph`], keeping the audit trail.

pub mod audit;
pub mod graph;
pub mod nodes;
pub mod state;

use std::sync::Arc;

use adscan_model::{
    LogKind, LogLevel, PipelineNode, PipelineStep, ScanEvent, ScanStatus, ScanSummary, StepStatus,
};

use self::audit::StepRun;
use self::graph::OnFailure;
use self::state::{NodeError, NodeResult, ScanJob, ScanState};
use super::AppError;
use super::ports::{
    CaptureSource, Clock, EventSink, IdGenerator, LlmClient, LogRepository, RepoResult,
    ScanRepository, SourceFormatter,
};
use crate::domain::pricing::Budget;

#[derive(Clone)]
pub struct PipelineDeps {
    pub scans: Arc<dyn ScanRepository>,
    pub logs: Arc<dyn LogRepository>,
    pub llm: Arc<dyn LlmClient>,
    pub capture: Arc<dyn CaptureSource>,
    pub formatter: Arc<dyn SourceFormatter>,
    pub events: Arc<dyn EventSink>,
    pub clock: Arc<dyn Clock>,
    pub ids: Arc<dyn IdGenerator>,
}

enum RunOutcome {
    Completed,
    Cancelled,
    Failed(AppError),
}

pub struct Pipeline {
    deps: PipelineDeps,
}

impl Pipeline {
    pub fn new(deps: PipelineDeps) -> Self {
        Self { deps }
    }

    /// Runs the scan to a final status. The returned summary is what was
    /// persisted, unless storage itself failed, in which case it still carries
    /// the final status for the caller to write.
    pub async fn run(&self, job: ScanJob) -> ScanSummary {
        let budget = match Budget::new(job.settings.budget_usd) {
            Ok(budget) => budget,
            Err(error) => return self.finalize(job.summary.clone(), RunOutcome::Failed(error.into())).await,
        };
        let mut state = ScanState::new(job.summary.clone(), budget);
        let outcome = match self.execute(&job, &mut state).await {
            Ok(outcome) => outcome,
            Err(error) => RunOutcome::Failed(error.into()),
        };
        self.finalize(state.summary, outcome).await
    }

    async fn execute(&self, job: &ScanJob, state: &mut ScanState) -> RepoResult<RunOutcome> {
        state.summary.status = ScanStatus::Running;
        self.publish_summary(&state.summary).await?;

        let steps = self.plan_steps(&state.summary.id).await?;
        let mut outcome = RunOutcome::Completed;
        let mut remaining = steps.into_iter();
        for step in remaining.by_ref() {
            if job.cancel.is_cancelled() {
                self.close_unrun_step(step, "Cancelled").await?;
                outcome = RunOutcome::Cancelled;
                break;
            }
            if let Some(reason) = graph::skip_reason(step.node, job, state) {
                self.close_unrun_step(step, &reason).await?;
                continue;
            }
            if let Some(stopped) = self.run_step(step, job, state).await? {
                outcome = stopped;
                break;
            }
        }
        for step in remaining {
            self.close_unrun_step(step, "Not run").await?;
        }
        Ok(outcome)
    }

    /// Every node gets its step up front so the UI can show the whole plan.
    async fn plan_steps(&self, scan_id: &str) -> RepoResult<Vec<PipelineStep>> {
        let mut steps = Vec::with_capacity(graph::NODES.len());
        for node in graph::NODES {
            let step = PipelineStep {
                id: self.deps.ids.new_id(),
                scan_id: scan_id.to_string(),
                node,
                status: StepStatus::Pending,
                started_at: None,
                duration_ms: None,
                summary: None,
                model: None,
                input_tokens: 0,
                output_tokens: 0,
                cost_usd: 0.0,
                log_count: 0,
            };
            self.publish_step(&step).await?;
            steps.push(step);
        }
        Ok(steps)
    }

    /// Returns the outcome that ends the run, or `None` to continue.
    async fn run_step(
        &self,
        mut step: PipelineStep,
        job: &ScanJob,
        state: &mut ScanState,
    ) -> RepoResult<Option<RunOutcome>> {
        let node = step.node;
        step.status = StepStatus::Running;
        step.started_at = Some(self.deps.clock.now_ms());
        self.publish_step(&step).await?;

        let mut run = StepRun {
            deps: &self.deps,
            job,
            state,
            step,
        };
        let result = run_node(node, &mut run).await;
        let now = self.deps.clock.now_ms();
        let stopped = match result {
            Ok(summary) => {
                run.finish(StepStatus::Completed, summary, now);
                None
            }
            Err(NodeError::Cancelled) => {
                run.finish(StepStatus::Failed, "Cancelled".into(), now);
                Some(RunOutcome::Cancelled)
            }
            Err(NodeError::Failed(error)) => {
                // Best effort: the failure is also recorded on the step itself.
                let _ = run
                    .log(LogLevel::Error, LogKind::Message, error.to_string(), None)
                    .await;
                run.finish(StepStatus::Failed, error.to_string(), now);
                match graph::on_failure(node) {
                    OnFailure::FailScan => Some(RunOutcome::Failed(error)),
                    OnFailure::ContinueWithoutAi => {
                        run.state.ai_failure = Some(format!("{} failed: {error}", node_label(node)));
                        None
                    }
                }
            }
        };
        self.publish_step(&run.step).await?;
        Ok(stopped)
    }

    async fn close_unrun_step(&self, mut step: PipelineStep, reason: &str) -> RepoResult<()> {
        step.status = StepStatus::Skipped;
        step.summary = Some(reason.to_string());
        self.publish_step(&step).await
    }

    async fn finalize(&self, mut summary: ScanSummary, outcome: RunOutcome) -> ScanSummary {
        let now = self.deps.clock.now_ms();
        summary.finished_at = Some(now);
        summary.duration_ms = Some(now - summary.started_at);
        match outcome {
            RunOutcome::Completed => summary.status = ScanStatus::Completed,
            RunOutcome::Cancelled => summary.status = ScanStatus::Cancelled,
            RunOutcome::Failed(error) => {
                summary.status = ScanStatus::Failed;
                summary.error = Some(error.to_string());
            }
        }
        // If storage is failing the scan manager writes the final status again.
        let _ = self.publish_summary(&summary).await;
        summary
    }

    async fn publish_summary(&self, summary: &ScanSummary) -> RepoResult<()> {
        self.deps.scans.save_summary(summary).await?;
        self.deps.events.emit(ScanEvent::ScanUpdated {
            summary: summary.clone(),
        });
        Ok(())
    }

    async fn publish_step(&self, step: &PipelineStep) -> RepoResult<()> {
        self.deps.scans.save_step(step).await?;
        self.deps.events.emit(ScanEvent::StepUpdated { step: step.clone() });
        Ok(())
    }
}

async fn run_node(node: PipelineNode, run: &mut StepRun<'_>) -> NodeResult {
    match node {
        PipelineNode::LaunchBrowser => nodes::capture::launch_browser(run).await,
        PipelineNode::Capture => nodes::capture::capture_page(run).await,
        PipelineNode::Telemetry => nodes::telemetry::derive_telemetry(run).await,
        PipelineNode::Heuristics => nodes::heuristics::run_rules(run).await,
        PipelineNode::FilterScripts => nodes::filter::classify_scripts(run).await,
        PipelineNode::Beautify => nodes::beautify::beautify_scripts(run).await,
        PipelineNode::AnalyzeLogic => nodes::analyze::analyze_logic(run).await,
        PipelineNode::Synthesize => nodes::synthesize::synthesize(run).await,
    }
}

pub fn node_label(node: PipelineNode) -> &'static str {
    match node {
        PipelineNode::LaunchBrowser => "Browser launch",
        PipelineNode::Capture => "Page capture",
        PipelineNode::Telemetry => "Telemetry",
        PipelineNode::Heuristics => "Heuristic checks",
        PipelineNode::FilterScripts => "Script classification",
        PipelineNode::Beautify => "Beautify",
        PipelineNode::AnalyzeLogic => "Logic analysis",
        PipelineNode::Synthesize => "Synthesis",
    }
}
