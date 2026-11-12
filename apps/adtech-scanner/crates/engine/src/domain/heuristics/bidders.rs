//! Rules built on the bidder diagnostics: adapter params, ads.txt,
//! sellers.json and the consent each bidder was sent.

use adscan_model::{
    AdsTxtStatus, BidderDiagnostics, DiagnosticArea, DiagnosticCheck, DiagnosticStatus,
    FindingCategory, Severity,
};

use super::delivery::silent_bidders;
use super::{Rule, RuleContext, heuristic};
use crate::domain::diagnostics::codes;
use crate::domain::finding::{FindingDraft, log_line, metric};

/// A check outcome together with the bidder it belongs to.
type Hit<'a> = (&'a str, &'a DiagnosticCheck);

fn hits<'a>(diagnostics: &'a BidderDiagnostics, wanted: &[&str]) -> Vec<Hit<'a>> {
    diagnostics
        .bidders
        .iter()
        .flat_map(|bidder| {
            bidder
                .checks
                .iter()
                .filter(|check| wanted.contains(&check.code.as_str()))
                .map(move |check| (bidder.bidder.as_str(), check))
        })
        .collect()
}

fn bidders_of(hits: &[Hit]) -> Vec<String> {
    let mut names: Vec<String> = hits.iter().map(|(bidder, _)| bidder.to_string()).collect();
    names.dedup();
    names
}

fn ad_units_of(hits: &[Hit]) -> Vec<String> {
    let mut ad_units: Vec<String> = hits
        .iter()
        .flat_map(|(_, check)| check.ad_units.iter().cloned())
        .collect();
    ad_units.sort();
    ad_units.dedup();
    ad_units
}

/// One evidence entry per outcome: the excerpt when there is one, the
/// summary otherwise.
fn add_evidence(finding: &mut FindingDraft, hits: &[Hit]) {
    for (bidder, check) in hits {
        let label = format!("{bidder}: {}", check.summary);
        finding.evidence.push(match &check.excerpt {
            Some(excerpt) => log_line(label, excerpt.clone()),
            None => metric(
                label,
                if check.ad_units.is_empty() {
                    "Applies to every ad unit of this bidder".to_string()
                } else {
                    format!("Ad units: {}", check.ad_units.join(", "))
                },
            ),
        });
    }
}

fn bullet_list(hits: &[Hit]) -> String {
    hits.iter()
        .map(|(bidder, check)| format!("- `{bidder}`: {}", check.summary))
        .collect::<Vec<_>>()
        .join("\n")
}

/// What to do about one check outcome.
pub(super) fn advice(code: &str) -> Option<&'static str> {
    Some(match code {
        codes::PARAMS_MISSING => {
            "add the missing param to that bidder's entry in every ad unit (Prebid drops a bid whose params fail the adapter's own check before any request is made)"
        }
        codes::PARAMS_WRONG_TYPE => "pass the param in the type the adapter expects",
        codes::PARAMS_TEST_ID => {
            "replace the test id from the adapter documentation with the ids the partner issued for this site"
        }
        codes::PARAMS_SHARED_ID => {
            "ask the partner whether each ad unit should have its own placement id"
        }
        codes::RESPONSE_REASON => {
            "fix what the endpoint's answer names, or take that answer to the partner's support"
        }
        codes::RESPONSE_HTTP_ERROR => {
            "check the adapter params and the account's status with the partner; an HTTP error usually means a malformed request or a blocked account"
        }
        codes::RESPONSE_FAILED => {
            "find out what blocks the request: the page's Content-Security-Policy, a DNS or network problem, or an ad blocker in the browser used for the scan"
        }
        codes::RESPONSE_PREBID_ERROR => {
            "open the page with `?pbjs_debug=true` and fix the error Prebid reports for this adapter"
        }
        codes::ADS_TXT_NOT_LISTED => {
            "add the line the partner provides to ads.txt; buyers that enforce ads.txt do not bid on inventory from sellers it does not list"
        }
        codes::ADS_TXT_ACCOUNT_MISMATCH | codes::ADS_TXT_SCHAIN_MISMATCH => {
            "make ads.txt list the account the bid request actually uses, or correct the id in the wrapper"
        }
        codes::SELLERS_ID_MISSING => {
            "ask the partner to publish this account in its sellers.json, or correct the id in ads.txt"
        }
        codes::SELLERS_CONFIDENTIAL => {
            "ask the partner to lift the confidentiality flag; many buyers avoid confidential sellers"
        }
        codes::SELLERS_TYPE_MISMATCH => {
            "use DIRECT in ads.txt only for accounts the exchange lists as PUBLISHER or BOTH, and RESELLER for INTERMEDIARY"
        }
        codes::SELLERS_DOMAIN_MISMATCH => {
            "declare the owning company with OWNERDOMAIN in ads.txt, or ask the partner to correct the domain"
        }
        codes::CONSENT_NO_STRING => {
            "configure Prebid's `consentManagement` so the TC string reaches the bidders"
        }
        codes::CONSENT_PENDING => {
            "let the auction wait for the user's consent choice (CMP integration and `consentManagement.timeout`)"
        }
        codes::CONSENT_VENDOR_DENIED => {
            "make sure the CMP lists this vendor and asks consent for it"
        }
        _ => return None,
    })
}

