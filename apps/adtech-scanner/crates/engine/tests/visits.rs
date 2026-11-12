//! Visits of more than one page view: how telemetry is derived from them,
//! how findings of several page views become one, and the rules that only a
//! multi-page visit, a refreshed slot or the visit's environment can trigger.

mod common;

use adscan_engine::domain::finding::FindingDraft;
use adscan_engine::domain::heuristics::{self, RuleInput};
use adscan_engine::domain::telemetry::{self, ScanTelemetry};
use adscan_engine::domain::{environment, scripts};
use adscan_model::{
    AutomationDetector, BidStatus, Capture, CaptureEnvironment, DetectorKind, EvidenceKind,
    PageReach, ProxyKind, ProxyUse, ScanEnvironment, Severity, SlotOutcome, TrackedAdUnit,
    TrackerEvent, TrackerEventKind as Ev, VisitorBehavior,
};
use common::*;

const ARTICLE: &str = "https://pub.example/";
const SECOND: &str = "https://pub.example/sport/final";

fn derived(capture: &Capture) -> ScanTelemetry {
    telemetry::derive(capture)
}

fn evaluate_in(
    capture: &Capture,
    environment: Option<&ScanEnvironment>,
) -> Vec<(&'static str, FindingDraft)> {
    let telemetry = derived(capture);
    let records: Vec<_> = scripts::describe(capture)
        .into_iter()
        .map(|script| script.record)
        .collect();
    heuristics::evaluate(&RuleInput {
        capture,
        telemetry: &telemetry,
        diagnostics: None,
        scripts: &records,
        environment,
    })
    .into_iter()
    .map(|hit| (hit.rule, hit.finding))
    .collect()
}

fn evaluate(capture: &Capture) -> Vec<(&'static str, FindingDraft)> {
    evaluate_in(capture, None)
}

fn fired(capture: &Capture) -> Vec<&'static str> {
    evaluate(capture)
        .into_iter()
        .map(|(rule, _)| rule)
        .collect()
}

fn only(capture: &Capture, rule: &str) -> FindingDraft {
    let mut matches: Vec<FindingDraft> = evaluate(capture)
        .into_iter()
        .filter(|(id, _)| *id == rule)
        .map(|(_, finding)| finding)
        .collect();
    assert_eq!(
        matches.len(),
        1,
        "expected `{rule}` once in {:?}",
        fired(capture)
    );
    matches.remove(0)
}

fn healthy_page() -> Vec<TrackerEvent> {
    let mut events = page_setup();
    events.extend(auction_round(360.0, "auction-1"));
    events.push(lifecycle(950.0, "load"));
    events.extend(scroll_events(4000.0));
    events
}

fn entry(ended_at_ms: f64) -> adscan_model::CapturedPage {
    page_view(ARTICLE, PageReach::Entry, 0.0, ended_at_ms)
}

fn requested_with(t: f64, id: &str, hb_ad_id: Option<&str>) -> TrackerEvent {
    ev(
        t,
        Ev::SlotRequested {
            element_id: id.into(),
            prebid_targeting_keys: if hb_ad_id.is_some() {
                strings(&["hb_pb", "hb_adid"])
            } else {
                vec![]
            },
            prebid_ad_id: hb_ad_id.map(str::to_string),
        },
    )
}

fn tcf(t: f64, status: &str) -> TrackerEvent {
    ev(
        t,
        Ev::TcfState {
            event_status: Some(status.into()),
            gdpr_applies: Some(true),
            has_tc_string: status != "cmpuishown",
            purpose_consents: vec![],
            vendor_consents: vec![],
        },
    )
}

/// One auction for a single ad unit that is not one of the page's slots.
fn side_auction(start: f64, auction: &str, code: &str) -> Vec<TrackerEvent> {
    vec![
        ev(
            start,
            Ev::AuctionInit {
                auction_id: auction.into(),
                timeout: Some(1500.0),
                ad_units: vec![TrackedAdUnit {
                    code: code.into(),
                    sizes: strings(&["300x250"]),
                    bidders: strings(&["rubicon"]),
                    bids: vec![],
                }],
            },
        ),
        bid_response(start + 200.0, auction, "rubicon", code, 0.9),
        ev(
            start + 300.0,
            Ev::AuctionEnd {
                auction_id: auction.into(),
            },
        ),
    ]
}

