//! Scans that use Jev System One: the real pipeline, storage, Anthropic client
//! and TypeSafe client, with both APIs mocked over HTTP.

mod common;

use std::time::{Duration, Instant};

use adscan_engine::application::AppError;
use adscan_model::{
    ApiKeyStatus, ApiProvider, DeviceProfile, FindingSource, LogEntry, LogKind, LogLevel,
    PipelineNode, ScanDetail, ScanStatus, ScriptCategory, ScriptRecord, SettingsPatch, StepStatus,
};
use common::*;
use serde_json::{Value, json};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

const ANTHROPIC_KEY: &str = "sk-ant-jev-pipeline-test";
const TYPESAFE_KEY: &str = "typesafe-jev-pipeline-test";
const HAIKU: &str = "claude-haiku-5-5";
const SONNET: &str = "claude-sonnet-5-5";
const JEV: &str = "jev-1.14.0";
/// What the TypeSafe mock reports per request.
const JEV_INPUT_TOKENS: u64 = 1_000;
const JEV_CALL_COST: f64 = JEV_INPUT_TOKENS as f64 * 0.042 / 1e6;
const SONNET_TURN_COST: f64 = (1000.0 * 2.0 + 200.0 * 10.0) / 1e6;

// -- the TypeSafe mock -----------------------------------------------------------

type Scoring = fn(code: &str, question: &str) -> f64;

/// Answers every question of a request with `scoring(state.code, question id)`.
fn judged(request: &Value, scoring: Scoring) -> ResponseTemplate {
    let code = request["state"]["code"].as_str().unwrap();
    let answers: serde_json::Map<String, Value> = request["questions"]
        .as_object()
        .unwrap()
        .keys()
        .map(|id| {
            (
                id.clone(),
                json!({ "type": "noul", "noul": scoring(code, id) }),
            )
        })
        .collect();
    ResponseTemplate::new(200).set_body_json(json!({
        "model": JEV,
        "answers": answers,
        "usage": { "input_tokens": JEV_INPUT_TOKENS, "output_tokens": 12 }
    }))
}

struct Typesafe<F>(F);

impl<F: Fn(&Value) -> ResponseTemplate + Send + Sync> Respond for Typesafe<F> {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        (self.0)(&serde_json::from_slice(&request.body).unwrap())
    }
}

/// A TypeSafe API that accepts only `TYPESAFE_KEY`.
async fn typesafe(
    respond: impl Fn(&Value) -> ResponseTemplate + Send + Sync + 'static,
) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .and(header(
            "authorization",
            format!("Bearer {TYPESAFE_KEY}").as_str(),
        ))
        .respond_with(Typesafe(respond))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .respond_with(unauthenticated())
        .mount(&server)
        .await;
    server
}

fn unauthenticated() -> ResponseTemplate {
    ResponseTemplate::new(401).set_body_json(json!({ "detail": {
        "error_type": "authentication_error",
        "message": "Cannot authenticate with the server. Please check your API key and try again."
    }}))
}

/// The lazy-loading script is the publisher's ad code, the wrapper holds the
/// scroll gate, and nothing else has to do with ads.
fn page_scoring(code: &str, question: &str) -> f64 {
    if code.contains("innerWidth") {
        0.9
    } else if code.contains("dataset.adunit") {
        match question {
            "ad_orchestration" => 0.93,
            "publisher_specific" => 0.8,
            _ => 0.1,
        }
    } else {
        0.02
    }
}

async fn judged_requests(server: &MockServer) -> Vec<Value> {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|request| serde_json::from_slice::<Value>(&request.body).unwrap())
        .collect()
}

fn question_count(request: &Value) -> usize {
    request["questions"].as_object().unwrap().len()
}

/// Triage asks two questions per request, the reading guide eight.
fn triage_requests(requests: &[Value]) -> Vec<&Value> {
    requests
        .iter()
        .filter(|request| question_count(request) == 2)
        .collect()
}

// -- the Anthropic mock ----------------------------------------------------------

struct Anthropic {
    /// What the filter model answers; a request for it is an error otherwise.
    haiku: Option<ResponseTemplate>,
}

impl Respond for Anthropic {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        match body["model"].as_str() {
            Some(HAIKU) => self.haiku.clone().unwrap_or_else(|| {
                ResponseTemplate::new(400).set_body_json(json!({
                    "type": "error",
                    "error": { "type": "invalid_request_error", "message": "the filter model was not expected" }
                }))
            }),
            _ => message(
                json!([{ "type": "tool_use", "id": "toolu_done", "name": "finish", "input": {
                    "overview": "Nothing proven."
                }}]),
                "tool_use",
                json!({ "input_tokens": 1000, "output_tokens": 200 }),
            ),
        }
    }
}

fn message(content: Value, stop_reason: &str, usage: Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({
        "id": "msg_test", "type": "message", "role": "assistant",
        "content": content, "stop_reason": stop_reason, "usage": usage,
    }))
}

fn haiku_says(scripts: Value) -> ResponseTemplate {
    message(
        json!([{ "type": "text", "text": json!({ "scripts": scripts }).to_string() }]),
        "end_turn",
        json!({ "input_tokens": 2000, "output_tokens": 100 }),
    )
}

/// The analysis model finishes at once; the filter model answers `haiku`.
async fn anthropic(haiku: Option<ResponseTemplate>) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(Anthropic { haiku })
        .mount(&server)
        .await;
    server
}

async fn requests_for(server: &MockServer, model: &str) -> Vec<Value> {
    judged_requests(server)
        .await
        .into_iter()
        .filter(|body| body["model"] == model)
        .collect()
}

// -- scans -------------------------------------------------------------------------

fn page_with(extra_scripts: &[(&str, &str)]) -> CaptureBehavior {
    let mut capture = scroll_gated_capture(DeviceProfile::Desktop);
    for (url, body) in extra_scripts {
        capture.scripts.push(script(url, body));
    }
    CaptureBehavior::Return(Box::new(capture))
}

