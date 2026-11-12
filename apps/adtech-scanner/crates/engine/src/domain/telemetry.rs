//! Derives page views, auctions, ad slots and metrics from the raw tracker
//! events.
//!
//! A visit is one or more page views. Every event carries the page view it
//! belongs to and a time since that page view started. Derivation works on
//! one timeline in absolute time (ms since the scan's navigation start), so
//! an auction that outlives a route change stays one auction, and converts
//! back to page-relative times at the end: a single-page scan reads exactly
//! as it did before page views existed.

use std::collections::BTreeSet;

use adscan_model::{
    AdSlot, Auction, AuctionAdUnit, Bid, BidStatus, Capture, CapturedPage, PageMetrics, PageReach,
    PageView, ScanMetrics, SlotOutcome, SlotRequest, TrackerEvent, TrackerEventKind as Ev,
};

/// Auctions, slots and metrics of one scope: one page view, or the whole scan.
#[derive(Debug, Clone, PartialEq)]
pub struct Telemetry {
    pub auctions: Vec<Auction>,
    pub slots: Vec<AdSlot>,
    pub metrics: ScanMetrics,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScanTelemetry {
    pub pages: Vec<PageView>,
    /// One entry per page view: what started on it.
    pub by_page: Vec<Telemetry>,
    /// Everything, in page order. Counts are sums over the page views; load
    /// and time-to-first figures are the entry page's.
    pub total: Telemetry,
}

impl ScanTelemetry {
    /// Index of the page view that loaded the document `page` lives in: a
    /// route change stays in the document of the page view before it.
    pub fn document_of(&self, page: u32) -> u32 {
        document_of(&self.pages, page, |view| view.reached_by)
    }

    /// ms since the scan's navigation start of a time on `page`.
    pub fn absolute_ms(&self, page: u32, at_ms: f64) -> f64 {
        self.pages
            .get(page as usize)
            .map_or(at_ms, |view| view.started_at_ms + at_ms)
    }
}

fn document_of<P>(pages: &[P], page: u32, reached_by: impl Fn(&P) -> PageReach) -> u32 {
    let mut index = (page as usize).min(pages.len().saturating_sub(1));
    while index > 0 && reached_by(&pages[index]) == PageReach::RouteChange {
        index -= 1;
    }
    index as u32
}

/// One event with its time since the scan's navigation start.
struct Timed<'a> {
    at_ms: f64,
    event: &'a TrackerEvent,
}

pub fn derive(capture: &Capture) -> ScanTelemetry {
    let mut timeline: Vec<Timed> = capture
        .events
        .iter()
        .map(|event| Timed {
            at_ms: capture.absolute_ms(event.page, event.t),
            event,
        })
        .collect();
    timeline.sort_by(|a, b| a.at_ms.total_cmp(&b.at_ms));

    let mut auctions = derive_auctions(&timeline);
    let mut slots = derive_slots(&timeline, &capture.pages, &auctions);
    for auction in &mut auctions {
        rebase_auction(auction, page_start(&capture.pages, auction.page));
    }
    for slot in &mut slots {
        rebase_slot(slot, page_start(&capture.pages, slot.page));
    }
    auctions.sort_by_key(|auction| auction.page);
    slots.sort_by_key(|slot| slot.page);

    let by_page: Vec<Telemetry> = (0u32..)
        .zip(&capture.pages)
        .map(|(index, page)| {
            let auctions: Vec<Auction> = on_page(&auctions, index, |auction| auction.page);
            let slots: Vec<AdSlot> = on_page(&slots, index, |slot| slot.page);
            let events = capture.events.iter().filter(|event| event.page == index);
            let metrics = derive_metrics(capture, events, page.load_ms, &auctions, &slots);
            Telemetry {
                auctions,
                slots,
                metrics,
            }
        })
        .collect();

    let mut metrics = derive_metrics(
        capture,
        capture.events.iter(),
        capture.page_load_ms,
        &auctions,
        &slots,
    );
    let entry = by_page.first().map(|page| &page.metrics);
    metrics.time_to_first_auction_ms = entry.and_then(|entry| entry.time_to_first_auction_ms);
    metrics.time_to_first_ad_ms = entry.and_then(|entry| entry.time_to_first_ad_ms);

    let pages = (0u32..)
        .zip(&capture.pages)
        .zip(&by_page)
        .map(|((index, page), telemetry)| page_view(index, page, &telemetry.metrics))
        .collect();
    ScanTelemetry {
        pages,
        by_page,
        total: Telemetry {
            auctions,
            slots,
            metrics,
        },
    }
}

