//! Export of a finished scan: structure of the JSON and Markdown documents.

mod common;

use adscan_engine::application::AppError;
use adscan_model::{DeviceProfile, ExportFormat, ScanDetail};
use common::*;
use serde_json::json;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

fn turn(content: serde_json::Value, stop_reason: &str) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({
        "content": content,
        "stop_reason": stop_reason,
        "usage": { "input_tokens": 1000, "output_tokens": 200 }
    }))
}

/// One Phase 1 answer, then a reported finding, then `finish`.
async fn scripted_api() -> MockServer {
    let server = MockServer::start().await;
    let classification = json!({ "scripts": [
        { "id": "s5", "category": "application", "adRelated": false, "confidence": 0.9, "reason": "UI helper" }
    ] });
    let finding = json!({
        "title": "Inverted device check | delays bidding",
        "severity": "critical",
        "category": "logicBug",
        "description": "Desktop takes the scroll-gated path.",
        "recommendation": "Use `<= 768`.",
        "impact": "No bids before scroll on desktop",
        "confidence": 0.9,
        "estimated_revenue_impact_pct": 20,
        "evidence": [{ "script_id": "s3", "line": 1, "excerpt": "return window.innerWidth > 768", "explanation": "Inverted comparison" }],
        "affected_slots": ["top"],
        "affected_bidders": []
    });
    let responses = [
        turn(
            json!([{ "type": "text", "text": classification.to_string() }]),
            "end_turn",
        ),
        turn(
            json!([
                { "type": "thinking", "thinking": "The comparison in isMobile is inverted.", "signature": "s" },
                { "type": "tool_use", "id": "t1", "name": "report_finding", "input": finding }
            ]),
            "tool_use",
        ),
        turn(
            json!([{ "type": "tool_use", "id": "t2", "name": "finish", "input": { "overview": "Fix the device check first." } }]),
            "tool_use",
        ),
    ];
    for response in responses {
        Mock::given(method("POST"))
            .respond_with(response)
            .up_to_n_times(1)
            .mount(&server)
            .await;
    }
    server
}

async fn exported_scan() -> (Harness, ScanDetail, MockServer) {
    let server = scripted_api().await;
    let capture = CaptureBehavior::Return(Box::new(scroll_gated_capture(DeviceProfile::Desktop)));
    let harness = harness(capture, &server.uri(), Some("sk-ant-export-key"));
    let detail = run_scan(&harness, scan_options(true)).await;
    (harness, detail, server)
}

#[tokio::test]
async fn json_export_is_the_scan_detail() {
    let (harness, detail, _server) = exported_scan().await;
    let document = harness
        .scans
        .export(&detail.summary.id, ExportFormat::Json)
        .await
        .unwrap();
    let parsed: ScanDetail = serde_json::from_str(&document).unwrap();
    assert_eq!(parsed, detail);
    let raw: serde_json::Value = serde_json::from_str(&document).unwrap();
    assert_eq!(
        raw["summary"]["healthScore"], 73,
        "contract casing is camelCase"
    );
    assert!(raw["findings"][0]["evidence"][0]["scriptUrl"].is_string());
    assert!(!document.contains("sk-ant-export-key"));
    assert!(!document.contains(WRAPPER_SOURCE));
}

