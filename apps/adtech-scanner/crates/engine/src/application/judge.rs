//! Shape of a call to a decision model: one state, several yes/no questions
//! about it, one probability per question. Nothing here resembles a chat
//! turn, which is why it is not an [`LlmRequest`](super::llm::LlmRequest).

use serde_json::{Value, json};

use crate::domain::pricing::TokenUsage;
use crate::domain::questions::{CHARS_PER_TOKEN, REQUEST_OVERHEAD_TOKENS, YesNoQuestion};

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
            input: ((self.state.to_string().len() + question_chars) as u64)
                .div_ceil(CHARS_PER_TOKEN)
                + REQUEST_OVERHEAD_TOKENS,
            ..TokenUsage::default()
        }
    }
}

/// The questions in full, as the audit trail records them once per step.
pub fn questions_for_audit(questions: &[YesNoQuestion]) -> Value {
    questions
        .iter()
        .map(|question| {
            json!({
                "id": question.id,
                "instructions": question.instructions,
                "yes": question.yes,
                "no": question.no,
            })
        })
        .collect()
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::chunks::TEXT_CHUNK_CHARS;
    use crate::domain::questions::triage_questions;

    #[test]
    fn a_full_triage_part_is_estimated_no_lower_than_it_was_billed() {
        let request = JudgeRequest {
            model: "jev-1.14.0".into(),
            state: json!({
                "script": { "url": "https://pub.example/a.js", "firstParty": false },
                "code": "x".repeat(TEXT_CHUNK_CHARS),
            }),
            questions: triage_questions(),
        };
        // The real run billed 6,000-character parts at 2,470 to 3,110 tokens.
        let estimate = request.estimated_usage().input;
        assert!((3_110..4_500).contains(&estimate), "{estimate}");
    }
}
