//! Telemetry derivation and every heuristic rule, on canned captures: a
//! healthy page must stay silent, and each rule must fire on its own fixture.

mod common;

use adscan_engine::domain::finding::FindingDraft;
use adscan_engine::domain::heuristics::{self, RuleContext};
use adscan_engine::domain::scripts;
use adscan_engine::domain::telemetry::{self, Telemetry};
use adscan_model::{
    BidStatus, Capture, DeviceProfile, FindingCategory, FindingSource, Severity, SlotOutcome,
    TrackedAdUnit, TrackerEventKind as Ev,
};
use common::*;

fn evaluate(capture: &Capture) -> Vec<(&'static str, FindingDraft)> {
    let telemetry = telemetry::derive(capture);
    let records: Vec<_> = scripts::describe(capture)
        .into_iter()
        .map(|script| script.record)
        .collect();
    heuristics::evaluate(&RuleContext {
        capture,
        telemetry: &telemetry,
        diagnostics: None,
        scripts: &records,
    })
    .into_iter()
    .map(|hit| (hit.rule, hit.finding))
    .collect()
}

fn fired(capture: &Capture) -> Vec<&'static str> {
    evaluate(capture)
        .into_iter()
        .map(|(rule, _)| rule)
        .collect()
}

fn only(capture: &Capture, rule: &str) -> FindingDraft {
    let hits = evaluate(capture);
    let rules: Vec<_> = hits.iter().map(|(rule, _)| *rule).collect();
    assert_eq!(rules, [rule], "expected exactly `{rule}` to fire");
    let finding = hits.into_iter().next().unwrap().1;
    assert_eq!(finding.source, FindingSource::Heuristic);
    assert!(
        !finding.description.is_empty()
            && !finding.recommendation.is_empty()
            && !finding.impact.is_empty()
    );
    assert!(!finding.evidence.is_empty(), "a finding needs evidence");
    if let Some(pct) = finding.estimated_revenue_impact_pct {
        assert_eq!(pct, pct.round(), "estimates do not pretend to precision");
        let basis = finding
            .evidence
            .iter()
            .find(|evidence| evidence.label == "How the revenue impact was estimated")
            .expect("an estimate states what it rests on");
        assert!(
            basis
                .body
                .starts_with(&format!("About {pct:.0}% of ad revenue at risk: "))
        );
        assert!(
            basis
                .body
                .ends_with("An estimate from one page view, not a measurement.")
        );
    }
    finding
}

fn derived(capture: &Capture) -> Telemetry {
    telemetry::derive(capture)
}

/// The healthy page with its events edited.
fn healthy_with(edit: impl FnOnce(&mut Vec<adscan_model::TrackerEvent>)) -> Capture {
    let mut capture = healthy_capture(DeviceProfile::Desktop);
    edit(&mut capture.events);
    capture.events.sort_by(|a, b| a.t.total_cmp(&b.t));
    capture
}

// -- telemetry -----------------------------------------------------------------

#[test]
fn healthy_page_derives_auction_bids_and_statuses() {
    let telemetry = derived(&healthy_capture(DeviceProfile::Desktop));
    assert_eq!(telemetry.auctions.len(), 1);
    let auction = &telemetry.auctions[0];
    assert_eq!(
        (
            auction.started_at_ms,
            auction.ended_at_ms,
            auction.duration_ms
        ),
        (360.0, Some(700.0), Some(340.0))
    );
    assert_eq!(auction.timeout_ms, Some(1500.0));

    let top = &auction.ad_units[0];
    assert_eq!(
        (top.winning_bidder.as_deref(), top.winning_cpm),
        (Some("appnexus"), Some(1.5))
    );
    let status = |code: &str, bidder: &str| {
        let unit = auction
            .ad_units
            .iter()
            .find(|unit| unit.code == code)
            .unwrap();
        unit.bids
            .iter()
            .find(|bid| bid.bidder == bidder)
            .unwrap()
            .clone()
    };
    assert_eq!(status("top", "appnexus").status, BidStatus::Rendered);
    assert_eq!(status("top", "rubicon").status, BidStatus::Bid);
    assert_eq!(status("side", "appnexus").status, BidStatus::NoBid);
    assert_eq!(status("top", "rubicon").requested_at_ms, Some(365.0));
    assert_eq!(status("top", "rubicon").response_time_ms, Some(235.0));
    assert_eq!(status("top", "rubicon").cpm, Some(1.2));
}

