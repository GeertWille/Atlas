//! Phase 2: an agentic tool loop in which the analysis model reads the
//! beautified ad scripts and the telemetry and reports proven logic bugs.

mod prompt;
mod render;
mod toolbox;

use adscan_model::{Effort, FindingSource, LogKind, LogLevel};
use serde_json::json;

use self::prompt::SYSTEM_PROMPT;
use self::toolbox::{ScriptSource, ToolEffect, ToolOutcome, Toolbox};
use crate::application::AppError;
use crate::application::llm::{
    AssistantBlock, AssistantTurn, LlmMessage, LlmRequest, StopReason, UserBlock,
};
use crate::application::pipeline::audit::StepRun;
use crate::application::pipeline::state::{NodeError, NodeResult};
use crate::domain::pricing::{TokenUsage, affordable_output_tokens, estimate_tokens};

const MAX_TOKENS_PER_TURN: u32 = 8_000;
/// Below this a turn cannot hold thinking plus a useful tool call, so the
/// loop stops instead of paying for a turn that will be cut off.
const MIN_USEFUL_OUTPUT_TOKENS: u64 = 1_000;
/// Tool results are stretches of captured script source and raw telemetry,
/// which must not be persisted. The audit trail keeps a short excerpt of each
/// result and stops keeping any once a scan's results add up to the total, so
/// reading a script chunk by chunk cannot copy it into the log.
const MAX_LOGGED_TOOL_RESULT_CHARS: usize = 800;
const MAX_LOGGED_TOOL_RESULT_CHARS_PER_SCAN: usize = 24_000;

