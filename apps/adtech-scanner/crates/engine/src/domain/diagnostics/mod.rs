//! Bidder diagnostics: what can be verified about each bidder's setup without
//! asking the partner. Adapter params, the endpoint's own answers, the site's
//! ads.txt, the exchanges' sellers.json and the consent each bidder was sent.

pub mod adapters;
pub mod ads_txt;
pub mod consent;
pub mod params;
pub mod sellers_json;
pub mod site;

use std::collections::{BTreeMap, BTreeSet};

use adscan_model::{
    AdsTxtLineProblem, AdsTxtReport, AdsTxtStatus, BidStatus, BidderDiagnosis, BidderDiagnostics,
    Capture, DiagnosticArea, DiagnosticCheck, DiagnosticStatus, SchainNode, TrackerEventKind as Ev,
};
use serde_json::Value;

use self::adapters::{AdapterSpec, adapter_for};
use self::ads_txt::{AdsTxt, AdsTxtOutcome, Record, Relationship};
use self::consent::{ConsentTimeline, RequestSignals};
use self::params::{BidConfig, check_params, scalar_text};
use self::sellers_json::{SellersOutcome, SiteIdentity};
use super::telemetry::Telemetry;

/// Stable identifiers of the check outcomes.
pub mod codes {
    pub const PARAMS_OK: &str = "params-ok";
    pub const PARAMS_MISSING: &str = "params-missing";
    pub const PARAMS_WRONG_TYPE: &str = "params-wrong-type";
    pub const PARAMS_TEST_ID: &str = "params-test-id";
    pub const PARAMS_SHARED_ID: &str = "params-shared-id";
    pub const PARAMS_REJECTED: &str = "params-rejected";
    pub const PARAMS_UNKNOWN_ADAPTER: &str = "params-unknown-adapter";
    pub const PARAMS_NOT_CAPTURED: &str = "params-not-captured";
    pub const PARAMS_SERVER_SIDE: &str = "params-server-side";

    pub const RESPONSE_OK: &str = "response-ok";
    pub const RESPONSE_REASON: &str = "response-reason";
    pub const RESPONSE_HTTP_ERROR: &str = "response-http-error";
    pub const RESPONSE_FAILED: &str = "response-failed";
    pub const RESPONSE_PREBID_ERROR: &str = "response-prebid-error";
    pub const RESPONSE_NOT_SEEN: &str = "response-not-seen";

    pub const ADS_TXT_AUTHORISED: &str = "ads-txt-authorised";
    pub const ADS_TXT_NOT_LISTED: &str = "ads-txt-not-listed";
    pub const ADS_TXT_ACCOUNT_MISMATCH: &str = "ads-txt-account-mismatch";
    pub const ADS_TXT_SCHAIN_MISMATCH: &str = "ads-txt-schain-mismatch";
    pub const ADS_TXT_UNAVAILABLE: &str = "ads-txt-unavailable";
    pub const ADS_TXT_UNKNOWN_SYSTEM: &str = "ads-txt-unknown-system";

    pub const SELLERS_OK: &str = "sellers-json-ok";
    pub const SELLERS_ID_MISSING: &str = "sellers-json-id-missing";
    pub const SELLERS_CONFIDENTIAL: &str = "sellers-json-confidential";
    pub const SELLERS_TYPE_MISMATCH: &str = "sellers-json-type-mismatch";
    pub const SELLERS_DOMAIN_MISMATCH: &str = "sellers-json-domain-mismatch";
    pub const SELLERS_UNAVAILABLE: &str = "sellers-json-unavailable";

    pub const CONSENT_OK: &str = "consent-ok";
    pub const CONSENT_NO_STRING: &str = "consent-no-string";
    pub const CONSENT_PENDING: &str = "consent-pending";
    pub const CONSENT_VENDOR_DENIED: &str = "consent-vendor-denied";
    pub const CONSENT_NOT_APPLICABLE: &str = "consent-not-applicable";
}

/// Longest excerpt stored with a check or a report.
const MAX_EXCERPT_CHARS: usize = 300;
const MAX_REPORTED_INVALID_LINES: usize = 10;
/// Seller ids looked up per ad system when the bid request does not say
/// which account is in use.
const MAX_SELLER_IDS_PER_SYSTEM: usize = 10;
/// Exchanges whose sellers.json one scan reads, however many ads.txt names.
const MAX_SELLERS_JSON_HOSTS: usize = 12;
/// Distinct endpoint reasons quoted for one bidder.
const MAX_QUOTED_REASONS: usize = 3;

