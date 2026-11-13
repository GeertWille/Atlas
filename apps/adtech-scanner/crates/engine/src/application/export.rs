//! Renders a finished scan as JSON or as a Markdown report.

use std::fmt::Write;

use adscan_model::{
    AdSlot, Auction, BrowserUse, BrowserbaseProxyKind, BrowserbaseVisit, EvidenceKind, LogEntry,
    LogKind, PageView, PipelineNode, ProxyKind, ScanDetail, ScanMetrics, ScriptCategory,
    ScriptRecord, VisitorBehavior,
};

use super::AppError;
use super::pipeline::node_label;
use crate::domain::diagnostics::capitalized;

const MAX_TRAIL_ENTRIES: usize = 60;
const MAX_TRAIL_ENTRY_CHARS: usize = 400;

pub fn json(detail: &ScanDetail) -> Result<String, AppError> {
    serde_json::to_string_pretty(detail)
        .map_err(|error| AppError::Internal(format!("could not serialise the scan: {error}")))
}

pub fn markdown(detail: &ScanDetail, logs: &[LogEntry]) -> String {
    let mut out = String::new();
    summary_section(&mut out, detail);
    visit_section(&mut out, detail);
    metrics_section(&mut out, &detail.metrics);
    findings_section(&mut out, detail);
    auctions_section(&mut out, detail);
    slots_section(&mut out, detail);
    scripts_section(&mut out, detail);
    pipeline_section(&mut out, detail);
    ai_trail_section(&mut out, logs);
    out
}

// `write!` to a String cannot fail, so results are discarded throughout.

fn summary_section(out: &mut String, detail: &ScanDetail) {
    let summary = &detail.summary;
    let _ = writeln!(out, "# Ad stack scan: {}\n", summary.hostname);
    let _ = writeln!(out, "- **URL:** {}", summary.url);
    let _ = writeln!(out, "- **Device profile:** {:?}", summary.device);
    if let Some(configuration) = &summary.configuration {
        let _ = writeln!(out, "- **Scan configuration:** {}", configuration.name);
    }
    let _ = writeln!(out, "- **Status:** {:?}", summary.status);
    if let Some(error) = &summary.error {
        let _ = writeln!(out, "- **Error:** {error}");
    }
    let _ = writeln!(
        out,
        "- **Health score:** {}",
        summary
            .health_score
            .map_or("n/a".to_string(), |score| format!("{score}/100"))
    );
    let counts = summary.finding_counts;
    let _ = writeln!(
        out,
        "- **Findings:** {} critical, {} high, {} medium, {} low, {} info",
        counts.critical, counts.high, counts.medium, counts.low, counts.info
    );
    let _ = writeln!(
        out,
        "- **AI analysis:** {} (LLM cost ${:.4})",
        if summary.ai_used { "used" } else { "not used" },
        summary.cost_usd
    );
    if let Some(version) = &summary.prebid_version {
        let _ = writeln!(out, "- **Prebid.js:** {version}");
    }
    if let Some(overview) = &detail.overview {
        let _ = writeln!(out, "\n## Summary\n\n{overview}");
    }
}

