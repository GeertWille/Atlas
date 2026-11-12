//! TypeSafe System One ("Jev") adapter over raw HTTP.
//!
//! Written from the published API reference only; no call against the real
//! service was possible. Anything the reference leaves open is read
//! defensively: an answer that is not exactly what was asked for is an
//! invalid response, never a verdict.

use std::collections::HashMap;
use std::time::Duration;

use async_trait::async_trait;
use reqwest::StatusCode;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use tokio_util::sync::CancellationToken;

use super::http::{RetryPolicy, Transport, is_safe_base_url, unsafe_base_url_error};
use crate::application::judge::{JudgeRequest, JudgeResponse};
use crate::application::llm::LlmError;
use crate::application::ports::JudgeClient;
use crate::domain::pricing::TokenUsage;

pub const DEFAULT_BASE_URL: &str = "https://api.typesafe.ai";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// Calls usually answer in a fraction of a second; no latency is published
/// for the largest states this client sends.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// A scan has another classifier to fall back on, so a long server-requested
/// wait is not worth honouring.
const MAX_RETRY_AFTER: Duration = Duration::from_secs(10);
/// Error text ends up in the audit trail; the API bounds it, this makes sure.
const MAX_ERROR_MESSAGE_CHARS: usize = 300;

/// The retry behaviour of TypeSafe's own SDKs: two retries, 0.5 s doubling to 5 s.
pub fn default_retry() -> RetryPolicy {
    RetryPolicy {
        max_attempts: 3,
        base_delay: Duration::from_millis(500),
        max_delay: Duration::from_secs(5),
    }
}

pub struct TypesafeClient {
    http: reqwest::Client,
    base_url: String,
    transport: Transport,
}

impl TypesafeClient {
    /// `base_url` must be https unless it points at this machine, because
    /// every request carries the API key.
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
}

#[async_trait]
impl JudgeClient for TypesafeClient {
    async fn judge(
        &self,
        api_key: &str,
        request: &JudgeRequest,
        cancel: &CancellationToken,
    ) -> Result<JudgeResponse, LlmError> {
        let body = request_body(request);
        let url = format!("{}/v1/systemone", self.base_url);
        let response = self
            .transport
            .send_with_retry(
                || self.http.post(&url).bearer_auth(api_key).json(&body),
                cancel,
            )
            .await?;
        let answer: ResponseDto = tokio::select! {
            parsed = response.json() => parsed.map_err(|error| LlmError::InvalidResponse(error.to_string()))?,
            () = cancel.cancelled() => return Err(LlmError::Cancelled),
        };
        answer.into_response(request)
    }

    /// Listing the models runs no model and is not billed.
    async fn validate_key(&self, api_key: &str) -> Result<(), LlmError> {
        let url = format!("{}/v1/models", self.base_url);
        self.transport
            .send_with_retry(
                || self.http.get(&url).bearer_auth(api_key),
                &CancellationToken::new(),
            )
            .await
            .map(drop)
    }
}

fn request_body(request: &JudgeRequest) -> Value {
    let questions: Map<String, Value> = request
        .questions
        .iter()
        .map(|question| {
            (
                question.id.to_string(),
                json!({
                    "type": "noul",
                    "instructions": question.instructions,
                    "criteria": { "true": question.yes, "false": question.no },
                }),
            )
        })
        .collect();
    json!({
        "model": request.model,
        "state": request.state,
        "questions": questions,
    })
}

#[derive(Deserialize)]
struct ResponseDto {
    model: Option<String>,
    answers: HashMap<String, Value>,
    usage: Option<UsageDto>,
}

#[derive(Deserialize)]
struct UsageDto {
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
}