/// The public files a scan looked at. Parsed and reduced; never the raw text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SiteFiles {
    pub ads_txt_url: String,
    pub ads_txt: AdsTxtOutcome,
    /// By ad system domain. `None` when the sellers.json lookup is switched off.
    pub sellers: Option<BTreeMap<String, SellersOutcome>>,
}

/// The seller ids to look up in one ad system's sellers.json.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SellersLookup {
    pub ad_system_domain: String,
    pub seller_ids: Vec<String>,
}

pub(crate) fn check(
    area: DiagnosticArea,
    status: DiagnosticStatus,
    code: &str,
    summary: impl Into<String>,
) -> DiagnosticCheck {
    DiagnosticCheck {
        area,
        status,
        code: code.to_string(),
        summary: summary.into(),
        excerpt: None,
        ad_units: Vec::new(),
    }
}

/// Cuts text down to what may be stored: one line of at most 300 printable
/// characters.
pub(crate) fn excerpt(text: &str) -> String {
    clipped(text.lines().next().unwrap_or_default(), MAX_EXCERPT_CHARS)
}

/// An id or name quoted inside a summary: ids come from the page and from
/// files it publishes, so their length is not ours to trust.
pub(crate) fn brief(text: &str) -> String {
    const MAX_QUOTED_ID_CHARS: usize = 40;
    clipped(text, MAX_QUOTED_ID_CHARS + 1)
}

/// At most `max` characters, the last one an ellipsis when text was cut.
/// The text is the site's or an exchange's: control characters (a tab
/// aside) and the invisible ones that reorder or hide text are not passed on.
fn clipped(text: &str, max: usize) -> String {
    let is_invisible = |c: char| {
        (c.is_control() && c != '\t')
            || matches!(c, '\u{200b}'..='\u{200f}' | '\u{2028}'..='\u{202e}' | '\u{2060}'..='\u{2069}' | '\u{feff}')
    };
    let printable: String = text
        .chars()
        .map(|c| if is_invisible(c) { ' ' } else { c })
        .collect();
    let printable = printable.trim();
    if printable.chars().count() <= max {
        return printable.to_string();
    }
    let mut cut: String = printable.chars().take(max - 1).collect();
    cut.push('…');
    cut
}

pub(crate) fn capitalized(text: &str) -> String {
    let mut characters = text.chars();
    match characters.next() {
        Some(first) => first.to_uppercase().chain(characters).collect(),
        None => String::new(),
    }
}

/// One bidder of the page with everything the capture says about it.
struct BidderFacts<'a> {
    name: &'a str,
    adapter: Option<&'static AdapterSpec>,
    configs: Vec<BidConfig<'a>>,
    requests: Vec<RequestSignals>,
    schain: Vec<&'a SchainNode>,
}

impl BidderFacts<'_> {
    /// The ads.txt account ids the bidder's params name.
    fn account_ids(&self) -> Vec<String> {
        let Some(param) = self.adapter.and_then(|adapter| adapter.account_param) else {
            return Vec::new();
        };
        let ids: BTreeSet<String> = self
            .configs
            .iter()
            .filter_map(|config| scalar_text(config.params.get(param)))
            .filter(|id| !id.is_empty())
            .collect();
        ids.into_iter().collect()
    }
}