fn page_start(pages: &[CapturedPage], page: u32) -> f64 {
    pages
        .get(page as usize)
        .map_or(0.0, |page| page.started_at_ms)
}

fn on_page<T: Clone>(items: &[T], page: u32, page_of: impl Fn(&T) -> u32) -> Vec<T> {
    items
        .iter()
        .filter(|item| page_of(item) == page)
        .cloned()
        .collect()
}

fn page_view(index: u32, page: &CapturedPage, metrics: &ScanMetrics) -> PageView {
    PageView {
        index,
        url: page.url.clone(),
        title: page.title.clone(),
        started_at_ms: page.started_at_ms,
        dwell_ms: (page.ended_at_ms - page.started_at_ms).max(0.0),
        reached_by: page.reached_by,
        link: page.link.clone(),
        load_ms: page.load_ms,
        max_scroll_depth: page.max_scroll_depth,
        metrics: PageMetrics {
            auction_count: metrics.auction_count,
            bid_request_count: metrics.bid_request_count,
            bid_response_count: metrics.bid_response_count,
            slot_count: metrics.slot_count,
            empty_slot_count: metrics.empty_slot_count,
            fill_rate: metrics.fill_rate,
            time_to_first_auction_ms: metrics.time_to_first_auction_ms,
            time_to_first_ad_ms: metrics.time_to_first_ad_ms,
        },
    }
}

fn rebase_auction(auction: &mut Auction, page_start: f64) {
    auction.started_at_ms -= page_start;
    if let Some(ended_at) = &mut auction.ended_at_ms {
        *ended_at -= page_start;
    }
    let requested = auction
        .ad_units
        .iter_mut()
        .flat_map(|unit| &mut unit.bids)
        .filter_map(|bid| bid.requested_at_ms.as_mut());
    for requested_at in requested {
        *requested_at -= page_start;
    }
}

fn rebase_slot(slot: &mut AdSlot, page_start: f64) {
    let times = [
        &mut slot.defined_at_ms,
        &mut slot.requested_at_ms,
        &mut slot.response_at_ms,
        &mut slot.rendered_at_ms,
        &mut slot.viewable_at_ms,
    ];
    for at in times.into_iter().flatten() {
        *at -= page_start;
    }
    for request in &mut slot.requests {
        request.at_ms -= page_start;
    }
}

/// A GPT slot and a Prebid ad unit describe the same placement when the ad
/// unit code is the slot's element id or its ad unit path.
pub fn slot_matches_code(slot: &AdSlot, code: &str) -> bool {
    slot.element_id == code || slot.ad_unit_path.as_deref() == Some(code)
}

/// Whether the auction asked for bids for this slot.
pub fn auction_covers(auction: &Auction, slot: &AdSlot) -> bool {
    auction
        .ad_units
        .iter()
        .any(|unit| slot_matches_code(slot, &unit.code))
}

pub fn prebid_version(capture: &Capture) -> Option<String> {
    capture.events.iter().find_map(|event| match &event.kind {
        Ev::PrebidDetected { version, .. } => version.clone(),
        _ => None,
    })
}

// -- auctions -----------------------------------------------------------------

