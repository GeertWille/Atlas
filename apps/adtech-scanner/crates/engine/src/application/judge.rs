//! Shape of a call to a decision model: one state, several yes/no questions
//! about it, one probability per question. Nothing here resembles a chat
//! turn, which is why it is not an [`LlmRequest`](super::llm::LlmRequest).

use serde_json::Value;

use crate::domain::pricing::{TokenUsage, estimate_tokens};
use crate::domain::questions::{REQUEST_OVERHEAD_TOKENS, YesNoQuestion};

#[derive(Debug, Clone, PartialEq)]
pub struct JudgeRequest {
    pub model: String,
    /// What the questions are about. Billed once however many questions follow.
    pub state: Value,
    pub questions: &'static [YesNoQuestion],
}

impl JudgeRequest {
    /// What the request will be billed, as well as it can be told beforehand.
    pub fn estimated_usage(&self) -> TokenUsage {
        let question_chars: usize = self
            .questions
            .iter()
            .map(|question| question.instructions.len() + question.yes.len() + question.no.len())
            .sum();
        TokenUsage {
            input: estimate_tokens(self.state.to_string().len() + question_chars)
                + REQUEST_OVERHEAD_TOKENS,
            ..TokenUsage::default()
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct JudgeResponse {
    /// Probability of yes per question, in the order of the request.
    pub probabilities: Vec<f64>,
    /// The versioned model that answered, which can differ from an alias asked for.
    pub answered_by: String,
    pub usage: TokenUsage,
    /// False when the API reported no usage and the caller has to estimate it.
    pub usage_reported: bool,
}
