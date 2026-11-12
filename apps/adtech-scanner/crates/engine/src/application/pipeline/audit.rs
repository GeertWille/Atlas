//! One running pipeline step: its audit trail, token accounting and findings.

use adscan_model::{
    ApiProvider, Finding, LogEntry, LogKind, LogLevel, PipelineStep, ScanEvent, StepStatus,
};
use serde_json::{Value, json};

use super::PipelineDeps;
use super::state::{NodeError, ScanJob, ScanState};
use crate::domain::finding::FindingDraft;
use crate::domain::pricing::{TokenUsage, cost_of};
use crate::domain::synthesis::severity_counts;

/// Backstop for every entry. Nodes that log captured data (script source,
/// tool output) cut it down to short excerpts themselves before it gets here.
const MAX_LOG_MESSAGE_CHARS: usize = 20_000;
const MAX_LOG_DATA_STRING_CHARS: usize = 4_000;

/// One billed model call.
pub struct ModelCall<'a> {
    pub provider: ApiProvider,
    pub model: &'a str,
    pub usage: TokenUsage,
    /// True when the provider reported no usage and `usage` is our estimate.
    pub usage_estimated: bool,
}

/// How much captured text (script source, raw telemetry) a step may still
/// copy into the audit trail. Captured data must not be persisted, so each
/// entry keeps a short excerpt and a scan's excerpts stop once they add up
/// to the allowance: reading or classifying a script piece by piece cannot
/// copy it into the log.
pub struct AuditExcerpts {
    remaining_chars: usize,
}

impl AuditExcerpts {
    pub fn new(chars: usize) -> Self {
        Self {
            remaining_chars: chars,
        }
    }

    /// The start of `content`: at most `limit` characters, and no more than
    /// the allowance still holds.
    pub fn excerpt(&mut self, content: &str, limit: usize) -> String {
        let kept = content
            .chars()
            .take(limit.min(self.remaining_chars))
            .collect::<String>();
        self.remaining_chars -= kept.chars().count();
        kept
    }

    /// Like [`excerpt`](Self::excerpt), with a note of what was left out
    /// (`what` names the data) when something was.
    pub fn take(&mut self, content: &str, limit: usize, what: &str) -> String {
        let total = content.chars().count();
        let excerpt = self.excerpt(content, limit);
        let kept = excerpt.chars().count();
        if kept == total {
            return excerpt;
        }
        format!(
            "{excerpt}\n[audit excerpt: {kept} of {total} characters; {what} is not stored in full]"
        )
    }
}

pub struct StepRun<'a> {
    pub deps: &'a PipelineDeps,
    pub job: &'a ScanJob,
    pub state: &'a mut ScanState,
    pub(super) step: PipelineStep,
}