fn max_iterations(effort: Effort) -> u32 {
    match effort {
        Effort::Low => 12,
        Effort::Medium => 20,
        Effort::High => 30,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StopCause {
    Finished,
    EndedTurn,
    BudgetReached,
    IterationLimit,
    Refused,
    OutputLimit,
}

impl StopCause {
    fn describe(self) -> &'static str {
        match self {
            Self::Finished => "the model finished",
            Self::EndedTurn => "the model stopped without calling finish",
            Self::BudgetReached => "stopped early: the scan budget was reached",
            Self::IterationLimit => "stopped early: the iteration limit was reached",
            Self::Refused => "stopped early: the model declined to continue",
            Self::OutputLimit => "stopped early: a model turn hit the output token limit",
        }
    }
}

pub async fn analyze_logic(run: &mut StepRun<'_>) -> NodeResult {
    let model = run.job.settings.analysis_model.clone();
    let effort = run.job.settings.analysis_effort;
    let api_key = run.job.api_key.clone().ok_or(AppError::ApiKeyMissing)?;
    let toolbox = build_toolbox(run)?;
    mark_scripts_analyzed(run).await?;

    let heuristic_findings: Vec<_> = run
        .state
        .findings
        .iter()
        .filter(|finding| finding.source == FindingSource::Heuristic)
        .cloned()
        .collect();
    let opening = prompt::opening_message(
        run.state.capture()?,
        run.state.telemetry()?,
        &heuristic_findings,
        &toolbox,
    );
    let tools = prompt::tool_specs();
    let prefix_chars = SYSTEM_PROMPT.len()
        + tools
            .iter()
            .map(|tool| {
                tool.name.len() + tool.description.len() + tool.input_schema.to_string().len()
            })
            .sum::<usize>();
    run.log(
        LogLevel::Debug,
        LogKind::LlmRequest,
        format!("System:\n{SYSTEM_PROMPT}\n\nUser:\n{opening}"),
        Some(json!({
            "model": model,
            "effort": effort,
            "tools": tools.iter().map(|tool| tool.name).collect::<Vec<_>>(),
        })),
    )
    .await?;

    let mut messages = vec![LlmMessage::User(vec![UserBlock::Text(opening.clone())])];
    let mut last_usage: Option<TokenUsage> = None;
    let mut pending_chars = prefix_chars + opening.len();
    let mut audit_excerpts = AuditExcerpts {
        remaining_chars: MAX_LOGGED_TOOL_RESULT_CHARS_PER_SCAN,
    };
    let mut reported = 0;
    let mut iterations = 0;
    let mut stop = StopCause::IterationLimit;

    while iterations < max_iterations(effort) {
        run.check_cancelled()?;
        // The output limit of each call is what the budget still pays for, so
        // a turn cannot overspend however long the model would like to write.
        let affordable = affordable_output_tokens(
            &model,
            projected_prompt(last_usage, pending_chars),
            run.state.budget.remaining_usd(),
        );
        if affordable < MIN_USEFUL_OUTPUT_TOKENS {
            stop = StopCause::BudgetReached;
            break;
        }
        iterations += 1;
        let request = LlmRequest {
            model: model.clone(),
            max_tokens: affordable.min(u64::from(MAX_TOKENS_PER_TURN)) as u32,
            effort,
            system: SYSTEM_PROMPT.to_string(),
            tools: tools.clone(),
            messages: messages.clone(),
            output_schema: None,
            cache_prefix: true,
        };
        let response = run
            .deps
            .llm
            .complete(&api_key, &request, &run.job.cancel)
            .await?;
        run.record_llm_call(&model, response.usage).await?;
        last_usage = Some(response.usage);
        log_model_output(run, &response.turn).await?;

        match response.stop_reason {
            StopReason::Refusal { category } => {
                run.warn(format!(
                    "The model declined to continue ({})",
                    category.as_deref().unwrap_or("no category given")
                ))
                .await?;
                stop = StopCause::Refused;
                break;
            }
            // Tool calls in a truncated turn may be cut off mid-input; never run them.
            StopReason::MaxTokens => {
                stop = StopCause::OutputLimit;
                break;
            }
            StopReason::PauseTurn => {
                messages.push(LlmMessage::Assistant(response.turn));
                pending_chars = 0;
            }
            StopReason::ToolUse => {
                let (results, finished) = run_tool_calls(
                    run,
                    &toolbox,
                    &response.turn,
                    &mut audit_excerpts,
                    &mut reported,
                )
                .await?;
                if finished || results.is_empty() {
                    stop = if finished {
                        StopCause::Finished
                    } else {
                        StopCause::EndedTurn
                    };
                    break;
                }
                pending_chars = results.iter().map(result_chars).sum();
                messages.push(LlmMessage::Assistant(response.turn));
                messages.push(LlmMessage::User(results));
            }
            StopReason::EndTurn | StopReason::Other(_) => {
                let text = response.turn.text();
                if !text.trim().is_empty() {
                    run.state.model_overview = Some(text);
                }
                stop = StopCause::EndedTurn;
                break;
            }
        }
    }

    if stop != StopCause::Finished && stop != StopCause::EndedTurn {
        run.warn(format!("Logic analysis {}", stop.describe()))
            .await?;
        run.state.caveats.push(format!(
            "The AI logic analysis is partial ({})",
            stop.describe()
        ));
    }
    Ok(format!(
        "{iterations} model turns, {reported} findings reported; {}",
        stop.describe()
    ))
}

fn build_toolbox(run: &StepRun<'_>) -> Result<Toolbox, NodeError> {
    let scripts = run
        .state
        .sources
        .iter()
        .map(|source| {
            let record = &run.state.scripts[source.script_index].record;
            ScriptSource {
                id: record.id.clone(),
                url: record.url.clone(),
                category: record.category,
                size_bytes: record.size_bytes,
                lines: source.lines.clone(),
            }
        })
        .collect();
    Ok(Toolbox::new(
        scripts,
        run.state.capture()?,
        run.state.telemetry()?,
    ))
}

async fn mark_scripts_analyzed(run: &mut StepRun<'_>) -> Result<(), NodeError> {
    let indexes: Vec<usize> = run
        .state
        .sources
        .iter()
        .map(|source| source.script_index)
        .collect();
    for index in indexes {
        run.state.scripts[index].record.analyzed = true;
    }
    let scan_id = run.state.summary.id.clone();
    run.deps
        .scans
        .save_scripts(&scan_id, &run.state.script_records())
        .await?;
    Ok(())
}

/// Prompt of the next call, priced the way it will be billed. Everything new
/// (the whole prompt before the first call, the latest tool results later) is
/// a cache write; the conversation so far is a cache read. When the previous
/// call shows the cache is not in use, the whole conversation is plain input.
fn projected_prompt(last_usage: Option<TokenUsage>, pending_chars: usize) -> TokenUsage {
    let pending = estimate_tokens(pending_chars);
    match last_usage {
        None => TokenUsage {
            cache_write: pending,
            ..TokenUsage::default()
        },
        Some(last) if last.cache_read + last.cache_write == 0 => TokenUsage {
            input: last.prompt_tokens() + last.output + pending,
            ..TokenUsage::default()
        },
        Some(last) => TokenUsage {
            cache_read: last.prompt_tokens() + last.output,
            cache_write: pending,
            ..TokenUsage::default()
        },
    }
}

struct AuditExcerpts {
    remaining_chars: usize,
}

impl AuditExcerpts {
    /// The part of a tool result that goes into the audit trail.
    fn take(&mut self, content: &str) -> String {
        let total = content.chars().count();
        let kept = total
            .min(MAX_LOGGED_TOOL_RESULT_CHARS)
            .min(self.remaining_chars);
        self.remaining_chars -= kept;
        if kept == total {
            return content.to_string();
        }
        let excerpt: String = content.chars().take(kept).collect();
        format!(
            "{excerpt}\n[audit excerpt: {kept} of {total} characters; tool output is not stored in full]"
        )
    }
}

async fn log_model_output(run: &mut StepRun<'_>, turn: &AssistantTurn) -> Result<(), NodeError> {
    for block in &turn.blocks {
        match block {
            AssistantBlock::Thinking(summary) if !summary.trim().is_empty() => {
                run.log(LogLevel::Debug, LogKind::Thinking, summary.clone(), None)
                    .await?;
            }
            AssistantBlock::Text(text) if !text.trim().is_empty() => {
                run.log(LogLevel::Info, LogKind::LlmResponse, text.clone(), None)
                    .await?;
            }
            _ => {}
        }
    }
    Ok(())
}

/// Runs every tool call of the turn and returns all results for one user
/// message, plus whether the model called `finish`.
async fn run_tool_calls(
    run: &mut StepRun<'_>,
    toolbox: &Toolbox,
    turn: &AssistantTurn,
    audit_excerpts: &mut AuditExcerpts,
    reported: &mut u32,
) -> Result<(Vec<UserBlock>, bool), NodeError> {
    let mut results = Vec::new();
    let mut finished = false;
    for block in &turn.blocks {
        let AssistantBlock::ToolUse { id, name, input } = block else {
            continue;
        };
        run.log(
            LogLevel::Info,
            LogKind::ToolCall,
            format!("{name}({input})"),
            Some(json!({ "tool": name, "input": input })),
        )
        .await?;
        let ToolOutcome {
            content,
            is_error,
            effect,
        } = toolbox.execute(name, input);
        let level = if is_error {
            LogLevel::Warn
        } else {
            LogLevel::Debug
        };
        // Only successful lookups return captured data; errors and
        // acknowledgements are our own short messages.
        let carries_captured_data = !is_error && effect == ToolEffect::None;
        let logged = if carries_captured_data {
            audit_excerpts.take(&content)
        } else {
            content.clone()
        };
        run.log(
            level,
            LogKind::ToolResult,
            logged,
            Some(json!({
                "tool": name,
                "isError": is_error,
                "outputChars": content.chars().count(),
            })),
        )
        .await?;
        match effect {
            ToolEffect::None => {}
            ToolEffect::Finding(draft) => {
                run.add_finding(*draft, json!({ "tool": "report_finding" }))
                    .await?;
                *reported += 1;
            }
            ToolEffect::Finish { overview } => {
                if !overview.trim().is_empty() {
                    run.state.model_overview = Some(overview);
                }
                finished = true;
            }
        }
        results.push(UserBlock::ToolResult {
            tool_use_id: id.clone(),
            content,
            is_error,
        });
    }
    Ok((results, finished))
}

fn result_chars(block: &UserBlock) -> usize {
    match block {
        UserBlock::Text(text) => text.len(),
        UserBlock::ToolResult { content, .. } => content.len(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::pricing::cost_of;

    const MODEL: &str = "claude-sonnet-5-5";

    #[test]
    fn the_first_prompt_is_projected_as_a_cache_write() {
        let projected = projected_prompt(None, 30_000);
        assert_eq!(
            projected,
            TokenUsage {
                cache_write: 10_000,
                ..TokenUsage::default()
            }
        );
    }

    #[test]
    fn a_cached_conversation_is_projected_as_a_cache_read_plus_the_new_results() {
        let last = TokenUsage {
            input: 50,
            output: 500,
            cache_read: 0,
            cache_write: 10_000,
        };
        assert_eq!(
            projected_prompt(Some(last), 3_000),
            TokenUsage {
                cache_read: 10_550,
                cache_write: 1_000,
                ..TokenUsage::default()
            }
        );
    }

    #[test]
    fn without_a_working_cache_the_whole_conversation_is_projected_at_the_input_price() {
        let last = TokenUsage {
            input: 20_000,
            output: 500,
            cache_read: 0,
            cache_write: 0,
        };
        let projected = projected_prompt(Some(last), 3_000);
        assert_eq!(
            projected,
            TokenUsage {
                input: 21_500,
                ..TokenUsage::default()
            }
        );
        // Billing the next call the same way must not exceed the projection.
        let billed = TokenUsage {
            input: 21_400,
            ..TokenUsage::default()
        };
        assert!(cost_of(MODEL, billed).usd <= cost_of(MODEL, projected).usd);
    }

    #[test]
    fn audit_excerpts_are_capped_per_result_and_per_scan() {
        let mut excerpts = AuditExcerpts {
            remaining_chars: MAX_LOGGED_TOOL_RESULT_CHARS + 100,
        };
        assert_eq!(excerpts.take("3 matching lines"), "3 matching lines");

        let source = "z".repeat(5_000);
        let first = excerpts.take(&source);
        assert!(first.starts_with(&"z".repeat(MAX_LOGGED_TOOL_RESULT_CHARS)));
        assert!(first.ends_with(
            "[audit excerpt: 800 of 5000 characters; tool output is not stored in full]"
        ));
        assert_eq!(first.matches('z').count(), MAX_LOGGED_TOOL_RESULT_CHARS);

        let second = excerpts.take(&source);
        assert_eq!(second.matches('z').count(), 100 - "3 matching lines".len());
        let third = excerpts.take(&source);
        assert_eq!(
            third,
            "\n[audit excerpt: 0 of 5000 characters; tool output is not stored in full]"
        );
    }
}