#[test]
fn healthy_page_derives_slots_and_metrics() {
    let telemetry = derived(&healthy_capture(DeviceProfile::Desktop));
    assert_eq!(telemetry.slots.len(), 3);
    let top = &telemetry.slots[0];
    assert_eq!(top.outcome, SlotOutcome::Filled);
    assert_eq!(top.ad_unit_path.as_deref(), Some("/123/top"));
    assert_eq!(
        (top.defined_at_ms, top.requested_at_ms, top.rendered_at_ms),
        (Some(200.0), Some(710.0), Some(900.0))
    );
    assert_eq!(top.in_initial_viewport, Some(true));
    assert_eq!(top.had_prebid_targeting, Some(true));
    assert_eq!(top.requested_before_auction_end, Some(false));

    let metrics = &telemetry.metrics;
    assert_eq!(
        (
            metrics.auction_count,
            metrics.ad_unit_count,
            metrics.bidder_count
        ),
        (1, 3, 2)
    );
    assert_eq!(
        (metrics.bid_request_count, metrics.bid_response_count),
        (6, 5)
    );
    assert_eq!((metrics.no_bid_count, metrics.bid_timeout_count), (1, 0));
    assert_eq!(metrics.timeout_rate, 0.0);
    assert_eq!(
        (
            metrics.slot_count,
            metrics.empty_slot_count,
            metrics.fill_rate
        ),
        (3, 0, 1.0)
    );
    assert_eq!(
        (metrics.avg_winning_cpm, metrics.total_winning_cpm),
        (Some(1.5), 1.5)
    );
    assert_eq!(metrics.time_to_first_auction_ms, Some(360.0));
    assert_eq!(metrics.time_to_first_ad_ms, Some(900.0));
    assert_eq!(metrics.prebid_timeout_ms, Some(1500.0));
    assert_eq!(metrics.page_load_ms, Some(950.0));
    assert_eq!(metrics.script_count, 5);
    assert!(metrics.has_prebid && metrics.has_gpt && !metrics.has_amazon_tam);
    assert_eq!(
        telemetry::prebid_version(&healthy_capture(DeviceProfile::Desktop)).as_deref(),
        Some("9.20.0")
    );
}

#[test]
fn timeouts_errors_and_pending_bids_get_their_status() {
    let capture = healthy_with(|events| {
        events.retain(|event| {
            !matches!(&event.kind, Ev::BidResponse { bidder, .. } | Ev::NoBid { bidder, .. } if bidder == "appnexus")
                && !matches!(event.kind, Ev::BidWon { .. } | Ev::AdRenderSucceeded { .. })
        });
        events.push(ev(
            690.0,
            Ev::BidTimeout {
                auction_id: "auction-1".into(),
                bidder: "appnexus".into(),
                ad_unit_code: "top".into(),
            },
        ));
        events.push(ev(
            695.0,
            Ev::BidderError {
                auction_id: Some("auction-1".into()),
                bidder: "appnexus".into(),
                message: Some("503".into()),
            },
        ));
        // A response after the timeout must not turn the bid into a bid.
        events.push(bid_response(1200.0, "auction-1", "appnexus", "top", 9.0));
    });
    let telemetry = derived(&capture);
    let statuses: Vec<BidStatus> = telemetry.auctions[0]
        .ad_units
        .iter()
        .map(|unit| {
            unit.bids
                .iter()
                .find(|bid| bid.bidder == "appnexus")
                .unwrap()
                .status
        })
        .collect();
    assert_eq!(
        statuses,
        [BidStatus::Timeout, BidStatus::Error, BidStatus::Error]
    );
    assert_eq!(telemetry.metrics.bid_timeout_count, 1);

    let pending = healthy_with(|events| {
        events.retain(|event| !matches!(&event.kind, Ev::BidResponse { bidder, ad_unit_code, .. } if bidder == "rubicon" && ad_unit_code == "footer"));
    });
    let footer = &derived(&pending).auctions[0].ad_units[2];
    assert_eq!(
        footer
            .bids
            .iter()
            .find(|bid| bid.bidder == "rubicon")
            .unwrap()
            .status,
        BidStatus::Pending
    );
}