/// What a visit in a Browserbase browser adds: the session, what was asked
/// of Browserbase, what the browser reported and what it cost.
fn browserbase_lines(out: &mut String, visit: &BrowserbaseVisit) {
    let on = |enabled: bool| if enabled { "on" } else { "off" };
    let _ = writeln!(
        out,
        "- **Browserbase session:** {}{}",
        cell(&visit.session_id),
        visit
            .browser_version
            .as_deref()
            .map(|version| format!(", {}", cell(version)))
            .unwrap_or_default(),
    );
    let _ = writeln!(
        out,
        "- **Browserbase settings sent:** CAPTCHA solving {}, session video {}, session log {}, ad blocking {}",
        on(visit.settings.solve_captchas),
        on(visit.settings.record_session),
        on(visit.settings.log_session),
        on(visit.settings.block_ads),
    );
    let original = visit.original_timezone.as_deref().unwrap_or("not read");
    let timezone = match &visit.applied_timezone {
        Some(applied) => format!(
            "{} (set by the scanner to match the region; the browser reported {})",
            cell(applied),
            cell(original)
        ),
        None => format!("{} (as the browser reported it)", cell(original)),
    };
    let _ = writeln!(
        out,
        "- **Timezone and language:** {timezone}; language {}",
        cell(visit.language.as_deref().unwrap_or("not read")),
    );
    if let Some(round_trip) = visit.round_trip_ms {
        let _ = writeln!(
            out,
            "- **Round trip to the browser:** {round_trip:.0} ms (frames and requests the scanner holds for a moment wait this long)"
        );
    }
    let duration = visit
        .session_seconds
        .map_or("not reported".to_string(), |seconds| {
            format!("{seconds:.0} s")
        });
    let traffic = match (visit.proxy_kind, visit.proxy_bytes) {
        (BrowserbaseProxyKind::None, _) => String::new(),
        (_, Some(bytes)) => format!(", {:.1} MB through the proxy", bytes / 1_000_000.0),
        (_, None) => ", proxy traffic not reported".to_string(),
    };
    let _ = writeln!(
        out,
        "- **Browserbase usage:** {duration} of browser time{traffic} (billed by Browserbase, not part of the LLM cost)"
    );
}

