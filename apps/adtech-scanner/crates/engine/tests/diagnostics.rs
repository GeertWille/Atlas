//! Bidder diagnostics on canned captures: what each check concludes, and the
//! findings the rules build from those conclusions.

mod common;

use std::collections::BTreeMap;

use adscan_engine::domain::diagnostics::ads_txt::{self, AdsTxtOutcome};
use adscan_engine::domain::diagnostics::sellers_json::{Seller, SellerType, SellersOutcome};
use adscan_engine::domain::diagnostics::{self, SellersLookup, SiteFiles};
use adscan_engine::domain::finding::FindingDraft;
use adscan_engine::domain::heuristics::{self, RuleInput};
use adscan_engine::domain::telemetry;
use adscan_model::{
    AdsTxtStatus, BidderDiagnostics, Capture, DeviceProfile, DiagnosticArea, DiagnosticStatus,
    ObservedBidResponse, PageReach, SchainNode, Severity, TrackedAdUnit, TrackedBid, TrackerEvent,
    TrackerEventKind as Ev,
};
use common::*;
use serde_json::{Value, json};

const UNITS: [&str; 4] = ["top", "rail", "article", "footer"];

/// One bidder of the page: its params per ad unit and how it answers.
struct Bidder {
    name: &'static str,
    /// One entry per ad unit of [`UNITS`]; `None` leaves the bidder off it.
    params: [Option<Value>; 4],
    /// Per ad unit: a bid, or else a no-bid.
    bids_on: [bool; 4],
    gdpr_applies: Option<bool>,
    has_consent_string: Option<bool>,
    schain: Vec<SchainNode>,
}

fn bidder(name: &'static str, params: Value) -> Bidder {
    Bidder {
        name,
        params: std::array::from_fn(|_| Some(params.clone())),
        bids_on: [true; 4],
        gdpr_applies: None,
        has_consent_string: None,
        schain: vec![],
    }
}

fn silent(name: &'static str, params: Value) -> Bidder {
    Bidder {
        bids_on: [false; 4],
        ..bidder(name, params)
    }
}

/// A page whose one auction asks every bidder for each of its ad units.
fn page(bidders: &[Bidder], extra: Vec<TrackerEvent>) -> Capture {
    let ad_units = UNITS
        .iter()
        .enumerate()
        .map(|(index, code)| TrackedAdUnit {
            code: code.to_string(),
            sizes: strings(&[["728x90", "300x600", "336x280", "320x50"][index]]),
            bidders: bidders
                .iter()
                .filter(|bidder| bidder.params[index].is_some())
                .map(|bidder| bidder.name.to_string())
                .collect(),
            bids: bidders
                .iter()
                .filter_map(|bidder| {
                    Some(TrackedBid {
                        bidder: bidder.name.to_string(),
                        params: bidder.params[index].clone()?,
                    })
                })
                .collect(),
        })
        .collect();
    let mut events = vec![
        prebid_detected(100.0, "pbjs", "10.2.0"),
        prebid_config(110.0, true, true),
        ev(
            1_000.0,
            Ev::AuctionInit {
                auction_id: "a1".into(),
                timeout: Some(1_500.0),
                ad_units,
            },
        ),
    ];
    for bidder in bidders {
        let units: Vec<(usize, &str)> = UNITS
            .iter()
            .enumerate()
            .filter(|(index, _)| bidder.params[*index].is_some())
            .map(|(index, code)| (index, *code))
            .collect();
        let codes: Vec<&str> = units.iter().map(|(_, code)| *code).collect();
        events.push(ev(
            1_010.0,
            Ev::BidRequested {
                auction_id: "a1".into(),
                bidder: bidder.name.into(),
                ad_unit_codes: strings(&codes),
                gdpr_applies: bidder.gdpr_applies,
                has_consent_string: bidder.has_consent_string,
                schain: bidder.schain.clone(),
                bidder_request_id: None,
                bids: Vec::new(),
            },
        ));
        for (index, code) in units {
            events.push(if bidder.bids_on[index] {
                bid_response(1_200.0, "a1", bidder.name, code, 1.0)
            } else {
                ev(
                    1_200.0,
                    Ev::NoBid {
                        auction_id: "a1".into(),
                        bidder: bidder.name.into(),
                        ad_unit_code: code.into(),
                    },
                )
            });
        }
    }
    events.push(ev(
        1_400.0,
        Ev::AuctionEnd {
            auction_id: "a1".into(),
        },
    ));
    events.extend(extra);
    capture_with(DeviceProfile::Desktop, events, 0.0)
}

fn found(text: &str) -> SiteFiles {
    files(AdsTxtOutcome::Found(ads_txt::parse(text)))
}

fn files(ads_txt: AdsTxtOutcome) -> SiteFiles {
    SiteFiles {
        ads_txt_url: SITE_ADS_TXT_URL.into(),
        ads_txt,
        sellers: None,
    }
}

fn diagnose(capture: &Capture, files: &SiteFiles) -> BidderDiagnostics {
    diagnostics::diagnose(capture, &telemetry::derive(capture), files)
}

/// `(area, status, code)` of every check of one bidder, in report order.
fn checks(
    diagnosed: &BidderDiagnostics,
    bidder: &str,
) -> Vec<(DiagnosticArea, DiagnosticStatus, String)> {
    diagnosed
        .bidders
        .iter()
        .find(|diagnosis| diagnosis.bidder == bidder)
        .unwrap_or_else(|| panic!("{bidder} was not diagnosed"))
        .checks
        .iter()
        .map(|check| (check.area, check.status, check.code.clone()))
        .collect()
}

fn check_of<'a>(
    diagnosed: &'a BidderDiagnostics,
    bidder: &str,
    code: &str,
) -> &'a adscan_model::DiagnosticCheck {
    diagnosed
        .bidders
        .iter()
        .find(|diagnosis| diagnosis.bidder == bidder)
        .and_then(|diagnosis| diagnosis.checks.iter().find(|check| check.code == code))
        .unwrap_or_else(|| panic!("{bidder} has no `{code}` check: {diagnosed:#?}"))
}

fn findings(capture: &Capture, diagnosed: &BidderDiagnostics) -> Vec<(&'static str, FindingDraft)> {
    let telemetry = telemetry::derive(capture);
    heuristics::evaluate(&RuleInput {
        capture,
        telemetry: &telemetry,
        diagnostics: Some(diagnosed),
        scripts: &[],
        environment: None,
    })
    .into_iter()
    .map(|hit| (hit.rule, hit.finding))
    .collect()
}

