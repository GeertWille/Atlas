//! The TypeSafe System One adapter against a local mock of its API. The real
//! service was never called; the shapes here are the ones its reference
//! documents.

use std::time::{Duration, Instant};

use adscan_engine::application::judge::JudgeRequest;
use adscan_engine::application::llm::LlmError;
use adscan_engine::application::ports::JudgeClient;
use adscan_engine::domain::pricing::TokenUsage;
use adscan_engine::domain::questions::{QUESTIONS, triage_questions};
use adscan_engine::infrastructure::http::RetryPolicy;
use adscan_engine::infrastructure::typesafe::{TypesafeClient, default_retry};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const KEY: &str = "typesafe-test-key";
const MODEL: &str = "jev-1.14.0";

fn client(server: &MockServer) -> TypesafeClient {
    TypesafeClient::new(
        &server.uri(),
        RetryPolicy {
            max_attempts: 3,
            base_delay: Duration::from_millis(10),
            max_delay: Duration::from_millis(40),
        },
    )
    .unwrap()
}

fn request() -> JudgeRequest {
    JudgeRequest {
        model: MODEL.into(),
        state: json!({
            "script": { "url": "https://pub.example/js/lazy.js", "firstParty": true },
            "code": "loadAds(window.adUnits)",
        }),
        questions: triage_questions(),
    }
}

fn answers(ad_orchestration: Value, publisher_specific: Value) -> Value {
    json!({
        "model": MODEL,
        "answers": {
            "publisher_specific": publisher_specific,
            "ad_orchestration": ad_orchestration,
        },
        "usage": { "input_tokens": 412, "output_tokens": 9 }
    })
}

fn noul(probability: f64) -> Value {
    json!({ "type": "noul", "noul": probability })
}

fn ok(body: Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(body)
}

fn api_error(status: u16, kind: &str, text: &str) -> ResponseTemplate {
    ResponseTemplate::new(status)
        .set_body_json(json!({ "detail": { "error_type": kind, "message": text } }))
}

async fn judge(
    server: &MockServer,
) -> Result<adscan_engine::application::judge::JudgeResponse, LlmError> {
    client(server)
        .judge(KEY, &request(), &CancellationToken::new())
        .await
}

async fn posts(server: &MockServer) -> usize {
    server.received_requests().await.unwrap().len()
}

#[tokio::test]
async fn request_carries_the_bearer_key_the_state_and_each_question_as_a_noul() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .and(header("authorization", format!("Bearer {KEY}").as_str()))
        .and(header("content-type", "application/json"))
        .respond_with(ok(answers(noul(0.93), noul(0.71))))
        .expect(1)
        .mount(&server)
        .await;

    let response = judge(&server).await.unwrap();

    assert_eq!(
        response.probabilities,
        [0.93, 0.71],
        "answers follow the question order of the request, not the order of the response"
    );
    assert_eq!(response.answered_by, MODEL);
    assert_eq!(
        response.usage,
        TokenUsage {
            input: 412,
            output: 9,
            ..TokenUsage::default()
        }
    );
    assert!(response.usage_reported);

    let body: Value =
        serde_json::from_slice(&server.received_requests().await.unwrap()[0].body).unwrap();
    assert_eq!(body["model"], MODEL);
    assert_eq!(body["state"]["code"], "loadAds(window.adUnits)");
    assert_eq!(body["state"]["script"]["firstParty"], true);
    let questions = body["questions"].as_object().unwrap();
    assert_eq!(questions.len(), 2);
    let ad_orchestration = &questions["ad_orchestration"];
    assert_eq!(ad_orchestration["type"], "noul");
    assert_eq!(ad_orchestration["instructions"], QUESTIONS[0].instructions);
    assert_eq!(ad_orchestration["criteria"]["true"], QUESTIONS[0].yes);
    assert_eq!(ad_orchestration["criteria"]["false"], QUESTIONS[0].no);
    assert_eq!(body.as_object().unwrap().len(), 3, "{body}");
}

#[tokio::test]
async fn the_versioned_model_that_answered_and_missing_usage_are_passed_on() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ok(json!({
            "model": "jev-1.15.0",
            "answers": { "ad_orchestration": noul(0.0), "publisher_specific": noul(1.0) },
            "usage": { "input_tokens": null, "output_tokens": null },
            "something_new": true
        })))
        .mount(&server)
        .await;
    let response = judge(&server).await.unwrap();
    assert_eq!(response.answered_by, "jev-1.15.0");
    assert_eq!(response.probabilities, [0.0, 1.0]);
    assert!(
        !response.usage_reported,
        "the caller has to estimate the cost"
    );
    assert_eq!(response.usage, TokenUsage::default());
}