#[test]
fn slots_correlate_with_ad_units_by_path_and_report_request_races() {
    let capture = healthy_with(|events| {
        for event in events.iter_mut() {
            match &mut event.kind {
                // The ad unit code is the GPT path rather than the div id.
                Ev::AuctionInit { ad_units, .. } => ad_units[0].code = "/123/top".into(),
                Ev::BidRequested { ad_unit_codes, .. } => ad_unit_codes[0] = "/123/top".into(),
                Ev::BidResponse { ad_unit_code, .. } | Ev::BidWon { ad_unit_code, .. }
                    if ad_unit_code == "top" =>
                {
                    *ad_unit_code = "/123/top".into();
                }
                // The side slot is requested while the auction is still running.
                Ev::SlotRequested {
                    element_id,
                    prebid_targeting_keys,
                    ..
                } if element_id == "side" => {
                    event.t = 500.0;
                    prebid_targeting_keys.clear();
                }
                _ => {}
            }
        }
    });
    let telemetry = derived(&capture);
    let slot = |id: &str| {
        telemetry
            .slots
            .iter()
            .find(|slot| slot.element_id == id)
            .unwrap()
    };
    assert_eq!(slot("top").requested_before_auction_end, Some(false));
    assert_eq!(slot("side").requested_before_auction_end, Some(true));
    assert_eq!(slot("side").had_prebid_targeting, Some(false));
}

#[test]
fn slot_outcomes_cover_empty_pending_and_never_requested() {
    let capture = healthy_with(|events| {
        events.retain(|event| match &event.kind {
            Ev::SlotRequested { element_id, .. } | Ev::SlotResponseReceived { element_id } => {
                element_id != "footer"
            }
            Ev::SlotRenderEnded { element_id, .. } => element_id == "top",
            _ => true,
        });
        events.retain(|event| !matches!(&event.kind, Ev::SlotRenderEnded { .. }));
        events.push(slot_rendered(900.0, "top", true));
    });
    let telemetry = derived(&capture);
    let outcomes: Vec<SlotOutcome> = telemetry.slots.iter().map(|slot| slot.outcome).collect();
    assert_eq!(
        outcomes,
        [
            SlotOutcome::Empty,
            SlotOutcome::Pending,
            SlotOutcome::NeverRequested
        ]
    );
    assert_eq!(telemetry.slots[2].requested_before_auction_end, None);
    assert_eq!(telemetry.metrics.empty_slot_count, 1);
    assert_eq!(telemetry.metrics.fill_rate, 0.0);
}

// -- heuristics: no false positives ----------------------------------------------

#[test]
fn healthy_page_triggers_no_rule_on_either_device() {
    assert_eq!(
        fired(&healthy_capture(DeviceProfile::Desktop)),
        Vec::<&str>::new()
    );
    assert_eq!(
        fired(&healthy_capture(DeviceProfile::Mobile)),
        Vec::<&str>::new()
    );
}

#[test]
fn lazy_slots_below_the_fold_are_not_flagged_when_the_page_was_not_scrolled() {
    let mut capture = healthy_with(|events| {
        events.retain(|event| match &event.kind {
            Ev::SlotRequested { element_id, .. }
            | Ev::SlotResponseReceived { element_id }
            | Ev::SlotRenderEnded { element_id, .. } => element_id != "footer",
            Ev::PageLifecycle { phase } => phase != "firstScroll",
            Ev::Scroll { .. } => false,
            _ => true,
        });
    });
    capture.max_scroll_depth = 0.0;
    assert_eq!(fired(&capture), Vec::<&str>::new());
}

#[test]
fn rule_ids_are_unique() {
    let mut ids: Vec<&str> = heuristics::all_rules()
        .iter()
        .map(|rule| rule.id())
        .collect();
    let total = ids.len();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), total);
}

// -- heuristics: the reference bug -----------------------------------------------