// -- telemetry ---------------------------------------------------------------

#[test]
fn a_single_page_visit_is_one_page_view_that_holds_everything() {
    let capture = healthy_capture(adscan_model::DeviceProfile::Desktop);
    let telemetry = derived(&capture);

    assert_eq!(telemetry.pages.len(), 1);
    let page = &telemetry.pages[0];
    assert_eq!((page.index, page.reached_by), (0, PageReach::Entry));
    assert_eq!(page.started_at_ms, 0.0);
    assert_eq!(page.dwell_ms, 12_000.0);
    assert_eq!(page.load_ms, Some(950.0));
    assert_eq!(telemetry.by_page.len(), 1);
    // The page view and the scan are the same thing.
    assert_eq!(telemetry.by_page[0], telemetry.total);
    assert_eq!(
        page.metrics.auction_count,
        telemetry.total.metrics.auction_count
    );
    assert_eq!(page.metrics.slot_count, 3);
    assert_eq!(page.metrics.time_to_first_auction_ms, Some(360.0));
    assert!(
        telemetry
            .total
            .auctions
            .iter()
            .all(|auction| auction.page == 0)
    );
    assert!(telemetry.total.slots.iter().all(|slot| slot.page == 0));
}

#[test]
fn a_followed_link_starts_a_page_view_with_its_own_clock_and_slots() {
    let mut second = page_setup();
    second.extend(auction_round(500.0, "auction-2"));
    let capture = visit(vec![
        (entry(12_000.0), healthy_page()),
        (
            page_view(SECOND, PageReach::LinkClick, 12_400.0, 20_400.0),
            second,
        ),
    ]);
    let telemetry = derived(&capture);

    assert_eq!(telemetry.pages.len(), 2);
    assert_eq!(telemetry.pages[1].url, SECOND);
    assert_eq!(telemetry.pages[1].reached_by, PageReach::LinkClick);
    assert_eq!(telemetry.pages[1].started_at_ms, 12_400.0);
    assert_eq!(telemetry.pages[1].dwell_ms, 8_000.0);

    // The same containers on two page views are two sets of slots.
    assert_eq!(telemetry.total.slots.len(), 6);
    let on_second: Vec<(&str, Option<f64>)> = telemetry.by_page[1]
        .slots
        .iter()
        .map(|slot| (slot.element_id.as_str(), slot.requested_at_ms))
        .collect();
    assert_eq!(
        on_second,
        [
            ("top", Some(850.0)),
            ("side", Some(850.0)),
            ("footer", Some(850.0))
        ]
    );

    // Times count from the start of the page view, not of the scan.
    let auction = &telemetry.by_page[1].auctions[0];
    assert_eq!((auction.page, auction.started_at_ms), (1, 500.0));
    assert_eq!(auction.ended_at_ms, Some(840.0));
    assert_eq!(telemetry.absolute_ms(1, auction.started_at_ms), 12_900.0);

    // Counts are sums; load and time-to-first figures are the entry page's.
    let metrics = &telemetry.total.metrics;
    assert_eq!(metrics.auction_count, 2);
    assert_eq!(metrics.slot_count, 6);
    assert_eq!(metrics.page_load_ms, Some(950.0));
    assert_eq!(metrics.time_to_first_auction_ms, Some(360.0));
    assert_eq!(
        telemetry.pages[1].metrics.time_to_first_auction_ms,
        Some(500.0)
    );
    assert_eq!(telemetry.pages[1].metrics.auction_count, 1);
    assert_eq!(telemetry.pages[1].metrics.fill_rate, 1.0);
}