async fn jev_harness(
    behavior: CaptureBehavior,
    anthropic: Option<&MockServer>,
    typesafe: &MockServer,
) -> Harness {
    let anthropic_base_url = anthropic.map_or("http://127.0.0.1:1".to_string(), MockServer::uri);
    let harness = harness_with_providers(
        behavior,
        Providers {
            anthropic_base_url: &anthropic_base_url,
            api_key: anthropic.map(|_| ANTHROPIC_KEY),
            typesafe_base_url: &typesafe.uri(),
            typesafe_key: Some(TYPESAFE_KEY),
        },
    );
    harness
        .settings
        .update(SettingsPatch {
            jev_enabled: Some(true),
            ..SettingsPatch::default()
        })
        .await
        .unwrap();
    harness
}

fn step(detail: &ScanDetail, node: PipelineNode) -> &adscan_model::PipelineStep {
    detail.steps.iter().find(|step| step.node == node).unwrap()
}

fn script_record<'a>(detail: &'a ScanDetail, id: &str) -> &'a ScriptRecord {
    detail
        .scripts
        .iter()
        .find(|script| script.id == id)
        .unwrap()
}

fn classifications(logs: &[LogEntry]) -> Vec<&LogEntry> {
    logs.iter()
        .filter(|entry| entry.kind == LogKind::Classification)
        .collect()
}

fn warnings(logs: &[LogEntry]) -> Vec<&LogEntry> {
    logs.iter()
        .filter(|entry| entry.level == LogLevel::Warn)
        .collect()
}

// -- the Jev path -------------------------------------------------------------------