/// Times are absolute until [`derive`] rebases them.
fn derive_auctions(timeline: &[Timed]) -> Vec<Auction> {
    let mut auctions: Vec<Auction> = Vec::new();
    for Timed { at_ms, event } in timeline {
        let t = *at_ms;
        match &event.kind {
            Ev::AuctionInit {
                auction_id,
                timeout,
                ad_units,
            } => auctions.push(Auction {
                id: auction_id.clone(),
                page: event.page,
                started_at_ms: t,
                ended_at_ms: None,
                duration_ms: None,
                timeout_ms: *timeout,
                ad_units: ad_units
                    .iter()
                    .map(|unit| new_ad_unit(&unit.code, unit.sizes.clone()))
                    .collect(),
            }),
            Ev::BidRequested {
                auction_id,
                bidder,
                ad_unit_codes,
                ..
            } => {
                if let Some(auction) = find_auction(&mut auctions, auction_id) {
                    for code in ad_unit_codes {
                        bid_entry(auction, code, bidder).requested_at_ms = Some(t);
                    }
                }
            }
            Ev::BidResponse {
                auction_id,
                bidder,
                ad_unit_code,
                cpm,
                currency,
                size,
                time_to_respond,
                deal_id,
            } => {
                if let Some(auction) = find_auction(&mut auctions, auction_id) {
                    let bid = bid_entry(auction, ad_unit_code, bidder);
                    // A response that arrives after Prebid already timed the bidder
                    // out did not take part in the auction.
                    if bid.status == BidStatus::Pending {
                        bid.status = BidStatus::Bid;
                        bid.cpm = Some(*cpm);
                        bid.currency = currency.clone();
                        bid.size = size.clone();
                        bid.deal_id = deal_id.clone();
                        bid.response_time_ms =
                            time_to_respond.or(bid.requested_at_ms.map(|requested| t - requested));
                    }
                }
            }
            Ev::NoBid {
                auction_id,
                bidder,
                ad_unit_code,
            } => settle_pending(
                &mut auctions,
                auction_id,
                ad_unit_code,
                bidder,
                BidStatus::NoBid,
                t,
            ),
            Ev::BidTimeout {
                auction_id,
                bidder,
                ad_unit_code,
            } => settle_pending(
                &mut auctions,
                auction_id,
                ad_unit_code,
                bidder,
                BidStatus::Timeout,
                t,
            ),
            Ev::BidderError {
                auction_id, bidder, ..
            } => mark_bidder_error(&mut auctions, auction_id.as_deref(), bidder),
            Ev::AuctionEnd { auction_id } => {
                if let Some(auction) = find_auction(&mut auctions, auction_id) {
                    auction.ended_at_ms = Some(t);
                    auction.duration_ms = Some(t - auction.started_at_ms);
                }
            }
            Ev::BidWon {
                auction_id,
                bidder,
                ad_unit_code,
                cpm,
            } => mark_won(
                &mut auctions,
                auction_id.as_deref(),
                ad_unit_code,
                bidder,
                *cpm,
            ),
            Ev::AdRenderSucceeded {
                ad_unit_code: Some(code),
                bidder,
            } => mark_rendered(&mut auctions, code, bidder.as_deref()),
            _ => {}
        }
    }
    auctions
}

fn new_ad_unit(code: &str, sizes: Vec<String>) -> AuctionAdUnit {
    AuctionAdUnit {
        code: code.to_string(),
        sizes,
        bids: Vec::new(),
        winning_bidder: None,
        winning_cpm: None,
    }
}

fn find_auction<'a>(auctions: &'a mut [Auction], id: &str) -> Option<&'a mut Auction> {
    auctions.iter_mut().rev().find(|auction| auction.id == id)
}

fn bid_entry<'a>(auction: &'a mut Auction, code: &str, bidder: &str) -> &'a mut Bid {
    let unit_index = match auction.ad_units.iter().position(|unit| unit.code == code) {
        Some(index) => index,
        None => {
            auction.ad_units.push(new_ad_unit(code, Vec::new()));
            auction.ad_units.len() - 1
        }
    };
    let unit = &mut auction.ad_units[unit_index];
    let bid_index = match unit.bids.iter().position(|bid| bid.bidder == bidder) {
        Some(index) => index,
        None => {
            unit.bids.push(Bid {
                bidder: bidder.to_string(),
                status: BidStatus::Pending,
                cpm: None,
                currency: None,
                size: None,
                requested_at_ms: None,
                response_time_ms: None,
                deal_id: None,
                exchange: None,
            });
            unit.bids.len() - 1
        }
    };
    &mut unit.bids[bid_index]
}

fn settle_pending(
    auctions: &mut [Auction],
    auction_id: &str,
    code: &str,
    bidder: &str,
    status: BidStatus,
    t: f64,
) {
    let Some(auction) = find_auction(auctions, auction_id) else {
        return;
    };
    let bid = bid_entry(auction, code, bidder);
    if bid.status == BidStatus::Pending {
        bid.status = status;
        bid.response_time_ms = bid.requested_at_ms.map(|requested| t - requested);
    }
}

