//! Merges heuristic and AI findings, ranks them and scores the ad stack.

use std::cmp::Ordering;

use adscan_model::{
    DeviceProfile, EvidenceKind, Finding, FindingCategory, FindingSource, ScanMetrics, Severity,
    SeverityCounts,
};

/// Fold every heuristic finding that describes the same root cause as an AI
/// finding into that AI finding, then rank: severity, then estimated impact,
/// then confidence.
pub fn merge_and_rank(findings: Vec<Finding>) -> Vec<Finding> {
    let (mut ai, heuristic): (Vec<Finding>, Vec<Finding>) = findings
        .into_iter()
        .partition(|finding| finding.source == FindingSource::Ai);
    let mut merged = Vec::new();
    for rule_finding in heuristic {
        match ai
            .iter_mut()
            .find(|candidate| same_root_cause(candidate, &rule_finding))
        {
            Some(ai_finding) => absorb(ai_finding, rule_finding),
            None => merged.push(rule_finding),
        }
    }
    merged.extend(ai);
    merged.sort_by(rank);
    merged
}

/// Two findings share a root cause when their categories are related and they
/// do not point at disjoint slots or bidders.
fn same_root_cause(ai: &Finding, heuristic: &Finding) -> bool {
    let both_unscoped = |a: &Finding, b: &Finding| {
        a.affected_slots.is_empty()
            && a.affected_bidders.is_empty()
            && b.affected_slots.is_empty()
            && b.affected_bidders.is_empty()
    };
    if !categories_related(ai.category, heuristic.category) {
        return false;
    }
    if both_unscoped(ai, heuristic) {
        return ai.category == heuristic.category;
    }
    compatible(&ai.affected_slots, &heuristic.affected_slots)
        && compatible(&ai.affected_bidders, &heuristic.affected_bidders)
}

/// A logic bug is the cause behind a lazy-load symptom; otherwise categories
/// must be equal.
fn categories_related(a: FindingCategory, b: FindingCategory) -> bool {
    use FindingCategory::{LazyLoad, LogicBug};
    a == b || matches!((a, b), (LogicBug, LazyLoad) | (LazyLoad, LogicBug))
}

fn compatible(a: &[String], b: &[String]) -> bool {
    a.is_empty() || b.is_empty() || a.iter().any(|item| b.contains(item))
}

/// Keep the AI's narrative and code evidence, add the rule's measurements.
fn absorb(ai: &mut Finding, heuristic: Finding) {
    ai.severity = ai.severity.max(heuristic.severity);
    ai.confidence = ai.confidence.max(heuristic.confidence);
    ai.estimated_revenue_impact_pct = match (
        ai.estimated_revenue_impact_pct,
        heuristic.estimated_revenue_impact_pct,
    ) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (a, b) => a.or(b),
    };
    ai.evidence.extend(
        heuristic
            .evidence
            .into_iter()
            .filter(|evidence| evidence.kind != EvidenceKind::Code),
    );
    extend_unique(&mut ai.affected_slots, heuristic.affected_slots);
    extend_unique(&mut ai.affected_bidders, heuristic.affected_bidders);
}

fn extend_unique(target: &mut Vec<String>, extra: Vec<String>) {
    for item in extra {
        if !target.contains(&item) {
            target.push(item);
        }
    }
}

fn rank(a: &Finding, b: &Finding) -> Ordering {
    b.severity
        .cmp(&a.severity)
        .then_with(|| {
            let impact = |finding: &Finding| finding.estimated_revenue_impact_pct.unwrap_or(0.0);
            impact(b).total_cmp(&impact(a))
        })
        .then_with(|| b.confidence.total_cmp(&a.confidence))
}

pub fn severity_counts(findings: &[Finding]) -> SeverityCounts {
    let mut counts = SeverityCounts::default();
    for finding in findings {
        match finding.severity {
            Severity::Critical => counts.critical += 1,
            Severity::High => counts.high += 1,
            Severity::Medium => counts.medium += 1,
            Severity::Low => counts.low += 1,
            Severity::Info => counts.info += 1,
        }
    }
    counts
}