#[tokio::test]
async fn jev_classifies_the_undecided_script_and_guides_the_analysis() {
    let claude = anthropic(None).await;
    let jev = typesafe(|request| judged(request, page_scoring)).await;
    let harness = jev_harness(page_with(&[]), Some(&claude), &jev).await;

    let detail = run_scan(&harness, scan_options(true)).await;

    assert_eq!(detail.summary.status, ScanStatus::Completed);
    assert_eq!(
        detail
            .steps
            .iter()
            .map(|step| step.status)
            .collect::<Vec<_>>(),
        [StepStatus::Completed; 9]
    );
    assert!(
        requests_for(&claude, HAIKU).await.is_empty(),
        "the filter model is not asked when Jev answers"
    );

    // The script the knowledge base could not decide.
    let lazy = script_record(&detail, "s5");
    assert_eq!(
        (
            lazy.category,
            lazy.ad_related,
            lazy.classified_by,
            lazy.analyzed
        ),
        (ScriptCategory::Wrapper, true, FindingSource::Ai, true)
    );
    assert_eq!(lazy.classifier_model.as_deref(), Some(JEV));
    assert_eq!(lazy.ad_probability, Some(0.93));
    assert_eq!(lazy.classification_confidence, Some(0.93));
    assert_eq!(
        lazy.classification_reason.as_deref(),
        Some(
            "Jev: ad orchestration 0.93 in its best-scoring part, publisher-specific 0.80; rule jev-yes (>= 0.50)"
        )
    );
    // Scripts the knowledge base decided are untouched.
    let wrapper = script_record(&detail, "s3");
    assert_eq!(
        (
            wrapper.classified_by,
            &wrapper.classifier_model,
            wrapper.ad_probability
        ),
        (FindingSource::Heuristic, &None, None)
    );

    // What Jev was sent: one triage request for s5, then one guide request per
    // chunk of the three scripts handed to the analysis.
    let requests = judged_requests(&jev).await;
    let triage = triage_requests(&requests);
    assert_eq!(triage.len(), 1);
    assert_eq!(triage[0]["model"], JEV);
    assert_eq!(
        triage[0]["state"],
        json!({
            "script": { "url": "https://pub.example/js/lazy.js", "firstParty": true },
            "code": "function lazy(el){if(el.dataset.adunit){observe(el)}}",
        })
    );
    assert_eq!(
        triage[0]["questions"]
            .as_object()
            .unwrap()
            .keys()
            .collect::<Vec<_>>(),
        ["ad_orchestration", "publisher_specific"]
    );
    let guide: Vec<&Value> = requests
        .iter()
        .filter(|request| question_count(request) == 8)
        .collect();
    assert_eq!(
        guide.len(),
        3,
        "the wrapper, the lazy loader and the edges of Prebid"
    );
    assert_eq!(requests.len(), 4);
    assert!(
        guide
            .iter()
            .all(|request| request["state"]["script"]["lines"]
                .as_str()
                .unwrap()
                .contains(" of ")),
        "guide chunks are addressed by the line numbers the analysis reads"
    );

    // Cost: Jev is billed on what it reports, per step, and the scan total is the sum.
    let filter = step(&detail, PipelineNode::FilterScripts);
    assert_eq!(filter.model.as_deref(), Some(JEV));
    assert_eq!((filter.input_tokens, filter.output_tokens), (1000, 12));
    assert!(
        (filter.cost_usd - JEV_CALL_COST).abs() < 1e-12,
        "{}",
        filter.cost_usd
    );
    assert_eq!(
        filter.summary.as_deref(),
        Some("Classified 1 of 1 undecided scripts; 4 scripts are ad-related")
    );
    let analyze = step(&detail, PipelineNode::AnalyzeLogic);
    assert_eq!(
        analyze.model.as_deref(),
        Some("jev-1.14.0, claude-sonnet-5-5"),
        "a step that used two models names both"
    );
    assert!(
        (analyze.cost_usd - (3.0 * JEV_CALL_COST + SONNET_TURN_COST)).abs() < 1e-12,
        "{}",
        analyze.cost_usd
    );
    assert!(
        (detail.summary.cost_usd - (4.0 * JEV_CALL_COST + SONNET_TURN_COST)).abs() < 1e-12,
        "{}",
        detail.summary.cost_usd
    );
    let step_costs: f64 = detail.steps.iter().map(|step| step.cost_usd).sum();
    assert!((step_costs - detail.summary.cost_usd).abs() < 1e-12);

    // The analysis model starts from the guide and the inlined source.
    let opening = requests_for(&claude, SONNET).await[0]["messages"][0]["content"][0]["text"]
        .as_str()
        .unwrap()
        .to_string();
    let guide_section = opening
        .split("# Reading guide\n")
        .nth(1)
        .unwrap()
        .split("\n\n")
        .next()
        .unwrap();
    assert!(guide_section.contains("It is a hint and can be wrong or incomplete"));
    let ranked: Vec<&str> = guide_section.lines().skip(1).collect();
    assert_eq!(
        ranked.len(),
        1,
        "only the wrapper's chunk ranks high enough: {ranked:?}"
    );
    assert!(
        ranked[0].starts_with("s3 L1-")
            && ranked[0].ends_with("  auction gating 0.90, device detection 0.90, timing 0.90"),
        "{}",
        ranked[0]
    );
    assert!(opening.contains("# Source to start with\n"));

    // Audit trail: the question set once, one decision per script, no key.
    let logs = harness.scans.logs(&detail.summary.id, None).await.unwrap();
    let asked = logs
        .iter()
        .find(|entry| {
            entry.kind == LogKind::LlmRequest && entry.node == PipelineNode::FilterScripts
        })
        .unwrap();
    assert!(
        asked
            .message
            .starts_with("Jev System One: 1 scripts, up to 1 parts to judge, 2 questions each.")
    );
    let asked_data = asked.data.as_ref().unwrap();
    assert_eq!(asked_data["model"], JEV);
    assert_eq!(asked_data["questionSet"], "jev-questions-v1");
    assert_eq!(asked_data["questions"].as_array().unwrap().len(), 2);
    assert!(
        asked_data["questions"][0]["instructions"]
            .as_str()
            .unwrap()
            .starts_with("Does the JavaScript in `code` call or configure an advertising stack")
    );
    assert_eq!(
        asked_data["thresholds"],
        json!({ "analyse": 0.5, "drop": 0.15, "publisherSpecific": 0.5 })
    );

    let decisions = classifications(&logs);
    assert_eq!(decisions.len(), 1);
    assert_eq!(
        decisions[0].message,
        "s5 https://pub.example/js/lazy.js: ad orchestration 0.93 (analyse >= 0.50, drop < 0.15) over 1 of 1 parts -> ad-related, Phase 2 candidate [jev-yes]"
    );
    let decision = decisions[0].data.as_ref().unwrap();
    assert_eq!(decision["scriptId"], "s5");
    assert_eq!(decision["classifier"], "jev");
    assert_eq!(decision["model"], JEV);
    assert_eq!(decision["modelAnswered"], JEV);
    assert_eq!(
        decision["answers"],
        json!([
            { "questionId": "ad_orchestration", "pYes": 0.93 },
            { "questionId": "publisher_specific", "pYes": 0.8 },
        ])
    );
    assert_eq!(
        decision["decision"],
        json!({ "adRelated": true, "category": "wrapper", "route": "jev-yes" })
    );
    assert_eq!(
        decision["parts"]["judged"],
        json!([{ "part": 0, "identifiers": ["adunit"], "adOrchestration": 0.93, "publisherSpecific": 0.8 }])
    );
    assert_eq!(
        decision["excerpt"],
        "function lazy(el){if(el.dataset.adunit){observe(el)}}"
    );

    let usage: Vec<&Value> = logs
        .iter()
        .filter(|entry| entry.message.starts_with("Token usage"))
        .filter_map(|entry| entry.data.as_ref())
        .collect();
    assert_eq!(
        usage
            .iter()
            .filter(|data| data["provider"] == "typesafe" && data["usageEstimated"] == false)
            .count(),
        4
    );
    assert_eq!(
        usage
            .iter()
            .filter(|data| data["provider"] == "anthropic")
            .count(),
        1
    );

    let persisted = format!(
        "{}{}",
        serde_json::to_string(&detail).unwrap(),
        serde_json::to_string(&logs).unwrap()
    );
    assert!(!persisted.contains(TYPESAFE_KEY) && !persisted.contains(ANTHROPIC_KEY));

    // A developer finds the decision by rule, script or question.
    for query in ["jev-yes", "lazy.js", "ad_orchestration"] {
        let hits = harness.scans.search_logs(query, None).await.unwrap();
        assert!(
            hits.iter()
                .any(|hit| hit.entry.kind == LogKind::Classification),
            "{query}"
        );
    }
}