#[test]
fn scroll_gated_auction_is_reported_with_the_device_and_a_rescan_hint() {
    let desktop = only(
        &scroll_gated_capture(DeviceProfile::Desktop),
        "auction-gated-on-interaction",
    );
    assert_eq!(desktop.category, FindingCategory::LazyLoad);
    assert_eq!(desktop.severity, Severity::High);
    assert_eq!(
        desktop.title,
        "Bidding waits for the first scroll on desktop"
    );
    assert!(desktop.description.contains("**desktop** scan"));
    assert!(
        desktop
            .recommendation
            .contains("Re-scan with the mobile profile")
    );
    assert_eq!(desktop.affected_slots, ["top", "side"]);
    assert!(
        desktop.evidence[0]
            .body
            .contains("first interaction 4.0 s -> first auction 4.1 s")
    );
    assert_eq!(desktop.estimated_revenue_impact_pct, Some(15.0));

    let mobile = only(
        &scroll_gated_capture(DeviceProfile::Mobile),
        "auction-gated-on-interaction",
    );
    assert_eq!(mobile.title, "Bidding waits for the first scroll on mobile");
    assert!(
        mobile
            .recommendation
            .contains("Re-scan with the desktop profile")
    );
}

#[test]
fn the_same_site_is_clean_on_the_unaffected_device_class() {
    // Desktop waits for scroll (bug), mobile bids on load: the two scans differ
    // in exactly this finding.
    assert_eq!(
        fired(&scroll_gated_capture(DeviceProfile::Desktop)),
        ["auction-gated-on-interaction"]
    );
    assert_eq!(
        fired(&healthy_capture(DeviceProfile::Mobile)),
        Vec::<&str>::new()
    );
}

#[test]
fn auction_that_never_starts_is_critical_once_the_page_was_scrolled() {
    let scrolled = only(
        &never_started_capture(DeviceProfile::Desktop, true),
        "auction-gated-on-interaction",
    );
    assert_eq!(scrolled.severity, Severity::Critical);
    assert_eq!(
        scrolled.title,
        "Prebid is loaded but no auction ever started on desktop"
    );
    assert!(
        scrolled
            .recommendation
            .contains("re-scan with the mobile profile")
    );
    assert_eq!(scrolled.affected_slots, ["top", "side", "footer"]);
    assert_eq!(scrolled.estimated_revenue_impact_pct, Some(30.0));

    let idle = only(
        &never_started_capture(DeviceProfile::Mobile, false),
        "auction-gated-on-interaction",
    );
    assert_eq!(idle.severity, Severity::High);
    assert!(idle.description.contains("never scrolled"));
    assert!(idle.confidence < scrolled.confidence);
}

/// GPT requests the ads on load, the auction only starts after the first
/// scroll: the requests without bids are the gate's symptom, not a second bug.
#[test]
fn ad_requests_that_left_before_a_scroll_gated_auction_are_not_reported_twice() {
    let mut events = page_setup();
    for slot in ["top", "side", "footer"] {
        events.push(slot_requested(500.0, slot, false));
        events.push(slot_rendered(700.0, slot, false));
    }
    events.push(lifecycle(950.0, "load"));
    events.extend(scroll_events(4000.0));
    events.extend(auction_round(4060.0, "auction-1"));
    let capture = capture_with(DeviceProfile::Desktop, events, 1.0);

    let slots = derived(&capture).slots;
    assert!(
        slots
            .iter()
            .all(|slot| slot.requested_before_auction_end == Some(true)),
        "the telemetry still records the race"
    );
    assert_eq!(fired(&capture), ["auction-gated-on-interaction"]);
}

/// A slot with its own auction on load whose ad request races that auction is
/// a second bug, even on a page that also gates its main auction on scroll.
#[test]
fn a_request_racing_its_own_auction_is_still_reported_next_to_a_scroll_gate() {
    let mut events = page_setup();
    events.push(slot_defined(210.0, "anchor", false));
    events.push(ev(
        400.0,
        Ev::AuctionInit {
            auction_id: "anchor-auction".into(),
            timeout: Some(1500.0),
            ad_units: vec![TrackedAdUnit {
                code: "anchor".into(),
                sizes: strings(&["320x50"]),
                bidders: strings(&["rubicon"]),
                bids: vec![],
            }],
        },
    ));
    events.push(slot_requested(410.0, "anchor", false));
    events.push(ev(
        700.0,
        Ev::AuctionEnd {
            auction_id: "anchor-auction".into(),
        },
    ));
    events.push(slot_rendered(720.0, "anchor", false));
    events.push(lifecycle(950.0, "load"));
    events.extend(scroll_events(4000.0));
    events.extend(auction_round(4060.0, "auction-1"));
    let capture = capture_with(DeviceProfile::Desktop, events, 1.0);

    assert_eq!(
        fired(&capture),
        [
            "auction-gated-on-interaction",
            "ad-request-before-auction-end"
        ]
    );
    let hits = evaluate(&capture);
    assert_eq!(hits[1].1.affected_slots, ["anchor"]);
}