fn advice_for(hits: &[Hit]) -> String {
    let mut advice_lines: Vec<&str> = Vec::new();
    for (_, check) in hits {
        if let Some(line) = advice(&check.code).filter(|line| !advice_lines.contains(line)) {
            advice_lines.push(line);
        }
    }
    sentence(&advice_lines.join("; "))
}

fn sentence(text: &str) -> String {
    let mut characters = text.chars();
    match characters.next() {
        Some(first) => format!("{}{}.", first.to_uppercase(), characters.as_str()),
        None => String::new(),
    }
}

/// Bids Prebid drops before the request because the adapter rejects their
/// params.
pub struct BidderParamsInvalid;

impl Rule for BidderParamsInvalid {
    fn id(&self) -> &'static str {
        "bidder-params-invalid"
    }

    fn evaluate(&self, ctx: &RuleContext) -> Vec<FindingDraft> {
        let Some(diagnostics) = ctx.diagnostics else {
            return Vec::new();
        };
        let invalid = hits(
            diagnostics,
            &[codes::PARAMS_MISSING, codes::PARAMS_WRONG_TYPE],
        );
        if invalid.is_empty() {
            return Vec::new();
        }
        let bidders = bidders_of(&invalid);
        let mut finding = heuristic(
            Severity::High,
            FindingCategory::Configuration,
            0.85,
            format!(
                "{} bidder(s) have adapter params the adapter rejects",
                bidders.len()
            ),
        );
        finding.description = format!(
            "The params of these bids fail the check the adapter itself runs (`isBidRequestValid`, Prebid.js 11). \
             Prebid then drops the bid before any request is made, so the bidder is silently absent from those ad units:\n\n{}",
            bullet_list(&invalid)
        );
        finding.recommendation = advice_for(&invalid);
        finding.impact =
            "The affected ad units are auctioned without these bidders, so there is less competition for them"
                .into();
        add_evidence(&mut finding, &invalid);
        finding.affected_slots = ad_units_of(&invalid);
        finding.affected_bidders = bidders;
        vec![finding]
    }
}

/// Ids copied from the adapter documentation.
pub struct BidderTestIds;

impl Rule for BidderTestIds {
    fn id(&self) -> &'static str {
        "bidder-test-ids"
    }

    fn evaluate(&self, ctx: &RuleContext) -> Vec<FindingDraft> {
        let Some(diagnostics) = ctx.diagnostics else {
            return Vec::new();
        };
        let test_ids = hits(diagnostics, &[codes::PARAMS_TEST_ID]);
        if test_ids.is_empty() {
            return Vec::new();
        }
        let bidders = bidders_of(&test_ids);
        let mut finding = heuristic(
            Severity::Medium,
            FindingCategory::Configuration,
            0.8,
            format!(
                "{} bidder(s) run on test ids from the adapter documentation",
                bidders.len()
            ),
        );
        finding.description = format!(
            "These bids use the example ids printed in the Prebid adapter documentation. A test placement \
             answers with test creatives or nothing, and whatever it earns is not credited to this site:\n\n{}",
            bullet_list(&test_ids)
        );
        finding.recommendation = advice_for(&test_ids);
        finding.impact = "These bidders bring no real demand to the affected ad units".into();
        add_evidence(&mut finding, &test_ids);
        finding.affected_slots = ad_units_of(&test_ids);
        finding.affected_bidders = bidders;
        vec![finding]
    }
}

