//! Executes the Phase 2 tools against in-memory sources and telemetry.

use adscan_model::{
    Capture, Evidence, EvidenceKind, FindingCategory, FindingSource, ScriptCategory, Severity,
};
use regex::{Regex, RegexBuilder};
use serde::Deserialize;
use serde_json::Value;

use super::render;
use crate::domain::finding::FindingDraft;
use crate::domain::telemetry::Telemetry;

const MAX_READ_LINES: usize = 250;
const MAX_LINE_CHARS: usize = 300;
const MAX_SEARCH_MATCHES: usize = 40;
const SEARCH_CONTEXT_LINES: usize = 2;
const MAX_TOOL_OUTPUT_CHARS: usize = 24_000;
const MAX_EVIDENCE_CHARS: usize = 600;
/// The model may have joined or split lines differently from the beautifier.
const CITATION_EXTRA_LINES: usize = 2;

pub struct ScriptSource {
    pub id: String,
    pub url: String,
    pub category: ScriptCategory,
    pub size_bytes: f64,
    pub lines: Vec<String>,
}

struct TelemetrySections {
    auctions: String,
    slots: String,
    prebid_config: String,
    console: String,
    page_events: String,
}

pub struct Toolbox {
    scripts: Vec<ScriptSource>,
    sections: TelemetrySections,
}

#[derive(Debug, PartialEq)]
pub enum ToolEffect {
    None,
    Finding(Box<FindingDraft>),
    Finish { overview: String },
}

#[derive(Debug, PartialEq)]
pub struct ToolOutcome {
    pub content: String,
    pub is_error: bool,
    pub effect: ToolEffect,
}

impl ToolOutcome {
    fn ok(content: String) -> Self {
        Self {
            content: cap_output(content),
            is_error: false,
            effect: ToolEffect::None,
        }
    }

    fn error(message: impl Into<String>) -> Self {
        Self {
            content: message.into(),
            is_error: true,
            effect: ToolEffect::None,
        }
    }
}

#[derive(Deserialize)]
struct ReadScriptInput {
    script_id: String,
    start_line: i64,
    end_line: i64,
}

#[derive(Deserialize)]
struct SearchScriptsInput {
    pattern: String,
    #[serde(default)]
    script_id: Option<String>,
}

#[derive(Deserialize)]
struct TelemetryInput {
    section: String,
}

#[derive(Deserialize)]
struct FinishInput {
    overview: String,
}

#[derive(Deserialize)]
struct FindingInput {
    title: String,
    severity: Severity,
    category: FindingCategory,
    description: String,
    recommendation: String,
    impact: String,
    confidence: f64,
    #[serde(default)]
    estimated_revenue_impact_pct: Option<f64>,
    evidence: Vec<CitationInput>,
    #[serde(default)]
    affected_slots: Vec<String>,
    #[serde(default)]
    affected_bidders: Vec<String>,
}

#[derive(Deserialize)]
struct CitationInput {
    script_id: String,
    line: i64,
    excerpt: String,
    explanation: String,
}

impl Toolbox {
    pub fn new(scripts: Vec<ScriptSource>, capture: &Capture, telemetry: &Telemetry) -> Self {
        Self {
            scripts,
            sections: TelemetrySections {
                auctions: render::auctions_detail(&telemetry.auctions),
                slots: render::slots(&telemetry.slots),
                prebid_config: render::prebid_config(capture),
                console: render::console(capture),
                page_events: render::page_events(capture),
            },
        }
    }