#[test]
fn an_unanswered_consent_banner_makes_the_idle_auction_finding_less_certain() {
    let mut capture = never_started_capture(DeviceProfile::Desktop, true);
    capture.scripts.push(script(
        "https://sdk.privacy-center.org/loader.js",
        "/* didomi */",
    ));
    let blocked = only(&capture, "auction-gated-on-interaction");
    assert_eq!(
        (blocked.severity, blocked.confidence),
        (Severity::High, 0.5)
    );
    assert!(
        blocked
            .description
            .contains("waiting for the user's consent choice")
    );
    assert!(
        blocked
            .recommendation
            .starts_with("First re-scan with consent acceptance turned on")
    );

    capture.consent_accepted = true;
    let accepted = only(&capture, "auction-gated-on-interaction");
    assert_eq!(
        (accepted.severity, accepted.confidence),
        (Severity::Critical, 0.8)
    );
    assert!(!accepted.description.contains("consent"));
}

/// An anchor slot is auctioned on load, the on-screen slots only after the
/// first scroll: the gate is found even though it is not the first auction.
#[test]
fn scroll_gate_behind_an_earlier_auction_for_another_slot_is_still_found() {
    let mut events = page_setup();
    events.push(slot_defined(210.0, "anchor", false));
    events.push(ev(
        400.0,
        Ev::AuctionInit {
            auction_id: "anchor-auction".into(),
            timeout: Some(1500.0),
            ad_units: vec![TrackedAdUnit {
                code: "anchor".into(),
                sizes: strings(&["320x50"]),
                bidders: strings(&["rubicon"]),
                bids: vec![],
            }],
        },
    ));
    events.push(bid_response(
        600.0,
        "anchor-auction",
        "rubicon",
        "anchor",
        0.7,
    ));
    events.push(ev(
        700.0,
        Ev::AuctionEnd {
            auction_id: "anchor-auction".into(),
        },
    ));
    events.push(slot_requested(710.0, "anchor", true));
    events.push(slot_rendered(900.0, "anchor", false));
    events.extend(scroll_events(4000.0));
    events.extend(auction_round(4060.0, "auction-1"));
    let capture = capture_with(DeviceProfile::Desktop, events, 1.0);

    assert_eq!(
        telemetry::derive(&capture).metrics.time_to_first_auction_ms,
        Some(400.0)
    );
    let finding = only(&capture, "auction-gated-on-interaction");
    assert_eq!(finding.affected_slots, ["top", "side"]);
    assert_eq!(finding.severity, Severity::High);
}

#[test]
fn lazy_auction_for_a_below_the_fold_slot_after_scrolling_is_not_a_gate() {
    let mut events = page_setup();
    events.extend(auction_round(360.0, "auction-1"));
    events.push(slot_defined(220.0, "deep", false));
    events.extend(scroll_events(4000.0));
    events.push(ev(
        4100.0,
        Ev::AuctionInit {
            auction_id: "lazy-auction".into(),
            timeout: Some(1500.0),
            ad_units: vec![TrackedAdUnit {
                code: "deep".into(),
                sizes: strings(&["300x250"]),
                bidders: strings(&["rubicon"]),
                bids: vec![],
            }],
        },
    ));
    events.push(bid_response(4300.0, "lazy-auction", "rubicon", "deep", 0.9));
    events.push(ev(
        4400.0,
        Ev::AuctionEnd {
            auction_id: "lazy-auction".into(),
        },
    ));
    events.push(slot_requested(4410.0, "deep", true));
    events.push(slot_rendered(4600.0, "deep", false));
    let capture = capture_with(DeviceProfile::Mobile, events, 1.0);
    assert_eq!(fired(&capture), Vec::<&str>::new());
}