impl ResponseDto {
    fn into_response(self, request: &JudgeRequest) -> Result<JudgeResponse, LlmError> {
        let probabilities = request
            .questions
            .iter()
            .map(|question| {
                self.answers
                    .get(question.id)
                    .and_then(yes_probability)
                    .ok_or_else(|| {
                        LlmError::InvalidResponse(format!(
                            "question `{}` has no yes/no probability between 0 and 1",
                            question.id
                        ))
                    })
            })
            .collect::<Result<Vec<f64>, LlmError>>()?;
        let input_tokens = self.usage.as_ref().and_then(|usage| usage.input_tokens);
        Ok(JudgeResponse {
            probabilities,
            answered_by: self.model.unwrap_or_else(|| request.model.clone()),
            usage: TokenUsage {
                input: input_tokens.unwrap_or(0),
                output: self
                    .usage
                    .and_then(|usage| usage.output_tokens)
                    .unwrap_or(0),
                ..TokenUsage::default()
            },
            usage_reported: input_tokens.is_some(),
        })
    }
}

fn yes_probability(answer: &Value) -> Option<f64> {
    if answer.get("type").and_then(Value::as_str) != Some("noul") {
        return None;
    }
    answer
        .get("noul")
        .and_then(Value::as_f64)
        .filter(|probability| (0.0..=1.0).contains(probability))
}

fn permanent_error(status: StatusCode, message: String) -> LlmError {
    match status.as_u16() {
        // The live API answers 403 where its documentation says 401.
        401 | 403 => LlmError::ApiKeyInvalid(message),
        402 => LlmError::Billing(message),
        404 => LlmError::NotFound(message),
        413 => LlmError::RequestTooLarge,
        400 if message.contains("max_tokens_exceeded") => LlmError::RequestTooLarge,
        400 | 415 | 422 => LlmError::BadRequest(message),
        other => LlmError::InvalidResponse(format!("HTTP {other}: {message}")),
    }
}

/// The API's own words from `detail`, which is a string, an object with
/// `error_type` and `message`, or a list of validation problems. A validation
/// problem's `input` echoes the request and is never read.
fn error_message(body: &Value) -> Option<String> {
    let detail = body.get("detail")?;
    let text = match detail {
        Value::String(text) => text.clone(),
        Value::Object(fields) => {
            let field = |name: &str| fields.get(name).and_then(Value::as_str);
            match (field("error_type"), field("message")) {
                (Some(kind), Some(message)) => format!("{kind}: {message}"),
                (Some(only), None) | (None, Some(only)) => only.to_string(),
                (None, None) => return None,
            }
        }
        Value::Array(problems) => problems
            .iter()
            .filter_map(|problem| problem.get("msg").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("; "),
        _ => return None,
    };
    let text: String = text.chars().take(MAX_ERROR_MESSAGE_CHARS).collect();
    (!text.is_empty()).then_some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_text_is_read_from_every_documented_detail_shape() {
        assert_eq!(
            error_message(&json!({ "detail": "Body is not valid UTF-8" })).as_deref(),
            Some("Body is not valid UTF-8")
        );
        assert_eq!(
            error_message(&json!({ "detail": {
                "error_type": "authentication_error",
                "message": "Cannot authenticate with the server."
            }}))
            .as_deref(),
            Some("authentication_error: Cannot authenticate with the server.")
        );
        assert_eq!(
            error_message(&json!({ "detail": { "error_type": "max_tokens_exceeded" } })).as_deref(),
            Some("max_tokens_exceeded")
        );
        assert_eq!(error_message(&json!({ "detail": 7 })), None);
        assert_eq!(error_message(&json!({ "error": "other envelope" })), None);
    }

    #[test]
    fn validation_errors_never_repeat_the_input_they_complain_about() {
        let body = json!({ "detail": [
            { "loc": ["body", "state"], "msg": "Field required", "type": "missing", "input": "pbjs.requestBids()" },
            { "loc": ["body", "questions"], "msg": "Too few items", "type": "too_short" }
        ]});
        assert_eq!(
            error_message(&body).as_deref(),
            Some("Field required; Too few items")
        );
    }

    #[test]
    fn long_error_text_is_cut() {
        let message = error_message(&json!({ "detail": "x".repeat(5_000) })).unwrap();
        assert_eq!(message.chars().count(), MAX_ERROR_MESSAGE_CHARS);
    }
}