fn mark_bidder_error(auctions: &mut [Auction], auction_id: Option<&str>, bidder: &str) {
    let pending_bids = auctions
        .iter_mut()
        .filter(|auction| auction_id.is_none_or(|id| auction.id == id))
        .flat_map(|auction| auction.ad_units.iter_mut())
        .flat_map(|unit| unit.bids.iter_mut())
        .filter(|bid| bid.bidder == bidder && bid.status == BidStatus::Pending);
    for bid in pending_bids {
        bid.status = BidStatus::Error;
    }
}

fn mark_won(
    auctions: &mut [Auction],
    auction_id: Option<&str>,
    code: &str,
    bidder: &str,
    cpm: f64,
) {
    let unit = auctions
        .iter_mut()
        .rev()
        .filter(|auction| auction_id.is_none_or(|id| auction.id == id))
        .flat_map(|auction| auction.ad_units.iter_mut())
        .find(|unit| unit.code == code && unit.bids.iter().any(|bid| bid.bidder == bidder));
    let Some(unit) = unit else {
        return;
    };
    unit.winning_bidder = Some(bidder.to_string());
    unit.winning_cpm = Some(cpm);
    if let Some(bid) = unit.bids.iter_mut().find(|bid| bid.bidder == bidder) {
        bid.status = BidStatus::Won;
        bid.cpm = Some(cpm);
    }
}

fn mark_rendered(auctions: &mut [Auction], code: &str, bidder: Option<&str>) {
    let won_bid = auctions
        .iter_mut()
        .rev()
        .flat_map(|auction| auction.ad_units.iter_mut())
        .filter(|unit| unit.code == code)
        .flat_map(|unit| unit.bids.iter_mut())
        .find(|bid| bid.status == BidStatus::Won && bidder.is_none_or(|name| bid.bidder == name));
    if let Some(bid) = won_bid {
        bid.status = BidStatus::Rendered;
    }
}

// -- slots --------------------------------------------------------------------

/// Times are absolute until [`derive`] rebases them.
fn derive_slots(timeline: &[Timed], pages: &[CapturedPage], auctions: &[Auction]) -> Vec<AdSlot> {
    let mut slots: Vec<AdSlot> = Vec::new();
    for Timed { at_ms, event } in timeline {
        let t = *at_ms;
        let page = event.page;
        match &event.kind {
            Ev::SlotDefined {
                element_id,
                ad_unit_path,
                sizes,
                in_viewport,
            } => {
                let slot = slot_on_page(&mut slots, pages, page, element_id);
                slot.ad_unit_path = ad_unit_path.clone();
                slot.sizes = sizes.clone();
                slot.in_initial_viewport = *in_viewport;
                slot.defined_at_ms.get_or_insert(t);
            }
            Ev::SlotRequested {
                element_id,
                prebid_targeting_keys,
                prebid_ad_id,
            } => {
                let slot = slot_on_page(&mut slots, pages, page, element_id);
                let had_prebid_targeting = !prebid_targeting_keys.is_empty();
                if slot.requested_at_ms.is_none() {
                    slot.requested_at_ms = Some(t);
                    slot.had_prebid_targeting = Some(had_prebid_targeting);
                    slot.outcome = SlotOutcome::Pending;
                }
                slot.requests.push(SlotRequest {
                    at_ms: t,
                    had_prebid_targeting,
                    prebid_ad_id: prebid_ad_id.clone().filter(|id| !id.is_empty()),
                    request_id: None,
                });
            }
            Ev::SlotResponseReceived { element_id } => {
                answered_slot(&mut slots, pages, page, element_id)
                    .response_at_ms
                    .get_or_insert(t);
            }
            Ev::SlotRenderEnded {
                element_id,
                is_empty,
                size,
                advertiser_id,
                line_item_id,
            } => {
                let slot = answered_slot(&mut slots, pages, page, element_id);
                if *is_empty {
                    if slot.outcome != SlotOutcome::Filled {
                        slot.outcome = SlotOutcome::Empty;
                    }
                } else if slot.outcome != SlotOutcome::Filled {
                    slot.outcome = SlotOutcome::Filled;
                    slot.rendered_at_ms = Some(t);
                    slot.rendered_size = size.clone();
                    slot.advertiser_id = advertiser_id.clone();
                    slot.line_item_id = line_item_id.clone();
                }
            }
            Ev::ImpressionViewable { element_id } => {
                answered_slot(&mut slots, pages, page, element_id)
                    .viewable_at_ms
                    .get_or_insert(t);
            }
            _ => {}
        }
    }
    for slot in &mut slots {
        slot.requested_before_auction_end = requested_before_auction_end(slot, auctions, pages);
    }
    slots
}