fn bidder_facts(capture: &Capture) -> Vec<BidderFacts<'_>> {
    let mut facts: BTreeMap<&str, BidderFacts> = BTreeMap::new();
    fn entry<'m, 'a>(
        facts: &'m mut BTreeMap<&'a str, BidderFacts<'a>>,
        name: &'a str,
    ) -> &'m mut BidderFacts<'a> {
        facts.entry(name).or_insert_with(|| BidderFacts {
            name,
            adapter: adapter_for(name),
            configs: Vec::new(),
            requests: Vec::new(),
            schain: Vec::new(),
        })
    }
    for event in &capture.events {
        match &event.kind {
            Ev::AuctionInit { ad_units, .. } => {
                for unit in ad_units {
                    for bidder in &unit.bidders {
                        entry(&mut facts, bidder);
                    }
                    for bid in &unit.bids {
                        let configs = &mut entry(&mut facts, &bid.bidder).configs;
                        // A refreshed ad unit is the same configuration again.
                        let seen = configs
                            .iter()
                            .any(|known| known.ad_unit == unit.code && known.params == &bid.params);
                        if !seen {
                            configs.push(BidConfig {
                                ad_unit: &unit.code,
                                sizes: &unit.sizes,
                                params: &bid.params,
                            });
                        }
                    }
                }
            }
            Ev::BidRequested {
                bidder,
                gdpr_applies,
                has_consent_string,
                schain,
                ..
            } => {
                let bidder = entry(&mut facts, bidder);
                bidder.requests.push(RequestSignals {
                    t: event.t,
                    gdpr_applies: *gdpr_applies,
                    has_consent_string: *has_consent_string,
                });
                for node in schain {
                    if !bidder.schain.contains(&node) {
                        bidder.schain.push(node);
                    }
                }
            }
            _ => {}
        }
    }
    facts.into_values().collect()
}

/// Whether the page configured or called any Prebid bidder.
pub fn has_bidders(capture: &Capture) -> bool {
    capture.events.iter().any(|event| match &event.kind {
        Ev::AuctionInit { ad_units, .. } => ad_units.iter().any(|unit| !unit.bidders.is_empty()),
        Ev::BidRequested { .. } => true,
        _ => false,
    })
}

/// The records of the bidder's ad system whose seller ids are worth looking
/// up: those of the account its params name, or else the direct ones.
fn records_to_verify<'a>(bidder: &BidderFacts, file: &'a AdsTxt) -> Vec<&'a Record> {
    let Some(adapter) = bidder.adapter else {
        return Vec::new();
    };
    let account_ids = bidder.account_ids();
    file.records_for(adapter.ad_system_domains)
        .filter(|record| {
            if account_ids.is_empty() {
                record.relationship == Relationship::Direct
            } else {
                account_ids
                    .iter()
                    .any(|id| id.eq_ignore_ascii_case(&record.account_id))
            }
        })
        .take(MAX_SELLER_IDS_PER_SYSTEM)
        .collect()
}

/// Which sellers.json files to read, and which ids to look for in each. Only
/// ad systems of the adapter table are asked, never a domain that is merely
/// written in the site's ads.txt.
pub fn sellers_lookups(capture: &Capture, file: &AdsTxt) -> Vec<SellersLookup> {
    let mut by_domain: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for bidder in bidder_facts(capture) {
        for record in records_to_verify(&bidder, file) {
            by_domain
                .entry(&record.domain)
                .or_default()
                .insert(&record.account_id);
        }
    }
    by_domain
        .into_iter()
        .filter(|(domain, _)| site::sellers_json_url(domain).is_some())
        .take(MAX_SELLERS_JSON_HOSTS)
        .map(|(domain, ids)| SellersLookup {
            ad_system_domain: domain.to_string(),
            seller_ids: ids.into_iter().map(str::to_string).collect(),
        })
        .collect()
}

