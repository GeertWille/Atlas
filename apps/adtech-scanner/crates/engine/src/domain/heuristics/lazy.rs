use adscan_model::{FindingCategory, Severity, SlotOutcome};

use super::{AuctionGate, Rule, RuleContext, heuristic, ms};
use crate::domain::finding::{FindingDraft, timeline};

/// Share of header-bidding revenue assumed lost when bidding waits for a
/// scroll: the above-the-fold impressions of users who never scroll.
const GATED_AUCTION_HB_LOSS: f64 = 0.5;

/// The reference bug: bidding waits for a scroll / interaction (a mobile
/// lazy-load optimisation) on a device class where it should start on load, or
/// never starts at all.
pub struct AuctionGatedOnInteraction;

impl Rule for AuctionGatedOnInteraction {
    fn id(&self) -> &'static str {
        "auction-gated-on-interaction"
    }

    fn evaluate(&self, ctx: &RuleContext) -> Vec<FindingDraft> {
        match ctx.auction_gate() {
            Some(AuctionGate::NeverStarted { interaction_at_ms }) => {
                vec![never_started(ctx, interaction_at_ms)]
            }
            Some(AuctionGate::AfterInteraction {
                interaction_at_ms,
                auction_at_ms,
            }) => vec![started_after_interaction(ctx, interaction_at_ms, auction_at_ms)],
            None => Vec::new(),
        }
    }
}

fn never_started(ctx: &RuleContext, interaction_at_ms: Option<f64>) -> FindingDraft {
    let device = ctx.device_name();
    let other = ctx.other_device_name();
    let scrolled = interaction_at_ms.is_some();
    let (severity, confidence) = if scrolled {
        (Severity::Critical, 0.8)
    } else {
        (Severity::High, 0.6)
    };
    let mut finding = heuristic(
        severity,
        FindingCategory::LazyLoad,
        confidence,
        format!("Prebid is loaded but no auction ever started on {device}"),
    );
    finding.description = if scrolled {
        format!(
            "Prebid.js was present on this **{device}** scan, the page was scrolled (first interaction at {}), \
             and still no auction started during the whole observation. Every impression on this device class \
             is sold without header bidding demand. This is the signature of bidding logic that is gated on a \
             condition which never becomes true for this device, for example a lazy-load branch chosen by a \
             wrong device check.",
            ms(interaction_at_ms.unwrap_or_default())
        )
    } else {
        format!(
            "Prebid.js was present on this **{device}** scan but no auction started during the observation, \
             and the page was never scrolled. Bidding is probably waiting for a scroll or interaction. Users \
             who do not scroll generate no header bidding demand at all."
        )
    };
    finding.recommendation = format!(
        "Find the condition that calls `pbjs.requestBids` and check which branch runs on {device}. \
         Start the auction on page load for slots in the initial viewport. Then **re-scan with the {other} \
         profile**: if auctions start there, the wrapper's device detection is choosing the wrong path on {device}."
    );
    finding.impact = format!("No header bidding demand on {device}: slots are sold without bids or stay empty");
    finding.estimated_revenue_impact_pct = Some(super::HEADER_BIDDING_REVENUE_SHARE_PCT);
    finding.evidence.push(timeline(
        "Auction timeline",
        format!(
            "{device} scan, viewport {}x{}: Prebid detected, 0 auctions, first interaction {}, scroll depth reached {:.0}%",
            ctx.capture.viewport.width,
            ctx.capture.viewport.height,
            interaction_at_ms.map_or("never".to_string(), ms),
            ctx.capture.max_scroll_depth * 100.0
        ),
    ));
    finding.affected_slots = slots_waiting_for_bids(ctx);
    finding
}

fn started_after_interaction(ctx: &RuleContext, interaction_at_ms: f64, auction_at_ms: f64) -> FindingDraft {
    let device = ctx.device_name();
    let other = ctx.other_device_name();
    let in_view = slots_in_initial_viewport(ctx);
    let severity = if in_view.is_empty() {
        Severity::Medium
    } else {
        Severity::High
    };
    let mut finding = heuristic(
        severity,
        FindingCategory::LazyLoad,
        0.75,
        format!("Bidding waits for the first scroll on {device}"),
    );
    finding.description = format!(
        "On this **{device}** scan the first Prebid auction started at {}, {} after the first scroll / interaction \
         at {}, although Prebid was ready long before. Nothing is auctioned for users who do not scroll, and \
         above-the-fold slots miss their first impression. Scroll-gated bidding is normally a mobile lazy-load \
         optimisation; when it also (or only) runs on the other device class, the wrapper's device detection is wrong.",
        ms(auction_at_ms),
        ms(auction_at_ms - interaction_at_ms),
        ms(interaction_at_ms),
    );
    finding.recommendation = format!(
        "Start the auction on load for slots in the initial viewport and lazy-load only below-the-fold slots. \
         **Re-scan with the {other} profile** and compare when the first auction starts: if only one device \
         class waits for scroll, fix the device check that selects the lazy path."
    );
    finding.impact = format!("No bids before the first scroll on {device}; non-scrolling users are not monetised by header bidding");
    finding.estimated_revenue_impact_pct =
        Some(super::HEADER_BIDDING_REVENUE_SHARE_PCT * GATED_AUCTION_HB_LOSS);
    finding.evidence.push(timeline(
        "Auction timeline",
        format!(
            "{device} scan: first interaction {} -> first auction {} (page load {})",
            ms(interaction_at_ms),
            ms(auction_at_ms),
            ctx.capture.page_load_ms.map_or("unknown".to_string(), ms),
        ),
    ));
    if !in_view.is_empty() {
        finding.evidence.push(timeline(
            "Slots in the initial viewport",
            in_view.join(", "),
        ));
    }
    finding.affected_slots = in_view;
    finding
}

fn slots_in_initial_viewport(ctx: &RuleContext) -> Vec<String> {
    ctx.slots()
        .iter()
        .filter(|slot| slot.in_initial_viewport == Some(true))
        .map(|slot| slot.element_id.clone())
        .collect()
}

fn slots_waiting_for_bids(ctx: &RuleContext) -> Vec<String> {
    ctx.slots()
        .iter()
        .filter(|slot| slot.outcome != SlotOutcome::Filled)
        .map(|slot| slot.element_id.clone())
        .collect()
}
