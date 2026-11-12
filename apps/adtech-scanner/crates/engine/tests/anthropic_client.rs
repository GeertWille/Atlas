//! The Anthropic adapter against a local mock of the Messages API.

use std::time::{Duration, Instant};

use adscan_engine::application::llm::{
    AssistantBlock, LlmError, LlmMessage, LlmRequest, StopReason, ToolSpec, UserBlock,
};
use adscan_engine::application::ports::LlmClient;
use adscan_engine::domain::pricing::TokenUsage;
use adscan_engine::infrastructure::anthropic::{AnthropicClient, RetryPolicy};
use adscan_model::Effort;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const KEY: &str = "sk-ant-test-key";

fn client(server: &MockServer) -> AnthropicClient {
    AnthropicClient::new(
        &server.uri(),
        RetryPolicy {
            max_attempts: 3,
            base_delay: Duration::from_millis(10),
            max_delay: Duration::from_millis(40),
        },
    )
    .unwrap()
}

fn request(model: &str) -> LlmRequest {
    LlmRequest {
        model: model.into(),
        max_tokens: 8000,
        effort: Effort::Medium,
        system: "You audit ad stacks.".into(),
        tools: vec![],
        messages: vec![LlmMessage::User(vec![UserBlock::Text("Go".into())])],
        output_schema: None,
        cache_prefix: false,
    }
}

fn message(content: Value, stop_reason: &str) -> Value {
    json!({
        "id": "msg_1",
        "type": "message",
        "role": "assistant",
        "model": "claude-sonnet-5-5",
        "content": content,
        "stop_reason": stop_reason,
        "usage": { "input_tokens": 10, "output_tokens": 5 }
    })
}

fn ok(body: Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(body)
}

fn api_error(status: u16, kind: &str, text: &str) -> ResponseTemplate {
    ResponseTemplate::new(status).set_body_json(json!({
        "type": "error",
        "error": { "type": kind, "message": text },
        "request_id": "req_1"
    }))
}

async fn complete(
    server: &MockServer,
    request: &LlmRequest,
) -> Result<adscan_engine::application::llm::LlmResponse, LlmError> {
    client(server)
        .complete(KEY, request, &CancellationToken::new())
        .await
}

async fn sent_body(server: &MockServer, index: usize) -> Value {
    serde_json::from_slice(&server.received_requests().await.unwrap()[index].body).unwrap()
}

#[tokio::test]
async fn agentic_request_has_the_current_api_shape() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(header("x-api-key", KEY))
        .and(header("anthropic-version", "2026-10-21"))
        .and(header("content-type", "application/json"))
        .respond_with(ok(message(
            json!([{ "type": "text", "text": "done" }]),
            "end_turn",
        )))
        .expect(1)
        .mount(&server)
        .await;

    let thinking = json!({ "type": "thinking", "thinking": "", "signature": "sig-abc" });
    let tool_use = json!({ "type": "tool_use", "id": "toolu_1", "name": "read_script", "input": { "script_id": "s1" } });
    let mut request = request("claude-sonnet-5-5");
    request.effort = Effort::High;
    request.cache_prefix = true;
    request.tools = vec![ToolSpec {
        name: "read_script",
        description: "Read a script",
        input_schema: json!({ "type": "object", "properties": {}, "required": [], "additionalProperties": false }),
    }];
    request.messages.push(LlmMessage::Assistant(
        adscan_engine::application::llm::AssistantTurn {
            blocks: vec![],
            replay: vec![thinking.clone(), tool_use.clone()],
        },
    ));
    request.messages.push(LlmMessage::User(vec![
        UserBlock::ToolResult {
            tool_use_id: "toolu_1".into(),
            content: "1| code".into(),
            is_error: false,
        },
        UserBlock::ToolResult {
            tool_use_id: "toolu_2".into(),
            content: "no such script".into(),
            is_error: true,
        },
    ]));
    complete(&server, &request).await.unwrap();

    let body = sent_body(&server, 0).await;
    assert_eq!(body["model"], "claude-sonnet-5-5");
    assert_eq!(body["max_tokens"], 8000);
    assert_eq!(
        body["thinking"],
        json!({ "type": "adaptive", "display": "summarized" })
    );
    assert_eq!(body["output_config"], json!({ "effort": "high" }));
    for forbidden in ["temperature", "top_p", "top_k", "stream", "fallbacks"] {
        assert!(
            body.get(forbidden).is_none(),
            "{forbidden} must not be sent"
        );
    }
    assert!(body["thinking"].get("budget_tokens").is_none());

    assert_eq!(body["tool_choice"], json!({ "type": "auto" }));
    assert_eq!(body["tools"][0]["name"], "read_script");
    assert_eq!(body["tools"][0]["strict"], true);
    assert_eq!(
        body["tools"][0]["input_schema"]["additionalProperties"],
        false
    );
    assert!(body["tools"][0].get("cache_control").is_none());

    // One breakpoint after the stable tools + system prefix, plus automatic
    // caching for the growing conversation.
    assert_eq!(
        body["system"],
        json!([{ "type": "text", "text": "You audit ad stacks.", "cache_control": { "type": "ephemeral" } }])
    );
    assert_eq!(body["cache_control"], json!({ "type": "ephemeral" }));

    let messages = body["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 3);
    assert_eq!(
        messages[0],
        json!({ "role": "user", "content": [{ "type": "text", "text": "Go" }] })
    );
    assert_eq!(
        messages[1],
        json!({ "role": "assistant", "content": [thinking, tool_use] }),
        "thinking blocks are replayed unchanged"
    );
    assert_eq!(
        messages[2],
        json!({ "role": "user", "content": [
            { "type": "tool_result", "tool_use_id": "toolu_1", "content": "1| code" },
            { "type": "tool_result", "tool_use_id": "toolu_2", "content": "no such script", "is_error": true }
        ] }),
        "all tool results go back in one user message"
    );
    assert_eq!(
        messages.last().unwrap()["role"],
        "user",
        "no assistant prefill"
    );
}