/// Health score, 0-100:
///
/// `score = round(clamp(100 - sum(penalty(severity) * confidence), 0, 100))`
///
/// with penalties critical 30, high 18, medium 8, low 3, info 0. Weighting by
/// confidence keeps a speculative finding from sinking the score.
pub fn health_score(findings: &[Finding]) -> u32 {
    let deductions: f64 = findings
        .iter()
        .map(|finding| severity_penalty(finding.severity) * finding.confidence.clamp(0.0, 1.0))
        .sum();
    (100.0 - deductions).clamp(0.0, 100.0).round() as u32
}

fn severity_penalty(severity: Severity) -> f64 {
    match severity {
        Severity::Critical => 30.0,
        Severity::High => 18.0,
        Severity::Medium => 8.0,
        Severity::Low => 3.0,
        Severity::Info => 0.0,
    }
}

/// Executive summary used when the analysis model did not write one.
pub fn template_overview(
    hostname: &str,
    device: DeviceProfile,
    metrics: &ScanMetrics,
    findings: &[Finding],
    score: u32,
) -> String {
    let device = match device {
        DeviceProfile::Desktop => "desktop",
        DeviceProfile::Mobile => "mobile",
    };
    let stack = match (metrics.has_prebid, metrics.has_gpt) {
        (true, true) => "Prebid.js with Google Publisher Tag",
        (true, false) => "Prebid.js (no Google Publisher Tag seen)",
        (false, true) => "Google Publisher Tag without Prebid.js",
        (false, false) => "no Prebid.js or Google Publisher Tag",
    };
    let mut overview = format!(
        "{hostname} ({device}) runs {stack}: {} auction(s), {} bidder(s), {} slot(s), fill rate {:.0}%. Ad stack health is {score}/100.",
        metrics.auction_count,
        metrics.bidder_count,
        metrics.slot_count,
        metrics.fill_rate * 100.0,
    );
    let counts = severity_counts(findings);
    if findings.is_empty() {
        overview.push_str(" No revenue leaks were detected in this scan.");
    } else {
        overview.push_str(&format!(
            " {} finding(s): {} critical, {} high, {} medium, {} low.",
            findings.len(),
            counts.critical,
            counts.high,
            counts.medium,
            counts.low + counts.info,
        ));
    }
    if let Some(top) = findings.first() {
        overview.push_str(&format!(
            " Most important: {}.",
            top.title.trim_end_matches('.')
        ));
    }
    overview
}

#[cfg(test)]
mod tests {
    use super::*;
    use adscan_model::Evidence;

    fn finding(source: FindingSource, category: FindingCategory, severity: Severity) -> Finding {
        Finding {
            id: format!("{source:?}-{category:?}-{severity:?}"),
            scan_id: "scan".into(),
            severity,
            category,
            source,
            title: format!("{source:?} {category:?}"),
            description: String::new(),
            recommendation: String::new(),
            confidence: 0.8,
            impact: String::new(),
            estimated_revenue_impact_pct: None,
            evidence: vec![Evidence {
                kind: if source == FindingSource::Ai {
                    EvidenceKind::Code
                } else {
                    EvidenceKind::Metric
                },
                label: format!("{source:?} evidence"),
                body: String::new(),
                script_url: None,
                line: None,
            }],
            affected_slots: vec![],
            affected_bidders: vec![],
        }
    }

    #[test]
    fn ai_logic_bug_absorbs_the_lazy_load_symptom() {
        let mut ai = finding(FindingSource::Ai, FindingCategory::LogicBug, Severity::High);
        ai.confidence = 0.7;
        ai.affected_slots = vec!["top".into()];
        let mut rule = finding(
            FindingSource::Heuristic,
            FindingCategory::LazyLoad,
            Severity::Critical,
        );
        rule.affected_slots = vec!["top".into(), "side".into()];
        rule.estimated_revenue_impact_pct = Some(30.0);

        let merged = merge_and_rank(vec![rule, ai]);

        assert_eq!(merged.len(), 1);
        let finding = &merged[0];
        assert_eq!(finding.source, FindingSource::Ai);
        assert_eq!(finding.title, "Ai LogicBug");
        assert_eq!(finding.severity, Severity::Critical);
        assert_eq!(finding.confidence, 0.8);
        assert_eq!(finding.estimated_revenue_impact_pct, Some(30.0));
        assert_eq!(finding.affected_slots, ["top", "side"]);
        let kinds: Vec<EvidenceKind> = finding.evidence.iter().map(|e| e.kind).collect();
        assert_eq!(kinds, [EvidenceKind::Code, EvidenceKind::Metric]);
    }