/// Bidders that do bid at times, but whose endpoint also fails. A bidder that
/// never bids is reported, with these causes, by the never-bid rule.
pub struct BidderEndpointErrors;

impl Rule for BidderEndpointErrors {
    fn id(&self) -> &'static str {
        "bidder-endpoint-errors"
    }

    fn evaluate(&self, ctx: &RuleContext) -> Vec<FindingDraft> {
        let Some(diagnostics) = ctx.diagnostics else {
            return Vec::new();
        };
        let silent = silent_bidders(ctx);
        let mut errors = hits(
            diagnostics,
            &[
                codes::RESPONSE_HTTP_ERROR,
                codes::RESPONSE_FAILED,
                codes::RESPONSE_PREBID_ERROR,
            ],
        );
        errors.retain(|(bidder, _)| !silent.iter().any(|(name, _)| name == bidder));
        if errors.is_empty() {
            return Vec::new();
        }
        let bidders = bidders_of(&errors);
        let mut finding = heuristic(
            Severity::Medium,
            FindingCategory::Configuration,
            0.8,
            format!("{} bidder(s) had failing bid requests", bidders.len()),
        );
        finding.description = format!(
            "Requests to these bidders failed outright during the scan. A request that fails cannot bid, \
             whatever the placement is worth:\n\n{}",
            bullet_list(&errors)
        );
        finding.recommendation = advice_for(&errors);
        finding.impact = "Part of these bidders' demand never reaches the auction".into();
        add_evidence(&mut finding, &errors);
        finding.affected_bidders = bidders;
        vec![finding]
    }
}

/// The site answers that it has no ads.txt.
pub struct AdsTxtMissing;

impl Rule for AdsTxtMissing {
    fn id(&self) -> &'static str {
        "ads-txt-missing"
    }

    fn evaluate(&self, ctx: &RuleContext) -> Vec<FindingDraft> {
        let Some(diagnostics) = ctx.diagnostics else {
            return Vec::new();
        };
        let report = &diagnostics.ads_txt;
        if report.status != AdsTxtStatus::Missing {
            return Vec::new();
        }
        let mut finding = heuristic(
            Severity::High,
            FindingCategory::Configuration,
            0.85,
            "The site has no ads.txt",
        );
        finding.description = format!(
            "`{}` answered that there is no such file. Without ads.txt no seller is authorised for this \
             site: most demand-side platforms then lower their bids or do not bid at all, whichever exchange the \
             request comes through.",
            report.url
        );
        finding.recommendation = "Publish an ads.txt at the root of the domain with the line each partner provides \
             (`<ad system domain>, <account id>, DIRECT|RESELLER[, <certification id>]`), served as `text/plain`."
            .into();
        finding.impact = "Every header bidding partner sells this inventory as unauthorised".into();
        finding.evidence.push(metric(
            "ads.txt request",
            format!("{}: not found", report.url),
        ));
        finding.affected_bidders = diagnostics
            .bidders
            .iter()
            .map(|bidder| bidder.bidder.clone())
            .collect();
        vec![finding]
    }
}

/// Bidders the site's ads.txt does not authorise, or authorises under another
/// account than the one in use.
pub struct BiddersNotInAdsTxt;

