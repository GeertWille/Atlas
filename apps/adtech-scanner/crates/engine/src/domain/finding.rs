//! A finding before it is given an identity and attached to a scan.

use adscan_model::{Evidence, EvidenceKind, Finding, FindingCategory, FindingSource, Severity};

#[derive(Debug, Clone, PartialEq)]
pub struct FindingDraft {
    pub severity: Severity,
    pub category: FindingCategory,
    pub source: FindingSource,
    pub title: String,
    pub description: String,
    pub recommendation: String,
    pub confidence: f64,
    pub impact: String,
    pub estimated_revenue_impact_pct: Option<f64>,
    pub evidence: Vec<Evidence>,
    pub affected_slots: Vec<String>,
    pub affected_bidders: Vec<String>,
    /// Page views it was seen on; empty when it is not specific to a page.
    pub pages: Vec<u32>,
}

impl FindingDraft {
    pub fn into_finding(self, id: String, scan_id: &str) -> Finding {
        Finding {
            id,
            scan_id: scan_id.to_string(),
            severity: self.severity,
            category: self.category,
            source: self.source,
            title: self.title,
            description: self.description,
            recommendation: self.recommendation,
            confidence: self.confidence.clamp(0.0, 1.0),
            impact: self.impact,
            estimated_revenue_impact_pct: self
                .estimated_revenue_impact_pct
                .map(|pct| pct.clamp(0.0, 100.0)),
            evidence: self.evidence,
            affected_slots: self.affected_slots,
            affected_bidders: self.affected_bidders,
            pages: self.pages,
        }
    }
}

pub fn metric(label: impl Into<String>, body: impl Into<String>) -> Evidence {
    evidence(EvidenceKind::Metric, label, body)
}

pub fn timeline(label: impl Into<String>, body: impl Into<String>) -> Evidence {
    evidence(EvidenceKind::Timeline, label, body)
}

pub fn log_line(label: impl Into<String>, body: impl Into<String>) -> Evidence {
    evidence(EvidenceKind::Log, label, body)
}

fn evidence(kind: EvidenceKind, label: impl Into<String>, body: impl Into<String>) -> Evidence {
    Evidence {
        kind,
        label: label.into(),
        body: body.into(),
        script_url: None,
        line: None,
        end_line: None,
        request_id: None,
        script_id: None,
    }
}