pub fn diagnose(capture: &Capture, telemetry: &Telemetry, files: &SiteFiles) -> BidderDiagnostics {
    let timeline = ConsentTimeline::from_events(&capture.events);
    let prebid_config = capture.events.iter().find_map(|event| match &event.kind {
        Ev::PrebidConfig {
            has_consent_management,
            raw,
            ..
        } => Some((*has_consent_management, raw)),
        _ => None,
    });
    let config_has = |key: &str| {
        prebid_config.is_some_and(|(_, raw)| raw.get(key).is_some_and(|value| !value.is_null()))
    };
    let server_side = server_side_bidders(prebid_config.map(|(_, raw)| raw));
    let auctions_with_consent: BTreeSet<&str> = capture
        .events
        .iter()
        .filter_map(|event| match &event.kind {
            Ev::BidRequested {
                auction_id,
                has_consent_string: Some(true),
                ..
            } => Some(auction_id.as_str()),
            _ => None,
        })
        .collect();
    let site = site::ads_txt_location(&capture.final_url)
        .map(|location| location.site)
        .unwrap_or_default();

    let bidders = bidder_facts(capture)
        .into_iter()
        .map(|bidder| {
            let mut checks = Vec::new();
            checks.extend(response_checks(capture, &bidder));
            checks.extend(param_checks(
                capture,
                &bidder,
                server_side.contains(bidder.name),
                config_has,
            ));
            checks.extend(timeline.bidder_checks(
                &bidder.requests,
                bidder.adapter.map(|adapter| (adapter.gvl_id, bidder.name)),
            ));
            checks.extend(ads_txt_checks(&bidder, &files.ads_txt));
            checks.extend(sellers_checks(&bidder, files, &site));
            // Most certain first: what was wrong before what was merely
            // noticed, and within that the order the checks were made in,
            // which runs from direct observation to inference.
            checks.sort_by_key(|check| match check.status {
                DiagnosticStatus::Problem => 0,
                DiagnosticStatus::Notice => 1,
                DiagnosticStatus::Pass => 2,
                DiagnosticStatus::NotChecked => 3,
            });
            let (requests, bids) = bid_counts(telemetry, bidder.name);
            BidderDiagnosis {
                bidder: bidder.name.to_string(),
                requests,
                bids,
                checks,
            }
        })
        .collect();

    BidderDiagnostics {
        ads_txt: ads_txt_report(files),
        sellers_json_checked: files.sellers.is_some(),
        consent: timeline.report(
            &telemetry.auctions,
            prebid_config.map(|(configured, _)| configured),
            &auctions_with_consent,
        ),
        bidders,
    }
}

fn bid_counts(telemetry: &Telemetry, bidder: &str) -> (u32, u32) {
    let bids = telemetry
        .auctions
        .iter()
        .flat_map(|auction| &auction.ad_units)
        .flat_map(|unit| &unit.bids)
        .filter(|bid| bid.bidder == bidder);
    let mut requests = 0;
    let mut answered = 0;
    for bid in bids {
        requests += 1;
        if matches!(
            bid.status,
            BidStatus::Bid | BidStatus::Won | BidStatus::Rendered
        ) {
            answered += 1;
        }
    }
    (requests, answered)
}

fn ads_txt_report(files: &SiteFiles) -> AdsTxtReport {
    let mut report = AdsTxtReport {
        url: files.ads_txt_url.clone(),
        status: AdsTxtStatus::Unreadable,
        detail: None,
        record_count: 0,
        invalid_line_count: 0,
        invalid_lines: Vec::new(),
        owner_domain: None,
        manager_domains: Vec::new(),
    };
    match &files.ads_txt {
        AdsTxtOutcome::Found(file) => {
            report.status = AdsTxtStatus::Found;
            report.record_count = file.records.len() as u32;
            report.invalid_line_count = file.invalid.len() as u32;
            report.invalid_lines = file
                .invalid
                .iter()
                .take(MAX_REPORTED_INVALID_LINES)
                .map(|line| AdsTxtLineProblem {
                    line: line.line,
                    excerpt: line.text.clone(),
                    problem: line.problem.to_string(),
                })
                .collect();
            report.owner_domain.clone_from(&file.owner_domain);
            report.manager_domains.clone_from(&file.manager_domains);
        }
        AdsTxtOutcome::Missing => report.status = AdsTxtStatus::Missing,
        AdsTxtOutcome::Unreadable(reason) => report.detail = Some(excerpt(reason)),
    }
    report
}

/// The bidders Prebid hands to a Prebid Server (`s2sConfig.bidders`). The
/// server validates their params by its own rules, which differ from the
/// adapter's: several accept fewer or other params.
fn server_side_bidders(prebid_config: Option<&Value>) -> BTreeSet<&str> {
    let configs = match prebid_config.map(|raw| &raw["s2sConfig"]) {
        Some(Value::Array(configs)) => configs.iter().collect(),
        Some(config) => vec![config],
        None => Vec::new(),
    };
    configs
        .into_iter()
        .filter(|config| config["enabled"] != false)
        .filter_map(|config| config["bidders"].as_array())
        .flatten()
        .filter_map(Value::as_str)
        .collect()
}