/// How the pages were visited and what they could tell about the visit.
/// Scans stored before this was recorded have no such section.
fn visit_section(out: &mut String, detail: &ScanDetail) {
    if let Some(environment) = &detail.environment {
        let _ = writeln!(out, "\n## Visit\n");
        let visitor = match environment.visitor {
            VisitorBehavior::None => "Observed only, no input",
            VisitorBehavior::Scroll => "Scrolled through the page",
            VisitorBehavior::Browse => "Browsed like a visitor, following links",
            VisitorBehavior::Manual => "A person browsed; the scanner recorded",
        };
        let _ = writeln!(out, "- **Visitor:** {visitor} (seed {})", environment.seed);
        let _ = writeln!(
            out,
            "- **Browser:** {}, {} profile{}",
            match &environment.browser {
                BrowserUse::Local if environment.headless => "headless".to_string(),
                BrowserUse::Local => "shown".to_string(),
                BrowserUse::Browserbase(visit) => format!(
                    "Browserbase, {} (headless, not shown in its user agent)",
                    visit.region.as_str()
                ),
            },
            match (&environment.browser, environment.persistent_profile) {
                (BrowserUse::Local, true) => "persistent scan",
                (BrowserUse::Browserbase(_), true) => "kept at Browserbase",
                (_, false) => "throwaway",
            },
            if environment.emulated_mobile {
                ", mobile emulation"
            } else {
                ""
            },
        );
        let proxy = match environment.proxy.kind {
            ProxyKind::None => "none (direct connection)".to_string(),
            ProxyKind::System => "system setting".to_string(),
            ProxyKind::Profile => format!(
                "{} ({})",
                environment.proxy.name.as_deref().unwrap_or("unnamed"),
                environment.proxy.host.as_deref().unwrap_or("unknown host"),
            ),
            ProxyKind::BrowserbaseLocation => format!(
                "Browserbase residential exit, asked for {}",
                environment.proxy.name.as_deref().unwrap_or("no place"),
            ),
        };
        let _ = writeln!(out, "- **Proxy:** {}", cell(&proxy));
        if let BrowserUse::Browserbase(visit) = &environment.browser {
            browserbase_lines(out, visit);
        }
        let _ = writeln!(
            out,
            "- **Automation signals the pages could see:** navigator.webdriver {}, user agent {}, viewport {}",
            match environment.webdriver {
                Some(true) => "true",
                Some(false) => "false",
                None => "not read",
            },
            if environment.user_agent_overridden {
                "overridden"
            } else {
                "the browser's own"
            },
            if environment.viewport_overridden {
                "overridden"
            } else {
                "the window's own"
            },
        );
        if !environment.detectors.is_empty() {
            let _ = writeln!(
                out,
                "- **Detection vendors on the page:** {}",
                environment
                    .detectors
                    .iter()
                    .map(|detector| format!("{} ({})", detector.vendor, cell(&detector.host)))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
    }
    if detail.pages.len() < 2 {
        return;
    }
    let _ = writeln!(
        out,
        "\n### Page views\n\n| # | URL | Reached by | Start (ms) | Dwell (ms) | Load (ms) | Auctions | Slots (empty) | Fill rate |\n|---|---|---|---|---|---|---|---|---|"
    );
    for page in &detail.pages {
        let _ = writeln!(
            out,
            "| {} | {} | {:?} | {:.0} | {:.0} | {} | {} | {} ({}) | {:.0}% |",
            page.index + 1,
            cell(&page.url),
            page.reached_by,
            page.started_at_ms,
            page.dwell_ms,
            page.load_ms
                .map_or("-".to_string(), |load| format!("{load:.0}")),
            page.metrics.auction_count,
            page.metrics.slot_count,
            page.metrics.empty_slot_count,
            page.metrics.fill_rate * 100.0,
        );
    }
    let _ = writeln!(
        out,
        "\nTimes of auctions and slots below count from the start of their page view."
    );
}

/// "page 2 (https://site/path)", numbered from one.
fn page_name(pages: &[PageView], page: u32) -> String {
    match pages.get(page as usize) {
        Some(view) => format!("page {} ({})", page + 1, view.url),
        None => format!("page {}", page + 1),
    }
}

fn metrics_section(out: &mut String, metrics: &ScanMetrics) {
    let ms = |value: Option<f64>| value.map_or("n/a".to_string(), |value| format!("{value:.0} ms"));
    let _ = writeln!(out, "\n## Metrics\n\n| Metric | Value |\n|---|---|");
    let rows = [
        ("Page load", ms(metrics.page_load_ms)),
        (
            "Time to first auction",
            ms(metrics.time_to_first_auction_ms),
        ),
        ("Time to first ad", ms(metrics.time_to_first_ad_ms)),
        ("Auctions", metrics.auction_count.to_string()),
        ("Ad units", metrics.ad_unit_count.to_string()),
        ("Bidders", metrics.bidder_count.to_string()),
        (
            "Bid requests / responses / timeouts / no-bids",
            format!(
                "{} / {} / {} / {}",
                metrics.bid_request_count,
                metrics.bid_response_count,
                metrics.bid_timeout_count,
                metrics.no_bid_count
            ),
        ),
        (
            "Timeout rate",
            format!("{:.0}%", metrics.timeout_rate * 100.0),
        ),
        ("Prebid timeout", ms(metrics.prebid_timeout_ms)),
        (
            "Slots (empty)",
            format!("{} ({})", metrics.slot_count, metrics.empty_slot_count),
        ),
        ("Fill rate", format!("{:.0}%", metrics.fill_rate * 100.0)),
        (
            "Average winning CPM",
            metrics
                .avg_winning_cpm
                .map_or("n/a".to_string(), |cpm| format!("{cpm:.2}")),
        ),
        (
            "Scripts (ad-related)",
            format!("{} ({})", metrics.script_count, metrics.ad_script_count),
        ),
    ];
    for (name, value) in rows {
        let _ = writeln!(out, "| {name} | {value} |");
    }
}

fn findings_section(out: &mut String, detail: &ScanDetail) {
    let findings = &detail.findings;
    let _ = writeln!(out, "\n## Findings\n");
    if findings.is_empty() {
        let _ = writeln!(out, "No findings.");
        return;
    }
    for (position, finding) in findings.iter().enumerate() {
        let _ = writeln!(out, "### {}. {}\n", position + 1, finding.title);
        let _ = writeln!(
            out,
            "**Severity:** {:?} | **Category:** {:?} | **Source:** {:?} | **Confidence:** {:.0}%{}\n",
            finding.severity,
            finding.category,
            finding.source,
            finding.confidence * 100.0,
            finding
                .estimated_revenue_impact_pct
                .map_or(String::new(), |pct| format!(
                    " | **Estimated revenue at risk:** {pct:.0}%"
                )),
        );
        let _ = writeln!(out, "{}\n", finding.description);
        let _ = writeln!(out, "**Impact:** {}\n", finding.impact);
        let _ = writeln!(out, "**Recommendation:** {}\n", finding.recommendation);
        if !finding.affected_slots.is_empty() {
            let _ = writeln!(
                out,
                "**Affected slots:** {}\n",
                finding.affected_slots.join(", ")
            );
        }
        if !finding.affected_bidders.is_empty() {
            let _ = writeln!(
                out,
                "**Affected bidders:** {}\n",
                finding.affected_bidders.join(", ")
            );
        }
        if detail.pages.len() > 1 && !finding.pages.is_empty() {
            let _ = writeln!(
                out,
                "**Seen on:** {}\n",
                finding
                    .pages
                    .iter()
                    .map(|page| page_name(&detail.pages, *page))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        for evidence in &finding.evidence {
            let location = match (&evidence.script_url, evidence.line) {
                (Some(url), Some(line)) => format!(" ({url}:{line})"),
                (Some(url), None) => format!(" ({url})"),
                _ => String::new(),
            };
            let _ = writeln!(
                out,
                "*{:?} evidence: {}{location}*\n",
                evidence.kind, evidence.label
            );
            let language = if evidence.kind == EvidenceKind::Code {
                "js"
            } else {
                "text"
            };
            let _ = writeln!(
                out,
                "```{language}\n{}\n```\n",
                evidence.body.replace("```", "` ` `")
            );
        }
    }
}

fn auctions_section(out: &mut String, detail: &ScanDetail) {
    let _ = writeln!(out, "## Auctions\n");
    if detail.auctions.is_empty() {
        let _ = writeln!(out, "No Prebid auctions were observed.");
        return;
    }
    for group in by_page(&detail.pages, &detail.auctions, |auction| auction.page) {
        if let Some(heading) = &group.heading {
            let _ = writeln!(out, "\n### {heading}\n");
        }
        auctions_table(out, &group.items);
    }
}

/// Items of one page view, each with its position in the scan.
struct PageGroup<'a, T> {
    /// `None` for a visit of a single page view, which needs no heading.
    heading: Option<String>,
    items: Vec<(usize, &'a T)>,
}

/// One group for a visit of a single page view; else one group per page
/// view that has any of the items.
fn by_page<'a, T>(
    pages: &[PageView],
    items: &'a [T],
    page_of: impl Fn(&T) -> u32,
) -> Vec<PageGroup<'a, T>> {
    let numbered = || items.iter().enumerate();
    if pages.len() < 2 {
        return vec![PageGroup {
            heading: None,
            items: numbered().collect(),
        }];
    }
    (0u32..)
        .zip(pages)
        .map(|(page, _)| PageGroup {
            heading: Some(capitalized(&page_name(pages, page))),
            items: numbered()
                .filter(|(_, item)| page_of(item) == page)
                .collect(),
        })
        .filter(|group| !group.items.is_empty())
        .collect()
}

/// `auctions` carry their position in the scan, which numbers them.
fn auctions_table(out: &mut String, auctions: &[(usize, &Auction)]) {
    let _ = writeln!(
        out,
        "| Auction | Start (ms) | Duration (ms) | Ad unit | Bidder | Status | CPM | Response (ms) |\n|---|---|---|---|---|---|---|---|"
    );
    let number = |value: Option<f64>, digits: usize| {
        value.map_or("-".to_string(), |value| format!("{value:.digits$}"))
    };
    for (position, auction) in auctions {
        for unit in &auction.ad_units {
            for bid in &unit.bids {
                let _ = writeln!(
                    out,
                    "| {} | {:.0} | {} | {} | {} | {:?} | {} | {} |",
                    position + 1,
                    auction.started_at_ms,
                    number(auction.duration_ms, 0),
                    cell(&unit.code),
                    cell(&bid.bidder),
                    bid.status,
                    number(bid.cpm, 2),
                    number(bid.response_time_ms, 0),
                );
            }
        }
    }
}

fn slots_section(out: &mut String, detail: &ScanDetail) {
    let _ = writeln!(out, "\n## Ad slots\n");
    if detail.slots.is_empty() {
        let _ = writeln!(out, "No GPT slots were observed.");
        return;
    }
    for group in by_page(&detail.pages, &detail.slots, |slot| slot.page) {
        if let Some(heading) = &group.heading {
            let _ = writeln!(out, "\n### {heading}\n");
        }
        slots_table(out, &group.items);
    }
}

fn slots_table(out: &mut String, slots: &[(usize, &AdSlot)]) {
    let _ = writeln!(
        out,
        "| Slot | Ad unit path | Outcome | Requested (ms) | Rendered (ms) | Prebid targeting | Requested before auction end |\n|---|---|---|---|---|---|---|"
    );
    let time = |value: Option<f64>| value.map_or("-".to_string(), |value| format!("{value:.0}"));
    let flag = |value: Option<bool>| value.map_or("-", |value| if value { "yes" } else { "no" });
    for (_, slot) in slots {
        let _ = writeln!(
            out,
            "| {} | {} | {:?} | {} | {} | {} | {} |",
            cell(&slot.element_id),
            cell(slot.ad_unit_path.as_deref().unwrap_or("-")),
            slot.outcome,
            time(slot.requested_at_ms),
            time(slot.rendered_at_ms),
            flag(slot.had_prebid_targeting),
            flag(slot.requested_before_auction_end),
        );
    }
}

fn scripts_section(out: &mut String, detail: &ScanDetail) {
    let ad_scripts: Vec<_> = detail
        .scripts
        .iter()
        .filter(|script| script.ad_related)
        .collect();
    let _ = writeln!(out, "\n## Ad-related scripts\n");
    if ad_scripts.is_empty() {
        let _ = writeln!(out, "No ad-related scripts were identified.");
        return;
    }
    let _ = writeln!(
        out,
        "| Script | Category | Vendor | Size (bytes) | Classified by | Analyzed by AI |\n|---|---|---|---|---|---|"
    );
    for script in ad_scripts {
        let _ = writeln!(
            out,
            "| {} | {} | {} | {:.0} | {} | {} |",
            cell(&script.url),
            category_label(script),
            cell(script.vendor.as_deref().unwrap_or("-")),
            script.size_bytes,
            cell(&classified_by(script)),
            if script.analyzed { "yes" } else { "no" },
        );
    }
}

/// Only ad-related scripts are listed, so one whose kind of ad code the
/// classifier could not name is not an unknown script.
fn category_label(script: &ScriptRecord) -> String {
    match script.category {
        ScriptCategory::Unknown => "Other ad code".into(),
        category => format!("{category:?}"),
    }
}

/// The rule base, or the model that made the call with the ad probability
/// when it reported one.
fn classified_by(script: &ScriptRecord) -> String {
    match (&script.classifier_model, script.ad_probability) {
        (Some(model), Some(probability)) => format!("{model} (ad probability {probability:.2})"),
        (Some(model), None) => model.clone(),
        (None, _) => format!("{:?}", script.classified_by),
    }
}

fn pipeline_section(out: &mut String, detail: &ScanDetail) {
    let _ = writeln!(
        out,
        "\n## How this report was produced\n\n| Step | Status | Duration (ms) | Model | Tokens in / out | Cost | Outcome |\n|---|---|---|---|---|---|---|"
    );
    for step in &detail.steps {
        let _ = writeln!(
            out,
            "| {} | {:?} | {} | {} | {} / {} | ${:.4} | {} |",
            node_label(step.node),
            step.status,
            step.duration_ms
                .map_or("-".to_string(), |ms| format!("{ms:.0}")),
            step.model.as_deref().unwrap_or("-"),
            step.input_tokens,
            step.output_tokens,
            step.cost_usd,
            cell(step.summary.as_deref().unwrap_or("-")),
        );
    }
}

/// What the analysis model looked at and concluded, in order.
fn ai_trail_section(out: &mut String, logs: &[LogEntry]) {
    let trail: Vec<&LogEntry> = logs
        .iter()
        .filter(|entry| entry.node == PipelineNode::AnalyzeLogic)
        .filter(|entry| {
            matches!(
                entry.kind,
                LogKind::Thinking | LogKind::LlmResponse | LogKind::ToolCall | LogKind::Finding
            )
        })
        .collect();
    if trail.is_empty() {
        return;
    }
    let _ = writeln!(out, "\n### AI reasoning trail\n");
    for entry in trail.iter().take(MAX_TRAIL_ENTRIES) {
        let label = match entry.kind {
            LogKind::Thinking => "Reasoning",
            LogKind::LlmResponse => "Model",
            LogKind::ToolCall => "Tool call",
            _ => "Finding",
        };
        let text: String = entry.message.chars().take(MAX_TRAIL_ENTRY_CHARS).collect();
        let _ = writeln!(out, "- **{label}:** {}", text.replace('\n', " "));
    }
    if trail.len() > MAX_TRAIL_ENTRIES {
        let _ = writeln!(
            out,
            "- ... {} more entries in the scan's audit log",
            trail.len() - MAX_TRAIL_ENTRIES
        );
    }
}

fn cell(text: &str) -> String {
    text.replace('|', "\\|").replace('\n', " ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use adscan_model::FindingSource;

    fn script(classifier_model: Option<&str>, ad_probability: Option<f64>) -> ScriptRecord {
        ScriptRecord {
            pages: Vec::new(),
            id: "s1".into(),
            url: "https://pub.example/js/lazy.js".into(),
            hostname: "pub.example".into(),
            inline: false,
            size_bytes: 100.0,
            hash: "0".repeat(16),
            category: ScriptCategory::Wrapper,
            ad_related: true,
            classification_reason: None,
            classification_confidence: None,
            classified_by: if classifier_model.is_some() {
                FindingSource::Ai
            } else {
                FindingSource::Heuristic
            },
            classifier_model: classifier_model.map(str::to_string),
            ad_probability,
            analyzed: false,
            vendor: None,
            loaded_at_ms: None,
            first_party: true,
            initiator: None,
            request_id: None,
            sha256: None,
        }
    }

    #[test]
    fn an_ad_script_of_no_named_kind_is_not_listed_as_unknown() {
        let mut unnamed = script(Some("jev-1.14.0"), Some(0.62));
        unnamed.category = ScriptCategory::Unknown;
        assert_eq!(category_label(&unnamed), "Other ad code");
        assert_eq!(category_label(&script(None, None)), "Wrapper");
    }

    #[test]
    fn the_script_table_names_the_model_that_classified_a_script() {
        assert_eq!(classified_by(&script(None, None)), "Heuristic");
        assert_eq!(
            classified_by(&script(Some("claude-haiku-5-5"), None)),
            "claude-haiku-5-5"
        );
        assert_eq!(
            classified_by(&script(Some("jev-1.14.0"), Some(0.934))),
            "jev-1.14.0 (ad probability 0.93)"
        );
    }
}