fn position(slots: &[AdSlot], page: u32, element_id: &str) -> Option<usize> {
    slots
        .iter()
        .position(|slot| slot.page == page && slot.element_id == element_id)
}

/// The slot of this container on this page view. The same container on two
/// page views is two slots: each page view is its own chance to sell it.
/// After a route change the page did not define the slot again, so what is
/// known about it is carried over from the page view before.
fn slot_on_page<'a>(
    slots: &'a mut Vec<AdSlot>,
    pages: &[CapturedPage],
    page: u32,
    element_id: &str,
) -> &'a mut AdSlot {
    let index = match position(slots, page, element_id) {
        Some(index) => index,
        None => {
            let earlier = earlier_in_document(slots, pages, page, element_id).map(|index| {
                let known = &slots[index];
                (known.ad_unit_path.clone(), known.sizes.clone())
            });
            let (ad_unit_path, sizes) = earlier.unwrap_or_default();
            slots.push(AdSlot {
                page,
                element_id: element_id.to_string(),
                ad_unit_path,
                sizes,
                outcome: SlotOutcome::NeverRequested,
                rendered_size: None,
                defined_at_ms: None,
                requested_at_ms: None,
                response_at_ms: None,
                rendered_at_ms: None,
                viewable_at_ms: None,
                in_initial_viewport: None,
                had_prebid_targeting: None,
                requested_before_auction_end: None,
                advertiser_id: None,
                line_item_id: None,
                requests: Vec::new(),
            });
            slots.len() - 1
        }
    };
    &mut slots[index]
}

/// The slot an answer (response, render, viewability) belongs to. An answer
/// that arrives just after a route change answers the request made on the
/// page view before it.
fn answered_slot<'a>(
    slots: &'a mut Vec<AdSlot>,
    pages: &[CapturedPage],
    page: u32,
    element_id: &str,
) -> &'a mut AdSlot {
    if position(slots, page, element_id).is_none()
        && let Some(index) = earlier_in_document(slots, pages, page, element_id)
    {
        return &mut slots[index];
    }
    slot_on_page(slots, pages, page, element_id)
}

/// The same container on the nearest earlier page view of the same document.
fn earlier_in_document(
    slots: &[AdSlot],
    pages: &[CapturedPage],
    page: u32,
    element_id: &str,
) -> Option<usize> {
    let document = document_of(pages, page, |view| view.reached_by);
    (document..page)
        .rev()
        .find_map(|earlier| position(slots, earlier, element_id))
}

/// `None` when the slot was never requested or no Prebid auction covers it.
/// `Some(true)` when the ad request left while its auction was still running,
/// or before the first auction for it had even started.
/// Only auctions of the slot's own document count: a new document starts
/// with no bids, whatever was auctioned before the navigation.
fn requested_before_auction_end(
    slot: &AdSlot,
    auctions: &[Auction],
    pages: &[CapturedPage],
) -> Option<bool> {
    let requested_at = slot.requested_at_ms?;
    let document = |page| document_of(pages, page, |view: &CapturedPage| view.reached_by);
    let mut covering = auctions
        .iter()
        .filter(|auction| document(auction.page) == document(slot.page))
        .filter(|auction| auction_covers(auction, slot))
        .peekable();
    covering.peek()?;
    let finished_before_request = covering.any(|auction| {
        auction
            .ended_at_ms
            .is_some_and(|ended_at| ended_at <= requested_at)
    });
    Some(!finished_before_request)
}

// -- metrics ------------------------------------------------------------------