#[tokio::test]
async fn markdown_report_has_every_section_in_order() {
    let (harness, detail, _server) = exported_scan().await;
    let report = harness
        .scans
        .export(&detail.summary.id, ExportFormat::Markdown)
        .await
        .unwrap();

    let headings = [
        "# Ad stack scan: pub.example",
        "## Summary",
        "## Metrics",
        "## Findings",
        "### 1. Inverted device check | delays bidding",
        "## Auctions",
        "## Ad slots",
        "## Ad-related scripts",
        "## How this report was produced",
        "### AI reasoning trail",
    ];
    let mut cursor = 0;
    for heading in headings {
        let position = report[cursor..]
            .find(heading)
            .unwrap_or_else(|| panic!("missing or out of order: {heading}\n{report}"));
        cursor += position + heading.len();
    }

    // Summary and metrics
    assert!(report.contains("- **URL:** https://pub.example/"));
    assert!(report.contains("- **Health score:** 73/100"));
    assert!(report.contains("- **Findings:** 1 critical, 0 high, 0 medium, 0 low, 0 info"));
    assert!(report.contains("- **AI analysis:** used (LLM cost $0.0"));
    assert!(report.contains("Fix the device check first."));
    assert!(report.contains("| Time to first auction | 4060 ms |"));
    assert!(report.contains("| Fill rate | 100% |"));

    // Findings with evidence
    assert!(report.contains("**Severity:** Critical | **Category:** LogicBug | **Source:** Ai | **Confidence:** 90% | **Estimated revenue at risk:** 20%"));
    assert!(report.contains("*Code evidence: Inverted comparison (https://pub.example/js/ads.js:"));
    assert!(report.contains("```js\n"));
    assert!(report.contains("return window.innerWidth > 768;"));
    assert!(report.contains("*Timeline evidence: Auction timeline*"));
    assert!(report.contains("**Affected slots:** top, side"));

    // Tables: one row per bid, per slot, per ad script, per pipeline step
    let rows = |prefix: &str| {
        report
            .lines()
            .filter(|line| line.starts_with(prefix))
            .count()
    };
    assert_eq!(rows("| 1 | 4060 |"), 6);
    assert!(report.contains("| 1 | 4060 | 340 | top | appnexus | Rendered | 1.50 | 295 |"));
    assert!(report.contains("| side | /123/side | Filled | 4410 | 4600 | yes | no |"));
    assert!(report.contains("| https://pub.example/js/ads.js | Wrapper | - |"));
    assert!(
        !report.contains("| https://pub.example/js/app.js |"),
        "only ad-related scripts are listed"
    );
    assert_eq!(
        rows("| Browser launch |") + rows("| Page capture |") + rows("| Synthesis |"),
        3
    );
    assert!(report.contains("| Logic analysis | Completed |"));
    assert!(report.contains("claude-sonnet-5-5 | 2000 / 400 | $0.0080 |"));

    // How the AI got there
    assert!(report.contains("- **Reasoning:** The comparison in isMobile is inverted."));
    assert!(report.contains("- **Tool call:** report_finding("));
    assert!(report.contains("- **Finding:** [Critical] Inverted device check | delays bidding"));
    assert!(!report.contains("sk-ant-export-key"));
}

#[tokio::test]
async fn markdown_report_of_a_heuristics_only_scan_omits_the_ai_trail() {
    let capture = CaptureBehavior::Return(Box::new(healthy_capture(DeviceProfile::Mobile)));
    let harness = harness(capture, "http://127.0.0.1:1", None);
    let mut options = scan_options(false);
    options.device = DeviceProfile::Mobile;
    let detail = run_scan(&harness, options).await;
    let report = harness
        .scans
        .export(&detail.summary.id, ExportFormat::Markdown)
        .await
        .unwrap();
    assert!(report.contains("- **Device profile:** Mobile"));
    assert!(report.contains("- **AI analysis:** not used (LLM cost $0.0000)"));
    assert!(report.contains("## Findings\n\nNo findings."));
    assert!(report.contains("| Logic analysis | Skipped | - | - | 0 / 0 | $0.0000 | AI analysis is turned off for this scan |"));
    assert!(!report.contains("AI reasoning trail"));
}

#[tokio::test]
async fn exporting_an_unknown_scan_is_not_found() {
    let harness = harness(CaptureBehavior::NoBrowser, "http://127.0.0.1:1", None);
    assert_eq!(
        harness
            .scans
            .export("missing", ExportFormat::Markdown)
            .await,
        Err(AppError::NotFound("Scan missing".into()))
    );
}