fn param_checks(
    capture: &Capture,
    bidder: &BidderFacts,
    runs_server_side: bool,
    config_has: impl Fn(&str) -> bool,
) -> Vec<DiagnosticCheck> {
    let not_checked = |code: &str, summary: &str| {
        check(
            DiagnosticArea::Params,
            DiagnosticStatus::NotChecked,
            code,
            summary,
        )
    };
    let mut checks = match bidder.adapter {
        None => vec![not_checked(
            codes::PARAMS_UNKNOWN_ADAPTER,
            "The scanner has no built-in knowledge of this adapter's params",
        )],
        Some(_) if runs_server_side => vec![not_checked(
            codes::PARAMS_SERVER_SIDE,
            "This bidder runs through Prebid Server, which checks params by its own rules",
        )],
        Some(_) if bidder.configs.is_empty() => vec![not_checked(
            codes::PARAMS_NOT_CAPTURED,
            "Prebid did not expose this bidder's params",
        )],
        Some(adapter) => check_params(adapter, &bidder.configs, config_has),
    };
    // Prebid says so itself when an adapter turns a bid down. That covers the
    // adapters and the rules (video params, for one) the table does not know.
    let already_explained = checks
        .iter()
        .any(|check| check.status == DiagnosticStatus::Problem);
    let rejections = prebid_rejections(capture, bidder);
    if let (false, Some(first)) = (already_explained, rejections.first()) {
        checks.retain(|check| check.status != DiagnosticStatus::Pass);
        let mut rejected = check(
            DiagnosticArea::Params,
            DiagnosticStatus::Problem,
            codes::PARAMS_REJECTED,
            format!(
                "Prebid reported {} whose params the adapter itself rejected",
                match rejections.len() {
                    1 => "1 bid".to_string(),
                    count => format!("{count} bids"),
                }
            ),
        );
        rejected.excerpt = Some(excerpt(first));
        checks.insert(0, rejected);
    }
    checks
}

/// Prebid's "Invalid bid sent to bidder" warnings for this bidder.
fn prebid_rejections<'a>(capture: &'a Capture, bidder: &BidderFacts) -> Vec<&'a str> {
    let names = [
        Some(bidder.name),
        bidder.adapter.map(|adapter| adapter.code),
    ];
    capture
        .events
        .iter()
        .filter_map(|event| match &event.kind {
            Ev::PrebidLog { message, .. } => Some(message.as_str()),
            _ => None,
        })
        .filter(|message| {
            names.iter().flatten().any(|name| {
                message
                    .strip_prefix("Invalid bid sent to bidder ")
                    .and_then(|rest| rest.strip_prefix(name))
                    .is_some_and(|rest| rest.starts_with(':'))
            })
        })
        .collect()
}

/// What the bidder's endpoint and Prebid's own error events said.
fn response_checks(capture: &Capture, bidder: &BidderFacts) -> Vec<DiagnosticCheck> {
    let mut checks = Vec::new();
    let responses: Vec<_> = capture
        .bid_responses
        .iter()
        .filter(|response| {
            response.bidder == bidder.name
                || bidder
                    .adapter
                    .is_some_and(|adapter| adapter.code == response.bidder)
        })
        .collect();
    let total = responses.len();
    let of_responses = |count: usize| {
        if count == total {
            match total {
                1 => "its 1 bid request".to_string(),
                total => format!("all {total} bid requests"),
            }
        } else {
            format!("{count} of {total} bid requests")
        }
    };
    let mut problem = |code: &str, summary: String, quote: Option<&str>| {
        let mut check = check(
            DiagnosticArea::Response,
            DiagnosticStatus::Problem,
            code,
            summary,
        );
        check.excerpt = quote.map(excerpt);
        checks.push(check);
    };

    let failed: Vec<_> = responses
        .iter()
        .filter(|response| response.status.is_none())
        .collect();
    if let Some(first) = failed.first() {
        problem(
            codes::RESPONSE_FAILED,
            format!(
                "The request to {} failed on {}",
                first.host,
                of_responses(failed.len())
            ),
            first.reason.as_deref(),
        );
    }
    let mut by_error_status: BTreeMap<u16, usize> = BTreeMap::new();
    for status in responses.iter().filter_map(|response| response.status) {
        if status >= 400 {
            *by_error_status.entry(status).or_default() += 1;
        }
    }
    for (status, count) in by_error_status {
        let quote = responses
            .iter()
            .find(|response| response.status == Some(status))
            .and_then(|response| response.reason.as_deref());
        problem(
            codes::RESPONSE_HTTP_ERROR,
            format!(
                "The bidder's endpoint answered HTTP {status} on {}",
                of_responses(count)
            ),
            quote,
        );
    }
    let mut reasons: Vec<(&str, usize)> = Vec::new();
    for response in &responses {
        let answered = response.status.is_some_and(|status| status < 400);
        if let Some(reason) = response.reason.as_deref().filter(|_| answered) {
            match reasons.iter_mut().find(|(known, _)| *known == reason) {
                Some((_, count)) => *count += 1,
                None => reasons.push((reason, 1)),
            }
        }
    }
    for (reason, count) in reasons.iter().take(MAX_QUOTED_REASONS) {
        problem(
            codes::RESPONSE_REASON,
            format!(
                "The bidder's endpoint explained why it did not bid on {}",
                of_responses(*count)
            ),
            Some(reason),
        );
    }

    let prebid_errors: Vec<&str> = capture
        .events
        .iter()
        .filter_map(|event| match &event.kind {
            Ev::BidderError {
                bidder: name,
                message,
                ..
            } if name == bidder.name => Some(message.as_deref().unwrap_or("no message")),
            _ => None,
        })
        .collect();
    if let Some(first) = prebid_errors.first() {
        problem(
            codes::RESPONSE_PREBID_ERROR,
            format!(
                "Prebid reported {} for this bidder",
                match prebid_errors.len() {
                    1 => "a bidder error".to_string(),
                    count => format!("{count} bidder errors"),
                }
            ),
            Some(first),
        );
    }

    if checks.is_empty() {
        checks.push(if total == 0 {
            check(
                DiagnosticArea::Response,
                DiagnosticStatus::NotChecked,
                codes::RESPONSE_NOT_SEEN,
                "No request to a known endpoint of this bidder was observed",
            )
        } else {
            check(
                DiagnosticArea::Response,
                DiagnosticStatus::Pass,
                codes::RESPONSE_OK,
                format!(
                    "The bidder's endpoint answered {} without an error or a stated reason",
                    of_responses(total)
                ),
            )
        });
    }
    checks
}