#[tokio::test]
async fn anything_but_a_probability_per_question_is_an_invalid_response() {
    let wrong_answers = [
        json!({ "model": MODEL, "answers": { "ad_orchestration": noul(0.9) }, "usage": {} }),
        answers(noul(0.9), noul(1.2)),
        answers(noul(0.9), noul(-0.1)),
        answers(noul(0.9), json!({ "type": "noul", "noul": "high" })),
        answers(noul(0.9), json!({ "type": "noul" })),
        answers(
            json!({ "type": "choice", "choice": "wrapper", "confidence": 1.0, "probabilities": { "wrapper": 1.0 } }),
            noul(0.5),
        ),
        answers(noul(0.9), json!({ "type": "something_new", "noul": 0.5 })),
        json!({ "model": MODEL, "usage": { "input_tokens": 1, "output_tokens": 0 } }),
        json!({ "model": MODEL, "answers": ["0.9", "0.5"] }),
    ];
    for body in wrong_answers {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ok(body.clone()))
            .mount(&server)
            .await;
        let error = judge(&server).await.unwrap_err();
        assert!(
            matches!(error, LlmError::InvalidResponse(_)),
            "{body}: {error:?}"
        );
        assert_eq!(
            posts(&server).await,
            1,
            "a wrong answer is not retried: {body}"
        );
    }

    let not_json = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string("<html>gateway</html>"))
        .mount(&not_json)
        .await;
    assert!(matches!(
        judge(&not_json).await.unwrap_err(),
        LlmError::InvalidResponse(_)
    ));
}

#[tokio::test]
async fn a_rejected_key_is_reported_for_401_and_403_without_a_retry() {
    for status in [401, 403] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(api_error(
                status,
                "authentication_error",
                "Cannot authenticate with the server.",
            ))
            .mount(&server)
            .await;
        let error = judge(&server).await.unwrap_err();
        assert_eq!(
            error,
            LlmError::ApiKeyInvalid(
                "authentication_error: Cannot authenticate with the server.".into()
            ),
            "status {status}"
        );
        assert_eq!(posts(&server).await, 1, "status {status}");
    }
}

#[tokio::test]
async fn client_errors_are_mapped_and_not_retried() {
    type Expectation = fn(&LlmError) -> bool;
    let cases: [(u16, Value, Expectation); 6] = [
        (
            400,
            json!({ "detail": "Body is not valid UTF-8" }),
            |e| matches!(e, LlmError::BadRequest(m) if m == "Body is not valid UTF-8"),
        ),
        (
            400,
            json!({ "detail": { "error_type": "max_tokens_exceeded", "message": "too long" } }),
            |e| matches!(e, LlmError::RequestTooLarge),
        ),
        (
            422,
            json!({ "detail": [{ "loc": ["body", "state"], "msg": "Field required", "type": "missing", "input": "pbjs.requestBids()" }] }),
            |e| matches!(e, LlmError::BadRequest(m) if m == "Field required"),
        ),
        (413, json!({}), |e| matches!(e, LlmError::RequestTooLarge)),
        (
            404,
            json!({ "detail": "Not Found" }),
            |e| matches!(e, LlmError::NotFound(m) if m == "Not Found"),
        ),
        (
            418,
            json!("undocumented"),
            |e| matches!(e, LlmError::InvalidResponse(m) if m.starts_with("HTTP 418")),
        ),
    ];
    for (status, body, expected) in cases {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(status).set_body_json(body))
            .mount(&server)
            .await;
        let error = judge(&server).await.unwrap_err();
        assert!(expected(&error), "status {status}: {error:?}");
        assert!(!error.to_string().contains("requestBids"), "{error}");
        assert_eq!(
            posts(&server).await,
            1,
            "status {status} must not be retried"
        );
    }
}

#[tokio::test]
async fn rate_limit_and_overload_are_retried_until_success() {
    for status in [429, 529, 500, 408] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(status).insert_header("retry-after", "0"))
            .up_to_n_times(2)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .respond_with(ok(answers(noul(0.5), noul(0.5))))
            .mount(&server)
            .await;
        assert_eq!(
            judge(&server).await.unwrap().probabilities,
            [0.5, 0.5],
            "status {status}"
        );
        assert_eq!(posts(&server).await, 3, "status {status}");
    }
}

#[tokio::test]
async fn retries_are_bounded_and_end_in_typed_errors() {
    let limited = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(429))
        .mount(&limited)
        .await;
    assert_eq!(
        judge(&limited).await.unwrap_err(),
        LlmError::RateLimited { attempts: 3 }
    );
    assert_eq!(posts(&limited).await, 3);

    let overloaded = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(529))
        .mount(&overloaded)
        .await;
    assert_eq!(
        judge(&overloaded).await.unwrap_err(),
        LlmError::Unavailable {
            status: 529,
            attempts: 3
        }
    );
}