#[test]
fn an_auction_long_after_the_scroll_is_late_not_gated() {
    let mut events = page_setup();
    events.extend(scroll_events(1000.0));
    events.extend(auction_round(6500.0, "auction-1"));
    let finding = only(
        &capture_with(DeviceProfile::Desktop, events, 1.0),
        "late-first-auction",
    );
    assert_eq!(finding.severity, Severity::High);
    assert_eq!(finding.category, FindingCategory::Timing);
}

// -- heuristics: one fixture per rule ----------------------------------------------

#[test]
fn late_first_auction_without_interaction_gate() {
    let mut events = page_setup();
    events.extend(auction_round(3400.0, "auction-1"));
    events.extend(scroll_events(9000.0));
    let finding = only(
        &capture_with(DeviceProfile::Desktop, events, 1.0),
        "late-first-auction",
    );
    assert_eq!(finding.severity, Severity::Medium);
    assert!(finding.title.contains("3.4 s"));
}

fn time_out(events: &mut Vec<adscan_model::TrackerEvent>, bidder: &str, codes: &[&str]) {
    events.retain(|event| match &event.kind {
        Ev::BidResponse {
            bidder: b,
            ad_unit_code,
            ..
        }
        | Ev::NoBid {
            bidder: b,
            ad_unit_code,
            ..
        } => !(b == bidder && codes.contains(&ad_unit_code.as_str())),
        Ev::BidWon { bidder: b, .. } => b != bidder,
        Ev::AdRenderSucceeded { bidder: b, .. } => b.as_deref() != Some(bidder),
        _ => true,
    });
    for code in codes {
        events.push(ev(
            699.0,
            Ev::BidTimeout {
                auction_id: "auction-1".into(),
                bidder: bidder.into(),
                ad_unit_code: code.to_string(),
            },
        ));
    }
}

#[test]
fn single_bidder_timing_out() {
    let capture = healthy_with(|events| time_out(events, "appnexus", &["top", "side"]));
    let finding = only(&capture, "bidder-timeouts");
    assert_eq!(finding.affected_bidders, ["appnexus"]);
    assert_eq!(
        finding.title,
        "Bidder appnexus timed out on 2 of 3 requests"
    );
    assert!(
        finding.evidence[0]
            .body
            .contains("3 requests, 2 timeouts, 1 bids")
    );
}

#[test]
fn a_single_timeout_is_not_reported() {
    let capture = healthy_with(|events| time_out(events, "appnexus", &["side"]));
    assert_eq!(fired(&capture), Vec::<&str>::new());
}

#[test]
fn timeouts_across_bidders_point_at_the_auction_timeout() {
    let capture = healthy_with(|events| {
        time_out(events, "appnexus", &["top", "side"]);
        time_out(events, "rubicon", &["footer"]);
    });
    let finding = only(&capture, "auction-timeout-too-short");
    assert_eq!(finding.severity, Severity::High);
    assert_eq!(finding.affected_bidders, ["appnexus", "rubicon"]);
    assert_eq!(
        finding.title,
        "Auction timeout of 1.5 s cuts off 50% of bid requests"
    );
    assert_eq!(finding.estimated_revenue_impact_pct, Some(15.0));
    assert!(
        finding
            .evidence
            .iter()
            .any(|evidence| evidence.label.contains("Latency"))
    );
}

#[test]
fn ad_request_sent_before_the_auction_ended() {
    let capture = healthy_with(|events| {
        for event in events.iter_mut() {
            if let Ev::SlotRequested {
                element_id,
                prebid_targeting_keys,
                ..
            } = &mut event.kind
                && element_id == "side"
            {
                event.t = 420.0;
                prebid_targeting_keys.clear();
            }
        }
    });
    let finding = only(&capture, "ad-request-before-auction-end");
    assert_eq!(finding.severity, Severity::High);
    assert_eq!(finding.affected_slots, ["side"]);
    assert!(
        finding.evidence[0]
            .body
            .contains("ad request at 420 ms, auction ended 700 ms")
    );
    assert_eq!(finding.estimated_revenue_impact_pct, Some(10.0));
}