impl Rule for BiddersNotInAdsTxt {
    fn id(&self) -> &'static str {
        "ads-txt-bidder-not-authorised"
    }

    fn evaluate(&self, ctx: &RuleContext) -> Vec<FindingDraft> {
        let Some(diagnostics) = ctx.diagnostics else {
            return Vec::new();
        };
        let unauthorised = hits(
            diagnostics,
            &[
                codes::ADS_TXT_NOT_LISTED,
                codes::ADS_TXT_ACCOUNT_MISMATCH,
                codes::ADS_TXT_SCHAIN_MISMATCH,
            ],
        );
        if unauthorised.is_empty() {
            return Vec::new();
        }
        let bidders = bidders_of(&unauthorised);
        let mut finding = heuristic(
            Severity::Medium,
            FindingCategory::Configuration,
            0.8,
            format!(
                "{} bidder(s) are not authorised by the site's ads.txt",
                bidders.len()
            ),
        );
        finding.description = format!(
            "The page sends bid requests to these bidders, but `{}` does not authorise the seller account \
             behind them. Buyers that enforce ads.txt skip such inventory, so the bidder answers without a bid or \
             with low ones:\n\n{}",
            diagnostics.ads_txt.url,
            bullet_list(&unauthorised)
        );
        finding.recommendation = advice_for(&unauthorised);
        finding.impact = "Demand from these partners is filtered out before it can bid".into();
        add_evidence(&mut finding, &unauthorised);
        finding.evidence.push(metric(
            "ads.txt",
            format!(
                "{}: {} seller records",
                diagnostics.ads_txt.url, diagnostics.ads_txt.record_count
            ),
        ));
        finding.affected_bidders = bidders;
        vec![finding]
    }
}

/// Lines of ads.txt that crawlers discard.
pub struct AdsTxtInvalidLines;

impl Rule for AdsTxtInvalidLines {
    fn id(&self) -> &'static str {
        "ads-txt-invalid-lines"
    }

    fn evaluate(&self, ctx: &RuleContext) -> Vec<FindingDraft> {
        let Some(report) = ctx.diagnostics.map(|diagnostics| &diagnostics.ads_txt) else {
            return Vec::new();
        };
        if report.invalid_line_count == 0 {
            return Vec::new();
        }
        let mut finding = heuristic(
            Severity::Low,
            FindingCategory::Hygiene,
            0.9,
            format!(
                "{} line(s) of ads.txt are not valid records",
                report.invalid_line_count
            ),
        );
        finding.description = format!(
            "`{}` contains lines that are neither a seller record, a variable nor a comment. Crawlers discard \
             such lines, so a seller that was meant to be authorised by one of them is not.",
            report.url
        );
        finding.recommendation = "Rewrite each line as `<ad system domain>, <account id>, DIRECT|RESELLER[, <certification id>]`, \
             with commas between the fields, or turn it into a comment with `#`."
            .into();
        finding.impact = "Sellers listed on the broken lines count as unauthorised".into();
        for line in &report.invalid_lines {
            finding.evidence.push(log_line(
                format!("Line {}: {}", line.line, line.problem),
                line.excerpt.clone(),
            ));
        }
        vec![finding]
    }
}

/// What the exchanges' sellers.json say about the accounts in ads.txt.
pub struct SellersJsonIssues;

impl Rule for SellersJsonIssues {
    fn id(&self) -> &'static str {
        "sellers-json-issues"
    }

    fn evaluate(&self, ctx: &RuleContext) -> Vec<FindingDraft> {
        let Some(diagnostics) = ctx.diagnostics else {
            return Vec::new();
        };
        let issues: Vec<Hit> = diagnostics
            .bidders
            .iter()
            .flat_map(|bidder| {
                bidder
                    .checks
                    .iter()
                    .filter(|check| {
                        check.area == DiagnosticArea::SellersJson
                            && matches!(
                                check.status,
                                DiagnosticStatus::Problem | DiagnosticStatus::Notice
                            )
                    })
                    .map(move |check| (bidder.bidder.as_str(), check))
            })
            .collect();
        if issues.is_empty() {
            return Vec::new();
        }
        let has_problem = issues
            .iter()
            .any(|(_, check)| check.status == DiagnosticStatus::Problem);
        let bidders = bidders_of(&issues);
        let mut finding = heuristic(
            if has_problem {
                Severity::Medium
            } else {
                Severity::Low
            },
            FindingCategory::Configuration,
            if has_problem { 0.75 } else { 0.55 },
            format!(
                "sellers.json of {} exchange(s) does not match the site's ads.txt",
                bidders.len()
            ),
        );
        finding.description = format!(
            "Buyers verify a seller by reading the exchange's sellers.json next to the site's ads.txt. Where the \
             two disagree, the path looks unverified and is bid on less:\n\n{}",
            bullet_list(&issues)
        );
        finding.recommendation = advice_for(&issues);
        finding.impact = "Supply paths through these exchanges look unverified to buyers".into();
        add_evidence(&mut finding, &issues);
        finding.affected_bidders = bidders;
        vec![finding]
    }
}

