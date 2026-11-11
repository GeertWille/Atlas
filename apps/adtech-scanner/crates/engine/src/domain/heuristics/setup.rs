use std::collections::BTreeSet;

use adscan_model::{FindingCategory, ScriptCategory, Severity, TrackerEventKind as Ev};

use super::{Rule, RuleContext, heuristic, ms};
use crate::domain::finding::{FindingDraft, log_line, metric};

/// Prebid majors at or below this are several yearly releases behind (v7 dates
/// from 2022) and miss adapter and privacy-module updates.
const OUTDATED_PREBID_MAJOR: u32 = 7;

pub struct NoPriceFloors;

impl Rule for NoPriceFloors {
    fn id(&self) -> &'static str {
        "no-price-floors"
    }

    fn evaluate(&self, ctx: &RuleContext) -> Vec<FindingDraft> {
        let floors_missing = ctx.prebid_config().is_some_and(|config| !config.has_floors);
        if !floors_missing || ctx.auctions().is_empty() {
            return Vec::new();
        }
        let mut finding = heuristic(
            Severity::Low,
            FindingCategory::Pricing,
            0.6,
            "No Prebid price floors configured",
        );
        finding.description = "`pbjs.getConfig()` shows no `floors` configuration. Without the Price Floors module \
             bidders are not told a minimum price, and floors can only be enforced later in the ad server, where \
             bidders cannot react to them."
            .into();
        finding.recommendation = "Enable the Prebid Price Floors module with floors per ad unit / media type / size \
             (static or from a floor provider) and keep them aligned with the unified pricing rules in the ad server."
            .into();
        finding.impact = "Bidders may bid below the price the inventory could clear at".into();
        finding.evidence.push(metric("Prebid config", "floors: not configured"));
        vec![finding]
    }
}

pub struct DuplicatePrebid;

impl Rule for DuplicatePrebid {
    fn id(&self) -> &'static str {
        "duplicate-prebid"
    }

    fn evaluate(&self, ctx: &RuleContext) -> Vec<FindingDraft> {
        let mut globals = BTreeSet::new();
        let mut versions = BTreeSet::new();
        for event in &ctx.capture.events {
            if let Ev::PrebidDetected {
                global_name, version, ..
            } = &event.kind
            {
                globals.insert(global_name.as_str());
                versions.extend(version.as_deref());
            }
        }
        let library_builds: BTreeSet<&str> = ctx
            .scripts
            .iter()
            .filter(|script| script.category == ScriptCategory::Prebid)
            .map(|script| script.hash.as_str())
            .collect();
        if globals.len() < 2 && versions.len() < 2 && library_builds.len() < 2 {
            return Vec::new();
        }
        let mut finding = heuristic(
            Severity::Medium,
            FindingCategory::Hygiene,
            0.8,
            "Prebid.js is loaded more than once",
        );
        finding.description = format!(
            "The page runs {} Prebid global(s) ({}) with version(s) {} from {} distinct library build(s). Parallel \
             Prebid instances run separate auctions for the same inventory, double the bidder requests and user \
             syncs, and can overwrite each other's targeting.",
            globals.len(),
            globals.iter().copied().collect::<Vec<_>>().join(", "),
            if versions.is_empty() {
                "unknown".to_string()
            } else {
                versions.iter().copied().collect::<Vec<_>>().join(", ")
            },
            library_builds.len(),
        );
        finding.recommendation = "Keep a single Prebid build on the page. Find which tag (wrapper, tag manager, \
             video player or a third-party widget) injects the second copy and remove it or share one instance."
            .into();
        finding.impact = "Duplicate bid requests, slower auctions and conflicting key-values".into();
        finding.evidence.push(metric(
            "Prebid instances",
            ctx.scripts
                .iter()
                .filter(|script| script.category == ScriptCategory::Prebid)
                .map(|script| format!("{} ({})", script.url, script.hash))
                .collect::<Vec<_>>()
                .join("\n"),
        ));
        vec![finding]
    }
}

pub struct OutdatedPrebid;