#[test]
fn slots_defined_but_never_requested() {
    let capture = healthy_with(|events| {
        events.retain(|event| match &event.kind {
            Ev::SlotRequested { element_id, .. }
            | Ev::SlotResponseReceived { element_id }
            | Ev::SlotRenderEnded { element_id, .. } => element_id != "side",
            _ => true,
        });
    });
    let finding = only(&capture, "slots-never-requested");
    assert_eq!(finding.severity, Severity::High);
    assert_eq!(finding.category, FindingCategory::Fill);
    assert_eq!(finding.affected_slots, ["side"]);
}

#[test]
fn slots_requested_but_empty() {
    let capture = healthy_with(|events| {
        for event in events.iter_mut() {
            if let Ev::SlotRenderEnded {
                element_id,
                is_empty,
                ..
            } = &mut event.kind
                && element_id != "top"
            {
                *is_empty = true;
            }
        }
    });
    let finding = only(&capture, "empty-slots");
    assert_eq!(finding.severity, Severity::High);
    assert_eq!(finding.title, "2 of 3 requested slots came back empty");
    assert_eq!(finding.affected_slots, ["side", "footer"]);
}

#[test]
fn prebid_ad_unit_without_a_gpt_slot() {
    let capture = healthy_with(|events| {
        for event in events.iter_mut() {
            if let Ev::AuctionInit { ad_units, .. } = &mut event.kind {
                ad_units.push(TrackedAdUnit {
                    code: "sticky".into(),
                    sizes: strings(&["320x50"]),
                    bidders: strings(&["rubicon"]),
                    bids: vec![],
                });
            }
        }
        events.push(bid_response(600.0, "auction-1", "rubicon", "sticky", 0.9));
    });
    let finding = only(&capture, "ad-units-without-slot");
    assert_eq!(finding.category, FindingCategory::Configuration);
    assert_eq!(finding.affected_slots, ["sticky"]);
}

#[test]
fn gpt_slot_outside_header_bidding() {
    let capture = healthy_with(|events| {
        events.push(slot_defined(210.0, "sidebar-2", true));
        events.push(slot_requested(710.0, "sidebar-2", false));
        events.push(slot_rendered(900.0, "sidebar-2", false));
    });
    let finding = only(&capture, "slots-outside-header-bidding");
    assert_eq!(finding.affected_slots, ["sidebar-2"]);
    // 1 of 4 slots at the assumed 30% header bidding share is 7.5, reported as a whole percentage.
    assert_eq!(finding.estimated_revenue_impact_pct, Some(8.0));
    assert!(finding.evidence.last().unwrap().body.contains(
        "the header bidding revenue of 1 of 4 slots, taking every slot to earn the same; assumes header bidding \
         brings in about 30% of ad revenue, a planning figure that was not measured"
    ));
}

#[test]
fn winning_bid_that_failed_to_render() {
    let capture = healthy_with(|events| {
        events.retain(|event| !matches!(event.kind, Ev::AdRenderSucceeded { .. }));
        events.push(ev(
            920.0,
            Ev::AdRenderFailed {
                ad_unit_code: Some("top".into()),
                bidder: Some("appnexus".into()),
                reason: Some("cannotFindAd".into()),
            },
        ));
    });
    let finding = only(&capture, "won-bid-not-rendered");
    assert_eq!(finding.severity, Severity::High);
    assert_eq!(
        (finding.affected_slots, finding.affected_bidders),
        (strings(&["top"]), strings(&["appnexus"]))
    );
    assert!(finding.evidence[0].body.contains("cannotFindAd"));
}

#[test]
fn bidder_that_never_bids() {
    let capture = healthy_with(|events| {
        events.retain(|event| {
            !matches!(&event.kind, Ev::BidResponse { bidder, .. } | Ev::BidWon { bidder, .. } if bidder == "appnexus")
                && !matches!(event.kind, Ev::AdRenderSucceeded { .. })
        });
        for code in ["top", "footer"] {
            events.push(ev(
                660.0,
                Ev::NoBid {
                    auction_id: "auction-1".into(),
                    bidder: "appnexus".into(),
                    ad_unit_code: code.into(),
                },
            ));
        }
    });
    let finding = only(&capture, "bidders-never-bid");
    assert_eq!(finding.severity, Severity::Low);
    assert_eq!(finding.affected_bidders, ["appnexus"]);
    assert!(finding.confidence <= 0.5);
}