#[tokio::test]
async fn retry_after_sets_the_wait_and_a_long_wait_can_be_cancelled() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "1"))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .respond_with(ok(answers(noul(0.5), noul(0.5))))
        .mount(&server)
        .await;
    let started = Instant::now();
    judge(&server).await.unwrap();
    let waited = started.elapsed();
    assert!(
        waited >= Duration::from_millis(950),
        "waited only {waited:?}"
    );
    assert!(waited < Duration::from_secs(5));

    // Cancelling the scan ends a wait the server asked for.
    let patient = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "3600"))
        .mount(&patient)
        .await;
    let cancel = CancellationToken::new();
    let canceller = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        canceller.cancel();
    });
    let started = Instant::now();
    let error = client(&patient)
        .judge(KEY, &request(), &cancel)
        .await
        .unwrap_err();
    assert_eq!(error, LlmError::Cancelled);
    assert!(started.elapsed() < Duration::from_secs(1));
}

#[tokio::test]
async fn cancellation_interrupts_a_slow_request() {
    let slow = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ok(answers(noul(0.5), noul(0.5))).set_delay(Duration::from_secs(3)))
        .mount(&slow)
        .await;
    let cancel = CancellationToken::new();
    let canceller = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        canceller.cancel();
    });
    let started = Instant::now();
    let error = client(&slow)
        .judge(KEY, &request(), &cancel)
        .await
        .unwrap_err();
    assert_eq!(error, LlmError::Cancelled);
    assert!(started.elapsed() < Duration::from_secs(1));
}

#[tokio::test]
async fn connection_failures_are_retried_then_reported_without_the_key() {
    // Nothing listens on the discard port.
    let client = TypesafeClient::new(
        "http://127.0.0.1:1",
        RetryPolicy {
            max_attempts: 2,
            base_delay: Duration::from_millis(5),
            max_delay: Duration::from_millis(5),
        },
    )
    .unwrap();
    let error = client
        .judge(KEY, &request(), &CancellationToken::new())
        .await
        .unwrap_err();
    assert!(
        matches!(&error, LlmError::Network(message) if !message.contains(KEY) && !message.contains("127.0.0.1")),
        "{error:?}"
    );
}

#[tokio::test]
async fn key_validation_lists_the_models_and_runs_none() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .and(header("authorization", format!("Bearer {KEY}").as_str()))
        .respond_with(ok(json!({ "models": [
            { "name": "jev-latest", "description": "General-purpose system one model.", "release_date": "2026-09-15" }
        ]})))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .and(header("authorization", "Bearer bad-key"))
        .respond_with(api_error(
            401,
            "authentication_error",
            "Cannot authenticate with the server. Please check your API key and try again.",
        ))
        .mount(&server)
        .await;
    // What the live API answers when it does not see a key at all.
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .and(header("authorization", "Bearer forbidden-key"))
        .respond_with(api_error(
            403,
            "authentication_error",
            "Must supply an API key!",
        ))
        .mount(&server)
        .await;

    let client = client(&server);
    assert_eq!(client.validate_key(KEY).await, Ok(()));
    for rejected in ["bad-key", "forbidden-key"] {
        assert!(
            matches!(
                client.validate_key(rejected).await,
                Err(LlmError::ApiKeyInvalid(_))
            ),
            "{rejected}"
        );
    }
    let posted = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|request| request.method.as_str() == "POST")
        .count();
    assert_eq!(posted, 0, "validation must not spend tokens");
}

#[test]
fn the_key_is_never_sent_over_plain_http_to_another_machine() {
    for refused in [
        "http://api.typesafe.ai",
        "http://10.0.0.5:8080",
        "ftp://localhost",
        "not a url",
    ] {
        assert!(
            matches!(
                TypesafeClient::new(refused, default_retry()),
                Err(LlmError::Network(_))
            ),
            "{refused}"
        );
    }
    for accepted in [
        "https://api.typesafe.ai",
        "http://127.0.0.1:4010",
        "http://localhost:4010/",
    ] {
        assert!(
            TypesafeClient::new(accepted, default_retry()).is_ok(),
            "{accepted}"
        );
    }
}

#[test]
fn the_default_retry_matches_the_vendor_sdk() {
    let retry = default_retry();
    assert_eq!(retry.max_attempts, 3);
    assert_eq!(retry.base_delay, Duration::from_millis(500));
    assert_eq!(retry.max_delay, Duration::from_secs(5));
}
