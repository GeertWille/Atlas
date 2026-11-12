//! A script's source as the viewer shows it: where findings point, what the
//! analysis read, where the ad stack is called.

use adscan_model::{
    AdCall, EvidenceKind, Finding, LogEntry, LogKind, RegionKind, ScriptRecord, SourceAnchor,
    SourceRegion,
};
use serde_json::Value;

use super::guide::MIN_RANK;

/// `read_script` returns at most this many lines per call.
pub const MAX_READ_LINES: usize = 250;
const MAX_AD_CALLS: usize = 500;
/// How much of the original source after a position is looked for in the
/// formatted text, longest first. The formatter changes quotes and adds
/// semicolons, so a long stretch can fail where a short one is found.
const FOCUS_NEEDLE_CHARS: [usize; 4] = [80, 40, 20, 12];

/// (name shown, text looked for) of calls into the ad stack.
const AD_CALLS: &[(&str, &str)] = &[
    ("requestBids", ".requestBids("),
    ("addAdUnits", ".addAdUnits("),
    ("setConfig", ".setConfig("),
    ("setTargetingForGPTAsync", ".setTargetingForGPTAsync("),
    ("defineSlot", ".defineSlot("),
    ("defineOutOfPageSlot", ".defineOutOfPageSlot("),
    ("refresh", "pubads().refresh("),
    ("enableServices", ".enableServices("),
    ("fetchBids", ".fetchBids("),
    ("googletag.cmd", "googletag.cmd"),
    ("pbjs.que", "pbjs.que"),
    ("__tcfapi", "__tcfapi("),
];

