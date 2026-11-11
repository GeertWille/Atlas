//! Provider-neutral shape of a model call, as the pipeline sees it.

use adscan_model::Effort;
use serde_json::Value;

use crate::domain::pricing::TokenUsage;

#[derive(Debug, Clone, PartialEq)]
pub struct LlmRequest {
    pub model: String,
    pub max_tokens: u32,
    pub effort: Effort,
    pub system: String,
    pub tools: Vec<ToolSpec>,
    pub messages: Vec<LlmMessage>,
    /// JSON schema the final answer must follow (structured output).
    pub output_schema: Option<Value>,
    /// Cache the system prompt + tools prefix and the growing conversation.
    /// Only worth it when the same prefix is sent again within minutes.
    pub cache_prefix: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub input_schema: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LlmMessage {
    User(Vec<UserBlock>),
    Assistant(AssistantTurn),
}

#[derive(Debug, Clone, PartialEq)]
pub enum UserBlock {
    Text(String),
    ToolResult {
        tool_use_id: String,
        content: String,
        is_error: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct AssistantTurn {
    /// What the pipeline reads.
    pub blocks: Vec<AssistantBlock>,
    /// The provider's own representation of the turn. It is replayed verbatim
    /// on the next request because thinking blocks must come back unchanged.
    pub replay: Vec<Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AssistantBlock {
    Text(String),
    /// Summarised reasoning, when the provider returns it.
    Thinking(String),
    ToolUse { id: String, name: String, input: Value },
}

impl AssistantTurn {
    pub fn text(&self) -> String {
        self.blocks
            .iter()
            .filter_map(|block| match block {
                AssistantBlock::Text(text) => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum StopReason {
    EndTurn,
    ToolUse,
    MaxTokens,
    /// The provider declined the request; `category` says why when known.
    Refusal { category: Option<String> },
    PauseTurn,
    Other(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct LlmResponse {
    pub turn: AssistantTurn,
    pub stop_reason: StopReason,
    pub usage: TokenUsage,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum LlmError {
    #[error("{0}")]
    ApiKeyInvalid(String),
    #[error("the API key is not allowed to do this: {0}")]
    PermissionDenied(String),
    #[error("billing problem on the Anthropic account: {0}")]
    Billing(String),
    #[error("the request was rejected: {0}")]
    BadRequest(String),
    #[error("model or endpoint not found: {0}")]
    NotFound(String),
    #[error("the request was too large")]
    RequestTooLarge,
    #[error("rate limited by the API after {attempts} attempts")]
    RateLimited { attempts: u32 },
    #[error("the API is overloaded or failing (HTTP {status}) after {attempts} attempts")]
    Unavailable { status: u16, attempts: u32 },
    #[error("could not reach the API: {0}")]
    Network(String),
    #[error("unexpected API response: {0}")]
    InvalidResponse(String),
    #[error("cancelled")]
    Cancelled,
}
