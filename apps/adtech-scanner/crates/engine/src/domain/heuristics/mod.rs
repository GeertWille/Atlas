//! Deterministic revenue-leak rules evaluated on captured telemetry.
//!
//! Every rule is one unit behind [`Rule`]; [`all_rules`] is the single place
//! they are registered.

mod delivery;
mod lazy;
mod setup;
mod timing;

use std::collections::BTreeMap;

use adscan_model::{
    AdSlot, Auction, Bid, BidStatus, Capture, DeviceProfile, FindingCategory, FindingSource,
    ScriptRecord, Severity, TrackerEventKind as Ev,
};

use super::finding::FindingDraft;
use super::telemetry::{Telemetry, slot_matches_code};

/// Share of ad revenue assumed to come from header bidding when a rule
/// estimates the impact of slots that go to the ad server without bids.
/// A planning figure, not a measurement; findings state it as an estimate.
const HEADER_BIDDING_REVENUE_SHARE_PCT: f64 = 30.0;

pub trait Rule: Send + Sync {
    fn id(&self) -> &'static str;
    fn evaluate(&self, ctx: &RuleContext) -> Vec<FindingDraft>;
}

pub fn all_rules() -> Vec<Box<dyn Rule>> {
    vec![
        Box::new(lazy::AuctionGatedOnInteraction),
        Box::new(timing::AuctionTimeoutTooShort),
        Box::new(timing::BidderTimeouts),
        Box::new(timing::LateFirstAuction),
        Box::new(timing::SlowGptLoad),
        Box::new(delivery::AdRequestBeforeAuctionEnd),
        Box::new(delivery::SlotsNeverRequested),
        Box::new(delivery::EmptySlots),
        Box::new(delivery::AdUnitsWithoutSlot),
        Box::new(delivery::SlotsOutsideHeaderBidding),
        Box::new(delivery::WonBidNotRendered),
        Box::new(delivery::BiddersNeverBid),
        Box::new(setup::NoPriceFloors),
        Box::new(setup::DuplicatePrebid),
        Box::new(setup::AdLibraryLoadedTwice),
        Box::new(setup::OutdatedPrebid),
        Box::new(setup::ConsentNotWired),
        Box::new(setup::ConsentTimeout),
    ]
}

pub struct RuleHit {
    pub rule: &'static str,
    pub finding: FindingDraft,
}

pub fn evaluate(ctx: &RuleContext) -> Vec<RuleHit> {
    all_rules()
        .iter()
        .flat_map(|rule| {
            let id = rule.id();
            rule.evaluate(ctx)
                .into_iter()
                .map(move |finding| RuleHit { rule: id, finding })
        })
        .collect()
}

pub struct RuleContext<'a> {
    pub capture: &'a Capture,
    pub telemetry: &'a Telemetry,
    pub scripts: &'a [ScriptRecord],
}

/// Why bidding did not start on page load.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum AuctionGate {
    /// Prebid is on the page but no auction ran during the whole observation.
    NeverStarted { interaction_at_ms: Option<f64> },
    /// An auction started right after the first scroll / interaction.
    AfterInteraction {
        interaction_at_ms: f64,
        auction_at_ms: f64,
        /// Slots of that auction that had been on screen since page load.
        waiting_slots: Vec<String>,
    },
}

#[derive(Debug, Default, Clone, PartialEq)]
pub(crate) struct BidderStats {
    pub requests: u32,
    pub bids: u32,
    pub timeouts: u32,
    pub response_times_ms: Vec<f64>,
}

pub(crate) struct PrebidConfig {
    pub has_floors: bool,
    pub has_consent_management: bool,
}

/// An auction that starts within this window after the first interaction, on a
/// page where Prebid had been idle for at least `GATE_MIN_IDLE_MS`, was
/// waiting for that interaction.
const GATE_REACTION_WINDOW_MS: f64 = 2000.0;
const GATE_MIN_IDLE_MS: f64 = 1500.0;