fn ads_txt_checks(bidder: &BidderFacts, outcome: &AdsTxtOutcome) -> Vec<DiagnosticCheck> {
    let not_checked = |code: &str, summary: &str| {
        vec![check(
            DiagnosticArea::AdsTxt,
            DiagnosticStatus::NotChecked,
            code,
            summary,
        )]
    };
    let file = match outcome {
        AdsTxtOutcome::Found(file) => file,
        AdsTxtOutcome::Missing => {
            return not_checked(codes::ADS_TXT_UNAVAILABLE, "The site has no ads.txt");
        }
        AdsTxtOutcome::Unreadable(_) => {
            return not_checked(
                codes::ADS_TXT_UNAVAILABLE,
                "The site's ads.txt could not be read",
            );
        }
    };
    let mut checks = schain_checks(bidder, file);
    let domains = bidder
        .adapter
        .map(|adapter| adapter.ad_system_domains)
        .unwrap_or_default();
    let Some(main_domain) = domains.first() else {
        if checks.is_empty() {
            return not_checked(
                codes::ADS_TXT_UNKNOWN_SYSTEM,
                "The scanner does not know for certain which ads.txt entry this bidder needs",
            );
        }
        return checks;
    };
    let listed: Vec<&Record> = file.records_for(domains).collect();
    let Some(first_listed) = listed.first() else {
        checks.push(check(
            DiagnosticArea::AdsTxt,
            DiagnosticStatus::Problem,
            codes::ADS_TXT_NOT_LISTED,
            format!("ads.txt has no entry for {main_domain}"),
        ));
        return checks;
    };
    let account_ids = bidder.account_ids();
    let account_param = bidder
        .adapter
        .and_then(|adapter| adapter.account_param)
        .unwrap_or_default();
    let mut matched: Option<&Record> = None;
    for id in &account_ids {
        match listed
            .iter()
            .find(|record| record.account_id.eq_ignore_ascii_case(id))
        {
            Some(record) => matched = matched.or(Some(record)),
            None => {
                let mut check = check(
                    DiagnosticArea::AdsTxt,
                    DiagnosticStatus::Problem,
                    codes::ADS_TXT_ACCOUNT_MISMATCH,
                    format!(
                        "ads.txt lists {} but not account {}, which the `{account_param}` param uses",
                        brief(&first_listed.domain),
                        brief(id)
                    ),
                );
                check.excerpt = Some(first_listed.text.clone());
                checks.push(check);
            }
        }
    }
    let has_mismatch = checks
        .iter()
        .any(|check| check.code == codes::ADS_TXT_ACCOUNT_MISMATCH);
    if !has_mismatch {
        let record = matched.unwrap_or(first_listed);
        let summary = match matched {
            Some(record) => format!(
                "ads.txt authorises account {} at {} ({})",
                brief(&record.account_id),
                brief(&record.domain),
                record.relationship.as_str()
            ),
            None => format!(
                "ads.txt lists {} ({})",
                brief(&record.domain),
                match listed.len() {
                    1 => "1 entry".to_string(),
                    count => format!("{count} entries"),
                }
            ),
        };
        let mut check = check(
            DiagnosticArea::AdsTxt,
            DiagnosticStatus::Pass,
            codes::ADS_TXT_AUTHORISED,
            summary,
        );
        check.excerpt = Some(record.text.clone());
        checks.push(check);
    }
    checks
}