fn with_config(has_floors: bool, has_consent_management: bool) -> Capture {
    healthy_with(|events| {
        events.retain(|event| !matches!(event.kind, Ev::PrebidConfig { .. }));
        events.push(prebid_config(350.0, has_floors, has_consent_management));
    })
}

#[test]
fn no_price_floors_configured() {
    let finding = only(&with_config(false, true), "no-price-floors");
    assert_eq!(
        (finding.severity, finding.category),
        (Severity::Low, FindingCategory::Pricing)
    );
}

#[test]
fn prebid_loaded_twice_or_in_two_versions() {
    let second_global =
        healthy_with(|events| events.push(prebid_detected(180.0, "pbjs2", "9.20.0")));
    assert_eq!(
        only(&second_global, "duplicate-prebid").category,
        FindingCategory::Hygiene
    );

    let two_versions = healthy_with(|events| events.push(prebid_detected(180.0, "pbjs", "8.52.0")));
    assert!(
        only(&two_versions, "duplicate-prebid")
            .description
            .contains("8.52.0, 9.20.0")
    );

    let mut two_builds = healthy_capture(DeviceProfile::Desktop);
    two_builds.scripts.push(script(
        "https://widgets.example.net/prebid.js",
        "var auctionManager={};function registerBidAdapter(){}/* other build */",
    ));
    assert!(
        only(&two_builds, "duplicate-prebid").evidence[0]
            .body
            .contains("widgets.example.net")
    );
}

#[test]
fn ad_library_fetched_from_two_urls() {
    let mut capture = healthy_capture(DeviceProfile::Desktop);
    capture.scripts.push(script(
        "https://securepubads.g.doubleclick.net/tag/js/gpt.js?v=wrapper-4",
        "/* gpt */",
    ));
    let finding = only(&capture, "ad-library-loaded-twice");
    assert_eq!(
        (finding.severity, finding.category),
        (Severity::Low, FindingCategory::Hygiene)
    );
    assert!(
        finding.evidence[0]
            .body
            .contains("gpt.js\nhttps://securepubads.g.doubleclick.net/tag/js/gpt.js?v=wrapper-4")
    );
}

#[test]
fn very_old_prebid_version() {
    let capture = healthy_with(|events| {
        events.retain(|event| !matches!(event.kind, Ev::PrebidDetected { .. }));
        events.push(prebid_detected(120.0, "pbjs", "v6.29.0"));
    });
    let finding = only(&capture, "outdated-prebid");
    assert_eq!(
        finding.title,
        "Prebid.js v6.29.0 is several major versions old"
    );
}

#[test]
fn cmp_present_but_prebid_consent_management_missing() {
    let mut capture = with_config(true, false);
    assert_eq!(
        fired(&capture),
        Vec::<&str>::new(),
        "without a CMP there is nothing to wire"
    );
    capture.scripts.push(script(
        "https://sdk.privacy-center.org/loader.js",
        "/* didomi */",
    ));
    let finding = only(&capture, "consent-not-wired");
    assert_eq!(finding.category, FindingCategory::Consent);
    assert!(finding.evidence[0].body.contains("Didomi"));
}

#[test]
fn consent_timeout_logged_by_prebid() {
    let capture = healthy_with(|events| {
        events.push(ev(
            340.0,
            Ev::PrebidLog {
                level: "warning".into(),
                message: "consentManagement: CMP workflow exceeded timeout threshold. Canceling auction as per consentManagement config.".into(),
            },
        ));
        events.push(ev(
            341.0,
            Ev::PrebidLog {
                level: "info".into(),
                message: "Bid timeout is 1500".into(),
            },
        ));
    });
    let finding = only(&capture, "consent-timeout");
    assert_eq!(finding.severity, Severity::High);
    assert_eq!(finding.evidence.len(), 1);
}

#[test]
fn slow_gpt_load() {
    let capture = healthy_with(|events| {
        for event in events.iter_mut() {
            if matches!(event.kind, Ev::GptDetected { .. }) {
                event.t = 3400.0;
            }
        }
    });
    let finding = only(&capture, "slow-gpt-load");
    assert_eq!(finding.severity, Severity::Low);
}