#[test]
fn an_auction_that_outlives_a_route_change_stays_one_auction_of_its_first_page() {
    let mut first = healthy_page();
    first.push(ev(
        9_000.0,
        Ev::AuctionInit {
            auction_id: "auction-late".into(),
            timeout: Some(1500.0),
            ad_units: vec![TrackedAdUnit {
                code: "top".into(),
                sizes: strings(&["300x250"]),
                bidders: strings(&["rubicon"]),
                bids: vec![],
            }],
        },
    ));
    first.push(ev(
        9_005.0,
        Ev::BidRequested {
            auction_id: "auction-late".into(),
            bidder: "rubicon".into(),
            ad_unit_codes: strings(&["top"]),
            gdpr_applies: None,
            has_consent_string: None,
            schain: vec![],
        },
    ));
    // The route changes 9.1 s into the scan; the answers arrive after it.
    let after_route_change = vec![
        bid_response(100.0, "auction-late", "rubicon", "top", 2.0),
        ev(
            200.0,
            Ev::AuctionEnd {
                auction_id: "auction-late".into(),
            },
        ),
    ];
    let capture = visit(vec![
        (entry(9_100.0), first),
        (
            page_view(SECOND, PageReach::RouteChange, 9_100.0, 15_000.0),
            after_route_change,
        ),
    ]);
    let telemetry = derived(&capture);

    assert!(telemetry.by_page[1].auctions.is_empty());
    let auction = telemetry
        .total
        .auctions
        .iter()
        .find(|auction| auction.id == "auction-late")
        .unwrap();
    assert_eq!(auction.page, 0);
    assert_eq!(auction.started_at_ms, 9_000.0);
    // It ended 200 ms into the second page view: 9.3 s into its own.
    assert_eq!(auction.ended_at_ms, Some(9_300.0));
    assert_eq!(auction.duration_ms, Some(300.0));
    let bid = &auction.ad_units[0].bids[0];
    assert_eq!(bid.status, BidStatus::Bid);
    assert_eq!(bid.requested_at_ms, Some(9_005.0));
    assert_eq!(bid.response_time_ms, Some(195.0));
    assert_eq!(telemetry.pages[1].load_ms, None);
    assert_eq!(telemetry.document_of(1), 0);
}

#[test]
fn after_a_route_change_requests_open_slots_of_the_new_page_and_answers_go_to_the_old() {
    let mut first = page_setup();
    first.push(slot_requested(9_000.0, "side", false));
    let after_route_change = vec![
        // The answer to the request made just before the route change.
        slot_rendered(50.0, "side", false),
        // A new request for a container the page did not define again.
        slot_requested(400.0, "top", false),
    ];
    let capture = visit(vec![
        (entry(9_100.0), first),
        (
            page_view(SECOND, PageReach::RouteChange, 9_100.0, 15_000.0),
            after_route_change,
        ),
    ]);
    let telemetry = derived(&capture);

    let side = &telemetry.by_page[0].slots[1];
    assert_eq!(side.element_id, "side");
    assert_eq!(side.outcome, SlotOutcome::Filled);
    assert_eq!(side.rendered_at_ms, Some(9_150.0));

    assert_eq!(telemetry.by_page[1].slots.len(), 1);
    let top = &telemetry.by_page[1].slots[0];
    assert_eq!((top.page, top.element_id.as_str()), (1, "top"));
    // What is known about the container is carried over; it was not defined
    // on this page view.
    assert_eq!(top.ad_unit_path.as_deref(), Some("/123/top"));
    assert_eq!(top.defined_at_ms, None);
    assert_eq!(top.requested_at_ms, Some(400.0));
    assert_eq!(top.outcome, SlotOutcome::Pending);
}

#[test]
fn a_new_document_does_not_inherit_the_auctions_of_the_page_before() {
    // The second page requests its slots with no auction of its own.
    let mut second = page_setup();
    second.push(slot_requested(500.0, "top", false));
    let capture = visit(vec![
        (entry(12_000.0), healthy_page()),
        (
            page_view(SECOND, PageReach::LinkClick, 12_400.0, 20_400.0),
            second,
        ),
    ]);
    let telemetry = derived(&capture);
    let top = &telemetry.by_page[1].slots[0];
    assert_eq!(top.requested_before_auction_end, None);
    assert_eq!(
        telemetry.by_page[0].slots[0].requested_before_auction_end,
        Some(false)
    );
}