impl<'a> RuleContext<'a> {
    pub(crate) fn auctions(&self) -> &'a [Auction] {
        &self.telemetry.auctions
    }

    pub(crate) fn slots(&self) -> &'a [AdSlot] {
        &self.telemetry.slots
    }

    pub(crate) fn bids(&self) -> impl Iterator<Item = &'a Bid> {
        self.auctions()
            .iter()
            .flat_map(|auction| &auction.ad_units)
            .flat_map(|unit| &unit.bids)
    }

    pub(crate) fn bidder_stats(&self) -> BTreeMap<&'a str, BidderStats> {
        let mut stats: BTreeMap<&str, BidderStats> = BTreeMap::new();
        for bid in self.bids() {
            let entry = stats.entry(bid.bidder.as_str()).or_default();
            entry.requests += 1;
            match bid.status {
                BidStatus::Bid | BidStatus::Won | BidStatus::Rendered => {
                    entry.bids += 1;
                    entry.response_times_ms.extend(bid.response_time_ms);
                }
                BidStatus::Timeout => entry.timeouts += 1,
                BidStatus::NoBid | BidStatus::Error | BidStatus::Pending => {}
            }
        }
        stats
    }

    /// Timeouts hit several bidders at once: the auction timeout is the
    /// problem, not one slow partner.
    pub(crate) fn timeouts_are_systemic(&self) -> bool {
        let metrics = &self.telemetry.metrics;
        let bidders_timing_out = self
            .bidder_stats()
            .values()
            .filter(|stats| stats.timeouts > 0)
            .count();
        metrics.bid_request_count >= 4 && metrics.timeout_rate >= 0.3 && bidders_timing_out >= 2
    }

    pub(crate) fn first_event_at(&self, matches: impl Fn(&Ev) -> bool) -> Option<f64> {
        self.capture
            .events
            .iter()
            .find(|event| matches(&event.kind))
            .map(|event| event.t)
    }

    pub(crate) fn first_interaction_at(&self) -> Option<f64> {
        self.first_event_at(|kind| match kind {
            Ev::PageLifecycle { phase } => phase == "firstScroll" || phase == "firstInteraction",
            Ev::Scroll { depth } => *depth > 0.0,
            _ => false,
        })
    }

    pub(crate) fn prebid_config(&self) -> Option<PrebidConfig> {
        self.capture
            .events
            .iter()
            .find_map(|event| match &event.kind {
                Ev::PrebidConfig {
                    has_floors,
                    has_consent_management,
                    ..
                } => Some(PrebidConfig {
                    has_floors: *has_floors,
                    has_consent_management: *has_consent_management,
                }),
                _ => None,
            })
    }

    /// Finds an auction that was waiting for the first interaction: it starts
    /// right after it, long after Prebid was ready, and it is either the first
    /// auction of the page or covers slots that were in the initial viewport
    /// and had not been auctioned before. An auction for a below-the-fold slot
    /// that scrolls into view is ordinary lazy loading, not a gate.
    pub(crate) fn auction_gate(&self) -> Option<AuctionGate> {
        let prebid_ready_at =
            self.first_event_at(|kind| matches!(kind, Ev::PrebidDetected { .. }))?;
        let interaction_at_ms = self.first_interaction_at();
        if self.auctions().is_empty() {
            return Some(AuctionGate::NeverStarted { interaction_at_ms });
        }
        let interaction_at_ms = interaction_at_ms?;
        self.auctions()
            .iter()
            .enumerate()
            .find_map(|(index, auction)| {
                let auction_at_ms = auction.started_at_ms;
                let reacted_to_interaction =
                    (0.0..=GATE_REACTION_WINDOW_MS).contains(&(auction_at_ms - interaction_at_ms));
                let prebid_sat_idle = auction_at_ms - prebid_ready_at >= GATE_MIN_IDLE_MS;
                if !reacted_to_interaction || !prebid_sat_idle {
                    return None;
                }
                let earlier = &self.auctions()[..index];
                let waiting_slots: Vec<String> = self
                    .slots()
                    .iter()
                    .filter(|slot| slot.in_initial_viewport == Some(true))
                    .filter(|slot| auction_covers(auction, slot))
                    .filter(|slot| {
                        !earlier
                            .iter()
                            .any(|previous| auction_covers(previous, slot))
                    })
                    .map(|slot| slot.element_id.clone())
                    .collect();
                (index == 0 || !waiting_slots.is_empty()).then_some(AuctionGate::AfterInteraction {
                    interaction_at_ms,
                    auction_at_ms,
                    waiting_slots,
                })
            })
    }

    pub(crate) fn device_name(&self) -> &'static str {
        device_name(self.capture.device)
    }

    pub(crate) fn other_device_name(&self) -> &'static str {
        match self.capture.device {
            DeviceProfile::Desktop => device_name(DeviceProfile::Mobile),
            DeviceProfile::Mobile => device_name(DeviceProfile::Desktop),
        }
    }
}

fn auction_covers(auction: &Auction, slot: &AdSlot) -> bool {
    auction
        .ad_units
        .iter()
        .any(|unit| slot_matches_code(slot, &unit.code))
}

fn device_name(device: DeviceProfile) -> &'static str {
    match device {
        DeviceProfile::Desktop => "desktop",
        DeviceProfile::Mobile => "mobile",
    }
}

pub(crate) fn heuristic(
    severity: Severity,
    category: FindingCategory,
    confidence: f64,
    title: impl Into<String>,
) -> FindingDraft {
    FindingDraft {
        severity,
        category,
        source: FindingSource::Heuristic,
        title: title.into(),
        description: String::new(),
        recommendation: String::new(),
        confidence,
        impact: String::new(),
        estimated_revenue_impact_pct: None,
        evidence: Vec::new(),
        affected_slots: Vec::new(),
        affected_bidders: Vec::new(),
    }
}

pub(crate) fn ms(value: f64) -> String {
    if value >= 1000.0 {
        format!("{:.1} s", value / 1000.0)
    } else {
        format!("{value:.0} ms")
    }
}

pub(crate) fn pct(ratio: f64) -> String {
    format!("{:.0}%", ratio * 100.0)
}

pub(crate) fn share(part: usize, whole: usize) -> f64 {
    if whole == 0 {
        0.0
    } else {
        part as f64 / whole as f64
    }
}

pub(crate) fn median(values: &[f64]) -> Option<f64> {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    sorted.get(sorted.len().checked_sub(1)? / 2).copied()
}
