//! What the provider adapters share: the base-URL safety check and sending a
//! request with retries, backoff and cancellation.

use std::time::Duration;

use reqwest::header::{HeaderMap, RETRY_AFTER};
use reqwest::{RequestBuilder, Response, StatusCode};
use serde_json::Value;
use tokio_util::sync::CancellationToken;
use url::{Host, Url};

use crate::application::llm::LlmError;

#[derive(Debug, Clone, Copy)]
pub struct RetryPolicy {
    /// Total tries, including the first.
    pub max_attempts: u32,
    /// Delay before the first retry when the server sends no `retry-after`;
    /// doubles on each further retry.
    pub base_delay: Duration,
    pub max_delay: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 4,
            base_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(20),
        }
    }
}

/// How one provider's API is retried and how its failures read.
pub(super) struct Transport {
    pub retry: RetryPolicy,
    /// Longest wait a `retry-after` header can ask for.
    pub max_retry_after: Duration,
    /// The human-readable message inside the provider's error body.
    pub error_message: fn(&Value) -> Option<String>,
    /// The error for a status that is not worth retrying.
    pub permanent_error: fn(StatusCode, String) -> LlmError,
}

impl Transport {
    /// Sends the request, retrying rate limits, overload, server errors and
    /// connection failures with backoff. Returns the first 2xx response.
    pub async fn send_with_retry(
        &self,
        build: impl Fn() -> RequestBuilder,
        cancel: &CancellationToken,
    ) -> Result<Response, LlmError> {
        let mut attempt = 1;
        loop {
            let outcome = tokio::select! {
                outcome = build().send() => outcome,
                () = cancel.cancelled() => return Err(LlmError::Cancelled),
            };
            let (failure, retry_after) = match outcome {
                Ok(response) if response.status().is_success() => return Ok(response),
                Ok(response) => {
                    let status = response.status();
                    let retry_after = retry_after(response.headers(), self.max_retry_after);
                    let message = self.error_message(response).await;
                    if !is_retryable(status) {
                        return Err((self.permanent_error)(status, message));
                    }
                    (exhausted_error(status, attempt), retry_after)
                }
                Err(error) => (LlmError::Network(describe_transport_error(error)), None),
            };
            if attempt >= self.retry.max_attempts {
                return Err(failure);
            }
            let delay = retry_after.unwrap_or_else(|| self.backoff(attempt));
            tokio::select! {
                () = tokio::time::sleep(delay) => {}
                () = cancel.cancelled() => return Err(LlmError::Cancelled),
            }
            attempt += 1;
        }
    }

    fn backoff(&self, attempt: u32) -> Duration {
        self.retry
            .base_delay
            .saturating_mul(2u32.saturating_pow(attempt - 1))
            .min(self.retry.max_delay)
    }

    /// The provider's own message, or the HTTP reason when the body has none.
    async fn error_message(&self, response: Response) -> String {
        let status = response.status();
        let body: Option<Value> = response.json().await.ok();
        body.as_ref()
            .and_then(self.error_message)
            .unwrap_or_else(|| {
                status
                    .canonical_reason()
                    .unwrap_or("request failed")
                    .to_string()
            })
    }
}

/// Every request carries an API key, so the base URL must be https unless it
/// points at this machine.
pub(super) fn is_safe_base_url(base_url: &str) -> bool {
    let Ok(url) = Url::parse(base_url) else {
        return false;
    };
    let loopback = match url.host() {
        Some(Host::Domain(domain)) => domain == "localhost",
        Some(Host::Ipv4(ip)) => ip.is_loopback(),
        Some(Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    };
    url.scheme() == "https" || (url.scheme() == "http" && loopback)
}

pub(super) fn unsafe_base_url_error() -> LlmError {
    LlmError::Network(
        "the API base URL must use https (plain http is only accepted for localhost)".into(),
    )
}

/// reqwest errors can embed the request URL; keep only what went wrong.
pub(super) fn describe_transport_error(error: reqwest::Error) -> String {
    if error.is_timeout() {
        "the request timed out".into()
    } else if error.is_connect() {
        "could not connect".into()
    } else {
        error.without_url().to_string()
    }
}

fn is_retryable(status: StatusCode) -> bool {
    status == StatusCode::TOO_MANY_REQUESTS
        || status == StatusCode::REQUEST_TIMEOUT
        || status.is_server_error()
}

fn retry_after(headers: &HeaderMap, max: Duration) -> Option<Duration> {
    let seconds: f64 = headers
        .get(RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse()
        .ok()?;
    (seconds.is_finite() && seconds >= 0.0)
        .then(|| Duration::from_secs_f64(seconds.min(max.as_secs_f64())))
}

fn exhausted_error(status: StatusCode, attempts: u32) -> LlmError {
    if status == StatusCode::TOO_MANY_REQUESTS {
        LlmError::RateLimited { attempts }
    } else {
        LlmError::Unavailable {
            status: status.as_u16(),
            attempts,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::HeaderValue;

    fn headers(retry_after: &'static str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(RETRY_AFTER, HeaderValue::from_static(retry_after));
        headers
    }

    #[test]
    fn retry_after_is_read_in_seconds_and_capped() {
        let max = Duration::from_secs(10);
        assert_eq!(
            retry_after(&headers("1.5"), max),
            Some(Duration::from_millis(1500))
        );
        assert_eq!(retry_after(&headers("3600"), max), Some(max));
        assert_eq!(retry_after(&headers("1e300"), max), Some(max));
        assert_eq!(retry_after(&headers("0"), max), Some(Duration::ZERO));
        for unusable in ["-1", "NaN", "soon", "Wed, 21 Oct 2026 07:28:00 GMT"] {
            assert_eq!(retry_after(&headers(unusable), max), None, "{unusable}");
        }
        assert_eq!(retry_after(&HeaderMap::new(), max), None);
    }
}