#[test]
fn every_ad_request_of_a_slot_is_kept_in_order() {
    let mut events = healthy_page();
    events.push(requested_with(6_000.0, "side", Some("bid-7")));
    events.push(requested_with(9_000.0, "side", None));
    let capture = visit(vec![(entry(12_000.0), events)]);
    let telemetry = derived(&capture);
    let side = telemetry
        .total
        .slots
        .iter()
        .find(|slot| slot.element_id == "side")
        .unwrap();

    // The first request still is the slot's request.
    assert_eq!(side.requested_at_ms, Some(710.0));
    assert_eq!(side.had_prebid_targeting, Some(true));
    let requests: Vec<(f64, bool, Option<&str>)> = side
        .requests
        .iter()
        .map(|request| {
            (
                request.at_ms,
                request.had_prebid_targeting,
                request.prebid_ad_id.as_deref(),
            )
        })
        .collect();
    assert_eq!(
        requests,
        [
            (710.0, true, None),
            (6_000.0, true, Some("bid-7")),
            (9_000.0, false, None)
        ]
    );
}

// -- merging findings of several page views ------------------------------------

fn scroll_gated_page() -> Vec<TrackerEvent> {
    let mut events = page_setup();
    events.push(lifecycle(950.0, "load"));
    events.extend(scroll_events(4000.0));
    events.extend(auction_round(4060.0, "auction-1"));
    events
}

#[test]
fn the_same_finding_on_two_page_views_is_one_finding_that_names_both() {
    let capture = visit(vec![
        (entry(12_000.0), scroll_gated_page()),
        (
            page_view(SECOND, PageReach::LinkClick, 12_400.0, 24_400.0),
            scroll_gated_page(),
        ),
    ]);
    let finding = only(&capture, "auction-gated-on-interaction");

    assert_eq!(finding.pages, [0, 1]);
    assert_eq!(finding.severity, Severity::High);
    assert_eq!(finding.affected_slots, ["top", "side"]);
    let per_page = finding
        .evidence
        .iter()
        .find(|evidence| evidence.label == "Per page view")
        .expect("a merged finding says what it saw on each page view");
    assert_eq!(per_page.kind, EvidenceKind::Timeline);
    assert_eq!(
        per_page.body,
        "page 1 (/): Bidding waits for the first scroll on desktop\n\
         page 2 (/sport/final): Bidding waits for the first scroll on desktop"
    );
}

#[test]
fn a_finding_of_one_page_view_names_that_page_view_only() {
    let capture = visit(vec![
        (entry(12_000.0), healthy_page()),
        (
            page_view(SECOND, PageReach::LinkClick, 12_400.0, 24_400.0),
            scroll_gated_page(),
        ),
    ]);
    let finding = only(&capture, "auction-gated-on-interaction");
    assert_eq!(finding.pages, [1]);
    assert!(
        finding
            .evidence
            .iter()
            .all(|evidence| evidence.label != "Per page view")
    );

    // A single-page visit: the one page view, and nothing about pages.
    let single = scroll_gated_capture(adscan_model::DeviceProfile::Desktop);
    let finding = only(&single, "auction-gated-on-interaction");
    assert_eq!(finding.pages, [0]);
}

#[test]
fn a_route_change_page_view_is_not_judged_by_page_load_rules() {
    // An auction four seconds after an in-app navigation is neither a late
    // first auction nor a scroll gate: nothing was loading.
    let mut after_route_change = scroll_events(3_000.0);
    after_route_change.extend(auction_round(4_000.0, "auction-2"));
    let capture = visit(vec![
        (entry(12_000.0), healthy_page()),
        (
            page_view(SECOND, PageReach::RouteChange, 12_000.0, 24_000.0),
            after_route_change,
        ),
    ]);
    assert_eq!(fired(&capture), Vec::<&str>::new());
}

// -- R1: later page views without bids -------------------------------------------

/// The second page view of a wrapper that only bids on the first page of a
/// session: the standard slots go to the ad server bare, one new ad unit is
/// auctioned.
fn unbid_second_page() -> Vec<TrackerEvent> {
    let mut events = page_setup();
    events.extend(side_auction(300.0, "auction-recirc", "recirc"));
    for slot in ["top", "side", "footer"] {
        events.push(slot_requested(600.0, slot, false));
        events.push(slot_rendered(900.0, slot, true));
    }
    events
}