/// Every paying node of the supply chain sent to the bidder names a seller
/// account, and ads.txt must list that account.
fn schain_checks(bidder: &BidderFacts, file: &AdsTxt) -> Vec<DiagnosticCheck> {
    bidder
        .schain
        .iter()
        .filter(|node| node.hp != Some(false))
        .filter_map(|node| {
            let asi = node.asi.trim().to_ascii_lowercase();
            let listed = file.records.iter().any(|record| {
                record.domain == asi && record.account_id.eq_ignore_ascii_case(node.sid.trim())
            });
            (!listed && !asi.is_empty()).then(|| {
                check(
                    DiagnosticArea::AdsTxt,
                    DiagnosticStatus::Problem,
                    codes::ADS_TXT_SCHAIN_MISMATCH,
                    format!(
                        "The supply chain sent to this bidder names seller {} at {}, which ads.txt does not list",
                        brief(&node.sid),
                        brief(&asi)
                    ),
                )
            })
        })
        .collect()
}

fn sellers_checks(bidder: &BidderFacts, files: &SiteFiles, site: &str) -> Vec<DiagnosticCheck> {
    let (Some(sellers), AdsTxtOutcome::Found(file)) = (&files.sellers, &files.ads_txt) else {
        return Vec::new();
    };
    let declared_domains: Vec<String> = file
        .owner_domain
        .iter()
        .chain(&file.manager_domains)
        .cloned()
        .collect();
    let identity = SiteIdentity {
        domain: site,
        declared_domains: &declared_domains,
    };
    let mut checks = Vec::new();
    let mut unavailable = BTreeSet::new();
    for record in records_to_verify(bidder, file) {
        match sellers.get(&record.domain) {
            Some(SellersOutcome::Read(found)) => {
                let seller = found
                    .iter()
                    .find(|seller| seller.seller_id.eq_ignore_ascii_case(&record.account_id));
                checks.extend(sellers_json::compare(record, seller, &identity));
            }
            // One note per exchange, however many of its records are listed.
            Some(SellersOutcome::Unavailable(reason)) if unavailable.insert(&record.domain) => {
                checks.push(check(
                    DiagnosticArea::SellersJson,
                    DiagnosticStatus::NotChecked,
                    codes::SELLERS_UNAVAILABLE,
                    format!(
                        "The sellers.json of {} could not be read: {}",
                        brief(&record.domain),
                        excerpt(reason)
                    ),
                ));
            }
            Some(SellersOutcome::Unavailable(_)) | None => {}
        }
    }
    merge_passes(checks)
}

/// Several verified seller ids are one piece of good news.
fn merge_passes(checks: Vec<DiagnosticCheck>) -> Vec<DiagnosticCheck> {
    let passes = checks
        .iter()
        .filter(|check| check.status == DiagnosticStatus::Pass)
        .count();
    if passes < 2 {
        return checks;
    }
    let mut merged: Vec<DiagnosticCheck> = checks
        .into_iter()
        .filter(|check| check.status != DiagnosticStatus::Pass)
        .collect();
    merged.push(check(
        DiagnosticArea::SellersJson,
        DiagnosticStatus::Pass,
        codes::SELLERS_OK,
        format!("{passes} seller ids from ads.txt are listed in the exchange's sellers.json"),
    ));
    merged
}