/// `events` and `page_load_ms` are those of the scope the auctions and slots
/// come from. Script figures are the scan's in every scope: a script is
/// loaded once however many page views use it.
fn derive_metrics<'a>(
    capture: &Capture,
    events: impl Iterator<Item = &'a TrackerEvent> + Clone,
    page_load_ms: Option<f64>,
    auctions: &[Auction],
    slots: &[AdSlot],
) -> ScanMetrics {
    let ad_units = || auctions.iter().flat_map(|auction| &auction.ad_units);
    let bids = || ad_units().flat_map(|unit| &unit.bids);
    let count_bids =
        |wanted: &[BidStatus]| bids().filter(|bid| wanted.contains(&bid.status)).count() as u32;

    let bid_request_count = bids().count() as u32;
    let bid_timeout_count = count_bids(&[BidStatus::Timeout]);
    let winning_cpms: Vec<f64> = ad_units().filter_map(|unit| unit.winning_cpm).collect();
    let total_winning_cpm: f64 = winning_cpms.iter().sum();

    let requested_slots = slots
        .iter()
        .filter(|slot| slot.requested_at_ms.is_some())
        .count();
    let filled_slots = slots
        .iter()
        .filter(|slot| slot.outcome == SlotOutcome::Filled)
        .count();

    let first_event_at = |matches: fn(&Ev) -> bool| {
        events
            .clone()
            .find(|event| matches(&event.kind))
            .map(|event| event.t)
    };
    let first_slot_render = slots
        .iter()
        .filter_map(|slot| slot.rendered_at_ms)
        .min_by(f64::total_cmp);
    let first_prebid_render = first_event_at(|kind| matches!(kind, Ev::AdRenderSucceeded { .. }));
    let time_to_first_ad_ms = [first_slot_render, first_prebid_render]
        .into_iter()
        .flatten()
        .min_by(f64::total_cmp);

    let configured_timeout = events.clone().find_map(|event| match &event.kind {
        Ev::PrebidConfig { bidder_timeout, .. } => *bidder_timeout,
        _ => None,
    });

    ScanMetrics {
        page_load_ms,
        auction_count: auctions.len() as u32,
        ad_unit_count: ad_units()
            .map(|unit| &unit.code)
            .collect::<BTreeSet<_>>()
            .len() as u32,
        bidder_count: bids().map(|bid| &bid.bidder).collect::<BTreeSet<_>>().len() as u32,
        bid_request_count,
        bid_response_count: count_bids(&[BidStatus::Bid, BidStatus::Won, BidStatus::Rendered]),
        bid_timeout_count,
        no_bid_count: count_bids(&[BidStatus::NoBid]),
        timeout_rate: ratio(bid_timeout_count as usize, bid_request_count as usize),
        slot_count: slots.len() as u32,
        empty_slot_count: slots
            .iter()
            .filter(|slot| slot.outcome == SlotOutcome::Empty)
            .count() as u32,
        fill_rate: ratio(filled_slots, requested_slots),
        avg_winning_cpm: (!winning_cpms.is_empty())
            .then(|| total_winning_cpm / winning_cpms.len() as f64),
        total_winning_cpm,
        time_to_first_auction_ms: auctions.first().map(|auction| auction.started_at_ms),
        time_to_first_ad_ms,
        script_count: capture.scripts.len() as u32,
        ad_script_count: 0,
        total_script_bytes: capture
            .scripts
            .iter()
            .map(|script| script.body.len() as f64)
            .sum(),
        prebid_timeout_ms: configured_timeout
            .or_else(|| auctions.iter().find_map(|auction| auction.timeout_ms)),
        has_prebid: !auctions.is_empty()
            || first_event_at(|kind| matches!(kind, Ev::PrebidDetected { .. })).is_some(),
        has_gpt: !slots.is_empty()
            || first_event_at(|kind| matches!(kind, Ev::GptDetected { .. })).is_some(),
        has_amazon_tam: first_event_at(|kind| {
            matches!(
                kind,
                Ev::AmazonDetected {} | Ev::ApsFetchBids { .. } | Ev::ApsBidsReturned { .. }
            )
        })
        .is_some(),
    }
}

fn ratio(part: usize, whole: usize) -> f64 {
    if whole == 0 {
        0.0
    } else {
        part as f64 / whole as f64
    }
}