    pub fn script_index(&self) -> String {
        self.scripts
            .iter()
            .map(|script| {
                format!(
                    "{} | {:?} | {} bytes | {} lines | {}",
                    script.id,
                    script.category,
                    script.size_bytes,
                    script.lines.len(),
                    script.url,
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Runs one tool call. Unknown tools and malformed input come back as
    /// error results so the model can correct itself.
    pub fn execute(&self, name: &str, input: &Value) -> ToolOutcome {
        match name.to_ascii_lowercase().as_str() {
            "list_scripts" => ToolOutcome::ok(self.script_index()),
            "read_script" => with_input(input, |args| self.read_script(args)),
            "search_scripts" => with_input(input, |args| self.search_scripts(args)),
            "get_telemetry" => with_input(input, |args| self.get_telemetry(args)),
            "report_finding" => with_input(input, |args| self.report_finding(args)),
            "finish" => with_input(input, |args: FinishInput| ToolOutcome {
                content: "Analysis finished.".into(),
                is_error: false,
                effect: ToolEffect::Finish {
                    overview: args.overview,
                },
            }),
            _ => ToolOutcome::error(format!(
                "Unknown tool `{name}`. Available tools: list_scripts, read_script, search_scripts, get_telemetry, report_finding, finish."
            )),
        }
    }

    fn script(&self, id: &str) -> Result<&ScriptSource, ToolOutcome> {
        self.scripts
            .iter()
            .find(|script| script.id.eq_ignore_ascii_case(id.trim()))
            .ok_or_else(|| {
                ToolOutcome::error(format!(
                    "No script with id `{id}`. Available ids: {}.",
                    self.scripts
                        .iter()
                        .map(|script| script.id.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ))
            })
    }

    fn read_script(&self, args: ReadScriptInput) -> ToolOutcome {
        let script = match self.script(&args.script_id) {
            Ok(script) => script,
            Err(outcome) => return outcome,
        };
        let total = script.lines.len();
        let start = args.start_line.max(1) as usize;
        if start > total {
            return ToolOutcome::error(format!("{} has only {total} lines.", script.id));
        }
        let requested_end = (args.end_line.max(args.start_line).max(1) as usize).min(total);
        let end = requested_end.min(start + MAX_READ_LINES - 1);
        let mut out = format!(
            "{} lines {start}-{end} of {total} ({})\n",
            script.id, script.url
        );
        out.push_str(&numbered(script, start, end));
        if end < requested_end {
            out.push_str(&format!(
                "\n[truncated: {MAX_READ_LINES} lines per call; continue with start_line={}]",
                end + 1
            ));
        }
        ToolOutcome::ok(out)
    }

    fn search_scripts(&self, args: SearchScriptsInput) -> ToolOutcome {
        let scripts: Vec<&ScriptSource> =
            match args.script_id.as_deref().filter(|id| !id.trim().is_empty()) {
                Some(id) => match self.script(id) {
                    Ok(script) => vec![script],
                    Err(outcome) => return outcome,
                },
                None => self.scripts.iter().collect(),
            };
        if args.pattern.is_empty() {
            return ToolOutcome::error("`pattern` must not be empty.");
        }
        let (regex, literal_fallback) = compile_pattern(&args.pattern);
        let mut out = String::new();
        let mut shown = 0;
        let mut total = 0;
        for script in scripts {
            for (index, line) in script.lines.iter().enumerate() {
                if !regex.is_match(line) {
                    continue;
                }
                total += 1;
                if shown == MAX_SEARCH_MATCHES {
                    continue;
                }
                shown += 1;
                let start = index.saturating_sub(SEARCH_CONTEXT_LINES) + 1;
                let end = (index + 1 + SEARCH_CONTEXT_LINES).min(script.lines.len());
                out.push_str(&format!(
                    "--- {} line {}\n{}\n",
                    script.id,
                    index + 1,
                    numbered(script, start, end)
                ));
            }
        }
        if total == 0 {
            return ToolOutcome::ok(format!("No matches for `{}`.", args.pattern));
        }
        let mut header = format!("{total} matching lines for `{}`", args.pattern);
        if literal_fallback {
            header.push_str(" (not a valid regular expression; searched as literal text)");
        }
        if shown < total {
            header.push_str(&format!(
                " [truncated: showing the first {shown}; narrow the pattern or pass script_id]"
            ));
        }
        ToolOutcome::ok(format!("{header}\n{out}"))
    }

    fn get_telemetry(&self, args: TelemetryInput) -> ToolOutcome {
        let section = match args.section.as_str() {
            "auctions" => &self.sections.auctions,
            "slots" => &self.sections.slots,
            "prebid_config" => &self.sections.prebid_config,
            "console" => &self.sections.console,
            "page_events" => &self.sections.page_events,
            other => {
                return ToolOutcome::error(format!(
                    "Unknown section `{other}`. Use auctions, slots, prebid_config, console or page_events."
                ));
            }
        };
        ToolOutcome::ok(section.clone())
    }

    fn report_finding(&self, args: FindingInput) -> ToolOutcome {
        if args.evidence.is_empty() {
            return ToolOutcome::error(
                "Finding rejected: it needs at least one code citation (script_id, line, excerpt) from source you have read.",
            );
        }
        let mut evidence = Vec::with_capacity(args.evidence.len());
        let mut corrections = Vec::new();
        for (position, citation) in args.evidence.iter().enumerate() {
            match self.verify_citation(citation) {
                Ok(verified) => {
                    if verified.line as i64 != citation.line {
                        corrections.push(format!(
                            "citation {} was found at line {} instead of {}",
                            position + 1,
                            verified.line,
                            citation.line
                        ));
                    }
                    evidence.push(verified.evidence);
                }
                Err(problem) => {
                    return ToolOutcome::error(format!(
                        "Finding rejected: citation {} is not valid. {problem} Read the code again with read_script and resubmit the finding with an exact excerpt.",
                        position + 1
                    ));
                }
            }
        }
        let draft = FindingDraft {
            severity: args.severity,
            category: args.category,
            source: FindingSource::Ai,
            title: args.title,
            description: args.description,
            recommendation: args.recommendation,
            confidence: args.confidence,
            impact: args.impact,
            estimated_revenue_impact_pct: args.estimated_revenue_impact_pct,
            evidence,
            affected_slots: args.affected_slots,
            affected_bidders: args.affected_bidders,
        };
        let mut content = "Finding recorded.".to_string();
        if !corrections.is_empty() {
            content.push_str(&format!(" Note: {}.", corrections.join("; ")));
        }
        ToolOutcome {
            content,
            is_error: false,
            effect: ToolEffect::Finding(Box::new(draft)),
        }
    }

    /// The excerpt must really be in the cited script. The stored evidence is
    /// the source text itself, not the model's copy of it.
    fn verify_citation(&self, citation: &CitationInput) -> Result<VerifiedCitation, String> {
        let script = self
            .scripts
            .iter()
            .find(|script| script.id.eq_ignore_ascii_case(citation.script_id.trim()))
            .ok_or_else(|| format!("There is no script `{}`.", citation.script_id))?;
        let needle = squash(&citation.excerpt);
        if needle.is_empty() {
            return Err("The excerpt is empty.".into());
        }
        let total = script.lines.len();
        if citation.line < 1 || citation.line as usize > total {
            return Err(format!(
                "Line {} does not exist; {} has {total} lines.",
                citation.line, script.id
            ));
        }
        let cited = citation.line as usize;
        let span = citation.excerpt.lines().count().max(1) + CITATION_EXTRA_LINES;
        let located = std::iter::once(cited)
            .chain(1..=total)
            .find_map(|start| tightest_match(script, start, span, &needle));
        let Some((line, end)) = located else {
            return Err(format!(
                "The excerpt does not appear in {} (checked at line {cited} and in the whole script).",
                script.id
            ));
        };
        let body: String = script.lines[line - 1..end].join("\n");
        Ok(VerifiedCitation {
            line,
            evidence: Evidence {
                kind: EvidenceKind::Code,
                label: citation.explanation.clone(),
                body: body.chars().take(MAX_EVIDENCE_CHARS).collect(),
                script_url: Some(script.url.clone()),
                line: Some(line as u32),
            },
        })
    }
}

struct VerifiedCitation {
    line: usize,
    evidence: Evidence,
}

fn with_input<T: for<'de> Deserialize<'de>>(
    input: &Value,
    run: impl FnOnce(T) -> ToolOutcome,
) -> ToolOutcome {
    match T::deserialize(input) {
        Ok(args) => run(args),
        Err(error) => ToolOutcome::error(format!(
            "Invalid tool input: {error}. Check the tool's input schema and call it again."
        )),
    }
}

fn compile_pattern(pattern: &str) -> (Regex, bool) {
    let build = |source: &str| {
        RegexBuilder::new(source)
            .case_insensitive(true)
            .size_limit(1 << 20)
            .build()
    };
    match build(pattern) {
        Ok(regex) => (regex, false),
        Err(_) => (
            build(&regex::escape(pattern))
                .unwrap_or_else(|_| Regex::new("$^").expect("static regex")),
            true,
        ),
    }
}

fn numbered(script: &ScriptSource, start: usize, end: usize) -> String {
    script.lines[start - 1..end]
        .iter()
        .enumerate()
        .map(|(offset, line)| format!("{:>5}| {}", start + offset, shorten(line)))
        .collect::<Vec<_>>()
        .join("\n")
}

fn shorten(line: &str) -> String {
    match line.char_indices().nth(MAX_LINE_CHARS) {
        Some((cut, _)) => format!(
            "{} [line truncated, {} chars total]",
            &line[..cut],
            line.chars().count()
        ),
        None => line.to_string(),
    }
}

/// The smallest line range inside `start..start+span` whose text contains the
/// excerpt, ignoring whitespace.
fn tightest_match(
    script: &ScriptSource,
    start: usize,
    span: usize,
    needle: &str,
) -> Option<(usize, usize)> {
    let contains =
        |from: usize, to: usize| squash(&script.lines[from - 1..to].join("\n")).contains(needle);
    let mut first = start;
    let mut last = (start + span - 1).min(script.lines.len());
    if !contains(first, last) {
        return None;
    }
    while first < last && contains(first + 1, last) {
        first += 1;
    }
    while last > first && contains(first, last - 1) {
        last -= 1;
    }
    Some((first, last))
}

/// Whitespace-insensitive comparison form.
fn squash(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

fn cap_output(content: String) -> String {
    match content.char_indices().nth(MAX_TOOL_OUTPUT_CHARS) {
        Some((cut, _)) => format!(
            "{}\n[truncated: output limited to {MAX_TOOL_OUTPUT_CHARS} characters; request a narrower range]",
            &content[..cut]
        ),
        None => content,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use adscan_model::{DeviceProfile, ScanMetrics, Viewport};
    use serde_json::json;

    fn toolbox() -> Toolbox {
        let source = [
            "function isMobile() {",
            "  return window.innerWidth > 768;",
            "}",
            "if (isMobile()) {",
            "  window.addEventListener('scroll', startAuction, { once: true });",
            "} else {",
            "  startAuction();",
            "}",
        ];
        let capture = Capture {
            final_url: "https://pub.example/".into(),
            page_title: None,
            device: DeviceProfile::Desktop,
            viewport: Viewport {
                width: 1440,
                height: 900,
                device_scale_factor: 1.0,
                mobile: false,
            },
            user_agent: "test".into(),
            navigation_start: 0.0,
            page_load_ms: Some(900.0),
            scripts: vec![],
            events: vec![],
            console: vec![],
            max_scroll_depth: 0.0,
            consent_accepted: false,
        };
        let telemetry = Telemetry {
            auctions: vec![],
            slots: vec![],
            metrics: ScanMetrics::default(),
        };
        Toolbox::new(
            vec![ScriptSource {
                id: "s1".into(),
                url: "https://pub.example/ads.js".into(),
                category: ScriptCategory::Wrapper,
                size_bytes: 200.0,
                lines: source.iter().map(|line| line.to_string()).collect(),
            }],
            &capture,
            &telemetry,
        )
    }

    fn finding_input(line: i64, excerpt: &str) -> Value {
        json!({
            "title": "Device check is inverted",
            "severity": "critical",
            "category": "logicBug",
            "description": "d",
            "recommendation": "r",
            "impact": "i",
            "confidence": 0.9,
            "estimated_revenue_impact_pct": null,
            "evidence": [{ "script_id": "s1", "line": line, "excerpt": excerpt, "explanation": "inverted comparison" }],
            "affected_slots": ["top"],
            "affected_bidders": []
        })
    }

    #[test]
    fn read_script_numbers_lines_and_clamps_the_range() {
        let outcome = toolbox().execute(
            "read_script",
            &json!({ "script_id": "s1", "start_line": 2, "end_line": 99 }),
        );
        assert!(!outcome.is_error);
        assert!(outcome.content.starts_with("s1 lines 2-8 of 8"));
        assert!(
            outcome
                .content
                .contains("    2|   return window.innerWidth > 768;")
        );
        let beyond = toolbox().execute(
            "read_script",
            &json!({ "script_id": "s1", "start_line": 50, "end_line": 60 }),
        );
        assert!(beyond.is_error);
    }

    #[test]
    fn read_script_caps_lines_per_call_with_a_truncation_note() {
        let mut toolbox = toolbox();
        toolbox.scripts[0].lines = (0..600).map(|n| format!("line{n}();")).collect();
        let outcome = toolbox.execute(
            "read_script",
            &json!({ "script_id": "s1", "start_line": 1, "end_line": 600 }),
        );
        assert!(outcome.content.contains("  250| line249();"));
        assert!(!outcome.content.contains("line250();"));
        assert!(
            outcome
                .content
                .contains("[truncated: 250 lines per call; continue with start_line=251]")
        );
    }

    #[test]
    fn search_supports_regex_literal_fallback_and_context() {
        let toolbox = toolbox();
        let regex = toolbox.execute(
            "search_scripts",
            &json!({ "pattern": "innerwidth\\s*>", "script_id": null }),
        );
        assert!(regex.content.starts_with("1 matching lines"));
        assert!(regex.content.contains("--- s1 line 2"));
        assert!(regex.content.contains("    4| if (isMobile()) {"));
        let literal = toolbox.execute(
            "search_scripts",
            &json!({ "pattern": "isMobile(", "script_id": "s1" }),
        );
        assert!(literal.content.contains("searched as literal text"));
        assert!(literal.content.starts_with("2 matching lines"));
        let none = toolbox.execute(
            "search_scripts",
            &json!({ "pattern": "requestBids", "script_id": null }),
        );
        assert_eq!(none.content, "No matches for `requestBids`.");
    }

    #[test]
    fn search_results_are_capped() {
        let mut toolbox = toolbox();
        toolbox.scripts[0].lines = (0..100).map(|n| format!("bid({n});")).collect();
        let outcome = toolbox.execute(
            "search_scripts",
            &json!({ "pattern": "bid", "script_id": null }),
        );
        assert!(outcome.content.contains("100 matching lines"));
        assert!(outcome.content.contains("showing the first 40"));
    }

    #[test]
    fn valid_citation_is_recorded_with_the_real_source_text() {
        let outcome = toolbox().execute(
            "report_finding",
            &finding_input(2, "return window.innerWidth>768;"),
        );
        assert!(!outcome.is_error, "{}", outcome.content);
        let ToolEffect::Finding(draft) = outcome.effect else {
            panic!("expected a finding");
        };
        assert_eq!(draft.source, FindingSource::Ai);
        assert_eq!(draft.severity, Severity::Critical);
        assert_eq!(draft.category, FindingCategory::LogicBug);
        let evidence = &draft.evidence[0];
        assert_eq!(evidence.line, Some(2));
        assert_eq!(evidence.body, "  return window.innerWidth > 768;");
        assert_eq!(
            evidence.script_url.as_deref(),
            Some("https://pub.example/ads.js")
        );
    }

    #[test]
    fn citation_on_the_wrong_line_is_relocated_and_the_model_is_told() {
        let outcome = toolbox().execute(
            "report_finding",
            &finding_input(8, "window.addEventListener('scroll', startAuction"),
        );
        assert!(!outcome.is_error);
        assert!(outcome.content.contains("found at line 5 instead of 8"));
        let ToolEffect::Finding(draft) = outcome.effect else {
            panic!("expected a finding");
        };
        assert_eq!(draft.evidence[0].line, Some(5));
    }

    #[test]
    fn invented_citations_are_rejected() {
        let toolbox = toolbox();
        let made_up = toolbox.execute(
            "report_finding",
            &finding_input(2, "if (navigator.userAgent.match(/iPad/))"),
        );
        assert!(made_up.is_error);
        assert_eq!(made_up.effect, ToolEffect::None);
        assert!(made_up.content.contains("does not appear in s1"));

        let bad_line = toolbox.execute("report_finding", &finding_input(400, "startAuction();"));
        assert!(bad_line.content.contains("Line 400 does not exist"));

        let mut unknown_script = finding_input(2, "startAuction();");
        unknown_script["evidence"][0]["script_id"] = json!("s9");
        assert!(
            toolbox
                .execute("report_finding", &unknown_script)
                .content
                .contains("no script `s9`")
        );

        let mut no_evidence = finding_input(2, "x");
        no_evidence["evidence"] = json!([]);
        assert!(
            toolbox
                .execute("report_finding", &no_evidence)
                .content
                .contains("at least one code citation")
        );
    }

    #[test]
    fn malformed_input_and_unknown_tools_are_error_results() {
        let toolbox = toolbox();
        let malformed = toolbox.execute(
            "read_script",
            &json!({ "script_id": "s1", "start_line": "one" }),
        );
        assert!(malformed.is_error);
        assert!(malformed.content.starts_with("Invalid tool input"));
        let unknown = toolbox.execute("run_code", &json!({}));
        assert!(unknown.is_error);
        assert!(unknown.content.contains("Unknown tool `run_code`"));
        let bad_section = toolbox.execute("get_telemetry", &json!({ "section": "cookies" }));
        assert!(bad_section.is_error);
    }

    #[test]
    fn tool_names_are_matched_case_insensitively_and_finish_carries_the_overview() {
        let outcome = toolbox().execute("Finish", &json!({ "overview": "All good." }));
        assert_eq!(
            outcome.effect,
            ToolEffect::Finish {
                overview: "All good.".into()
            }
        );
    }
}