fn two_pages(second: adscan_model::CapturedPage, events: Vec<TrackerEvent>) -> Capture {
    visit(vec![(entry(12_000.0), healthy_page()), (second, events)])
}

fn link_click(dwell_ms: f64) -> adscan_model::CapturedPage {
    page_view(SECOND, PageReach::LinkClick, 12_400.0, 12_400.0 + dwell_ms)
}

#[test]
fn slots_that_lose_their_bids_on_a_later_page_view_are_reported_once() {
    let capture = two_pages(link_click(8_000.0), unbid_second_page());
    let finding = only(&capture, "later-page-view-without-bids");

    assert_eq!(finding.severity, Severity::High);
    assert_eq!(finding.confidence, 0.8);
    assert_eq!(finding.pages, [1]);
    assert_eq!(finding.affected_slots, ["footer", "side", "top"]);
    assert!(finding.description.contains("page 2 (/sport/final)"));
    assert!(finding.description.contains("3 ad slot(s)"));
    // The slots do have a Prebid ad unit, so the "no ad unit" rule stays out
    // of it; empty slots are a different matter and still reported.
    let rules = fired(&capture);
    assert!(
        !rules.contains(&"slots-outside-header-bidding"),
        "{rules:?}"
    );
    assert!(
        !rules.contains(&"auction-gated-on-interaction"),
        "{rules:?}"
    );
}

#[test]
fn a_page_view_without_any_auction_is_not_also_called_a_scroll_gate() {
    let mut events = page_setup();
    for slot in ["top", "side", "footer"] {
        events.push(slot_requested(600.0, slot, false));
    }
    let capture = two_pages(link_click(8_000.0), events);
    let rules = fired(&capture);
    assert!(rules.contains(&"later-page-view-without-bids"), "{rules:?}");
    assert!(
        !rules.contains(&"auction-gated-on-interaction"),
        "{rules:?}"
    );
}

#[test]
fn later_page_views_are_not_blamed_without_ground() {
    let rule = "later-page-view-without-bids";
    let too_short = two_pages(link_click(3_000.0), unbid_second_page());
    assert!(!fired(&too_short).contains(&rule), "a 3 s page view");

    let route_change = two_pages(
        page_view(SECOND, PageReach::RouteChange, 12_400.0, 20_400.0),
        unbid_second_page(),
    );
    assert!(!fired(&route_change).contains(&rule), "no new document");

    let mut undecided = unbid_second_page();
    undecided.push(tcf(100.0, "cmpuishown"));
    let waiting_for_consent = two_pages(link_click(8_000.0), undecided);
    assert!(
        !fired(&waiting_for_consent).contains(&rule),
        "the consent prompt is open"
    );

    let mut one_slot = page_setup();
    one_slot.push(slot_requested(600.0, "top", false));
    assert!(
        !fired(&two_pages(link_click(8_000.0), one_slot)).contains(&rule),
        "a single slot"
    );

    let mut other_slots = page_setup();
    for slot in ["gallery-1", "gallery-2"] {
        other_slots.push(slot_requested(600.0, slot, false));
    }
    assert!(
        !fired(&two_pages(link_click(8_000.0), other_slots)).contains(&rule),
        "slots the first page never auctioned"
    );

    // Bids on both pages: nothing to report at all.
    let mut bidding = page_setup();
    bidding.extend(auction_round(360.0, "auction-2"));
    assert_eq!(
        fired(&two_pages(link_click(8_000.0), bidding)),
        Vec::<&str>::new()
    );
}

// -- R2: slots not reset on a route change ----------------------------------------

fn route_change(dwell_ms: f64) -> adscan_model::CapturedPage {
    page_view(
        SECOND,
        PageReach::RouteChange,
        12_000.0,
        12_000.0 + dwell_ms,
    )
}