impl Rule for OutdatedPrebid {
    fn id(&self) -> &'static str {
        "outdated-prebid"
    }

    fn evaluate(&self, ctx: &RuleContext) -> Vec<FindingDraft> {
        let outdated = ctx.capture.events.iter().find_map(|event| match &event.kind {
            Ev::PrebidDetected {
                version: Some(version),
                ..
            } => major_version(version)
                .filter(|major| *major <= OUTDATED_PREBID_MAJOR)
                .map(|major| (version.clone(), major)),
            _ => None,
        });
        let Some((version, major)) = outdated else {
            return Vec::new();
        };
        let mut finding = heuristic(
            Severity::Medium,
            FindingCategory::Hygiene,
            0.8,
            format!("Prebid.js {version} is several major versions old"),
        );
        finding.description = format!(
            "The page runs Prebid.js {version} (major {major}). Bidder adapters, consent and identity modules are \
             only maintained for recent majors; partners gradually stop supporting old adapter versions, which \
             shows up as lower bid rates and CPMs rather than as errors."
        );
        finding.recommendation = "Plan an upgrade to a current Prebid.js major, following the release notes for \
             breaking changes, and rebuild with only the adapters and modules in use."
            .into();
        finding.impact = "Outdated adapters bid less often and lack newer demand features".into();
        finding.evidence.push(metric("Prebid version", version));
        vec![finding]
    }
}

fn major_version(version: &str) -> Option<u32> {
    version
        .trim_start_matches(['v', 'V'])
        .split('.')
        .next()?
        .parse()
        .ok()
}

/// A CMP runs on the page but Prebid is not configured to read it.
pub struct ConsentNotWired;

impl Rule for ConsentNotWired {
    fn id(&self) -> &'static str {
        "consent-not-wired"
    }

    fn evaluate(&self, ctx: &RuleContext) -> Vec<FindingDraft> {
        let prebid_ignores_consent = ctx
            .prebid_config()
            .is_some_and(|config| !config.has_consent_management);
        let cmp = ctx
            .scripts
            .iter()
            .find(|script| script.category == ScriptCategory::Cmp);
        if !prebid_ignores_consent || (cmp.is_none() && !ctx.capture.consent_accepted) {
            return Vec::new();
        }
        let mut finding = heuristic(
            Severity::Medium,
            FindingCategory::Consent,
            0.7,
            "A consent platform is present but Prebid consent management is not configured",
        );
        finding.description = "The page runs a consent management platform, but `pbjs.getConfig()` has no \
             `consentManagement` section. Prebid then neither waits for the user's choice nor passes the TCF / GPP \
             string to bidders. In regulated regions bidders without a consent string bid far less or not at all."
            .into();
        finding.recommendation = "Add the `consentManagementTcf` (and GPP / USP where relevant) module to the Prebid \
             build and configure `consentManagement` with a `cmpApi` and a sensible `timeout`."
            .into();
        finding.impact = "Bidders receive no consent signal and suppress or lower their bids".into();
        finding.evidence.push(metric(
            "Consent setup",
            format!(
                "CMP: {} | Prebid consentManagement: not configured",
                cmp.map_or("consent banner accepted during scan".to_string(), |script| {
                    script.vendor.clone().unwrap_or_else(|| script.hostname.clone())
                }),
            ),
        ));
        vec![finding]
    }
}

/// Prebid logged that the CMP did not answer in time.
pub struct ConsentTimeout;

impl Rule for ConsentTimeout {
    fn id(&self) -> &'static str {
        "consent-timeout"
    }

    fn evaluate(&self, ctx: &RuleContext) -> Vec<FindingDraft> {
        let complaints: Vec<(f64, &str)> = ctx
            .capture
            .events
            .iter()
            .filter_map(|event| match &event.kind {
                Ev::PrebidLog { message, .. } if is_consent_failure(message) => Some((event.t, message.as_str())),
                _ => None,
            })
            .collect();
        if complaints.is_empty() {
            return Vec::new();
        }
        let mut finding = heuristic(
            Severity::High,
            FindingCategory::Consent,
            0.8,
            "Prebid could not get consent from the CMP in time",
        );
        finding.description = "Prebid's consent management module reported that the CMP did not respond or timed \
             out. Depending on configuration the auction is then cancelled or runs without a consent string, and \
             either way most bidders will not bid."
            .into();
        finding.recommendation = "Load the CMP stub before Prebid, make sure the CMP API (`__tcfapi` / `__gpp`) is \
             reachable from the frame Prebid runs in, and raise `consentManagement.timeout` if the CMP is slow."
            .into();
        finding.impact = "Auctions are blocked or run without consent, losing most header bidding demand".into();
        for (t, message) in complaints.into_iter().take(5) {
            finding.evidence.push(log_line(
                format!("Prebid log at {}", ms(t)),
                message.chars().take(300).collect::<String>(),
            ));
        }
        vec![finding]
    }
}

fn is_consent_failure(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    let about_consent = message.contains("consent") || message.contains("cmp");
    let failed = ["timed out", "timeout", "did not load", "not found", "failed"]
        .iter()
        .any(|marker| message.contains(marker));
    about_consent && failed
}