fn fired(capture: &Capture, diagnosed: &BidderDiagnostics) -> Vec<&'static str> {
    findings(capture, diagnosed)
        .into_iter()
        .map(|(rule, _)| rule)
        .collect()
}

fn finding(capture: &Capture, diagnosed: &BidderDiagnostics, rule: &str) -> FindingDraft {
    findings(capture, diagnosed)
        .into_iter()
        .find(|(fired, _)| *fired == rule)
        .unwrap_or_else(|| panic!("`{rule}` did not fire"))
        .1
}

const HEALTHY_ADS_TXT: &str = "appnexus.com, 958, DIRECT, f5ab79cb980f11d1\n\
     rubiconproject.com, 1001, DIRECT\n\
     pubmatic.com, 160262, RESELLER\n";

fn healthy_bidders() -> Vec<Bidder> {
    vec![
        bidder("appnexus", json!({ "placementId": 21804213 })),
        bidder(
            "rubicon",
            json!({ "accountId": 1001, "siteId": 2002, "zoneId": 3003 }),
        ),
    ]
}

#[test]
fn a_correct_setup_passes_every_check_and_raises_no_finding() {
    let mut bidders = healthy_bidders();
    // Different placements, as a real setup has.
    for (index, params) in bidders[0].params.iter_mut().enumerate() {
        *params = Some(json!({ "placementId": 21804213 + index }));
    }
    let capture = page(&bidders, vec![]);
    let diagnosed = diagnose(&capture, &found(HEALTHY_ADS_TXT));

    use DiagnosticArea::{AdsTxt, Consent, Params, Response};
    use DiagnosticStatus::{NotChecked, Pass};
    for name in ["appnexus", "rubicon"] {
        assert_eq!(
            checks(&diagnosed, name),
            [
                (Params, Pass, "params-ok".to_string()),
                (AdsTxt, Pass, "ads-txt-authorised".to_string()),
                (Response, NotChecked, "response-not-seen".to_string()),
                (Consent, NotChecked, "consent-not-applicable".to_string()),
            ],
            "{name}"
        );
    }
    assert_eq!(
        check_of(&diagnosed, "rubicon", "ads-txt-authorised").summary,
        "ads.txt authorises account 1001 at rubiconproject.com (DIRECT)"
    );
    assert_eq!(
        check_of(&diagnosed, "rubicon", "ads-txt-authorised")
            .excerpt
            .as_deref(),
        Some("rubiconproject.com, 1001, DIRECT")
    );
    // AppNexus placements do not name the seller account.
    assert_eq!(
        check_of(&diagnosed, "appnexus", "ads-txt-authorised").summary,
        "ads.txt lists appnexus.com (1 entry)"
    );
    assert_eq!(
        (diagnosed.bidders[0].requests, diagnosed.bidders[0].bids),
        (4, 4)
    );
    assert_eq!(
        (
            diagnosed.ads_txt.status,
            diagnosed.ads_txt.record_count,
            diagnosed.ads_txt.invalid_line_count
        ),
        (AdsTxtStatus::Found, 3, 0)
    );
    assert!(!diagnosed.sellers_json_checked);
    assert_eq!(diagnosed.consent.consent_management_configured, Some(true));
    assert_eq!(fired(&capture, &diagnosed), Vec::<&str>::new());
}

#[test]
fn a_silent_bidder_gets_its_most_likely_cause_and_the_rest_keeps_the_generic_advice() {
    let capture = page(
        &[
            bidder(
                "rubicon",
                json!({ "accountId": 1001, "siteId": 2, "zoneId": 3 }),
            ),
            silent("seedtag", json!({ "publisherId": "4521-7788-01" })),
            silent("appnexus", json!({ "placementId": 21804213 })),
            silent("my-alias", json!({ "anything": 1 })),
        ],
        vec![],
    );
    let diagnosed = diagnose(&capture, &found(HEALTHY_ADS_TXT));
    assert_eq!(
        check_of(&diagnosed, "seedtag", "ads-txt-not-listed").summary,
        "ads.txt has no entry for seedtag.com"
    );

    let never_bid = finding(&capture, &diagnosed, "bidders-never-bid");
    assert_eq!(never_bid.title, "3 bidder(s) never returned a bid");
    assert_eq!(
        (never_bid.severity, never_bid.confidence),
        (Severity::Medium, 0.7)
    );
    assert_eq!(
        never_bid.affected_bidders,
        ["appnexus", "my-alias", "seedtag"]
    );
    assert_eq!(
        never_bid.description,
        "`appnexus`, `my-alias`, `seedtag` answered every bid request in this scan without a bid. One page view is a small \
         sample, so read the causes below as leads to confirm, not as a verdict.\n\n\
         - `appnexus`: All 4 ad units use the same `placementId` (21804213) although their sizes differ.\n\n\
         - `seedtag`: ads.txt has no entry for seedtag.com.\n\n\
         - `my-alias`: the checks the scanner can run on its own (adapter params, the endpoint's answers, ads.txt, consent) \
         found nothing wrong. A bidder that never bids usually has wrong placement parameters, an unapproved domain \
         (ads.txt / sellers.json) or missing consent."
    );
    assert_eq!(
        never_bid.recommendation,
        "1. Ask the partner whether each ad unit should have its own placement id.\n\
         2. Add the line the partner provides to ads.txt; buyers that enforce ads.txt do not bid on inventory from sellers it does not list.\n\n\
         For `my-alias`: verify the adapter params (placement / site / zone ids) with the partner, check the domain is \
         approved in their platform and listed in ads.txt, and compare with their own reporting."
    );
    assert!(
        never_bid
            .evidence
            .iter()
            .any(|evidence| evidence.label == "seedtag: ads.txt has no entry for seedtag.com"),
        "{:#?}",
        never_bid.evidence
    );
    assert_eq!(
        never_bid.evidence.last().unwrap().body,
        "appnexus: 0 bids / 4 requests, my-alias: 0 bids / 4 requests, seedtag: 0 bids / 4 requests; \
         bidder errors in scan: 0; causes found for 2 of 3"
    );
    // The missing ads.txt entry is reported once, as the cause of the silence.
    assert_eq!(fired(&capture, &diagnosed), ["bidders-never-bid"]);
}

