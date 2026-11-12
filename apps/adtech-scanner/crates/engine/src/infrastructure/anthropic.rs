//! Claude Messages API adapter over raw HTTP (there is no official Rust SDK).

use std::time::Duration;

use adscan_model::Effort;
use async_trait::async_trait;
use reqwest::{RequestBuilder, StatusCode};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use tokio_util::sync::CancellationToken;

pub use super::http::RetryPolicy;
use super::http::{Transport, is_safe_base_url, unsafe_base_url_error};
use crate::application::llm::{
    AssistantBlock, AssistantTurn, LlmError, LlmMessage, LlmRequest, LlmResponse, StopReason,
    UserBlock,
};
use crate::application::ports::LlmClient;
use crate::domain::pricing::TokenUsage;

pub const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";
const API_VERSION: &str = "2026-10-21";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// Non-streaming turns with thinking can take minutes.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(300);
/// Checking a key is a tiny request the user waits on in Settings.
const VALIDATE_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_RETRY_AFTER: Duration = Duration::from_secs(60);

pub struct AnthropicClient {
    http: reqwest::Client,
    base_url: String,
    transport: Transport,
}

impl AnthropicClient {
    /// The client honours the proxy settings of the environment and, on macOS
    /// and Windows, of the system. `base_url` must be https unless it points at
    /// this machine, because every request carries the API key.
    pub fn new(base_url: &str, retry: RetryPolicy) -> Result<Self, LlmError> {
        if !is_safe_base_url(base_url) {
            return Err(unsafe_base_url_error());
        }
        let http = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .user_agent(concat!("adtech-scanner/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|error| LlmError::Network(error.to_string()))?;
        Ok(Self {
            http,
            base_url: base_url.trim_end_matches('/').to_string(),
            transport: Transport {
                retry,
                max_retry_after: MAX_RETRY_AFTER,
                error_message,
                permanent_error,
            },
        })
    }

    fn authorized(&self, request: RequestBuilder, api_key: &str) -> RequestBuilder {
        request
            .header("x-api-key", api_key)
            .header("anthropic-version", API_VERSION)
    }
}

#[async_trait]
impl LlmClient for AnthropicClient {
    async fn complete(
        &self,
        api_key: &str,
        request: &LlmRequest,
        cancel: &CancellationToken,
    ) -> Result<LlmResponse, LlmError> {
        let body = request_body(request);
        let url = format!("{}/v1/messages", self.base_url);
        let response = self
            .transport
            .send_with_retry(
                || self.authorized(self.http.post(&url), api_key).json(&body),
                cancel,
            )
            .await?;
        let message: MessageDto = tokio::select! {
            parsed = response.json() => parsed.map_err(|error| LlmError::InvalidResponse(error.to_string()))?,
            () = cancel.cancelled() => return Err(LlmError::Cancelled),
        };
        Ok(message.into_response())
    }

    async fn validate_key(&self, api_key: &str) -> Result<(), LlmError> {
        let url = format!("{}/v1/models?limit=1", self.base_url);
        self.transport
            .send_with_retry(
                || {
                    self.authorized(self.http.get(&url), api_key)
                        .timeout(VALIDATE_TIMEOUT)
                },
                &CancellationToken::new(),
            )
            .await
            .map(drop)
    }
}

// -- request ------------------------------------------------------------------

fn request_body(request: &LlmRequest) -> Value {
    let mut system_block = json!({ "type": "text", "text": request.system });
    let mut output_config = json!({ "effort": effort_name(request.effort) });
    if let Some(schema) = &request.output_schema {
        output_config["format"] = json!({ "type": "json_schema", "schema": schema });
    }
    let mut body = json!({
        "model": request.model,
        "max_tokens": request.max_tokens,
        "thinking": { "type": "adaptive", "display": "summarized" },
        "output_config": output_config,
        "messages": request.messages.iter().map(message_json).collect::<Vec<_>>(),
    });
    if request.cache_prefix {
        // Tools render before the system prompt, so this breakpoint covers both;
        // the top-level marker moves along the growing conversation.
        system_block["cache_control"] = json!({ "type": "ephemeral" });
        body["cache_control"] = json!({ "type": "ephemeral" });
    }
    body["system"] = json!([system_block]);
    if !request.tools.is_empty() {
        body["tools"] = request
            .tools
            .iter()
            .map(|tool| {
                json!({
                    "name": tool.name,
                    "description": tool.description,
                    "input_schema": tool.input_schema,
                    "strict": true,
                })
            })
            .collect();
        // Forced tool use is rejected by current models; `auto` plus strict schemas.
        body["tool_choice"] = json!({ "type": "auto" });
    }
    body
}

fn effort_name(effort: Effort) -> &'static str {
    match effort {
        Effort::Low => "low",
        Effort::Medium => "medium",
        Effort::High => "high",
    }
}

fn message_json(message: &LlmMessage) -> Value {
    match message {
        LlmMessage::User(blocks) => json!({
            "role": "user",
            "content": blocks.iter().map(user_block_json).collect::<Vec<_>>(),
        }),
        LlmMessage::Assistant(turn) => json!({ "role": "assistant", "content": turn.replay }),
    }
}

fn user_block_json(block: &UserBlock) -> Value {
    match block {
        UserBlock::Text(text) => json!({ "type": "text", "text": text }),
        UserBlock::ToolResult {
            tool_use_id,
            content,
            is_error,
        } => {
            let mut result =
                json!({ "type": "tool_result", "tool_use_id": tool_use_id, "content": content });
            if *is_error {
                result["is_error"] = json!(true);
            }
            result
        }
    }
}

// -- response -----------------------------------------------------------------

#[derive(Deserialize)]
struct MessageDto {
    #[serde(default)]
    content: Vec<Value>,
    stop_reason: Option<String>,
    stop_details: Option<Map<String, Value>>,
    #[serde(default)]
    usage: UsageDto,
}

#[derive(Deserialize, Default)]
struct UsageDto {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
    #[serde(default)]
    cache_creation_input_tokens: Option<u64>,
    #[serde(default)]
    cache_read_input_tokens: Option<u64>,
}

impl MessageDto {
    fn into_response(self) -> LlmResponse {
        let stop_reason = match self.stop_reason.as_deref() {
            Some("end_turn" | "stop_sequence") => StopReason::EndTurn,
            Some("tool_use") => StopReason::ToolUse,
            Some("max_tokens") => StopReason::MaxTokens,
            Some("pause_turn") => StopReason::PauseTurn,
            Some("refusal") => StopReason::Refusal {
                category: self
                    .stop_details
                    .as_ref()
                    .and_then(|details| details.get("category"))
                    .and_then(Value::as_str)
                    .map(str::to_string),
            },
            other => StopReason::Other(other.unwrap_or("missing").to_string()),
        };
        LlmResponse {
            turn: AssistantTurn {
                blocks: self.content.iter().filter_map(readable_block).collect(),
                replay: self.content,
            },
            stop_reason,
            usage: TokenUsage {
                input: self.usage.input_tokens,
                output: self.usage.output_tokens,
                cache_read: self.usage.cache_read_input_tokens.unwrap_or(0),
                cache_write: self.usage.cache_creation_input_tokens.unwrap_or(0),
            },
        }
    }
}

/// Blocks are read by type, never by position; unknown types are only replayed.
fn readable_block(block: &Value) -> Option<AssistantBlock> {
    let text_field = |name: &str| block.get(name).and_then(Value::as_str).map(str::to_string);
    match block.get("type").and_then(Value::as_str)? {
        "text" => text_field("text").map(AssistantBlock::Text),
        "thinking" => text_field("thinking").map(AssistantBlock::Thinking),
        "tool_use" => Some(AssistantBlock::ToolUse {
            id: text_field("id")?,
            name: text_field("name")?,
            input: block.get("input").cloned().unwrap_or(Value::Null),
        }),
        _ => None,
    }
}

// -- errors -------------------------------------------------------------------

fn permanent_error(status: StatusCode, message: String) -> LlmError {
    match status.as_u16() {
        401 => LlmError::ApiKeyInvalid(message),
        402 => LlmError::Billing(message),
        403 => LlmError::PermissionDenied(message),
        404 => LlmError::NotFound(message),
        413 => LlmError::RequestTooLarge,
        400 | 422 => LlmError::BadRequest(message),
        other => LlmError::InvalidResponse(format!("HTTP {other}: {message}")),
    }
}

/// `error.message` of the API's error envelope.
fn error_message(body: &Value) -> Option<String> {
    body.pointer("/error/message")
        .and_then(Value::as_str)
        .map(str::to_string)
}