#[tokio::test]
async fn structured_output_request_uses_output_config_format_and_no_caching() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ok(message(
            json!([{ "type": "text", "text": "{}" }]),
            "end_turn",
        )))
        .mount(&server)
        .await;
    let schema = json!({ "type": "object", "properties": {}, "required": [], "additionalProperties": false });
    let mut request = request("claude-haiku-5-5");
    request.effort = Effort::Low;
    request.output_schema = Some(schema.clone());
    complete(&server, &request).await.unwrap();

    let body = sent_body(&server, 0).await;
    assert_eq!(body["model"], "claude-haiku-5-5");
    assert_eq!(
        body["output_config"],
        json!({ "effort": "low", "format": { "type": "json_schema", "schema": schema } })
    );
    assert!(body.get("output_format").is_none());
    assert!(body.get("tools").is_none() && body.get("tool_choice").is_none());
    assert!(body.get("cache_control").is_none());
    assert!(body["system"][0].get("cache_control").is_none());
    assert_eq!(body["thinking"]["type"], "adaptive");
}

#[tokio::test]
async fn response_blocks_are_read_by_type_and_usage_includes_cache_tokens() {
    let server = MockServer::start().await;
    let content = json!([
        { "type": "thinking", "thinking": "The check looks inverted.", "signature": "sig" },
        { "type": "text", "text": "Reading the wrapper." },
        { "type": "future_block", "data": 1 },
        { "type": "tool_use", "id": "toolu_9", "name": "read_script", "input": { "script_id": "s3", "start_line": 1, "end_line": 40 } }
    ]);
    let mut body = message(content.clone(), "tool_use");
    body["usage"] = json!({
        "input_tokens": 120, "output_tokens": 45,
        "cache_creation_input_tokens": 900, "cache_read_input_tokens": 3000
    });
    Mock::given(method("POST"))
        .respond_with(ok(body))
        .mount(&server)
        .await;

    let response = complete(&server, &request("claude-sonnet-5-5"))
        .await
        .unwrap();
    assert_eq!(response.stop_reason, StopReason::ToolUse);
    assert_eq!(
        response.turn.blocks,
        [
            AssistantBlock::Thinking("The check looks inverted.".into()),
            AssistantBlock::Text("Reading the wrapper.".into()),
            AssistantBlock::ToolUse {
                id: "toolu_9".into(),
                name: "read_script".into(),
                input: json!({ "script_id": "s3", "start_line": 1, "end_line": 40 }),
            },
        ]
    );
    assert_eq!(
        Value::Array(response.turn.replay),
        content,
        "unknown blocks are still replayed"
    );
    assert_eq!(
        response.usage,
        TokenUsage {
            input: 120,
            output: 45,
            cache_read: 3000,
            cache_write: 900
        }
    );
}