#[test]
fn rejected_slot_definitions_after_a_route_change_are_a_high_finding() {
    let mut events = side_auction(300.0, "auction-2", "top");
    for slot in ["top", "side"] {
        events.push(ev(
            250.0,
            Ev::SlotDefinitionRejected {
                element_id: slot.into(),
            },
        ));
    }
    let capture = two_pages(route_change(2_000.0), events);
    let finding = only(&capture, "spa-slots-not-reset");
    assert_eq!(
        (finding.severity, finding.confidence),
        (Severity::High, 0.85)
    );
    assert_eq!(finding.pages, [1]);
    assert_eq!(finding.affected_slots, ["side", "top"]);
    assert!(finding.description.contains("rejected the new definitions"));
}

#[test]
fn slots_left_untouched_through_a_route_change_are_a_medium_finding() {
    let capture = two_pages(
        route_change(8_000.0),
        side_auction(300.0, "auction-2", "top"),
    );
    let finding = only(&capture, "spa-slots-not-reset");
    assert_eq!(
        (finding.severity, finding.confidence),
        (Severity::Medium, 0.6)
    );
    assert_eq!(finding.affected_slots, ["footer", "side", "top"]);
}

#[test]
fn a_route_change_that_handles_its_slots_is_fine() {
    let rule = "spa-slots-not-reset";
    let auction = || side_auction(300.0, "auction-2", "top");

    let mut refreshed = auction();
    refreshed.push(slot_requested(700.0, "top", true));
    assert!(!fired(&two_pages(route_change(8_000.0), refreshed)).contains(&rule));

    let mut destroyed = auction();
    destroyed.push(ev(
        100.0,
        Ev::SlotsDestroyed {
            element_ids: strings(&["top", "side", "footer"]),
        },
    ));
    assert!(!fired(&two_pages(route_change(8_000.0), destroyed)).contains(&rule));

    // Too short to tell, no auction to waste, or not a new path at all.
    assert!(!fired(&two_pages(route_change(3_000.0), auction())).contains(&rule));
    assert!(!fired(&two_pages(route_change(8_000.0), Vec::new())).contains(&rule));
    let same_path = page_view(
        "https://pub.example/#comments",
        PageReach::RouteChange,
        12_000.0,
        20_000.0,
    );
    assert!(!fired(&two_pages(same_path, auction())).contains(&rule));
}

// -- R3 and R4: refreshes ------------------------------------------------------------

fn refreshed(first: Option<&str>, second: Option<&str>, auction_between: bool) -> Capture {
    let mut events = page_setup();
    events.extend(side_auction(360.0, "auction-1", "side"));
    events.push(requested_with(700.0, "side", first));
    events.push(slot_rendered(900.0, "side", false));
    if auction_between {
        events.extend(side_auction(5_000.0, "auction-2", "side"));
    }
    events.push(requested_with(8_000.0, "side", second));
    visit(vec![(entry(12_000.0), events)])
}

#[test]
fn a_refresh_with_the_bid_of_the_request_before_is_reported() {
    let capture = refreshed(Some("bid-1"), Some("bid-1"), false);
    let finding = only(&capture, "refresh-with-stale-bids");
    assert_eq!(
        (finding.severity, finding.confidence),
        (Severity::High, 0.9)
    );
    assert_eq!(finding.affected_slots, ["side"]);
    assert_eq!(finding.pages, [0]);
    assert!(finding.evidence[0].body.contains("same hb_adid bid-1"));
    assert!(!fired(&capture).contains(&"refresh-without-auction"));
}

#[test]
fn a_refresh_with_new_bids_is_not_stale() {
    let rule = "refresh-with-stale-bids";
    assert!(!fired(&refreshed(Some("bid-1"), Some("bid-2"), true)).contains(&rule));
    // The same id after a new auction: the bid was offered again, which is
    // Prebid's business.
    assert!(!fired(&refreshed(Some("bid-1"), Some("bid-1"), true)).contains(&rule));
    // No id on either request is not "the same id".
    assert!(!fired(&refreshed(None, None, false)).contains(&rule));
}

#[test]
fn a_refresh_without_auction_or_bids_is_reported() {
    let capture = refreshed(Some("bid-1"), None, false);
    let finding = only(&capture, "refresh-without-auction");
    assert_eq!(
        (finding.severity, finding.confidence),
        (Severity::Medium, 0.7)
    );
    assert_eq!(finding.affected_slots, ["side"]);
    assert!(!fired(&capture).contains(&"refresh-with-stale-bids"));
}

