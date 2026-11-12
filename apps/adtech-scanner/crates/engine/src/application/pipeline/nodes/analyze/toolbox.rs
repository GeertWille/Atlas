//! Executes the Phase 2 tools against in-memory sources and telemetry.

use adscan_model::{
    BidderDiagnostics, Capture, Evidence, EvidenceKind, FindingCategory, FindingSource,
    ScanEnvironment, ScriptCategory, Severity,
};
use regex::{Regex, RegexBuilder};
use serde::Deserialize;
use serde_json::Value;

use super::render::{self, Scope};
use crate::domain::chunks::LineRange;
use crate::domain::finding::FindingDraft;
use crate::domain::telemetry::ScanTelemetry;

const MAX_READ_LINES: usize = 250;
const MAX_LINE_CHARS: usize = 300;
const MAX_SEARCH_MATCHES: usize = 40;
const SEARCH_CONTEXT_LINES: usize = 2;
const MAX_TOOL_OUTPUT_CHARS: usize = 24_000;
const MAX_EVIDENCE_CHARS: usize = 600;
/// Patterns and excerpts come from the model. The regex engine is linear in
/// the text and its compiled size is capped; these bounds keep the inputs
/// themselves small.
const MAX_PATTERN_CHARS: usize = 300;
const REGEX_SIZE_LIMIT: usize = 1 << 20;
const MAX_EXCERPT_CHARS: usize = 2_000;
/// Shorter excerpts (`}`, `else {`) match almost anywhere and prove nothing.
const MIN_EXCERPT_CHARS: usize = 8;

pub struct ScriptSource {
    pub id: String,
    pub url: String,
    pub category: ScriptCategory,
    pub size_bytes: f64,
    pub lines: Vec<String>,
    /// The lines a search of all scripts covers; `None` is every line.
    pub default_search: Option<Vec<LineRange>>,
}

impl ScriptSource {
    /// A stock vendor library: only searched in full when asked for by id.
    pub fn is_vendor_library(&self) -> bool {
        self.default_search.is_some()
    }

    fn in_default_search(&self, line: usize) -> bool {
        self.default_search
            .as_ref()
            .is_none_or(|ranges| ranges.iter().any(|range| range.contains(line)))
    }