#[test]
fn without_any_cause_the_never_bid_finding_stays_a_low_generic_lead() {
    let capture = page(
        &[
            bidder(
                "rubicon",
                json!({ "accountId": 1001, "siteId": 2, "zoneId": 3 }),
            ),
            silent("pubmatic", json!({ "publisherId": "160262" })),
        ],
        vec![],
    );
    let diagnosed = diagnose(&capture, &found(HEALTHY_ADS_TXT));
    let never_bid = finding(&capture, &diagnosed, "bidders-never-bid");
    assert_eq!(
        (never_bid.severity, never_bid.confidence),
        (Severity::Low, 0.4)
    );
    assert!(
        never_bid
            .description
            .contains("One page view is a small sample")
    );
    assert!(never_bid.description.contains("found nothing wrong"));
    assert_eq!(
        never_bid.recommendation,
        "For `pubmatic`: verify the adapter params (placement / site / zone ids) with the partner, check the domain is \
         approved in their platform and listed in ads.txt, and compare with their own reporting."
    );

    // A scan stored without bidder checks says that the checks did not run.
    let telemetry = telemetry::derive(&capture);
    let unchecked = heuristics::evaluate(&RuleInput {
        capture: &capture,
        telemetry: &telemetry,
        diagnostics: None,
        scripts: &[],
        environment: None,
    });
    assert_eq!(unchecked.len(), 1);
    assert!(
        unchecked[0]
            .finding
            .description
            .contains("- `pubmatic`: the bidder checks did not run for this scan."),
        "{}",
        unchecked[0].finding.description
    );
}

#[test]
fn causes_are_ranked_from_the_endpoints_own_answer_down_to_inference() {
    let mut appnexus = silent(
        "appnexus",
        json!({ "placementId": 13144370, "member": "7" }),
    );
    appnexus.gdpr_applies = Some(true);
    appnexus.has_consent_string = Some(false);
    let mut capture = page(
        &[appnexus],
        vec![
            ev(
                900.0,
                Ev::TcfState {
                    event_status: Some("tcloaded".into()),
                    gdpr_applies: Some(true),
                    has_tc_string: true,
                    purpose_consents: vec![1],
                    vendor_consents: vec![52],
                },
            ),
            ev(
                1_250.0,
                Ev::BidderError {
                    auction_id: Some("a1".into()),
                    bidder: "appnexus".into(),
                    message: Some("HTTP 400".into()),
                },
            ),
        ],
    );
    capture.bid_responses = vec![
        ObservedBidResponse {
            page: 0,
            t: 1_100.0,
            bidder: "appnexus".into(),
            host: "ib.adnxs.com".into(),
            status: Some(200),
            reason: Some("tag error: Invalid placement".into()),
            request_seq: None,
        },
        ObservedBidResponse {
            page: 0,
            t: 1_150.0,
            bidder: "appnexus".into(),
            host: "ib.adnxs.com".into(),
            status: Some(400),
            reason: None,
            request_seq: None,
        },
        ObservedBidResponse {
            page: 0,
            t: 1_160.0,
            bidder: "appnexus".into(),
            host: "ib.adnxs.com".into(),
            status: None,
            reason: Some("net::ERR_BLOCKED_BY_CLIENT".into()),
            request_seq: None,
        },
    ];
    let diagnosed = diagnose(&capture, &found(HEALTHY_ADS_TXT));

    let appnexus = &diagnosed.bidders[0];
    let problems: Vec<(&str, &str, Option<&str>)> = appnexus
        .checks
        .iter()
        .filter(|check| check.status == DiagnosticStatus::Problem)
        .map(|check| {
            (
                check.code.as_str(),
                check.summary.as_str(),
                check.excerpt.as_deref(),
            )
        })
        .collect();
    assert_eq!(
        problems,
        [
            (
                "response-failed",
                "The request to ib.adnxs.com failed on 1 of 3 bid requests",
                Some("net::ERR_BLOCKED_BY_CLIENT")
            ),
            (
                "response-http-error",
                "The bidder's endpoint answered HTTP 400 on 1 of 3 bid requests",
                None
            ),
            (
                "response-reason",
                "The bidder's endpoint explained why it did not bid on 1 of 3 bid requests",
                Some("tag error: Invalid placement")
            ),
            (
                "response-prebid-error",
                "Prebid reported a bidder error for this bidder",
                Some("HTTP 400")
            ),
            (
                "params-test-id",
                "`placementId: 13144370` is the test id from the adapter's documentation, used on all 4 ad units",
                None
            ),
            (
                "consent-no-string",
                "Its bid request went out without a consent string although GDPR applies",
                None
            ),
            (
                "consent-vendor-denied",
                "The user's consent does not cover appnexus (IAB vendor 32) on its bid request",
                None
            ),
            (
                "ads-txt-account-mismatch",
                "ads.txt lists appnexus.com but not account 7, which the `member` param uses",
                Some("appnexus.com, 958, DIRECT, f5ab79cb980f11d1")
            ),
        ]
    );
    let never_bid = finding(&capture, &diagnosed, "bidders-never-bid");
    assert!(
        never_bid
            .description
            .contains("- `appnexus`: The request to ib.adnxs.com failed on 1 of 3 bid requests. Also: The bidder's endpoint answered HTTP 400"),
        "{}",
        never_bid.description
    );
    // Every cause is in the one finding about this bidder.
    assert_eq!(fired(&capture, &diagnosed), ["bidders-never-bid"]);
}

#[test]
fn rejected_params_are_reported_as_prebid_shows_them_requests_that_end_without_a_bid() {
    // Prebid announces a bid whose params the adapter rejects, sends nothing
    // for it and reports a no-bid.
    let mut triplelift = bidder("triplelift", json!({}));
    triplelift.params[0] = Some(json!({ "inventoryCode": "site_top" }));
    triplelift.params[3] = None;
    triplelift.bids_on = [true, false, false, false];
    let pubmatic = silent("pubmatic", json!({ "publisherId": 160262 }));
    let capture = page(&[triplelift, pubmatic], vec![]);
    let diagnosed = diagnose(
        &capture,
        &found("triplelift.com, 11582, DIRECT\npubmatic.com, 160262, DIRECT"),
    );
    assert_eq!(
        check_of(&diagnosed, "triplelift", "params-missing").ad_units,
        ["rail", "article"]
    );

    // A bidder that bids where its params are right gets a finding of its own.
    let rejected = finding(&capture, &diagnosed, "bidder-params-invalid");
    assert_eq!(
        rejected.title,
        "1 bidder(s) have adapter params the adapter rejects"
    );
    assert_eq!(rejected.severity, Severity::High);
    assert_eq!(rejected.affected_bidders, ["triplelift"]);
    assert_eq!(rejected.affected_slots, ["article", "rail"]);
    assert!(
        rejected
            .description
            .ends_with("- `triplelift`: `inventoryCode` missing on 2 of 3 ad units"),
        "{}",
        rejected.description
    );
    assert_eq!(
        rejected.recommendation,
        "Add the missing param to that bidder's entry in every ad unit (Prebid sends nothing to a bidder whose params \
         fail the adapter's own check and records a no-bid)."
    );

    // A bidder whose params are wrong everywhere never bids: the rejected
    // params are the observed cause of that, reported once.
    let never_bid = finding(&capture, &diagnosed, "bidders-never-bid");
    assert_eq!(never_bid.affected_bidders, ["pubmatic"]);
    assert_eq!(
        (never_bid.severity, never_bid.confidence),
        (Severity::Medium, 0.85)
    );
    assert!(
        never_bid.description.ends_with(
            "- `pubmatic`: `publisherId` must be a string, found the number 160262 on all 4 ad units."
        ),
        "{}",
        never_bid.description
    );
    assert_eq!(
        never_bid.recommendation,
        "1. Pass the param in the type the adapter expects."
    );
    assert_eq!(
        fired(&capture, &diagnosed),
        ["bidders-never-bid", "bidder-params-invalid"]
    );
}