#[tokio::test]
async fn probabilities_route_scripts_to_analysis_to_the_end_of_the_queue_or_out() {
    let claude = anthropic(None).await;
    let jev = typesafe(|request| {
        judged(request, |code, question| match question {
            _ if code.contains("innerWidth") => 0.9,
            "ad_orchestration" if code.contains("maybeAds") => 0.3,
            "publisher_specific" if code.contains("maybeAds") => 0.9,
            "ad_orchestration" if code.contains("carousel") => 0.05,
            "ad_orchestration" if code.contains("trackPixel") => 0.2,
            "publisher_specific" if code.contains("trackPixel") => 0.1,
            _ => page_scoring(code, question),
        })
    })
    .await;
    let page = page_with(&[
        (
            "https://pub.example/js/maybe.js",
            "function maybeAds(c){return c.prebid}",
        ),
        (
            "https://cdn.widgets.example/carousel.js",
            "carousel({bidder:null})",
        ),
        (
            "https://cdn.pixel.example/events.js",
            "trackPixel({adunit:location.pathname})",
        ),
    ]);
    let harness = jev_harness(page, Some(&claude), &jev).await;

    let detail = run_scan(&harness, scan_options(true)).await;

    let unsure = script_record(&detail, "s6");
    assert_eq!(
        (unsure.category, unsure.ad_related, unsure.analyzed),
        (ScriptCategory::Unknown, true, true),
        "a script Jev is unsure about is kept for the analysis"
    );
    assert_eq!(unsure.ad_probability, Some(0.3));
    assert_eq!(unsure.classification_confidence, Some(0.3));
    assert!(
        unsure
            .classification_reason
            .as_ref()
            .unwrap()
            .contains("rule jev-unsure")
    );

    let dropped = script_record(&detail, "s7");
    assert_eq!(
        (dropped.category, dropped.ad_related, dropped.analyzed),
        (ScriptCategory::Library, false, false)
    );
    assert_eq!(dropped.ad_probability, Some(0.05));
    assert_eq!(dropped.classification_confidence, Some(0.95));
    assert_eq!(dropped.classifier_model.as_deref(), Some(JEV));

    // Not ruled out by its ad score, but generic third-party code: dropped.
    let generic = script_record(&detail, "s8");
    assert_eq!(
        (generic.category, generic.ad_related, generic.analyzed),
        (ScriptCategory::Library, false, false)
    );
    assert_eq!(generic.ad_probability, Some(0.2));
    assert_eq!(generic.classification_confidence, Some(0.8));
    assert!(
        generic
            .classification_reason
            .as_ref()
            .unwrap()
            .contains("rule jev-generic"),
        "{:?}",
        generic.classification_reason
    );

    // A strong knowledge-base signature is never put to Jev's vote.
    let triaged: Vec<String> = triage_requests(&judged_requests(&jev).await)
        .iter()
        .map(|request| {
            request["state"]["script"]["url"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect();
    assert_eq!(triaged.len(), 4);
    assert!(
        !triaged.iter().any(|url| url.ends_with("/ads.js")),
        "{triaged:?}"
    );

    // The analysis sees publisher code first, the unsure script after it and
    // the vendor library last.
    let opening = requests_for(&claude, SONNET).await[0]["messages"][0]["content"][0]["text"]
        .as_str()
        .unwrap()
        .to_string();
    let index: Vec<&str> = opening
        .split("# Script index\n")
        .nth(1)
        .unwrap()
        .lines()
        .take_while(|line| !line.is_empty())
        .map(|line| line.split(" | ").next().unwrap())
        .collect();
    assert_eq!(index, ["s3", "s5", "s6", "s1"]);

    let logs = harness.scans.logs(&detail.summary.id, None).await.unwrap();
    let routes: Vec<&str> = classifications(&logs)
        .iter()
        .map(|entry| {
            entry.data.as_ref().unwrap()["decision"]["route"]
                .as_str()
                .unwrap()
        })
        .collect();
    assert_eq!(routes, ["jev-yes", "jev-unsure", "jev-no", "jev-generic"]);
}

#[tokio::test]
async fn identical_scripts_are_judged_once_and_share_the_verdict() {
    let claude = anthropic(None).await;
    let jev = typesafe(|request| judged(request, page_scoring)).await;
    let page = page_with(&[(
        "https://pub.example/js/lazy.js?v=2",
        "function lazy(el){if(el.dataset.adunit){observe(el)}}",
    )]);
    let harness = jev_harness(page, Some(&claude), &jev).await;

    let detail = run_scan(&harness, scan_options(true)).await;

    assert_eq!(triage_requests(&judged_requests(&jev).await).len(), 1);
    assert!(
        step(&detail, PipelineNode::FilterScripts)
            .summary
            .as_ref()
            .unwrap()
            .starts_with("Classified 2 of 2 undecided scripts")
    );
    let (first, copy) = (script_record(&detail, "s5"), script_record(&detail, "s6"));
    assert_eq!(
        (copy.category, copy.ad_probability, &copy.classifier_model),
        (
            first.category,
            first.ad_probability,
            &first.classifier_model
        )
    );
    assert!(
        first.analyzed && !copy.analyzed,
        "the analysis reads one copy"
    );
    let logs = harness.scans.logs(&detail.summary.id, None).await.unwrap();
    assert_eq!(
        classifications(&logs)[0].data.as_ref().unwrap()["sameBodyAs"],
        json!(["s6"])
    );
}

#[tokio::test]
async fn a_part_that_settles_the_script_stops_the_questions_about_its_other_parts() {
    let claude = anthropic(None).await;
    let jev = typesafe(|request| judged(request, |_, _| 0.9)).await;
    // Twelve parts, every one naming an ad identifier.
    let bundle = "render(el.dataset.adunit);".repeat(2_700);
    let page = page_with(&[("https://pub.example/js/bundle.js", bundle.as_str())]);
    let harness = jev_harness(page, Some(&claude), &jev).await;

    let detail = run_scan(&harness, scan_options(true)).await;

    let requests = judged_requests(&jev).await;
    let about_the_bundle = triage_requests(&requests)
        .iter()
        .filter(|request| request["state"]["script"]["url"] == "https://pub.example/js/bundle.js")
        .count();
    assert_eq!(
        about_the_bundle, 4,
        "only the requests already under way when the first answer arrived were sent"
    );
    assert_eq!(
        script_record(&detail, "s6").category,
        ScriptCategory::Wrapper
    );
    let logs = harness.scans.logs(&detail.summary.id, None).await.unwrap();
    let parts = &classifications(&logs)
        .iter()
        .find(|entry| entry.data.as_ref().unwrap()["scriptId"] == "s6")
        .unwrap()
        .data
        .as_ref()
        .unwrap()["parts"];
    assert_eq!(
        (&parts["total"], &parts["chosen"]),
        (&json!(12), &json!(12))
    );
}

// -- Jev off, Jev alone, Jev failing --------------------------------------------------

#[tokio::test]
async fn a_stored_typesafe_key_is_not_used_while_jev_is_off() {
    let claude = anthropic(Some(haiku_says(json!([
        { "id": "s5", "category": "wrapper", "adRelated": true, "confidence": 0.8, "reason": "Observes ad unit containers" }
    ]))))
    .await;
    let jev = typesafe(|request| judged(request, page_scoring)).await;
    let harness = jev_harness(page_with(&[]), Some(&claude), &jev).await;
    harness
        .settings
        .update(SettingsPatch {
            jev_enabled: Some(false),
            ..SettingsPatch::default()
        })
        .await
        .unwrap();

    let detail = run_scan(&harness, scan_options(true)).await;

    assert!(judged_requests(&jev).await.is_empty());
    assert_eq!(requests_for(&claude, HAIKU).await.len(), 1);
    let lazy = script_record(&detail, "s5");
    assert_eq!(lazy.classifier_model.as_deref(), Some(HAIKU));
    assert_eq!(lazy.ad_probability, None);
    assert_eq!(
        step(&detail, PipelineNode::AnalyzeLogic).model.as_deref(),
        Some(SONNET)
    );
}

#[tokio::test]
async fn with_only_a_typesafe_key_scripts_are_classified_and_logic_analysis_is_skipped() {
    let jev = typesafe(|request| judged(request, page_scoring)).await;
    let harness = jev_harness(page_with(&[]), None, &jev).await;

    let detail = run_scan(&harness, scan_options(true)).await;

    assert_eq!(detail.summary.status, ScanStatus::Completed);
    assert!(detail.summary.ai_used);
    assert_eq!(
        step(&detail, PipelineNode::FilterScripts).status,
        StepStatus::Completed
    );
    for node in [PipelineNode::Beautify, PipelineNode::AnalyzeLogic] {
        assert_eq!(
            (
                step(&detail, node).status,
                step(&detail, node).summary.as_deref()
            ),
            (
                StepStatus::Skipped,
                Some("No Anthropic API key is configured")
            ),
            "{node:?}"
        );
    }
    assert_eq!(
        script_record(&detail, "s5").classifier_model.as_deref(),
        Some(JEV)
    );
    assert_eq!(
        judged_requests(&jev).await.len(),
        1,
        "no reading guide without an analysis"
    );
    assert!((detail.summary.cost_usd - JEV_CALL_COST).abs() < 1e-12);
}

#[tokio::test]
async fn a_jev_outage_mid_scan_hands_the_remaining_scripts_to_the_filter_model() {
    let claude = anthropic(Some(haiku_says(json!([
        { "id": "s6", "category": "application", "adRelated": false, "confidence": 0.7, "reason": "Reads a config value, no ad calls" }
    ]))))
    .await;
    let jev = typesafe(|request| {
        if request["state"]["code"]
            .as_str()
            .unwrap()
            .contains("hb_config")
        {
            ResponseTemplate::new(429).insert_header("retry-after", "0")
        } else {
            judged(request, page_scoring)
        }
    })
    .await;
    let page = page_with(&[(
        "https://pub.example/js/promo.js",
        "var x=window.hb_config;render(x)",
    )]);
    let harness = jev_harness(page, Some(&claude), &jev).await;

    let detail = run_scan(&harness, scan_options(true)).await;

    assert_eq!(detail.summary.status, ScanStatus::Completed);
    let filter = step(&detail, PipelineNode::FilterScripts);
    assert_eq!(filter.status, StepStatus::Completed);
    assert_eq!(
        filter.model.as_deref(),
        Some("jev-1.14.0, claude-haiku-5-5")
    );
    assert_eq!(
        filter.summary.as_deref(),
        Some("Classified 2 of 2 undecided scripts; 4 scripts are ad-related")
    );
    assert_eq!(
        script_record(&detail, "s5").classifier_model.as_deref(),
        Some(JEV)
    );
    let fallen_back = script_record(&detail, "s6");
    assert_eq!(fallen_back.classifier_model.as_deref(), Some(HAIKU));
    assert_eq!(fallen_back.ad_probability, None);
    assert_eq!(fallen_back.category, ScriptCategory::Application);

    let haiku = requests_for(&claude, HAIKU).await;
    assert_eq!(haiku.len(), 1);
    let prompt = haiku[0]["messages"][0]["content"][0]["text"]
        .as_str()
        .unwrap();
    assert!(prompt.contains("<script id=\"s6\">") && !prompt.contains("<script id=\"s5\">"));

    let logs = harness.scans.logs(&detail.summary.id, None).await.unwrap();
    let warning = warnings(&logs)
        .into_iter()
        .find(|entry| entry.node == PipelineNode::FilterScripts)
        .unwrap();
    assert_eq!(
        warning.message,
        "Jev System One failed during script triage (rate limited by the API after 3 attempts); 1 scripts fall back to claude-haiku-5-5"
    );
    assert_eq!(
        warning.data,
        Some(json!({
            "from": JEV,
            "to": HAIKU,
            "reason": "rate limited by the API after 3 attempts",
            "remainingScripts": 1,
        }))
    );
    assert!(
        detail
            .overview
            .as_ref()
            .unwrap()
            .contains("**Note:** Jev System One failed during script triage"),
        "{:?}",
        detail.overview
    );

    // After the first failure the scan asks Jev nothing more: no reading guide.
    let requests = judged_requests(&jev).await;
    assert_eq!(
        requests.len(),
        4,
        "one answer and three attempts at the failing part"
    );
    assert!(requests.iter().all(|request| question_count(request) == 2));
    assert_eq!(
        step(&detail, PipelineNode::AnalyzeLogic).model.as_deref(),
        Some(SONNET)
    );
}

#[tokio::test]
async fn a_rejected_typesafe_key_without_an_anthropic_key_leaves_scripts_to_the_knowledge_base() {
    let jev = typesafe(|_| unauthenticated()).await;
    let harness = jev_harness(page_with(&[]), None, &jev).await;

    let detail = run_scan(&harness, scan_options(true)).await;

    assert_eq!(detail.summary.status, ScanStatus::Completed);
    let filter = step(&detail, PipelineNode::FilterScripts);
    assert_eq!(
        (filter.status, filter.summary.as_deref()),
        (
            StepStatus::Completed,
            Some("Classified 0 of 1 undecided scripts; 3 scripts are ad-related")
        )
    );
    let lazy = script_record(&detail, "s5");
    assert_eq!(
        (lazy.classified_by, lazy.ad_related, &lazy.classifier_model),
        (FindingSource::Heuristic, false, &None)
    );
    assert!(
        lazy.classification_reason
            .as_ref()
            .unwrap()
            .contains("not classified by AI")
    );
    assert_eq!(
        judged_requests(&jev).await.len(),
        1,
        "a rejected key is not retried"
    );
    assert_eq!(detail.summary.cost_usd, 0.0);
    assert_eq!(
        step(&detail, PipelineNode::AnalyzeLogic).summary.as_deref(),
        Some("No Anthropic API key is configured")
    );

    let logs = harness.scans.logs(&detail.summary.id, None).await.unwrap();
    let warning = &warnings(&logs)[0];
    assert!(
        warning.message.starts_with(
            "Jev System One failed during script triage (authentication_error: Cannot authenticate"
        ) && warning
            .message
            .ends_with("1 scripts stay as the knowledge base left them"),
        "{}",
        warning.message
    );
    assert_eq!(warning.data.as_ref().unwrap()["to"], "the knowledge base");
    assert!(!serde_json::to_string(&logs).unwrap().contains(TYPESAFE_KEY));
}

#[tokio::test]
async fn when_jev_and_the_filter_model_both_fail_the_scan_degrades_to_heuristics() {
    let claude = anthropic(Some(ResponseTemplate::new(401).set_body_json(json!({
        "type": "error", "error": { "type": "authentication_error", "message": "invalid x-api-key" }
    }))))
    .await;
    let jev = typesafe(|_| ResponseTemplate::new(529)).await;
    let harness = jev_harness(page_with(&[]), Some(&claude), &jev).await;

    let detail = run_scan(&harness, scan_options(true)).await;

    assert_eq!(detail.summary.status, ScanStatus::Completed);
    let filter = step(&detail, PipelineNode::FilterScripts);
    assert_eq!(
        (filter.status, filter.summary.as_deref()),
        (
            StepStatus::Failed,
            Some("The Anthropic API key was rejected: invalid x-api-key")
        )
    );
    for node in [PipelineNode::Beautify, PipelineNode::AnalyzeLogic] {
        assert_eq!(
            step(&detail, node).summary.as_deref(),
            Some("Skipped because an earlier AI step failed")
        );
    }
    assert!(
        !detail.findings.is_empty(),
        "the deterministic findings stand"
    );
    assert_eq!(
        script_record(&detail, "s5").classified_by,
        FindingSource::Heuristic
    );
}

#[tokio::test]
async fn an_answer_jev_did_not_give_properly_is_a_failure_not_a_verdict() {
    let claude = anthropic(Some(haiku_says(json!([
        { "id": "s5", "category": "wrapper", "adRelated": true, "confidence": 0.8, "reason": "Observes ad unit containers" }
    ]))))
    .await;
    let jev = typesafe(|_| {
        ResponseTemplate::new(200).set_body_json(json!({
            "model": JEV,
            "answers": { "ad_orchestration": { "type": "noul", "noul": 0.01 } },
            "usage": { "input_tokens": 900, "output_tokens": 3 }
        }))
    })
    .await;
    let harness = jev_harness(page_with(&[]), Some(&claude), &jev).await;

    let detail = run_scan(&harness, scan_options(true)).await;

    let lazy = script_record(&detail, "s5");
    assert_eq!(lazy.classifier_model.as_deref(), Some(HAIKU));
    assert!(lazy.ad_related, "the half-answered 0.01 dropped nothing");
    let logs = harness.scans.logs(&detail.summary.id, None).await.unwrap();
    assert!(warnings(&logs)[0].message.contains(
        "unexpected API response: question `publisher_specific` has no yes/no probability between 0 and 1"
    ));
}

// -- budget, usage, cancellation ----------------------------------------------------

#[tokio::test]
async fn a_budget_too_small_for_jev_never_sends_the_script_to_the_dearer_filter_model() {
    let claude = anthropic(None).await;
    let jev = typesafe(|request| judged(request, page_scoring)).await;
    let harness = jev_harness(page_with(&[]), Some(&claude), &jev).await;
    harness
        .settings
        .update(SettingsPatch {
            budget_usd: Some(0.000_001),
            ..SettingsPatch::default()
        })
        .await
        .unwrap();

    let detail = run_scan(&harness, scan_options(true)).await;

    assert_eq!(detail.summary.status, ScanStatus::Completed);
    assert!(judged_requests(&jev).await.is_empty());
    assert!(requests_for(&claude, HAIKU).await.is_empty());
    assert!(requests_for(&claude, SONNET).await.is_empty());
    assert_eq!(detail.summary.cost_usd, 0.0);
    assert_eq!(
        step(&detail, PipelineNode::FilterScripts)
            .summary
            .as_deref(),
        Some(
            "Classified 0 of 1 undecided scripts; 3 scripts are ad-related (1 scripts were not classified by AI because the scan budget was reached)"
        )
    );
    assert_eq!(
        script_record(&detail, "s5").classified_by,
        FindingSource::Heuristic
    );
}

#[tokio::test]
async fn a_request_that_fails_after_the_budget_ran_out_does_not_send_scripts_to_the_filter_model() {
    let claude = anthropic(None).await;
    // The one request the budget pays for is still under way when the next
    // one turns out to be unaffordable, and then fails.
    let jev = typesafe(|_| {
        ResponseTemplate::new(400)
            .set_delay(Duration::from_millis(300))
            .set_body_json(json!({ "detail": "Body is not valid UTF-8" }))
    })
    .await;
    let page = page_with(&[(
        "https://pub.example/js/promo.js",
        "var x=window.hb_config;render(x)",
    )]);
    let harness = jev_harness(page, Some(&claude), &jev).await;
    harness
        .settings
        .update(SettingsPatch {
            // One triage request of a short script is estimated at about $0.00003.
            budget_usd: Some(0.000_05),
            ..SettingsPatch::default()
        })
        .await
        .unwrap();

    let detail = run_scan(&harness, scan_options(true)).await;

    assert_eq!(detail.summary.status, ScanStatus::Completed);
    assert_eq!(judged_requests(&jev).await.len(), 1);
    assert!(requests_for(&claude, HAIKU).await.is_empty());
    assert_eq!(
        step(&detail, PipelineNode::FilterScripts)
            .summary
            .as_deref(),
        Some(
            "Classified 0 of 2 undecided scripts; 3 scripts are ad-related (2 scripts were not classified by AI because the scan budget was reached)"
        )
    );
    let logs = harness.scans.logs(&detail.summary.id, None).await.unwrap();
    assert!(
        warnings(&logs)
            .iter()
            .all(|entry| !entry.message.contains("fall back")),
        "nothing is handed to the dearer model: {:?}",
        warnings(&logs)
            .iter()
            .map(|entry| &entry.message)
            .collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn usage_jev_does_not_report_is_estimated_and_marked_as_an_estimate() {
    let jev = typesafe(|request| {
        let code = request["state"]["code"].as_str().unwrap();
        ResponseTemplate::new(200).set_body_json(json!({
            "model": JEV,
            "answers": {
                "ad_orchestration": { "type": "noul", "noul": page_scoring(code, "ad_orchestration") },
                "publisher_specific": { "type": "noul", "noul": 0.8 },
            },
        }))
    })
    .await;
    let harness = jev_harness(page_with(&[]), None, &jev).await;

    let detail = run_scan(&harness, scan_options(true)).await;

    let filter = step(&detail, PipelineNode::FilterScripts);
    assert!(
        filter.input_tokens > 300 && filter.input_tokens < 2_000,
        "about the request's size plus the fixed overhead, got {}",
        filter.input_tokens
    );
    assert!(filter.cost_usd > 0.0 && filter.cost_usd < 0.0001);
    let logs = harness.scans.logs(&detail.summary.id, None).await.unwrap();
    let usage = logs
        .iter()
        .find(|entry| entry.message.starts_with("Token usage"))
        .unwrap();
    assert_eq!(usage.data.as_ref().unwrap()["usageEstimated"], true);
    assert_eq!(usage.data.as_ref().unwrap()["provider"], "typesafe");
}

#[tokio::test]
async fn cancelling_while_jev_is_answering_ends_the_scan_as_cancelled() {
    let claude = anthropic(None).await;
    let jev =
        typesafe(|request| judged(request, page_scoring).set_delay(Duration::from_secs(20))).await;
    let harness = jev_harness(page_with(&[]), Some(&claude), &jev).await;

    let started = Instant::now();
    let summary = harness.scans.start(scan_options(true)).await.unwrap();
    while judged_requests(&jev).await.is_empty() {
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "Jev was never asked"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    harness.scans.cancel(&summary.id).await.unwrap();
    let detail = wait_for_end(&harness, &summary.id).await;

    assert_eq!(detail.summary.status, ScanStatus::Cancelled);
    assert!(started.elapsed() < Duration::from_secs(10));
    let filter = step(&detail, PipelineNode::FilterScripts);
    assert_eq!(
        (filter.status, filter.summary.as_deref()),
        (StepStatus::Failed, Some("Cancelled"))
    );
    assert!(requests_for(&claude, HAIKU).await.is_empty());
}

// -- no raw payloads on the Jev path ---------------------------------------------------

fn distinct_marks(text: &str, prefix: &str) -> usize {
    text.match_indices(prefix)
        .map(|(at, _)| &text[at + prefix.len()..at + prefix.len() + 4])
        .collect::<std::collections::BTreeSet<_>>()
        .len()
}

#[tokio::test]
async fn persisted_audit_trail_of_a_jev_scan_holds_bounded_excerpts_never_script_bodies() {
    const MARKS: usize = 2000;
    // Every statement names an ad identifier, so Jev is sent every part of the script.
    let undecided: String = (0..MARKS)
        .map(|n| format!("window.lazyMark{n:04}=el.dataset.adunit;"))
        .collect();
    let claude = anthropic(None).await;
    // Unsure about everything: no part settles the script, and it goes on to
    // the analysis, where the reading guide asks about its chunks again.
    let jev = typesafe(|request| {
        judged(request, |_, question| match question {
            "publisher_specific" => 0.9,
            _ => 0.3,
        })
    })
    .await;
    let page = page_with(&[("https://pub.example/js/bundle.js", undecided.as_str())]);
    let harness = jev_harness(page, Some(&claude), &jev).await;

    let detail = run_scan(&harness, scan_options(true)).await;
    assert_eq!(detail.summary.status, ScanStatus::Completed);
    assert!(script_record(&detail, "s6").analyzed);

    // Jev itself did get the whole script, once for triage and once for the guide.
    let requests = judged_requests(&jev).await;
    let sent = |questions: usize| -> String {
        requests
            .iter()
            .filter(|request| question_count(request) == questions)
            .map(|request| request["state"]["code"].as_str().unwrap())
            .collect()
    };
    assert_eq!(distinct_marks(&sent(2), "lazyMark"), MARKS);
    assert!(distinct_marks(&sent(8), "lazyMark") > MARKS * 9 / 10);

    let logs = harness.scans.logs(&detail.summary.id, None).await.unwrap();
    let decisions = classifications(&logs);
    assert_eq!(decisions.len(), 2);
    for decision in &decisions {
        let excerpt = decision.data.as_ref().unwrap()["excerpt"].as_str().unwrap();
        assert!(
            excerpt.chars().count() <= 300,
            "a classification keeps at most 300 characters of source, got {}",
            excerpt.chars().count()
        );
    }
    for entry in &logs {
        let stored = format!(
            "{}{}",
            entry.message,
            entry
                .data
                .as_ref()
                .map(Value::to_string)
                .unwrap_or_default()
        );
        assert!(
            distinct_marks(&stored, "lazyMark") <= 300 / "window.lazyMark0000=".len(),
            "one audit entry holds one short excerpt at most: {:?} {}",
            entry.kind,
            entry.message.chars().take(120).collect::<String>()
        );
    }

    // Everything the scan left in the database, as one text.
    let persisted = format!(
        "{}{}",
        serde_json::to_string(&detail).unwrap(),
        serde_json::to_string(&logs).unwrap()
    );
    let marks = distinct_marks(&persisted, "lazyMark");
    assert!(
        marks > 0 && marks < MARKS / 20,
        "{marks} of {MARKS} statements of the classified script were persisted"
    );
}

#[tokio::test]
async fn the_excerpts_of_one_scan_are_bounded_however_many_scripts_are_classified() {
    const SCRIPTS: usize = 120;
    // Each script is longer than one excerpt and unlike every other.
    let bodies: Vec<(String, String)> = (0..SCRIPTS)
        .map(|n| {
            (
                format!("https://pub.example/js/widget{n}.js"),
                format!(
                    "var w{n}=el.dataset.adunit;{}",
                    format!("mark{n:03}();").repeat(40)
                ),
            )
        })
        .collect();
    let extra: Vec<(&str, &str)> = bodies
        .iter()
        .map(|(url, body)| (url.as_str(), body.as_str()))
        .collect();
    let jev = typesafe(|request| judged(request, |_, _| 0.02)).await;
    let harness = jev_harness(page_with(&extra), None, &jev).await;

    let detail = run_scan(&harness, scan_options(true)).await;
    assert_eq!(detail.summary.status, ScanStatus::Completed);

    let logs = harness.scans.logs(&detail.summary.id, None).await.unwrap();
    let decisions = classifications(&logs);
    assert_eq!(
        decisions.len(),
        SCRIPTS + 1,
        "every script still has its entry"
    );
    let excerpt_chars: usize = decisions
        .iter()
        .map(|decision| {
            decision.data.as_ref().unwrap()["excerpt"]
                .as_str()
                .unwrap()
                .chars()
                .count()
        })
        .sum();
    assert_eq!(excerpt_chars, 24_000);
    let last = decisions.last().unwrap().data.as_ref().unwrap();
    assert_eq!(last["excerpt"], "");
    assert_eq!(last["decision"]["route"], "jev-no");
}

// -- settings ---------------------------------------------------------------------------

#[tokio::test]
async fn jev_can_only_be_on_while_a_typesafe_key_is_stored() {
    let jev = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .and(header("authorization", "Bearer typesafe-good"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "models": [] })))
        .mount(&jev)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(unauthenticated())
        .mount(&jev)
        .await;
    let harness = harness_with_providers(
        page_with(&[]),
        Providers {
            anthropic_base_url: "http://127.0.0.1:1",
            api_key: Some(ANTHROPIC_KEY),
            typesafe_base_url: &jev.uri(),
            typesafe_key: None,
        },
    );
    let enable = || SettingsPatch {
        jev_enabled: Some(true),
        ..SettingsPatch::default()
    };

    assert_eq!(
        harness.settings.update(enable()).await,
        Err(AppError::InvalidInput(
            "Add a TypeSafe API key before turning on Jev.".into()
        ))
    );

    let rejected = harness
        .settings
        .set_api_key(ApiProvider::Typesafe, "typesafe-bad")
        .await
        .unwrap();
    assert_eq!(
        rejected,
        ApiKeyStatus {
            valid: false,
            message: Some("TypeSafe rejected this API key.".into())
        }
    );
    assert!(!harness.settings.get().await.unwrap().has_typesafe_key);

    let accepted = harness
        .settings
        .set_api_key(ApiProvider::Typesafe, " typesafe-good ")
        .await
        .unwrap();
    assert!(accepted.valid);
    let settings = harness.settings.get().await.unwrap();
    assert!(settings.has_typesafe_key && settings.has_api_key);
    assert!(!settings.jev_enabled, "saving a key does not turn Jev on");
    assert_eq!(
        harness
            .settings
            .api_key(ApiProvider::Typesafe)
            .await
            .as_deref(),
        Some("typesafe-good")
    );
    assert_eq!(
        harness
            .settings
            .api_key(ApiProvider::Anthropic)
            .await
            .as_deref(),
        Some(ANTHROPIC_KEY)
    );

    assert!(harness.settings.update(enable()).await.unwrap().jev_enabled);
    let stored = serde_json::to_string(&harness.settings.get().await.unwrap()).unwrap();
    assert!(
        !stored.contains("typesafe-good"),
        "the key is never part of the settings"
    );

    harness
        .settings
        .clear_api_key(ApiProvider::Typesafe)
        .await
        .unwrap();
    let settings = harness.settings.get().await.unwrap();
    assert!(!settings.has_typesafe_key && !settings.jev_enabled);
    assert!(settings.has_api_key, "the Anthropic key is untouched");
    // Storing a key again does not bring the old choice back.
    harness
        .settings
        .set_api_key(ApiProvider::Typesafe, "typesafe-good")
        .await
        .unwrap();
    assert!(!harness.settings.get().await.unwrap().jev_enabled);
}