    /// What the script index says about the limited search, if anything.
    fn search_note(&self) -> String {
        match self.default_search.as_deref() {
            None => String::new(),
            Some([]) => " | vendor library, searched only when script_id is given".into(),
            Some(ranges) => format!(
                " | vendor library, only lines {} are searched unless script_id is given",
                ranges
                    .iter()
                    .map(|range| format!("{}-{}", range.start, range.end))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }
}

/// Telemetry of the whole visit, rendered once.
struct TelemetrySections {
    pages: String,
    auctions: String,
    slots: String,
    prebid_config: String,
    console: String,
    page_events: String,
    bidder_checks: String,
}

/// The sections that can be asked for one page view at a time.
struct PageSections {
    auctions: String,
    slots: String,
    console: String,
    page_events: String,
}

pub struct Toolbox {
    scripts: Vec<ScriptSource>,
    sections: TelemetrySections,
    /// One entry per page view of the visit.
    page_sections: Vec<PageSections>,
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
    /// Page view, counted from one; absent for the whole visit.
    #[serde(default)]
    page: Option<i64>,
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
    /// Page views the bug shows on, counted from one.
    #[serde(default)]
    pages: Vec<i64>,
}

#[derive(Deserialize)]
struct CitationInput {
    script_id: String,
    line: i64,
    excerpt: String,
    explanation: String,
}

impl Toolbox {
    pub fn new(
        scripts: Vec<ScriptSource>,
        capture: &Capture,
        telemetry: &ScanTelemetry,
        environment: Option<&ScanEnvironment>,
    ) -> Self {
        let visit = Scope::Visit {
            pages: telemetry.pages.len(),
        };
        let page_sections: Vec<PageSections> = (0u32..)
            .zip(&telemetry.pages)
            .map(|(page, _)| {
                let scope = Scope::Page(page);
                PageSections {
                    auctions: render::auctions_detail(&telemetry.total.auctions, scope),
                    slots: render::slots(&telemetry.total.slots, scope),
                    console: render::console(capture, scope),
                    page_events: render::page_events(capture, page),
                }
            })
            .collect();
        // One page view reads without a heading, as it always did.
        let page_events = match page_sections.as_slice() {
            [only] => only.page_events.clone(),
            several => (1usize..)
                .zip(several)
                .map(|(number, page)| format!("## page {number}\n{}", page.page_events))
                .collect::<Vec<_>>()
                .join("\n\n"),
        };
        Self {
            scripts,
            sections: TelemetrySections {
                pages: render::visit(environment, &telemetry.pages),
                auctions: render::auctions_detail(&telemetry.total.auctions, visit),
                slots: render::slots(&telemetry.total.slots, visit),
                prebid_config: render::prebid_config(capture),
                console: render::console(capture, visit),
                page_events,
                bidder_checks: render::bidder_checks(None),
            },
            page_sections,
        }
    }

    /// The visit block of the opening message.
    pub fn visit(&self) -> &str {
        &self.sections.pages
    }

    /// The event timeline of one page view, by index.
    pub fn page_timeline(&self, page: u32) -> Option<&str> {
        self.page_sections
            .get(page as usize)
            .map(|sections| sections.page_events.as_str())
    }

    /// Adds what the bidder checks found, when they ran for this scan.
    pub fn with_bidder_checks(mut self, diagnostics: Option<&BidderDiagnostics>) -> Self {
        self.sections.bidder_checks = render::bidder_checks(diagnostics);
        self
    }

    pub fn script_index(&self) -> String {
        self.scripts
            .iter()
            .map(|script| {
                format!(
                    "{} | {:?} | {} bytes | {} lines | {}{}",
                    script.id,
                    script.category,
                    script.size_bytes,
                    script.lines.len(),
                    script.url,
                    script.search_note(),
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn scripts(&self) -> &[ScriptSource] {
        &self.scripts
    }

    /// The lines as `read_script` shows them, so that line numbers quoted
    /// from this text verify like any other citation.
    pub fn excerpt(&self, script: usize, range: LineRange) -> String {
        let script = &self.scripts[script];
        format!(
            "{} lines {}-{} of {} ({})\n{}",
            script.id,
            range.start,
            range.end,
            script.lines.len(),
            script.url,
            numbered(script, range.start, range.end)
        )
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
        // A named script is searched in full; a search of everything leaves
        // out the vendor-library lines and says how many matches that hid.
        let (scripts, everything): (Vec<&ScriptSource>, bool) =
            match args.script_id.as_deref().filter(|id| !id.trim().is_empty()) {
                Some(id) => match self.script(id) {
                    Ok(script) => (vec![script], false),
                    Err(outcome) => return outcome,
                },
                None => (self.scripts.iter().collect(), true),
            };
        if args.pattern.is_empty() {
            return ToolOutcome::error("`pattern` must not be empty.");
        }
        if args.pattern.chars().count() > MAX_PATTERN_CHARS {
            return ToolOutcome::error(format!(
                "`pattern` is longer than {MAX_PATTERN_CHARS} characters. Search for a short, distinctive token instead."
            ));
        }
        let Some((regex, literal_fallback)) = compile_pattern(&args.pattern) else {
            return ToolOutcome::error(
                "`pattern` is too complex to search with. Use a simpler expression or plain text.",
            );
        };
        let mut out = String::new();
        let mut shown = 0;
        let mut total = 0;
        let mut hidden_notes = Vec::new();
        for script in scripts {
            let mut hidden = 0;
            for (index, line) in script.lines.iter().enumerate() {
                if !regex.is_match(line) {
                    continue;
                }
                if everything && !script.in_default_search(index + 1) {
                    hidden += 1;
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
            if hidden > 0 {
                hidden_notes.push(format!(
                    "{hidden} more matching lines in vendor library {} hidden; pass script_id to see them",
                    script.id
                ));
            }
        }
        if total == 0 && hidden_notes.is_empty() {
            return ToolOutcome::ok(format!("No matches for `{}`.", args.pattern));
        }
        let hidden_notes = hidden_notes.join("\n");
        if total == 0 {
            return ToolOutcome::ok(format!(
                "No matches for `{}` in the publisher's scripts.\n{hidden_notes}",
                args.pattern
            ));
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
        ToolOutcome::ok(format!("{header}\n{out}{hidden_notes}"))
    }

    fn get_telemetry(&self, args: TelemetryInput) -> ToolOutcome {
        let page = match args.page {
            None => None,
            Some(number) => match self.page_index(number) {
                Some(index) => Some(&self.page_sections[index]),
                None => {
                    return ToolOutcome::error(format!(
                        "There is no page view {number}. This visit has {} page view(s), counted from 1.",
                        self.page_sections.len()
                    ));
                }
            },
        };
        // What is not recorded per page view is the same whatever page is asked for.
        let section = match (args.section.as_str(), page) {
            ("pages", _) => &self.sections.pages,
            ("prebid_config", _) => &self.sections.prebid_config,
            ("bidder_checks", _) => &self.sections.bidder_checks,
            ("auctions", None) => &self.sections.auctions,
            ("auctions", Some(page)) => &page.auctions,
            ("slots", None) => &self.sections.slots,
            ("slots", Some(page)) => &page.slots,
            ("console", None) => &self.sections.console,
            ("console", Some(page)) => &page.console,
            ("page_events", None) => &self.sections.page_events,
            ("page_events", Some(page)) => &page.page_events,
            (other, _) => {
                return ToolOutcome::error(format!(
                    "Unknown section `{other}`. Use pages, auctions, slots, prebid_config, console, page_events or bidder_checks."
                ));
            }
        };
        ToolOutcome::ok(section.clone())
    }

    /// Index of the page view the model calls `number` (it counts from one).
    fn page_index(&self, number: i64) -> Option<usize> {
        let index = usize::try_from(number.checked_sub(1)?).ok()?;
        (index < self.page_sections.len()).then_some(index)
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
        let mut pages: Vec<u32> = Vec::new();
        for number in &args.pages {
            match self.page_index(*number) {
                Some(index) => pages.push(index as u32),
                None => corrections.push(format!(
                    "page view {number} does not exist and was left out"
                )),
            }
        }
        pages.sort_unstable();
        pages.dedup();
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
            pages,
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
        let needle_chars = needle.chars().count();
        if needle_chars < MIN_EXCERPT_CHARS {
            return Err(format!(
                "The excerpt is too short to verify; quote at least {MIN_EXCERPT_CHARS} non-whitespace characters."
            ));
        }
        if needle_chars > MAX_EXCERPT_CHARS {
            return Err(format!(
                "The excerpt is too long; quote the few most telling lines (at most {MAX_EXCERPT_CHARS} characters)."
            ));
        }
        let total = script.lines.len();
        if citation.line < 1 || citation.line as usize > total {
            return Err(format!(
                "Line {} does not exist; {} has {total} lines.",
                citation.line, script.id
            ));
        }
        let cited = citation.line as usize;
        let Some((line, end)) = locate(&script.lines, &needle, cited) else {
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

/// The pattern as a regex, or as literal text when it is not a valid one
/// (the flag says which). `None` when it compiles to more than the size limit.
fn compile_pattern(pattern: &str) -> Option<(Regex, bool)> {
    let build = |source: &str| {
        RegexBuilder::new(source)
            .case_insensitive(true)
            .size_limit(REGEX_SIZE_LIMIT)
            .build()
    };
    match build(pattern) {
        Ok(regex) => Some((regex, false)),
        Err(regex::Error::CompiledTooBig(_)) => None,
        Err(_) => build(&regex::escape(pattern))
            .ok()
            .map(|regex| (regex, true)),
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

/// First and last line (1-based) of the occurrence of `needle` that starts
/// closest to the cited line, comparing without whitespace because the model
/// may have joined or split lines differently from the beautifier.
fn locate(lines: &[String], needle: &str, cited: usize) -> Option<(usize, usize)> {
    let mut text = String::new();
    let line_ends: Vec<usize> = lines
        .iter()
        .map(|line| {
            text.extend(line.chars().filter(|c| !c.is_whitespace()));
            text.len()
        })
        .collect();
    let line_at = |offset: usize| line_ends.partition_point(|&end| end <= offset) + 1;
    text.match_indices(needle)
        .map(|(start, matched)| (line_at(start), line_at(start + matched.len() - 1)))
        .min_by_key(|&(first, _)| first.abs_diff(cited))
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
            environment: adscan_model::CaptureEnvironment::default(),
            pages: vec![adscan_model::CapturedPage::entry("https://pub.example/")],
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
            bid_responses: vec![],
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
                default_search: None,
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

    /// The publisher's wrapper plus a Prebid build whose last lines hold the
    /// publisher's ad units.
    fn toolbox_with_vendor_library(default_search: Vec<LineRange>) -> Toolbox {
        let mut toolbox = toolbox();
        let mut library: Vec<String> = (0..50)
            .map(|n| format!("adapter{n}.timeout = startAuction;"))
            .collect();
        library.push("pbjs.addAdUnits([{ code: 'ad-leaderboard', timeout: 250 }]);".into());
        toolbox.scripts.push(ScriptSource {
            id: "s7".into(),
            url: "https://cdn.pub.example/prebid.js".into(),
            category: ScriptCategory::Prebid,
            size_bytes: 9000.0,
            lines: library,
            default_search: Some(default_search),
        });
        toolbox
    }

    #[test]
    fn a_search_of_all_scripts_leaves_vendor_library_internals_out_and_says_so() {
        let toolbox = toolbox_with_vendor_library(vec![]);
        let everywhere = toolbox.execute(
            "search_scripts",
            &json!({ "pattern": "startAuction", "script_id": null }),
        );
        assert!(
            everywhere.content.starts_with("2 matching lines"),
            "{}",
            everywhere.content
        );
        assert!(!everywhere.content.contains("--- s7"));
        assert!(everywhere.content.ends_with(
            "50 more matching lines in vendor library s7 hidden; pass script_id to see them"
        ));

        let only_in_the_library = toolbox.execute(
            "search_scripts",
            &json!({ "pattern": "adapter\\d+", "script_id": null }),
        );
        assert_eq!(
            only_in_the_library.content,
            "No matches for `adapter\\d+` in the publisher's scripts.\n50 more matching lines in vendor library s7 hidden; pass script_id to see them"
        );

        let by_id = toolbox.execute(
            "search_scripts",
            &json!({ "pattern": "startAuction", "script_id": "s7" }),
        );
        assert!(by_id.content.starts_with("50 matching lines"));
        assert!(by_id.content.contains("--- s7 line 1\n"));
        assert!(!by_id.content.contains("hidden"));

        let read = toolbox.execute(
            "read_script",
            &json!({ "script_id": "s7", "start_line": 1, "end_line": 3 }),
        );
        assert!(
            read.content
                .contains("    1| adapter0.timeout = startAuction;")
        );
    }

    #[test]
    fn publisher_code_inside_a_vendor_library_stays_in_the_default_search() {
        let toolbox = toolbox_with_vendor_library(vec![LineRange { start: 51, end: 51 }]);
        let outcome = toolbox.execute(
            "search_scripts",
            &json!({ "pattern": "timeout", "script_id": null }),
        );
        assert!(outcome.content.starts_with("1 matching lines"));
        assert!(outcome.content.contains("--- s7 line 51\n"));
        assert!(outcome.content.contains("code: 'ad-leaderboard'"));
        assert!(
            outcome
                .content
                .contains("50 more matching lines in vendor library s7 hidden")
        );
    }

    #[test]
    fn the_script_index_says_which_scripts_a_search_of_everything_covers() {
        let index = toolbox_with_vendor_library(vec![]).script_index();
        assert_eq!(
            index.lines().collect::<Vec<_>>(),
            [
                "s1 | Wrapper | 200 bytes | 8 lines | https://pub.example/ads.js",
                "s7 | Prebid | 9000 bytes | 51 lines | https://cdn.pub.example/prebid.js | vendor library, searched only when script_id is given",
            ]
        );
        let partly = toolbox_with_vendor_library(vec![
            LineRange { start: 1, end: 2 },
            LineRange { start: 51, end: 51 },
        ])
        .script_index();
        assert!(partly.ends_with(
            "| vendor library, only lines 1-2, 51-51 are searched unless script_id is given"
        ));
    }

    #[test]
    fn an_excerpt_reads_like_a_read_script_result() {
        let toolbox = toolbox();
        let read = toolbox.execute(
            "read_script",
            &json!({ "script_id": "s1", "start_line": 1, "end_line": 3 }),
        );
        assert_eq!(
            toolbox.excerpt(0, LineRange { start: 1, end: 3 }),
            read.content
        );
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
    fn a_repeated_excerpt_is_attributed_to_the_occurrence_nearest_the_cited_line() {
        let mut toolbox = toolbox();
        toolbox.scripts[0].lines = (0..300)
            .map(|n| {
                if n % 100 == 50 {
                    "  googletag.pubads().refresh();".to_string()
                } else {
                    format!("step{n}();")
                }
            })
            .collect();
        let outcome = toolbox.execute(
            "report_finding",
            &finding_input(249, "googletag.pubads().refresh();"),
        );
        let ToolEffect::Finding(draft) = outcome.effect else {
            panic!("expected a finding: {}", outcome.content);
        };
        assert_eq!(draft.evidence[0].line, Some(251));
        assert_eq!(draft.evidence[0].body, "  googletag.pubads().refresh();");
    }

    #[test]
    fn an_excerpt_split_across_lines_is_located_and_stored_as_those_lines() {
        let outcome = toolbox().execute(
            "report_finding",
            &finding_input(1, "if (isMobile()) { window.addEventListener('scroll'"),
        );
        let ToolEffect::Finding(draft) = outcome.effect else {
            panic!("expected a finding: {}", outcome.content);
        };
        assert_eq!(draft.evidence[0].line, Some(4));
        assert_eq!(
            draft.evidence[0].body,
            "if (isMobile()) {\n  window.addEventListener('scroll', startAuction, { once: true });"
        );
    }

    #[test]
    fn excerpts_too_short_to_prove_anything_or_too_long_are_rejected() {
        let toolbox = toolbox();
        let trivial = toolbox.execute("report_finding", &finding_input(3, "}"));
        assert!(trivial.is_error);
        assert_eq!(trivial.effect, ToolEffect::None);
        assert!(trivial.content.contains("too short to verify"));

        let whole_file = toolbox.execute(
            "report_finding",
            &finding_input(1, &"startAuction();".repeat(500)),
        );
        assert!(whole_file.is_error);
        assert!(whole_file.content.contains("too long"));
    }

    #[test]
    fn oversized_and_explosive_patterns_are_refused_without_searching() {
        let toolbox = toolbox();
        let long = toolbox.execute(
            "search_scripts",
            &json!({ "pattern": "a".repeat(MAX_PATTERN_CHARS + 1), "script_id": null }),
        );
        assert!(long.is_error);
        assert!(long.content.contains("longer than 300 characters"));

        let explosive = toolbox.execute(
            "search_scripts",
            &json!({ "pattern": "(\\w{1000}){1000}", "script_id": null }),
        );
        assert!(explosive.is_error);
        assert!(explosive.content.contains("too complex"));
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
    fn bidder_checks_are_a_telemetry_section_once_they_ran() {
        use adscan_model::{
            AdsTxtReport, AdsTxtStatus, BidderDiagnosis, ConsentReport, DiagnosticArea,
            DiagnosticCheck, DiagnosticStatus,
        };
        let section = |toolbox: &Toolbox| {
            toolbox
                .execute("get_telemetry", &json!({ "section": "bidder_checks" }))
                .content
        };
        assert_eq!(
            section(&toolbox()),
            "The bidder checks did not run for this scan."
        );

        let diagnostics = BidderDiagnostics {
            ads_txt: AdsTxtReport {
                url: "https://pub.example/ads.txt".into(),
                status: AdsTxtStatus::Found,
                detail: None,
                record_count: 9,
                invalid_line_count: 0,
                invalid_lines: vec![],
                owner_domain: None,
                manager_domains: vec![],
            },
            sellers_json_checked: false,
            consent: ConsentReport {
                cmp_present: true,
                consent_management_configured: Some(false),
                gdpr_applies: Some(true),
                resolved_at_ms: Some(812.0),
                auction_count: 2,
                auctions_before_consent: 1,
                consented_purposes: vec![1],
            },
            bidders: vec![BidderDiagnosis {
                bidder: "seedtag".into(),
                requests: 4,
                bids: 0,
                checks: vec![DiagnosticCheck {
                    pages: Vec::new(),
                    area: DiagnosticArea::AdsTxt,
                    status: DiagnosticStatus::Problem,
                    code: "ads-txt-not-listed".into(),
                    summary: "ads.txt has no entry for seedtag.com".into(),
                    excerpt: None,
                    ad_units: vec![],
                }],
            }],
        };
        assert_eq!(
            section(&toolbox().with_bidder_checks(Some(&diagnostics))),
            "ads.txt https://pub.example/ads.txt status=found records=9 invalidLines=0\n\
             sellers.json lookups: off (user setting)\n\
             consent: cmp=true prebidConsentManagement=no gdprApplies=yes consentKnownAt=812ms auctionsBeforeConsent=1/2\n\
             bidder seedtag requests=4 bids=0\n  \
             AdsTxt/Problem ads-txt-not-listed: ads.txt has no entry for seedtag.com"
        );
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