#[tokio::test]
async fn stop_reasons_are_mapped_including_refusal_category() {
    for (stop_reason, details, expected) in [
        ("end_turn", Value::Null, StopReason::EndTurn),
        ("max_tokens", Value::Null, StopReason::MaxTokens),
        ("pause_turn", Value::Null, StopReason::PauseTurn),
        (
            "refusal",
            json!({ "category": "cyber" }),
            StopReason::Refusal {
                category: Some("cyber".into()),
            },
        ),
        (
            "refusal",
            Value::Null,
            StopReason::Refusal { category: None },
        ),
        (
            "something_new",
            Value::Null,
            StopReason::Other("something_new".into()),
        ),
    ] {
        let server = MockServer::start().await;
        let mut body = message(json!([]), stop_reason);
        body["stop_details"] = details;
        Mock::given(method("POST"))
            .respond_with(ok(body))
            .mount(&server)
            .await;
        let response = complete(&server, &request("claude-sonnet-5-5"))
            .await
            .unwrap();
        assert_eq!(response.stop_reason, expected);
        assert!(response.turn.blocks.is_empty());
    }
}

#[tokio::test]
async fn rate_limit_and_overload_are_retried_until_success() {
    for status in [429, 529, 500, 503] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                api_error(status, "overloaded_error", "busy").insert_header("retry-after", "0"),
            )
            .up_to_n_times(2)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .respond_with(ok(message(
                json!([{ "type": "text", "text": "ok" }]),
                "end_turn",
            )))
            .mount(&server)
            .await;
        let response = complete(&server, &request("claude-sonnet-5-5"))
            .await
            .unwrap();
        assert_eq!(response.turn.text(), "ok", "status {status}");
        assert_eq!(
            server.received_requests().await.unwrap().len(),
            3,
            "status {status}"
        );
    }
}

#[tokio::test]
async fn retry_after_header_sets_the_wait() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            api_error(429, "rate_limit_error", "slow down").insert_header("retry-after", "1"),
        )
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .respond_with(ok(message(json!([]), "end_turn")))
        .mount(&server)
        .await;
    let started = Instant::now();
    complete(&server, &request("claude-sonnet-5-5"))
        .await
        .unwrap();
    let waited = started.elapsed();
    assert!(
        waited >= Duration::from_millis(950),
        "waited only {waited:?}"
    );
    assert!(waited < Duration::from_secs(5));
}

#[tokio::test]
async fn retries_are_bounded_and_end_in_typed_errors() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(api_error(429, "rate_limit_error", "slow down"))
        .mount(&server)
        .await;
    let error = complete(&server, &request("claude-sonnet-5-5"))
        .await
        .unwrap_err();
    assert_eq!(error, LlmError::RateLimited { attempts: 3 });
    assert_eq!(server.received_requests().await.unwrap().len(), 3);

    let overloaded = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(api_error(529, "overloaded_error", "overloaded"))
        .mount(&overloaded)
        .await;
    let error = complete(&overloaded, &request("claude-sonnet-5-5"))
        .await
        .unwrap_err();
    assert_eq!(
        error,
        LlmError::Unavailable {
            status: 529,
            attempts: 3
        }
    );
}