impl StepRun<'_> {
    pub fn check_cancelled(&self) -> Result<(), NodeError> {
        if self.job.cancel.is_cancelled() {
            Err(NodeError::Cancelled)
        } else {
            Ok(())
        }
    }

    pub async fn info(&mut self, message: impl Into<String>) -> Result<(), NodeError> {
        self.log(LogLevel::Info, LogKind::Message, message, None)
            .await
    }

    pub async fn warn(&mut self, message: impl Into<String>) -> Result<(), NodeError> {
        self.log(LogLevel::Warn, LogKind::Message, message, None)
            .await
    }

    pub async fn log(
        &mut self,
        level: LogLevel,
        kind: LogKind,
        message: impl Into<String>,
        data: Option<Value>,
    ) -> Result<(), NodeError> {
        let entry = LogEntry {
            id: self.deps.ids.new_id(),
            scan_id: self.step.scan_id.clone(),
            step_id: self.step.id.clone(),
            node: self.step.node,
            ts: self.deps.clock.now_ms(),
            level,
            kind,
            message: truncate_chars(&message.into(), MAX_LOG_MESSAGE_CHARS),
            data: data.map(cap_strings),
        };
        self.deps.logs.append(&entry).await?;
        self.step.log_count += 1;
        self.deps.events.emit(ScanEvent::Log { entry });
        Ok(())
    }

    /// Books one model call against the step, the scan and the budget.
    pub async fn record_llm_call(&mut self, call: ModelCall<'_>) -> Result<f64, NodeError> {
        let ModelCall {
            provider,
            model,
            usage,
            usage_estimated,
        } = call;
        let cost = cost_of(model, usage);
        // A step that fell back from one classifier to another names both.
        self.step.model = Some(match self.step.model.take() {
            Some(models) if models.split(", ").any(|used| used == model) => models,
            Some(models) => format!("{models}, {model}"),
            None => model.to_string(),
        });
        self.step.input_tokens =
            saturating_u32(self.step.input_tokens as u64 + usage.prompt_tokens());
        self.step.output_tokens = saturating_u32(self.step.output_tokens as u64 + usage.output);
        self.step.cost_usd += cost.usd;
        self.state.budget.charge(cost.usd);
        self.state.summary.cost_usd += cost.usd;
        self.state.summary.ai_used = true;
        self.log(
            LogLevel::Debug,
            LogKind::Message,
            format!(
                "Token usage: {} in ({} cached), {} out, ${:.4} (scan total ${:.4} of ${:.2})",
                usage.prompt_tokens(),
                usage.cache_read,
                usage.output,
                cost.usd,
                self.state.budget.spent_usd(),
                self.state.budget.limit_usd(),
            ),
            Some(json!({
                "provider": provider,
                "model": model,
                "usageEstimated": usage_estimated,
                "inputTokens": usage.input,
                "outputTokens": usage.output,
                "cacheReadTokens": usage.cache_read,
                "cacheWriteTokens": usage.cache_write,
                "costUsd": cost.usd,
            })),
        )
        .await?;
        if !cost.known_model {
            let caveat = format!(
                "No price is known for model `{model}`; its cost was estimated with a conservative fallback price"
            );
            if !self.state.caveats.contains(&caveat) {
                self.warn(caveat.clone()).await?;
                self.state.caveats.push(caveat);
            }
        }
        self.deps.scans.save_step(&self.step).await?;
        self.deps.scans.save_summary(&self.state.summary).await?;
        self.deps.events.emit(ScanEvent::StepUpdated {
            step: self.step.clone(),
        });
        self.deps.events.emit(ScanEvent::ScanUpdated {
            summary: self.state.summary.clone(),
        });
        Ok(cost.usd)
    }

    /// Persists a finding as soon as it is known, so a cancelled or failed
    /// scan keeps it.
    pub async fn add_finding(
        &mut self,
        draft: FindingDraft,
        origin: Value,
    ) -> Result<Finding, NodeError> {
        let finding = draft.into_finding(self.deps.ids.new_id(), &self.step.scan_id);
        self.state.findings.push(finding.clone());
        self.state.summary.finding_counts = severity_counts(&self.state.findings);
        self.deps
            .scans
            .save_findings(&self.step.scan_id, &self.state.findings)
            .await?;
        self.deps.scans.save_summary(&self.state.summary).await?;
        self.log(
            LogLevel::Info,
            LogKind::Finding,
            format!("[{:?}] {}", finding.severity, finding.title),
            Some(json!({ "findingId": finding.id, "origin": origin })),
        )
        .await?;
        self.deps.events.emit(ScanEvent::FindingAdded {
            finding: finding.clone(),
        });
        self.deps.events.emit(ScanEvent::ScanUpdated {
            summary: self.state.summary.clone(),
        });
        Ok(finding)
    }

    pub(super) fn finish(&mut self, status: StepStatus, summary: String, now_ms: f64) {
        self.step.status = status;
        self.step.summary = Some(summary);
        self.step.duration_ms = self.step.started_at.map(|started| now_ms - started);
    }
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    match text.char_indices().nth(max_chars) {
        Some((cut, _)) => format!(
            "{}\n[truncated, {} more characters]",
            &text[..cut],
            text[cut..].chars().count()
        ),
        None => text.to_string(),
    }
}

fn cap_strings(value: Value) -> Value {
    match value {
        Value::String(text) => Value::String(truncate_chars(&text, MAX_LOG_DATA_STRING_CHARS)),
        Value::Array(items) => Value::Array(items.into_iter().map(cap_strings).collect()),
        Value::Object(fields) => Value::Object(
            fields
                .into_iter()
                .map(|(key, value)| (key, cap_strings(value)))
                .collect(),
        ),
        other => other,
    }
}

fn saturating_u32(value: u64) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}