/// Auctions that start while the user's consent choice is still open.
pub struct AuctionBeforeConsent;

impl Rule for AuctionBeforeConsent {
    fn id(&self) -> &'static str {
        "auction-before-consent"
    }

    fn evaluate(&self, ctx: &RuleContext) -> Vec<FindingDraft> {
        let Some(consent) = ctx.diagnostics.map(|diagnostics| &diagnostics.consent) else {
            return Vec::new();
        };
        if consent.auctions_before_consent == 0 {
            return Vec::new();
        }
        let mut finding = heuristic(
            Severity::High,
            FindingCategory::Consent,
            0.75,
            format!(
                "{} of {} auction(s) ran before the user's consent was known",
                consent.auctions_before_consent, consent.auction_count
            ),
        );
        finding.description = "The consent platform says GDPR applies to this page view, and these auctions \
             started before it reported the user's choice. Bidders asked at that moment get no usable consent \
             string; most of them answer without a bid, and the impression is sold for less or not at all. \
             One page view shows the order of events for one visitor: a returning visitor whose choice is \
             already stored may not be affected."
            .into();
        finding.recommendation = "Start the first auction from the CMP's callback, or configure Prebid's \
             `consentManagement` with the CMP API and a `timeout` long enough for the stored choice to load, \
             and decide on purpose what happens to visitors who have not answered the banner yet."
            .into();
        finding.impact =
            "First-time visitors are auctioned without consent, where most bidders pass".into();
        finding.evidence.push(metric(
            "Consent timing",
            format!(
                "consent known at: {} | auctions before that: {} of {} | Prebid consentManagement: {}",
                consent
                    .resolved_at_ms
                    .map_or("never during the scan".to_string(), super::ms),
                consent.auctions_before_consent,
                consent.auction_count,
                match consent.consent_management_configured {
                    Some(true) => "configured",
                    Some(false) => "not configured",
                    None => "unknown",
                },
            ),
        ));
        vec![finding]
    }
}

/// Bidders that were asked without a consent string, or without the user's
/// consent for them as a vendor.
pub struct BiddersWithoutConsent;

impl Rule for BiddersWithoutConsent {
    fn id(&self) -> &'static str {
        "bidders-without-consent"
    }

    fn evaluate(&self, ctx: &RuleContext) -> Vec<FindingDraft> {
        let Some(diagnostics) = ctx.diagnostics else {
            return Vec::new();
        };
        let mut unconsented = hits(
            diagnostics,
            &[codes::CONSENT_NO_STRING, codes::CONSENT_VENDOR_DENIED],
        );
        // A request that left before the user chose has no string to carry;
        // that is the auction-before-consent finding, not a second one.
        let asked_too_early = bidders_of(&hits(diagnostics, &[codes::CONSENT_PENDING]));
        unconsented.retain(|(bidder, check)| {
            check.code != codes::CONSENT_NO_STRING
                || !asked_too_early.iter().any(|name| name == bidder)
        });
        if unconsented.is_empty() {
            return Vec::new();
        }
        let bidders = bidders_of(&unconsented);
        let mut finding = heuristic(
            Severity::High,
            FindingCategory::Consent,
            0.8,
            format!(
                "{} bidder(s) were asked without the consent they need",
                bidders.len()
            ),
        );
        finding.description = format!(
            "GDPR applies to this page view, and these bid requests did not carry consent the bidder can act \
             on. A bidder without a consent string, or without the user's consent for it as a vendor, must not \
             use the user's data and usually does not bid:\n\n{}",
            bullet_list(&unconsented)
        );
        finding.recommendation = advice_for(&unconsented);
        finding.impact = "These bidders pass on most requests from regulated regions".into();
        add_evidence(&mut finding, &unconsented);
        if diagnostics.consent.consent_management_configured == Some(false) {
            finding.evidence.push(metric(
                "Prebid consent setup",
                "`pbjs.getConfig()` has no `consentManagement` section, so Prebid does not read the CMP",
            ));
        }
        finding.affected_bidders = bidders;
        vec![finding]
    }
}