/// Whitespace-insensitive comparison form.
pub fn squash(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

/// First and last line (1-based) of the occurrence of `needle` that starts
/// closest to the cited line, comparing without whitespace because the text
/// may have been joined or split into lines differently.
pub fn locate(lines: &[String], needle: &str, cited: usize) -> Option<(usize, usize)> {
    if needle.is_empty() {
        return None;
    }
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

/// Where the code evidence of `findings` sits in the script's formatted
/// `lines`. Evidence is found again by its text, so a formatter that prints
/// differently than when the finding was made does not misplace it.
pub fn anchors(script: &ScriptRecord, findings: &[Finding], lines: &[String]) -> Vec<SourceAnchor> {
    let mut anchors = Vec::new();
    for finding in findings {
        for (index, evidence) in finding.evidence.iter().enumerate() {
            let cites_script = match &evidence.script_id {
                Some(id) => *id == script.id,
                // Scans stored before evidence named its script.
                None => evidence.script_url.as_deref() == Some(script.url.as_str()),
            };
            if evidence.kind != EvidenceKind::Code || !cites_script {
                continue;
            }
            let stored_line = evidence.line.map(|line| line as usize);
            let located = locate(lines, &squash(&evidence.body), stored_line.unwrap_or(1));
            let (start, end, located) = match (located, stored_line) {
                (Some((start, end)), _) => {
                    // The stored body is cut at a length; the stored end line is not.
                    let stored_end = evidence
                        .end_line
                        .filter(|_| stored_line == Some(start))
                        .map_or(end, |stored| (stored as usize).max(end));
                    (start, stored_end, true)
                }
                (None, Some(line)) => (
                    line,
                    evidence.end_line.map_or(line, |end| (end as usize).max(line)),
                    false,
                ),
                (None, None) => continue,
            };
            anchors.push(SourceAnchor {
                finding_id: finding.id.clone(),
                evidence_index: index as u32,
                start_line: start as u32,
                end_line: end as u32,
                located,
            });
        }
    }
    anchors
}

/// The stretches of the script the analysis looked at, from the audit trail:
/// the model's `read_script` calls and the chunks of Jev's reading guide.
pub fn regions(script_id: &str, logs: &[LogEntry], line_count: u32) -> Vec<SourceRegion> {
    let mut regions = Vec::new();
    let mut guide: Vec<(f64, bool, u32, u32)> = Vec::new();
    for entry in logs {
        let Some(data) = &entry.data else { continue };
        if entry.kind == LogKind::ToolCall && data["tool"] == "read_script" {
            let input = &data["input"];
            let same_script = input["script_id"]
                .as_str()
                .is_some_and(|id| id.trim().eq_ignore_ascii_case(script_id));
            let Some(start) = input["start_line"].as_u64().filter(|_| same_script) else {
                continue;
            };
            let start = start.max(1) as u32;
            let asked_end = input["end_line"].as_u64().unwrap_or(0) as u32;
            let end = asked_end
                .max(start)
                .min(start + MAX_READ_LINES as u32 - 1)
                .min(line_count);
            if start <= end {
                regions.push(SourceRegion {
                    kind: RegionKind::Read,
                    start_line: start,
                    end_line: end,
                    rank: None,
                });
            }
        } else if let Some(chunks) = data["chunks"].as_array() {
            guide.extend(chunks.iter().filter_map(|chunk| guide_chunk(chunk, script_id)));
        }
    }
    // Rank is the chunk's place among the guide's chunks of every script.
    guide.sort_by(|a, b| b.0.total_cmp(&a.0));
    regions.extend(
        guide
            .iter()
            .enumerate()
            .filter(|(_, (_, in_script, start, _))| *in_script && *start <= line_count)
            .map(|(position, (_, _, start, end))| SourceRegion {
                kind: RegionKind::Guide,
                start_line: *start,
                end_line: (*end).min(line_count),
                rank: Some(position as u32 + 1),
            }),
    );
    regions
}

/// (score, belongs to this script, first line, last line) of a chunk that
/// made the reading guide.
fn guide_chunk(chunk: &Value, script_id: &str) -> Option<(f64, bool, u32, u32)> {
    let score = chunk["rank"].as_f64().filter(|score| *score >= MIN_RANK)?;
    let start = chunk["lines"][0].as_u64()? as u32;
    let end = chunk["lines"][1].as_u64()? as u32;
    let in_script = chunk["scriptId"].as_str() == Some(script_id);
    (start >= 1 && end >= start).then_some((score, in_script, start, end))
}

/// Calls into the ad stack, by a scan of the lines: works on any text,
/// formatted or not.
pub fn ad_calls(lines: &[String]) -> Vec<AdCall> {
    let mut calls = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        for (name, needle) in AD_CALLS {
            if line.contains(needle) {
                calls.push(AdCall {
                    name: (*name).to_string(),
                    line: index as u32 + 1,
                });
                if calls.len() == MAX_AD_CALLS {
                    return calls;
                }
            }
        }
    }
    calls
}

/// The line of the formatted text that a position in the source as captured
/// is on (`line` and `column` 1-based), or `None` when the code there cannot
/// be found again.
pub fn focus_line(raw: &str, formatted: &[String], line: u32, column: u32) -> Option<u32> {
    let line_start: usize = raw
        .split_inclusive('\n')
        .take(line.saturating_sub(1) as usize)
        .map(str::len)
        .sum();
    let rest = raw.get(line_start..)?;
    let offset = line_start
        + rest
            .char_indices()
            .nth(column.saturating_sub(1) as usize)
            .map_or(rest.len(), |(at, _)| at);
    let ahead = squash(&raw[offset..].chars().take(400).collect::<String>());
    // Lines grow evenly enough with formatting for the nearest match to be
    // the right one among repeats.
    let hint = (offset as f64 / raw.len().max(1) as f64 * formatted.len() as f64) as usize + 1;
    FOCUS_NEEDLE_CHARS
        .iter()
        .filter_map(|length| {
            let needle: String = ahead.chars().take(*length).collect();
            (needle.chars().count() == *length).then_some(needle)
        })
        .find_map(|needle| locate(formatted, &needle, hint))
        .map(|(start, _)| start as u32)
}

#[cfg(test)]
mod tests {
    use adscan_model::{Evidence, FindingCategory, FindingSource, PipelineNode, ScriptCategory, Severity};
    use serde_json::json;

    use super::*;

    fn lines(text: &str) -> Vec<String> {
        text.lines().map(str::to_string).collect()
    }

    const WRAPPER: &str = "function start(cfg) {\n  var mq = window.matchMedia;\n  if (mq(\"(min-width: 768px)\").matches) {\n    waitForScroll(run);\n  }\n  pbjs.requestBids({\n    timeout: 250,\n  });\n}";

    fn script(id: &str, url: &str) -> ScriptRecord {
        ScriptRecord {
            id: id.into(),
            url: url.into(),
            hostname: "pub.example".into(),
            inline: false,
            size_bytes: 0.0,
            hash: String::new(),
            category: ScriptCategory::Wrapper,
            ad_related: true,
            classification_reason: None,
            classification_confidence: None,
            classified_by: FindingSource::Heuristic,
            classifier_model: None,
            ad_probability: None,
            analyzed: true,
            vendor: None,
            loaded_at_ms: None,
            first_party: true,
            pages: Vec::new(),
            initiator: None,
            request_id: None,
            sha256: None,
        }
    }

    fn finding(id: &str, evidence: Vec<Evidence>) -> Finding {
        Finding {
            id: id.into(),
            scan_id: "scan".into(),
            severity: Severity::High,
            category: FindingCategory::LogicBug,
            source: FindingSource::Ai,
            title: String::new(),
            description: String::new(),
            recommendation: String::new(),
            confidence: 0.9,
            impact: String::new(),
            estimated_revenue_impact_pct: None,
            evidence,
            affected_slots: Vec::new(),
            affected_bidders: Vec::new(),
            pages: Vec::new(),
        }
    }

    fn code(body: &str, script_id: Option<&str>, url: &str, line: Option<u32>, end: Option<u32>) -> Evidence {
        Evidence {
            kind: EvidenceKind::Code,
            label: String::new(),
            body: body.into(),
            script_url: Some(url.into()),
            line,
            end_line: end,
            script_id: script_id.map(str::to_string),
            request_id: None,
        }
    }

    #[test]
    fn locate_finds_the_occurrence_nearest_the_cited_line_without_regard_to_whitespace() {
        let text = lines("a();\nif (x) {\n  b();\n}\na();\nif (x) {\n  b();\n}");
        assert_eq!(locate(&text, &squash("if (x) { b(); }"), 6), Some((6, 8)));
        assert_eq!(locate(&text, &squash("if (x) { b(); }"), 1), Some((2, 4)));
        assert_eq!(locate(&text, &squash("if(x){b();}"), 100), Some((6, 8)));
        assert_eq!(locate(&text, &squash("nothing like it"), 1), None);
        assert_eq!(locate(&text, "", 1), None);
        assert_eq!(locate(&[], "a", 1), None);
    }

    #[test]
    fn code_evidence_is_anchored_where_its_text_is() {
        let script = script("s2", "https://pub.example/wrapper.js");
        let findings = [finding(
            "f1",
            vec![
                Evidence {
                    kind: EvidenceKind::Metric,
                    ..code("3 auctions", None, "https://pub.example/wrapper.js", None, None)
                },
                // Cited two lines off: found again by its text.
                code("if (mq(\"(min-width: 768px)\").matches) {", Some("s2"), "x", Some(5), Some(5)),
                code("anything", Some("s9"), "https://pub.example/wrapper.js", Some(1), None),
            ],
        )];
        assert_eq!(
            anchors(&script, &findings, &lines(WRAPPER)),
            vec![SourceAnchor {
                finding_id: "f1".into(),
                evidence_index: 1,
                start_line: 3,
                end_line: 3,
                located: true,
            }]
        );
    }

    #[test]
    fn evidence_that_spans_lines_is_anchored_over_all_of_them() {
        let script = script("s2", "u");
        let findings = [finding(
            "f1",
            vec![code("pbjs.requestBids({ timeout: 250, });", Some("s2"), "u", Some(6), Some(8))],
        )];
        let anchor = &anchors(&script, &findings, &lines(WRAPPER))[0];
        assert_eq!((anchor.start_line, anchor.end_line, anchor.located), (6, 8, true));
    }

    #[test]
    fn a_stored_excerpt_that_was_cut_keeps_its_stored_end_line() {
        let script = script("s2", "u");
        // The body holds the first line only; the citation covered 6 to 8.
        let findings = [finding("f1", vec![code("pbjs.requestBids({", Some("s2"), "u", Some(6), Some(8))])];
        let anchor = &anchors(&script, &findings, &lines(WRAPPER))[0];
        assert_eq!((anchor.start_line, anchor.end_line), (6, 8));
    }

    #[test]
    fn evidence_that_cannot_be_found_keeps_its_stored_lines_and_says_so() {
        let script = script("s2", "u");
        let findings = [finding(
            "f1",
            vec![
                code("code that is no longer there", Some("s2"), "u", Some(4), Some(5)),
                code("gone too, and never had a line", Some("s2"), "u", None, None),
            ],
        )];
        assert_eq!(
            anchors(&script, &findings, &lines(WRAPPER)),
            vec![SourceAnchor {
                finding_id: "f1".into(),
                evidence_index: 0,
                start_line: 4,
                end_line: 5,
                located: false,
            }]
        );
    }

    #[test]
    fn evidence_of_older_scans_is_matched_by_script_url() {
        let script = script("s2", "https://pub.example/wrapper.js");
        let findings = [finding(
            "f1",
            vec![
                code("waitForScroll(run);", None, "https://pub.example/wrapper.js", Some(4), None),
                code("waitForScroll(run);", None, "https://pub.example/other.js", Some(4), None),
            ],
        )];
        let found = anchors(&script, &findings, &lines(WRAPPER));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].evidence_index, 0);
    }

    fn log(kind: LogKind, data: Value) -> LogEntry {
        LogEntry {
            id: "l".into(),
            scan_id: "scan".into(),
            step_id: "step".into(),
            node: PipelineNode::AnalyzeLogic,
            ts: 0.0,
            level: adscan_model::LogLevel::Info,
            kind,
            message: String::new(),
            data: Some(data),
        }
    }

    #[test]
    fn regions_come_from_the_read_script_calls_of_the_audit_trail() {
        let logs = [
            log(LogKind::ToolCall, json!({"tool": "read_script", "input": {"script_id": "s2", "start_line": 10, "end_line": 40}})),
            log(LogKind::ToolCall, json!({"tool": "read_script", "input": {"script_id": "S2 ", "start_line": 100, "end_line": 900}})),
            log(LogKind::ToolCall, json!({"tool": "read_script", "input": {"script_id": "s2", "start_line": 480, "end_line": 600}})),
            log(LogKind::ToolCall, json!({"tool": "read_script", "input": {"script_id": "s2", "start_line": 0, "end_line": 0}})),
            log(LogKind::ToolCall, json!({"tool": "read_script", "input": {"script_id": "s2", "start_line": 700, "end_line": 710}})),
            log(LogKind::ToolCall, json!({"tool": "read_script", "input": {"script_id": "s3", "start_line": 1, "end_line": 5}})),
            log(LogKind::ToolCall, json!({"tool": "search_scripts", "input": {"script_id": "s2", "pattern": "x"}})),
            log(LogKind::ToolResult, json!({"tool": "read_script", "input": {"script_id": "s2", "start_line": 1, "end_line": 2}})),
        ];
        let found: Vec<(u32, u32)> = regions("s2", &logs, 500)
            .iter()
            .map(|region| {
                assert_eq!((region.kind, region.rank), (RegionKind::Read, None));
                (region.start_line, region.end_line)
            })
            .collect();
        // The 250-line cap of one call, the end of the file, and line 0 read
        // as the tool reads them; a call past the end shows nothing.
        assert_eq!(found, [(10, 40), (100, 349), (480, 500), (1, 1)]);
    }

    #[test]
    fn guide_chunks_are_regions_ranked_among_the_whole_guide() {
        let logs = [log(
            LogKind::Message,
            json!({"model": "jev-1.14.0", "chunks": [
                {"scriptId": "s2", "lines": [1, 160], "rank": 0.42, "answers": {}},
                {"scriptId": "s5", "lines": [1, 160], "rank": 0.91, "answers": {}},
                {"scriptId": "s2", "lines": [161, 320], "rank": 0.77, "answers": {}},
                {"scriptId": "s2", "lines": [321, 480], "rank": 0.05, "answers": {}},
            ]}),
        )];
        let found = regions("s2", &logs, 300);
        assert_eq!(
            found,
            vec![
                SourceRegion { kind: RegionKind::Guide, start_line: 161, end_line: 300, rank: Some(2) },
                SourceRegion { kind: RegionKind::Guide, start_line: 1, end_line: 160, rank: Some(3) },
            ],
            "chunks below the guide's threshold are not regions"
        );
    }

    #[test]
    fn ad_calls_are_found_by_line() {
        let calls = ad_calls(&lines(
            "googletag.cmd.push(function () {\n  googletag.defineSlot('/1/top', [728, 90], 'top');\n});\nwindow.__tcfapi('addEventListener', 2, cb);\nfoo.refresh();\ngoogletag.pubads().refresh([slot]);\npbjs.que.push(function () { pbjs.addAdUnits(units); pbjs.requestBids({}); });",
        ));
        let found: Vec<(&str, u32)> = calls.iter().map(|call| (call.name.as_str(), call.line)).collect();
        assert_eq!(
            found,
            [
                ("googletag.cmd", 1),
                ("defineSlot", 2),
                ("__tcfapi", 4),
                ("refresh", 6),
                ("requestBids", 7),
                ("addAdUnits", 7),
                ("pbjs.que", 7),
            ]
        );
        assert!(ad_calls(&lines(WRAPPER)).iter().any(|call| call.name == "requestBids" && call.line == 6));
    }

    #[test]
    fn a_position_in_minified_source_maps_to_its_formatted_line() {
        let raw = "function start(cfg){var mq=window.matchMedia;if(mq(\"(min-width: 768px)\").matches){waitForScroll(run)}pbjs.requestBids({timeout:250})}";
        let formatted = lines(WRAPPER);
        let column_of = |needle: &str| raw.find(needle).unwrap() as u32 + 1;
        assert_eq!(focus_line(raw, &formatted, 1, column_of("if(mq(")), Some(3));
        assert_eq!(focus_line(raw, &formatted, 1, column_of("pbjs.requestBids")), Some(6));
        assert_eq!(focus_line(raw, &formatted, 1, 1), Some(1));
        // Past the end of the source there is nothing to look for.
        assert_eq!(focus_line(raw, &formatted, 1, 5_000), None);
        assert_eq!(focus_line(raw, &formatted, 9, 1), None);
    }

    #[test]
    fn a_position_on_a_later_line_of_the_original_is_found_too() {
        let raw = "var a = 1;\nvar b = 2;\nfunction go(){return a+b+window.innerWidth}\n";
        let formatted = lines("var a = 1;\nvar b = 2;\nfunction go() {\n  return a + b + window.innerWidth;\n}");
        assert_eq!(focus_line(raw, &formatted, 3, 15), Some(4));
    }
}
