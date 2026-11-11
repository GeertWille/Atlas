//! Derives auctions, ad slots and metrics from the raw tracker events.

use std::collections::BTreeSet;

use adscan_model::{
    AdSlot, Auction, AuctionAdUnit, Bid, BidStatus, Capture, ScanMetrics, SlotOutcome,
    TrackerEventKind as Ev,
};

#[derive(Debug, Clone, PartialEq)]
pub struct Telemetry {
    pub auctions: Vec<Auction>,
    pub slots: Vec<AdSlot>,
    pub metrics: ScanMetrics,
}

pub fn derive(capture: &Capture) -> Telemetry {
    let auctions = derive_auctions(capture);
    let slots = derive_slots(capture, &auctions);
    let metrics = derive_metrics(capture, &auctions, &slots);
    Telemetry {
        auctions,
        slots,
        metrics,
    }
}

/// A GPT slot and a Prebid ad unit describe the same placement when the ad
/// unit code is the slot's element id or its ad unit path.
pub fn slot_matches_code(slot: &AdSlot, code: &str) -> bool {
    slot.element_id == code || slot.ad_unit_path.as_deref() == Some(code)
}

pub fn prebid_version(capture: &Capture) -> Option<String> {
    capture.events.iter().find_map(|event| match &event.kind {
        Ev::PrebidDetected { version, .. } => version.clone(),
        _ => None,
    })
}

// -- auctions -----------------------------------------------------------------

fn derive_auctions(capture: &Capture) -> Vec<Auction> {
    let mut auctions: Vec<Auction> = Vec::new();
    for event in &capture.events {
        let t = event.t;
        match &event.kind {
            Ev::AuctionInit {
                auction_id,
                timeout,
                ad_units,
            } => auctions.push(Auction {
                id: auction_id.clone(),
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
            } => settle_pending(&mut auctions, auction_id, ad_unit_code, bidder, BidStatus::NoBid, t),
            Ev::BidTimeout {
                auction_id,
                bidder,
                ad_unit_code,
            } => settle_pending(&mut auctions, auction_id, ad_unit_code, bidder, BidStatus::Timeout, t),
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
            } => mark_won(&mut auctions, auction_id.as_deref(), ad_unit_code, bidder, *cpm),
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

fn mark_won(auctions: &mut [Auction], auction_id: Option<&str>, code: &str, bidder: &str, cpm: f64) {
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

fn derive_slots(capture: &Capture, auctions: &[Auction]) -> Vec<AdSlot> {
    let mut slots: Vec<AdSlot> = Vec::new();
    for event in &capture.events {
        let t = event.t;
        match &event.kind {
            Ev::SlotDefined {
                element_id,
                ad_unit_path,
                sizes,
                in_viewport,
            } => {
                let slot = slot_entry(&mut slots, element_id);
                slot.ad_unit_path = ad_unit_path.clone();
                slot.sizes = sizes.clone();
                slot.in_initial_viewport = *in_viewport;
                slot.defined_at_ms.get_or_insert(t);
            }
            Ev::SlotRequested {
                element_id,
                prebid_targeting_keys,
            } => {
                let slot = slot_entry(&mut slots, element_id);
                if slot.requested_at_ms.is_none() {
                    slot.requested_at_ms = Some(t);
                    slot.had_prebid_targeting = Some(!prebid_targeting_keys.is_empty());
                    slot.outcome = SlotOutcome::Pending;
                }
            }
            Ev::SlotResponseReceived { element_id } => {
                slot_entry(&mut slots, element_id).response_at_ms.get_or_insert(t);
            }
            Ev::SlotRenderEnded {
                element_id,
                is_empty,
                size,
                advertiser_id,
                line_item_id,
            } => {
                let slot = slot_entry(&mut slots, element_id);
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
                slot_entry(&mut slots, element_id).viewable_at_ms.get_or_insert(t);
            }
            _ => {}
        }
    }
    for slot in &mut slots {
        slot.requested_before_auction_end = requested_before_auction_end(slot, auctions);
    }
    slots
}

fn slot_entry<'a>(slots: &'a mut Vec<AdSlot>, element_id: &str) -> &'a mut AdSlot {
    let index = match slots.iter().position(|slot| slot.element_id == element_id) {
        Some(index) => index,
        None => {
            slots.push(AdSlot {
                element_id: element_id.to_string(),
                ad_unit_path: None,
                sizes: Vec::new(),
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
            });
            slots.len() - 1
        }
    };
    &mut slots[index]
}

/// `None` when the slot was never requested or no Prebid auction covers it.
/// `Some(true)` when the ad request left while its auction was still running,
/// or before the first auction for it had even started.
fn requested_before_auction_end(slot: &AdSlot, auctions: &[Auction]) -> Option<bool> {
    let requested_at = slot.requested_at_ms?;
    let mut covering = auctions
        .iter()
        .filter(|auction| {
            auction
                .ad_units
                .iter()
                .any(|unit| slot_matches_code(slot, &unit.code))
        })
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

fn derive_metrics(capture: &Capture, auctions: &[Auction], slots: &[AdSlot]) -> ScanMetrics {
    let ad_units = || auctions.iter().flat_map(|auction| &auction.ad_units);
    let bids = || ad_units().flat_map(|unit| &unit.bids);
    let count_bids = |wanted: &[BidStatus]| bids().filter(|bid| wanted.contains(&bid.status)).count() as u32;

    let bid_request_count = bids().count() as u32;
    let bid_timeout_count = count_bids(&[BidStatus::Timeout]);
    let winning_cpms: Vec<f64> = ad_units().filter_map(|unit| unit.winning_cpm).collect();
    let total_winning_cpm: f64 = winning_cpms.iter().sum();

    let requested_slots = slots.iter().filter(|slot| slot.requested_at_ms.is_some()).count();
    let filled_slots = slots
        .iter()
        .filter(|slot| slot.outcome == SlotOutcome::Filled)
        .count();

    let first_event_at = |matches: fn(&Ev) -> bool| {
        capture
            .events
            .iter()
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

    let configured_timeout = capture.events.iter().find_map(|event| match &event.kind {
        Ev::PrebidConfig { bidder_timeout, .. } => *bidder_timeout,
        _ => None,
    });

    ScanMetrics {
        page_load_ms: capture.page_load_ms,
        auction_count: auctions.len() as u32,
        ad_unit_count: ad_units().map(|unit| &unit.code).collect::<BTreeSet<_>>().len() as u32,
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
        total_script_bytes: capture.scripts.iter().map(|script| script.body.len() as f64).sum(),
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