#[test]
fn a_bare_refresh_is_only_a_finding_where_bids_were_expected() {
    let rule = "refresh-without-auction";
    // The slot never carried header bids: its refresh loses nothing.
    assert!(!fired(&refreshed(None, None, false)).contains(&rule));
    // An auction ran for the refresh; that its keys are missing is the
    // "requested before the auction ended" family, not this rule.
    assert!(!fired(&refreshed(Some("bid-1"), None, true)).contains(&rule));
}

// -- R5: consent asked again -------------------------------------------------------------

fn consent_visit(decided_on_entry: bool, second_page_auction_at: Option<f64>) -> Capture {
    let mut first = healthy_page();
    first.push(tcf(100.0, "cmpuishown"));
    if decided_on_entry {
        first.push(tcf(200.0, "useractioncomplete"));
    }
    let mut second = page_setup();
    second.push(tcf(150.0, "cmpuishown"));
    if let Some(at) = second_page_auction_at {
        second.extend(auction_round(at, "auction-2"));
    }
    visit(vec![
        (entry(12_000.0), first),
        (link_click(8_000.0), second),
    ])
}

#[test]
fn a_consent_prompt_that_returns_after_the_choice_is_reported() {
    let held_up = consent_visit(true, Some(4_000.0));
    let finding = only(&held_up, "consent-asked-again");
    assert_eq!(finding.severity, Severity::High);
    assert_eq!(finding.pages, [1]);
    assert!(
        finding
            .description
            .contains("bidding started more than 2.0 s")
    );

    let prompt_only = consent_visit(true, Some(500.0));
    assert_eq!(
        only(&prompt_only, "consent-asked-again").severity,
        Severity::Medium
    );

    // The scanner's own accepted banner is a choice too.
    let mut accepted_by_scanner = consent_visit(false, Some(500.0));
    accepted_by_scanner.consent_accepted = true;
    assert!(fired(&accepted_by_scanner).contains(&"consent-asked-again"));
}

#[test]
fn a_prompt_nobody_answered_may_show_on_every_page_view() {
    let capture = consent_visit(false, Some(500.0));
    assert!(!fired(&capture).contains(&"consent-asked-again"));
}

/// On the second page the consent platform loads the stored choice, and the
/// prompt only appears later: a person opened the privacy settings.
#[test]
fn a_prompt_the_visitor_opened_themselves_is_not_the_choice_being_forgotten() {
    let mut first = healthy_page();
    first.push(tcf(100.0, "cmpuishown"));
    first.push(tcf(200.0, "useractioncomplete"));
    let mut second = page_setup();
    second.push(tcf(150.0, "tcloaded"));
    second.extend(auction_round(500.0, "auction-2"));
    second.push(tcf(6_000.0, "cmpuishown"));
    let capture = visit(vec![
        (entry(12_000.0), first),
        (link_click(8_000.0), second),
    ]);
    assert!(!fired(&capture).contains(&"consent-asked-again"));
}

// -- R6: identifiable as automated ---------------------------------------------------------

fn visit_environment(webdriver: Option<bool>, headless: bool, detector: bool) -> ScanEnvironment {
    ScanEnvironment {
        visitor: VisitorBehavior::Scroll,
        seed: 1,
        headless,
        persistent_profile: false,
        webdriver,
        user_agent_overridden: false,
        viewport_overridden: true,
        emulated_mobile: false,
        detectors: if detector {
            vec![AutomationDetector {
                vendor: "DoubleVerify".into(),
                kind: DetectorKind::InvalidTrafficMeasurement,
                host: "pub.doubleverify.com".into(),
            }]
        } else {
            Vec::new()
        },
        proxy: ProxyUse {
            kind: ProxyKind::System,
            name: None,
            host: None,
        },
    }
}