#[tokio::test]
async fn client_errors_are_mapped_and_not_retried() {
    type Expectation = fn(&LlmError) -> bool;
    let cases: [(u16, &str, Expectation); 7] = [
        (
            401,
            "authentication_error",
            |e| matches!(e, LlmError::ApiKeyInvalid(m) if m == "nope"),
        ),
        (402, "billing_error", |e| matches!(e, LlmError::Billing(_))),
        (403, "permission_error", |e| {
            matches!(e, LlmError::PermissionDenied(_))
        }),
        (
            404,
            "not_found_error",
            |e| matches!(e, LlmError::NotFound(m) if m == "nope"),
        ),
        (413, "request_too_large", |e| {
            matches!(e, LlmError::RequestTooLarge)
        }),
        (
            400,
            "invalid_request_error",
            |e| matches!(e, LlmError::BadRequest(m) if m == "nope"),
        ),
        (
            418,
            "teapot",
            |e| matches!(e, LlmError::InvalidResponse(m) if m.contains("418")),
        ),
    ];
    for (status, kind, is_expected) in cases {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(api_error(status, kind, "nope"))
            .mount(&server)
            .await;
        let error = complete(&server, &request("claude-sonnet-5-5"))
            .await
            .unwrap_err();
        assert!(is_expected(&error), "status {status} mapped to {error:?}");
        assert_eq!(
            server.received_requests().await.unwrap().len(),
            1,
            "status {status} must not be retried"
        );
        assert!(!error.to_string().contains(KEY));
    }
}

#[tokio::test]
async fn connection_failures_are_retried_then_reported_without_the_key() {
    // Nothing listens on the discard port.
    let client = AnthropicClient::new(
        "http://127.0.0.1:1",
        RetryPolicy {
            max_attempts: 2,
            base_delay: Duration::from_millis(5),
            max_delay: Duration::from_millis(5),
        },
    )
    .unwrap();
    let error = client
        .complete(
            KEY,
            &request("claude-sonnet-5-5"),
            &CancellationToken::new(),
        )
        .await
        .unwrap_err();
    assert!(
        matches!(&error, LlmError::Network(message) if !message.contains(KEY)),
        "{error:?}"
    );
}

#[tokio::test]
async fn malformed_success_body_is_an_invalid_response() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_string("<html>gateway</html>"))
        .mount(&server)
        .await;
    let error = complete(&server, &request("claude-sonnet-5-5"))
        .await
        .unwrap_err();
    assert!(matches!(error, LlmError::InvalidResponse(_)));
}

#[tokio::test]
async fn cancellation_interrupts_a_slow_request_and_a_backoff_wait() {
    let slow = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ok(message(json!([]), "end_turn")).set_delay(Duration::from_secs(3)))
        .mount(&slow)
        .await;
    let backing_off = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(api_error(429, "rate_limit_error", "wait").insert_header("retry-after", "30"))
        .mount(&backing_off)
        .await;

    for server in [&slow, &backing_off] {
        let cancel = CancellationToken::new();
        let canceller = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            canceller.cancel();
        });
        let started = Instant::now();
        let error = client(server)
            .complete(KEY, &request("claude-sonnet-5-5"), &cancel)
            .await
            .unwrap_err();
        assert_eq!(error, LlmError::Cancelled);
        assert!(started.elapsed() < Duration::from_secs(1));
    }
}

#[tokio::test]
async fn key_validation_is_a_cheap_authenticated_models_call() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .and(query_param("limit", "1"))
        .and(header("x-api-key", KEY))
        .and(header("anthropic-version", "2026-10-21"))
        .respond_with(ok(json!({ "data": [], "has_more": false })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .and(header("x-api-key", "sk-bad"))
        .respond_with(api_error(401, "authentication_error", "invalid x-api-key"))
        .mount(&server)
        .await;

    let client = client(&server);
    assert_eq!(client.validate_key(KEY).await, Ok(()));
    assert_eq!(
        client.validate_key("sk-bad").await,
        Err(LlmError::ApiKeyInvalid("invalid x-api-key".into()))
    );
    let posts = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.method.as_str() == "POST")
        .count();
    assert_eq!(posts, 0, "validation must not spend tokens");
}

#[test]
fn the_key_is_never_sent_over_plain_http_to_another_machine() {
    let retry = RetryPolicy::default();
    for refused in [
        "http://api.anthropic.com",
        "http://10.0.0.5:8080",
        "ftp://localhost",
        "not a url",
    ] {
        assert!(
            matches!(
                AnthropicClient::new(refused, retry),
                Err(LlmError::Network(_))
            ),
            "{refused}"
        );
    }
    for accepted in [
        "https://api.anthropic.com",
        "https://gateway.corp.example/anthropic/",
        "http://127.0.0.1:4010",
        "http://localhost:4010",
        "http://[::1]:4010",
    ] {
        assert!(AnthropicClient::new(accepted, retry).is_ok(), "{accepted}");
    }
}