#[test]
fn prebids_own_rejection_is_used_where_the_table_knows_no_better() {
    let warning = |message: &str| {
        ev(
            1_015.0,
            Ev::PrebidLog {
                level: "warning".into(),
                message: message.into(),
            },
        )
    };
    let mut rubicon = bidder(
        "rubicon",
        json!({ "accountId": 1001, "siteId": 2, "zoneId": 3 }),
    );
    rubicon.bids_on = [true, true, true, false];
    let mut triplelift = bidder("triplelift", json!({}));
    triplelift.bids_on = [false; 4];
    let capture = page(
        &[
            // An adapter the table does not know.
            silent("acmeBidder", json!({ "zone": 7 })),
            // Params the table accepts, turned down for a reason it does not model.
            rubicon,
            // Params the table itself finds wrong: the warning adds nothing.
            triplelift,
            bidder("pubmatic", json!({ "publisherId": "160262" })),
        ],
        vec![
            warning(
                r#"Invalid bid sent to bidder acmeBidder: {"bidder":"acmeBidder","params":{"zone":7}}"#,
            ),
            warning(
                r#"Invalid bid sent to bidder acmeBidder: {"bidder":"acmeBidder","params":{"zone":7},"adUnitCode":"rail"}"#,
            ),
            warning(
                r#"Invalid bid sent to bidder rubicon: {"bidder":"rubicon","mediaTypes":{"video":{}}}"#,
            ),
            warning(
                r#"Invalid bid sent to bidder triplelift: {"bidder":"triplelift","params":{}}"#,
            ),
            // Another bidder whose code merely starts the same.
            warning(r#"Invalid bid sent to bidder pubmaticServer: {"bidder":"pubmaticServer"}"#),
            warning("Late response from pubmatic ignored"),
        ],
    );
    let diagnosed = diagnose(
        &capture,
        &found(
            "rubiconproject.com, 1001, DIRECT\npubmatic.com, 160262, DIRECT\ntriplelift.com, 1, DIRECT",
        ),
    );

    let params = |name: &str| -> Vec<(DiagnosticStatus, String)> {
        checks(&diagnosed, name)
            .into_iter()
            .filter(|(area, _, _)| *area == DiagnosticArea::Params)
            .map(|(_, status, code)| (status, code))
            .collect()
    };
    use DiagnosticStatus::{NotChecked, Pass, Problem};
    assert_eq!(
        params("acmeBidder"),
        [
            (Problem, "params-rejected".to_string()),
            (NotChecked, "params-unknown-adapter".to_string()),
        ]
    );
    let rejected = check_of(&diagnosed, "acmeBidder", "params-rejected");
    assert_eq!(
        (rejected.summary.as_str(), rejected.excerpt.as_deref()),
        (
            "Prebid reported 2 bids whose params the adapter itself rejected",
            Some(
                r#"Invalid bid sent to bidder acmeBidder: {"bidder":"acmeBidder","params":{"zone":7}}"#
            )
        )
    );
    assert_eq!(
        params("rubicon"),
        [(Problem, "params-rejected".to_string())]
    );
    assert_eq!(
        params("triplelift"),
        [(Problem, "params-missing".to_string())]
    );
    assert_eq!(params("pubmatic"), [(Pass, "params-ok".to_string())]);

    // The unknown adapter never bids, and now there is a cause to name.
    let never_bid = finding(&capture, &diagnosed, "bidders-never-bid");
    assert_eq!(never_bid.affected_bidders, ["acmeBidder", "triplelift"]);
    assert!(
        never_bid.description.contains(
            "- `acmeBidder`: Prebid reported 2 bids whose params the adapter itself rejected."
        ),
        "{}",
        never_bid.description
    );
    assert_eq!(
        finding(&capture, &diagnosed, "bidder-params-invalid").affected_bidders,
        ["rubicon"]
    );
}

#[test]
fn test_ids_and_failing_requests_of_bidders_that_do_bid_are_reported() {
    let mut capture = page(
        &[
            bidder(
                "rubicon",
                json!({ "accountId": 14062, "siteId": 70608, "zoneId": 498816 }),
            ),
            bidder(
                "openx",
                json!({ "delDomain": "pub-d.openx.net", "unit": "1" }),
            ),
        ],
        vec![],
    );
    capture.bid_responses = vec![ObservedBidResponse {
        page: 0,
        t: 1_100.0,
        bidder: "openx".into(),
        host: "rtb.openx.net".into(),
        status: Some(403),
        reason: Some("error: account suspended".into()),
        request_seq: None,
    }];
    let diagnosed = diagnose(
        &capture,
        &found("rubiconproject.com, 14062, DIRECT\nopenx.com, 5, DIRECT"),
    );

    assert_eq!(
        fired(&capture, &diagnosed),
        ["bidder-test-ids", "bidder-endpoint-errors"]
    );
    let test_ids = finding(&capture, &diagnosed, "bidder-test-ids");
    assert_eq!(test_ids.affected_bidders, ["rubicon"]);
    assert_eq!(
        test_ids.affected_slots,
        ["article", "footer", "rail", "top"]
    );
    let errors = finding(&capture, &diagnosed, "bidder-endpoint-errors");
    assert_eq!(errors.affected_bidders, ["openx"]);
    assert_eq!(
        (
            errors.evidence[0].label.as_str(),
            errors.evidence[0].body.as_str()
        ),
        (
            "openx: The bidder's endpoint answered HTTP 403 on its 1 bid request",
            "error: account suspended"
        )
    );
}

#[test]
fn ads_txt_problems_of_the_site_and_of_bidders_that_do_bid() {
    let bidders = || {
        let mut rubicon = bidder(
            "rubicon",
            json!({ "accountId": 1001, "siteId": 2, "zoneId": 3 }),
        );
        rubicon.schain = vec![
            SchainNode {
                asi: "Reseller.example".into(),
                sid: "r-9".into(),
                hp: Some(true),
            },
            SchainNode {
                asi: "unpaid.example".into(),
                sid: "u-1".into(),
                hp: Some(false),
            },
        ];
        vec![
            rubicon,
            bidder("criteo", json!({ "networkId": 7100 })),
            bidder(
                "ttd",
                json!({ "supplySourceId": "acme", "publisherId": "1" }),
            ),
        ]
    };
    let capture = page(&bidders(), vec![]);

    // The account in use is not the account that is authorised.
    let wrong_account = diagnose(
        &capture,
        &found(
            "rubiconproject.com, 9999, DIRECT\nreseller.example, R-9, RESELLER\nbroken line\ngoogle.com pub-1 DIRECT",
        ),
    );
    assert_eq!(
        fired(&capture, &wrong_account),
        ["ads-txt-bidder-not-authorised", "ads-txt-invalid-lines"]
    );
    let unauthorised = finding(&capture, &wrong_account, "ads-txt-bidder-not-authorised");
    assert_eq!(unauthorised.affected_bidders, ["criteo", "rubicon"]);
    assert!(
        unauthorised.description.ends_with(
            "- `criteo`: ads.txt has no entry for criteo.com\n\
             - `rubicon`: ads.txt lists rubiconproject.com but not account 1001, which the `accountId` param uses"
        ),
        "{}",
        unauthorised.description
    );
    // The Trade Desk's ads.txt domain is not in the table, so it is not judged.
    assert_eq!(
        check_of(&wrong_account, "ttd", "ads-txt-unknown-system").status,
        DiagnosticStatus::NotChecked
    );
    let invalid = finding(&capture, &wrong_account, "ads-txt-invalid-lines");
    assert_eq!(invalid.title, "2 line(s) of ads.txt are not valid records");
    assert_eq!(
        invalid
            .evidence
            .iter()
            .map(|evidence| (evidence.label.as_str(), evidence.body.as_str()))
            .collect::<Vec<_>>(),
        [
            (
                "Line 3: a record needs an ad system domain, an account id and DIRECT or RESELLER",
                "broken line"
            ),
            (
                "Line 4: a record needs an ad system domain, an account id and DIRECT or RESELLER",
                "google.com pub-1 DIRECT"
            ),
        ]
    );

    // A supply chain node that ads.txt does not list.
    let schain = diagnose(
        &capture,
        &found("rubiconproject.com, 1001, DIRECT\ncriteo.com, B-1, DIRECT"),
    );
    assert_eq!(
        check_of(&schain, "rubicon", "ads-txt-schain-mismatch").summary,
        "The supply chain sent to this bidder names seller r-9 at reseller.example, which ads.txt does not list"
    );
    assert_eq!(fired(&capture, &schain), ["ads-txt-bidder-not-authorised"]);

    // No ads.txt at all is one finding about the site, not one per bidder.
    let missing = diagnose(&capture, &files(AdsTxtOutcome::Missing));
    assert_eq!(fired(&capture, &missing), ["ads-txt-missing"]);
    let no_file = finding(&capture, &missing, "ads-txt-missing");
    assert_eq!(no_file.affected_bidders, ["criteo", "rubicon", "ttd"]);
    assert_eq!(missing.ads_txt.status, AdsTxtStatus::Missing);

    // A file that could not be read proves nothing and raises nothing.
    let unreadable = diagnose(
        &capture,
        &files(AdsTxtOutcome::Unreadable(
            "the site answered HTTP 403".into(),
        )),
    );
    assert_eq!(fired(&capture, &unreadable), Vec::<&str>::new());
    assert_eq!(
        (
            unreadable.ads_txt.status,
            unreadable.ads_txt.detail.as_deref()
        ),
        (AdsTxtStatus::Unreadable, Some("the site answered HTTP 403"))
    );
    assert_eq!(
        check_of(&unreadable, "rubicon", "ads-txt-unavailable").summary,
        "The site's ads.txt could not be read"
    );
}

fn seller(id: &str, seller_type: SellerType, domain: &str, is_confidential: bool) -> Seller {
    Seller {
        seller_id: id.into(),
        seller_type: Some(seller_type),
        is_confidential,
        domain: Some(domain.into()),
    }
}

#[test]
fn sellers_json_is_looked_up_for_the_account_in_use_or_else_the_direct_records() {
    let capture = page(
        &[
            bidder(
                "rubicon",
                json!({ "accountId": 1001, "siteId": 2, "zoneId": 3 }),
            ),
            bidder("appnexus", json!({ "placementId": 5 })),
            bidder("criteo", json!({ "networkId": 7100 })),
            bidder(
                "ttd",
                json!({ "supplySourceId": "acme", "publisherId": "1" }),
            ),
        ],
        vec![],
    );
    let ads_txt = ads_txt::parse(
        "OWNERDOMAIN=pub-media.example\n\
         rubiconproject.com, 1001, DIRECT\nrubiconproject.com, 777, RESELLER\n\
         appnexus.com, 958, DIRECT\nxandr.com, 12, DIRECT\nappnexus.com, 3000, RESELLER\n\
         openx.com, 5, DIRECT\n",
    );
    let lookups = diagnostics::sellers_lookups(&capture, &ads_txt);
    let lookup = |domain: &str, ids: &[&str]| SellersLookup {
        ad_system_domain: domain.into(),
        seller_ids: strings(ids),
    };
    assert_eq!(
        lookups,
        [
            lookup("appnexus.com", &["958"]),
            lookup("rubiconproject.com", &["1001"]),
            lookup("xandr.com", &["12"]),
        ],
        "only exchanges of bidders on the page, and only ids that matter"
    );

    let mut files = files(AdsTxtOutcome::Found(ads_txt));
    files.sellers = Some(BTreeMap::from([
        (
            "rubiconproject.com".to_string(),
            SellersOutcome::Read(vec![seller(
                "1001",
                SellerType::Publisher,
                "pub-media.example",
                false,
            )]),
        ),
        (
            "appnexus.com".to_string(),
            SellersOutcome::Read(vec![seller(
                "958",
                SellerType::Intermediary,
                "network.example",
                true,
            )]),
        ),
        (
            "xandr.com".to_string(),
            SellersOutcome::Unavailable("the request timed out".into()),
        ),
    ]));
    let diagnosed = diagnose(&capture, &files);
    assert!(diagnosed.sellers_json_checked);

    let sellers_checks = |name: &str| -> Vec<(DiagnosticStatus, String)> {
        checks(&diagnosed, name)
            .into_iter()
            .filter(|(area, _, _)| *area == DiagnosticArea::SellersJson)
            .map(|(_, status, code)| (status, code))
            .collect()
    };
    use DiagnosticStatus::{NotChecked, Notice, Pass};
    assert_eq!(
        sellers_checks("rubicon"),
        [(Pass, "sellers-json-ok".to_string())]
    );
    assert_eq!(
        sellers_checks("appnexus"),
        [
            (Notice, "sellers-json-confidential".to_string()),
            (Notice, "sellers-json-type-mismatch".to_string()),
            (NotChecked, "sellers-json-unavailable".to_string()),
        ]
    );
    assert_eq!(sellers_checks("criteo"), []);
    assert_eq!(
        check_of(&diagnosed, "appnexus", "sellers-json-unavailable").summary,
        "The sellers.json of xandr.com could not be read: the request timed out"
    );

    let issues = finding(&capture, &diagnosed, "sellers-json-issues");
    assert_eq!(
        (issues.severity, issues.affected_bidders.as_slice()),
        (Severity::Low, &["appnexus".to_string()][..])
    );
    assert_eq!(
        fired(&capture, &diagnosed),
        ["ads-txt-bidder-not-authorised", "sellers-json-issues"],
        "criteo has no ads.txt entry; appnexus disagrees with its sellers.json"
    );

    // A seller id the exchange does not know is a problem, not a notice.
    files.sellers = Some(BTreeMap::from([(
        "rubiconproject.com".to_string(),
        SellersOutcome::Read(vec![]),
    )]));
    let unknown_id = diagnose(&capture, &files);
    let issues = finding(&capture, &unknown_id, "sellers-json-issues");
    assert_eq!(issues.severity, Severity::Medium);
    assert_eq!(
        issues.evidence[0].label,
        "rubicon: Seller id 1001 from ads.txt is not in the sellers.json of rubiconproject.com"
    );
}

fn tcf(t: f64, status: &str, vendors: &[u32]) -> TrackerEvent {
    ev(
        t,
        Ev::TcfState {
            event_status: Some(status.into()),
            gdpr_applies: Some(true),
            has_tc_string: status != "cmpuishown",
            purpose_consents: vec![1, 2, 7],
            vendor_consents: vendors.to_vec(),
        },
    )
}

fn consenting(name: &'static str, params: Value, has_string: bool) -> Bidder {
    Bidder {
        gdpr_applies: Some(true),
        has_consent_string: Some(has_string),
        ..bidder(name, params)
    }
}

#[test]
fn consent_findings_separate_a_late_choice_from_a_missing_string_and_a_denied_vendor() {
    let rubicon = json!({ "accountId": 1001, "siteId": 2, "zoneId": 3 });
    let appnexus = json!({ "placementId": 5 });
    let ads_txt = found(HEALTHY_ADS_TXT);

    // The banner is still open when the auction runs.
    let early = page(
        &[consenting("rubicon", rubicon.clone(), false)],
        vec![
            tcf(300.0, "cmpuishown", &[]),
            tcf(5_000.0, "useractioncomplete", &[52]),
        ],
    );
    let diagnosed = diagnose(&early, &ads_txt);
    assert_eq!(
        (
            diagnosed.consent.cmp_present,
            diagnosed.consent.gdpr_applies,
            diagnosed.consent.resolved_at_ms,
            diagnosed.consent.auctions_before_consent,
            diagnosed.consent.auction_count,
        ),
        (true, Some(true), Some(5_000.0), 1, 1)
    );
    assert_eq!(fired(&early, &diagnosed), ["auction-before-consent"]);
    let before_consent = finding(&early, &diagnosed, "auction-before-consent");
    assert_eq!(
        before_consent.title,
        "1 of 1 auction(s) ran before the user's consent was known"
    );
    assert_eq!(
        before_consent.evidence[0].body,
        "consent known at: 5.0 s | auctions before that: 1 of 1 | Prebid consentManagement: configured"
    );

    // The choice is known, but one bidder gets no string and another is not consented to.
    let decided = page(
        &[
            consenting("rubicon", rubicon.clone(), false),
            consenting("appnexus", appnexus.clone(), true),
        ],
        vec![tcf(300.0, "tcloaded", &[52])],
    );
    let diagnosed = diagnose(&decided, &ads_txt);
    assert_eq!(diagnosed.consent.auctions_before_consent, 0);
    assert_eq!(fired(&decided, &diagnosed), ["bidders-without-consent"]);
    let without = finding(&decided, &diagnosed, "bidders-without-consent");
    assert_eq!(without.affected_bidders, ["appnexus", "rubicon"]);
    assert!(
        without.description.ends_with(
            "- `appnexus`: The user's consent does not cover appnexus (IAB vendor 32) on its bid request\n\
             - `rubicon`: Its bid request went out without a consent string although GDPR applies"
        ),
        "{}",
        without.description
    );

    // Everything in order.
    let consented = page(
        &[
            consenting("rubicon", rubicon, true),
            consenting("appnexus", appnexus, true),
        ],
        vec![tcf(300.0, "tcloaded", &[32, 52])],
    );
    let diagnosed = diagnose(&consented, &ads_txt);
    assert_eq!(
        check_of(&diagnosed, "rubicon", "consent-ok").summary,
        "A consent string was passed and it covers rubicon (IAB vendor 52)"
    );
    assert_eq!(fired(&consented, &diagnosed), Vec::<&str>::new());
}

#[test]
fn an_auction_that_starts_with_the_consent_callback_is_not_before_consent() {
    // The wrapper's own listener runs a moment before the tracker's, and
    // Prebid already holds the string: the request carries it.
    let capture = page(
        &[consenting(
            "rubicon",
            json!({ "accountId": 1001, "siteId": 2, "zoneId": 3 }),
            true,
        )],
        vec![
            tcf(300.0, "cmpuishown", &[]),
            tcf(1_030.0, "useractioncomplete", &[52]),
        ],
    );
    let diagnosed = diagnose(&capture, &found(HEALTHY_ADS_TXT));
    assert_eq!(diagnosed.consent.auctions_before_consent, 0);
    assert_eq!(fired(&capture, &diagnosed), Vec::<&str>::new());
}

#[test]
fn consent_management_that_is_not_configured_is_named_once() {
    // A CMP answers, GDPR applies, and Prebid passes nothing to the bidders.
    let mut bidders = healthy_bidders();
    for bidder in &mut bidders {
        bidder.gdpr_applies = None;
        bidder.has_consent_string = None;
    }
    let mut capture = page(&bidders, vec![tcf(300.0, "tcloaded", &[32, 52])]);
    for event in &mut capture.events {
        if let Ev::PrebidConfig {
            has_consent_management,
            ..
        } = &mut event.kind
        {
            *has_consent_management = false;
        }
    }
    capture.consent_accepted = true;
    let diagnosed = diagnose(&capture, &found(HEALTHY_ADS_TXT));
    assert_eq!(
        fired(&capture, &diagnosed),
        ["bidders-without-consent"],
        "the per-bidder finding replaces the generic consent-not-wired one"
    );
    let without = finding(&capture, &diagnosed, "bidders-without-consent");
    assert_eq!(
        without.evidence.last().unwrap().body,
        "`pbjs.getConfig()` has no `consentManagement` section, so Prebid does not read the CMP"
    );
}

#[test]
fn stored_text_is_always_short() {
    let long_reason = "x".repeat(2_000);
    let mut capture = page(
        &[silent(
            "rubicon",
            json!({ "accountId": "y".repeat(900), "siteId": 2, "zoneId": 3 }),
        )],
        vec![],
    );
    capture.bid_responses = vec![ObservedBidResponse {
        page: 0,
        t: 1_100.0,
        bidder: "rubicon".into(),
        host: "fastlane.rubiconproject.com".into(),
        status: Some(200),
        reason: Some(long_reason),
        request_seq: None,
    }];
    let ads_txt = format!(
        "rubiconproject.com, {}, DIRECT\n{}\n",
        "9".repeat(700),
        "z".repeat(5_000)
    );
    let diagnosed = diagnose(&capture, &found(&ads_txt));
    let stored = serde_json::to_value(&diagnosed).unwrap();
    fn longest(value: &Value) -> usize {
        match value {
            Value::String(text) => text.chars().count(),
            Value::Array(items) => items.iter().map(longest).max().unwrap_or(0),
            Value::Object(fields) => fields.values().map(longest).max().unwrap_or(0),
            _ => 0,
        }
    }
    assert!(
        longest(&stored["adsTxt"]) <= 300,
        "{}",
        longest(&stored["adsTxt"])
    );
    for check in diagnosed.bidders.iter().flat_map(|bidder| &bidder.checks) {
        assert!(
            check
                .excerpt
                .as_ref()
                .is_none_or(|excerpt| excerpt.chars().count() <= 300),
            "{check:#?}"
        );
        assert!(check.summary.chars().count() <= 500, "{check:#?}");
    }
}

#[test]
fn sellers_json_is_only_asked_of_known_exchanges_and_of_a_bounded_number() {
    use adscan_engine::domain::diagnostics::adapters::ADAPTERS;

    let with_ad_system: Vec<_> = ADAPTERS
        .iter()
        .filter(|adapter| !adapter.ad_system_domains.is_empty())
        .collect();
    let bidders: Vec<Bidder> = with_ad_system
        .iter()
        .map(|adapter| bidder(adapter.code, json!({})))
        .collect();
    let capture = page(&bidders, vec![]);
    // Every exchange the scanner knows, and names only the file vouches for.
    let mut text = String::from(
        "evil.example, 1, DIRECT\nlocalhost.localdomain, 1, DIRECT\n10.0.0.1, 1, DIRECT\n\
         metadata.google.internal, 1, DIRECT\n",
    );
    for domain in with_ad_system
        .iter()
        .flat_map(|adapter| adapter.ad_system_domains)
    {
        text.push_str(&format!("{domain}, 1, DIRECT\n"));
    }
    let lookups = diagnostics::sellers_lookups(&capture, &ads_txt::parse(&text));

    assert_eq!(
        lookups.len(),
        12,
        "a scan contacts at most twelve exchanges"
    );
    let known: Vec<&str> = ADAPTERS
        .iter()
        .flat_map(|adapter| adapter.ad_system_domains.iter().copied())
        .collect();
    for lookup in &lookups {
        assert!(
            known.contains(&lookup.ad_system_domain.as_str()),
            "{} is not an exchange of the adapter table",
            lookup.ad_system_domain
        );
    }
}

#[test]
fn every_ad_system_domain_of_the_adapter_table_has_a_sellers_json_address() {
    use adscan_engine::domain::diagnostics::adapters::ADAPTERS;
    use adscan_engine::domain::diagnostics::site::sellers_json_url;

    for domain in ADAPTERS
        .iter()
        .flat_map(|adapter| adapter.ad_system_domains)
    {
        assert_eq!(
            sellers_json_url(domain),
            Some(format!("https://{domain}/sellers.json"))
        );
    }
}

#[test]
fn bidders_that_run_through_prebid_server_are_not_held_to_the_adapters_param_rules() {
    // Prebid Server accepts `placementId` alone for Teads and `pubid` for Criteo.
    let bidders = [
        bidder("teads", json!({ "placementId": 5 })),
        bidder("criteo", json!({ "pubid": "ABC123" })),
        bidder("improvedigital", json!({ "placementId": 5 })),
    ];
    let with_s2s = |s2s_config: Value| {
        let mut capture = page(&bidders, vec![]);
        for event in &mut capture.events {
            if let Ev::PrebidConfig { raw, .. } = &mut event.kind {
                raw["s2sConfig"] = s2s_config.clone();
            }
        }
        let diagnosed = diagnose(&capture, &files(AdsTxtOutcome::Missing));
        ["criteo", "improvedigital", "teads"].map(|name| {
            let params = checks(&diagnosed, name)
                .into_iter()
                .find(|(area, _, _)| *area == DiagnosticArea::Params)
                .unwrap();
            (params.1, params.2)
        })
    };
    let missing = (DiagnosticStatus::Problem, "params-missing".to_string());
    let server_side = (
        DiagnosticStatus::NotChecked,
        "params-server-side".to_string(),
    );

    assert_eq!(
        with_s2s(Value::Null),
        [missing.clone(), missing.clone(), missing.clone()]
    );
    assert_eq!(
        with_s2s(json!({ "enabled": true, "bidders": ["teads", "criteo"] })),
        [server_side.clone(), missing.clone(), server_side.clone()]
    );
    // Several servers, as an array; a switched-off one sends nothing.
    assert_eq!(
        with_s2s(json!([
            { "bidders": ["improvedigital"] },
            { "enabled": false, "bidders": ["teads"] },
            { "bidders": "criteo" },
        ])),
        [missing.clone(), server_side, missing]
    );
}

#[test]
fn text_from_the_site_is_stored_without_characters_that_hide_or_reorder_it() {
    let capture = page(&[bidder("appnexus", json!({ "placementId": 5 }))], vec![]);
    let diagnosed = diagnose(
        &capture,
        &found(
            "appnexus.com, 9\u{202e}58\u{1b}[31m\u{200b}, DIRECT\nbroken\u{0007}\u{2066} line\n",
        ),
    );
    let stored = serde_json::to_string(&diagnosed).unwrap();
    for hidden in ['\u{202e}', '\u{1b}', '\u{200b}', '\u{0007}', '\u{2066}'] {
        assert!(!stored.contains(hidden), "{hidden:?} in {stored}");
    }
    assert_eq!(diagnosed.ads_txt.invalid_lines[0].excerpt, "broken   line");
}

#[test]
fn a_page_served_from_this_machine_is_not_told_that_its_site_has_no_ads_txt() {
    let mut capture = page(&healthy_bidders(), vec![]);
    let missing = diagnose(&capture, &files(AdsTxtOutcome::Missing));
    assert_eq!(fired(&capture, &missing), ["ads-txt-missing"]);
    let no_file = finding(&capture, &missing, "ads-txt-missing");
    assert_eq!(no_file.severity, Severity::Medium);
    assert!(
        no_file
            .description
            .contains("The specification lets them buy it anyway"),
        "{}",
        no_file.description
    );

    for local in ["http://localhost:5173/article", "http://127.0.0.1:8080/"] {
        capture.final_url = local.into();
        let missing = diagnose(&capture, &files(AdsTxtOutcome::Missing));
        assert_eq!(fired(&capture, &missing), Vec::<&str>::new(), "{local}");
    }
}

#[test]
fn a_bidder_code_the_publisher_made_up_with_alias_bidder_raises_nothing() {
    // `pbjs.aliasBidder('rubicon', 'rubicon_east')`: the scanner cannot know
    // which adapter is behind the name, so nothing is held against its params.
    let capture = page(
        &[
            bidder("rubicon_east", json!({})),
            bidder("appnexus-video", json!({ "member": 1 })),
        ],
        vec![],
    );
    let diagnosed = diagnose(
        &capture,
        &found("rubiconproject.com, 1001, DIRECT\nappnexus.com, 958, DIRECT\n"),
    );
    for diagnosis in &diagnosed.bidders {
        let flagged: Vec<_> = diagnosis
            .checks
            .iter()
            .filter(|check| check.status != DiagnosticStatus::NotChecked)
            .collect();
        assert!(flagged.is_empty(), "{}: {flagged:#?}", diagnosis.bidder);
    }
    assert_eq!(fired(&capture, &diagnosed), Vec::<&str>::new());
}

/// The same page visited twice in one visit: the second page view starts
/// 12.4 s into the scan and reports nothing new about consent.
fn visited_twice(first: &Capture) -> Capture {
    let mut capture = first.clone();
    capture.pages = vec![
        page_view("https://pub.example/", PageReach::Entry, 0.0, 12_000.0),
        page_view(
            "https://pub.example/sport/final",
            PageReach::LinkClick,
            12_400.0,
            20_400.0,
        ),
    ];
    let again: Vec<TrackerEvent> = first
        .events
        .iter()
        .filter(|event| !matches!(event.kind, Ev::TcfState { .. }))
        .cloned()
        .map(|mut event| {
            event.page = 1;
            match &mut event.kind {
                Ev::AuctionInit { auction_id, .. }
                | Ev::BidRequested { auction_id, .. }
                | Ev::BidResponse { auction_id, .. }
                | Ev::NoBid { auction_id, .. }
                | Ev::AuctionEnd { auction_id } => *auction_id = "a2".into(),
                _ => {}
            }
            event
        })
        .collect();
    capture.events.extend(again);
    capture
}

#[test]
fn a_consent_choice_made_on_one_page_view_holds_on_the_next() {
    let rubicon = json!({ "accountId": 1001, "siteId": 2, "zoneId": 3 });
    let ads_txt = found(HEALTHY_ADS_TXT);

    // The choice comes 5 s into the first page view: its auction (at 1 s) was
    // early. The second page view's auction also starts 1 s into its page,
    // which is 13.4 s into the visit and long after the choice.
    let late_choice = visited_twice(&page(
        &[consenting("rubicon", rubicon.clone(), false)],
        vec![
            tcf(300.0, "cmpuishown", &[]),
            tcf(5_000.0, "useractioncomplete", &[52]),
        ],
    ));
    let diagnosed = diagnose(&late_choice, &ads_txt);
    assert_eq!(
        (
            diagnosed.consent.resolved_at_ms,
            diagnosed.consent.auctions_before_consent,
            diagnosed.consent.auction_count,
        ),
        (Some(5_000.0), 1, 2)
    );
    // Each check names the page views it was seen on.
    assert_eq!(
        check_of(&diagnosed, "rubicon", "consent-no-string").pages,
        [0, 1]
    );
    let pending = check_of(&diagnosed, "rubicon", "consent-pending");
    assert_eq!(pending.pages, [0]);
    assert_eq!(
        pending.summary,
        "1 of 2 bid requests went out before the user's consent choice was known"
    );
    assert_eq!(
        diagnosed.bidders[0].requests, 8,
        "requests of both page views"
    );

    // A single page view has nothing page-specific to say.
    let single = page(
        &[consenting("rubicon", rubicon, false)],
        vec![tcf(300.0, "tcloaded", &[52])],
    );
    let diagnosed = diagnose(&single, &ads_txt);
    assert!(
        check_of(&diagnosed, "rubicon", "consent-no-string")
            .pages
            .is_empty()
    );
}

#[test]
fn endpoint_answers_name_the_page_views_they_failed_on() {
    let appnexus = json!({ "placementId": 5 });
    let mut capture = visited_twice(&page(&[bidder("appnexus", appnexus)], vec![]));
    capture.bid_responses = vec![
        ObservedBidResponse {
            page: 0,
            t: 1_100.0,
            bidder: "appnexus".into(),
            host: "ib.adnxs.com".into(),
            status: Some(200),
            reason: None,
            request_seq: None,
        },
        ObservedBidResponse {
            page: 1,
            t: 1_100.0,
            bidder: "appnexus".into(),
            host: "ib.adnxs.com".into(),
            status: Some(503),
            reason: None,
            request_seq: None,
        },
    ];
    let diagnosed = diagnose(&capture, &found(HEALTHY_ADS_TXT));
    let failed = check_of(&diagnosed, "appnexus", "response-http-error");
    assert_eq!(failed.pages, [1]);
    assert_eq!(
        failed.summary,
        "The bidder's endpoint answered HTTP 503 on 1 of 2 bid requests"
    );
}