/// `appnexus` is asked for bids in two auctions and never bids.
fn capture_with_a_silent_bidder() -> Capture {
    let mut events = healthy_page();
    events.extend(auction_round(6_000.0, "auction-2"));
    events.retain(|event| {
        !matches!(&event.kind, Ev::BidResponse { bidder, .. } if bidder == "appnexus")
            && !matches!(
                &event.kind,
                Ev::BidWon { .. } | Ev::AdRenderSucceeded { .. }
            )
    });
    visit(vec![(entry(12_000.0), events)])
}

fn automation_finding(capture: &Capture, environment: &ScanEnvironment) -> Option<FindingDraft> {
    evaluate_in(capture, Some(environment))
        .into_iter()
        .find(|(rule, _)| *rule == heuristics::AUTOMATION_RULE)
        .map(|(_, finding)| finding)
}

#[test]
fn an_automated_visit_with_a_detection_vendor_on_the_page_is_flagged_as_information() {
    let capture = healthy_capture(adscan_model::DeviceProfile::Desktop);
    let finding = automation_finding(&capture, &visit_environment(Some(true), true, true)).unwrap();
    assert_eq!(
        (finding.severity, finding.confidence),
        (Severity::Info, 0.5)
    );
    assert_eq!(
        finding.description,
        "Bidders may be filtering this scan as automated traffic: navigator.webdriver was true, the browser ran \
         headless, DoubleVerify is on the page. Repeat as \"I'll browse myself\" or with the browser shown before \
         concluding a bidder is broken."
    );
    assert!(finding.affected_bidders.is_empty());
}

#[test]
fn an_automated_visit_with_a_bidder_that_never_bid_is_flagged_and_the_bidder_finding_says_so() {
    let capture = capture_with_a_silent_bidder();
    let environment = visit_environment(Some(false), true, false);
    let finding = automation_finding(&capture, &environment).unwrap();
    assert_eq!(finding.affected_bidders, ["appnexus"]);
    assert!(finding.description.contains("the browser ran headless."));

    let never_bid = evaluate_in(&capture, Some(&environment))
        .into_iter()
        .find(|(rule, _)| *rule == "bidders-never-bid")
        .map(|(_, finding)| finding)
        .unwrap();
    let caveat = never_bid
        .evidence
        .iter()
        .find(|evidence| evidence.label == "Caveat: this scan was identifiable as automated")
        .expect("the bidder finding carries the caveat");
    assert!(caveat.body.starts_with("the browser ran headless."));

    // Without a recorded environment nothing is claimed.
    let unqualified = evaluate(&capture)
        .into_iter()
        .find(|(rule, _)| *rule == "bidders-never-bid")
        .map(|(_, finding)| finding)
        .unwrap();
    assert!(
        unqualified
            .evidence
            .iter()
            .all(|evidence| !evidence.label.starts_with("Caveat"))
    );
}

#[test]
fn a_visit_is_not_flagged_without_both_an_indicator_and_a_reason() {
    let healthy = healthy_capture(adscan_model::DeviceProfile::Desktop);
    // A person's browser: nothing to tell it apart, whatever is on the page.
    assert!(automation_finding(&healthy, &visit_environment(Some(false), false, true)).is_none());
    assert!(automation_finding(&healthy, &visit_environment(None, false, true)).is_none());
    // Identifiable, but nobody on the page cares and every bidder bid.
    assert!(automation_finding(&healthy, &visit_environment(Some(true), true, false)).is_none());
    assert!(!fired(&healthy).contains(&heuristics::AUTOMATION_RULE));
}

#[test]
fn the_environment_of_a_capture_feeds_the_rule() {
    let mut capture = healthy_capture(adscan_model::DeviceProfile::Desktop);
    capture.environment = CaptureEnvironment {
        headless: true,
        webdriver: Some(true),
        request_hosts: strings(&["cdn.adsafeprotected.com"]),
        ..CaptureEnvironment::default()
    };
    let proxy = ProxyUse {
        kind: ProxyKind::None,
        name: None,
        host: None,
    };
    let environment = environment::detect(&capture, VisitorBehavior::Scroll, 5, proxy);
    let finding = automation_finding(&capture, &environment).unwrap();
    assert!(
        finding
            .description
            .contains("Integral Ad Science is on the page")
    );
}