    #[test]
    fn unrelated_or_disjoint_findings_stay_separate() {
        let mut ai = finding(FindingSource::Ai, FindingCategory::Timing, Severity::Medium);
        ai.affected_bidders = vec!["rubicon".into()];
        let mut other_bidder = finding(
            FindingSource::Heuristic,
            FindingCategory::Timing,
            Severity::Medium,
        );
        other_bidder.affected_bidders = vec!["criteo".into()];
        let pricing = finding(
            FindingSource::Heuristic,
            FindingCategory::Pricing,
            Severity::Low,
        );
        assert_eq!(merge_and_rank(vec![ai, other_bidder, pricing]).len(), 3);
    }

    #[test]
    fn unscoped_findings_merge_only_within_the_same_category() {
        let ai = finding(FindingSource::Ai, FindingCategory::LogicBug, Severity::High);
        let lazy = finding(
            FindingSource::Heuristic,
            FindingCategory::LazyLoad,
            Severity::High,
        );
        assert_eq!(merge_and_rank(vec![ai.clone(), lazy]).len(), 2);
        let mut same = finding(
            FindingSource::Heuristic,
            FindingCategory::LogicBug,
            Severity::Low,
        );
        same.id = "rule".into();
        assert_eq!(merge_and_rank(vec![ai, same]).len(), 1);
    }

    #[test]
    fn ranking_is_severity_then_impact_then_confidence() {
        let low = finding(
            FindingSource::Heuristic,
            FindingCategory::Pricing,
            Severity::Low,
        );
        let mut high_small = finding(
            FindingSource::Heuristic,
            FindingCategory::Fill,
            Severity::High,
        );
        high_small.estimated_revenue_impact_pct = Some(5.0);
        let mut high_big = finding(
            FindingSource::Heuristic,
            FindingCategory::Timing,
            Severity::High,
        );
        high_big.estimated_revenue_impact_pct = Some(20.0);
        let ranked = merge_and_rank(vec![low, high_small, high_big]);
        let categories: Vec<FindingCategory> = ranked.iter().map(|f| f.category).collect();
        assert_eq!(
            categories,
            [
                FindingCategory::Timing,
                FindingCategory::Fill,
                FindingCategory::Pricing
            ]
        );
    }

    #[test]
    fn health_score_follows_the_documented_formula() {
        assert_eq!(health_score(&[]), 100);
        let mut critical = finding(
            FindingSource::Heuristic,
            FindingCategory::LazyLoad,
            Severity::Critical,
        );
        critical.confidence = 0.5;
        let medium = finding(
            FindingSource::Heuristic,
            FindingCategory::Fill,
            Severity::Medium,
        );
        // 100 - 30*0.5 - 8*0.8 = 78.6
        assert_eq!(health_score(&[critical.clone(), medium]), 79);
        let many: Vec<Finding> = (0..10).map(|_| critical.clone()).collect();
        assert_eq!(health_score(&many), 0);
    }

    #[test]
    fn severity_counts_and_overview_summarise_findings() {
        let findings = vec![
            finding(
                FindingSource::Ai,
                FindingCategory::LogicBug,
                Severity::Critical,
            ),
            finding(
                FindingSource::Heuristic,
                FindingCategory::Pricing,
                Severity::Low,
            ),
        ];
        let counts = severity_counts(&findings);
        assert_eq!((counts.critical, counts.low, counts.high), (1, 1, 0));
        let metrics = ScanMetrics {
            has_prebid: true,
            has_gpt: true,
            auction_count: 2,
            ..ScanMetrics::default()
        };
        let overview = template_overview(
            "pub.example",
            DeviceProfile::Mobile,
            &metrics,
            &findings,
            67,
        );
        assert!(overview.contains("pub.example (mobile)"));
        assert!(overview.contains("67/100"));
        assert!(overview.contains("Most important: Ai LogicBug."));
        let clean = template_overview("pub.example", DeviceProfile::Desktop, &metrics, &[], 100);
        assert!(clean.contains("No revenue leaks"));
    }
}
