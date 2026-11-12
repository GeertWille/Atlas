use adscan_model::{FindingCategory, ScriptCategory, Severity, SlotOutcome};

use super::visit::unbid_after_navigation;
use super::{
    AuctionGate, HEADER_BIDDING_REVENUE_SHARE_PCT, HEADER_BIDDING_SHARE_ASSUMPTION, Rule,
    RuleContext, RuleScope, estimate_impact, heuristic, ms,
};
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

    fn scope(&self) -> RuleScope {
        RuleScope::Page
    }

    fn evaluate(&self, ctx: &RuleContext) -> Vec<FindingDraft> {
        match ctx.auction_gate() {
            // A later page view that lost its bids to state kept across the
            // navigation has its own finding; a scroll gate is not the cause.
            Some(AuctionGate::NeverStarted { .. }) if lost_bids_after_navigation(ctx) => Vec::new(),
            Some(AuctionGate::NeverStarted { interaction_at_ms }) => {
                vec![never_started(ctx, interaction_at_ms)]
            }
            Some(AuctionGate::AfterInteraction {
                interaction_at_ms,
                auction_at_ms,
                waiting_slots,
            }) => vec![started_after_interaction(
                ctx,
                interaction_at_ms,
                auction_at_ms,
                waiting_slots,
            )],
            None => Vec::new(),
        }
    }
}

fn lost_bids_after_navigation(ctx: &RuleContext) -> bool {
    ctx.page
        .is_some_and(|page| !unbid_after_navigation(ctx, page.index).is_empty())
}

fn never_started(ctx: &RuleContext, interaction_at_ms: Option<f64>) -> FindingDraft {
    let device = ctx.device_name();
    let other = ctx.other_device_name();
    let scrolled = interaction_at_ms.is_some();
    // A consent banner nobody answered is an equally good reason for Prebid to
    // sit idle, and waiting for consent is not a bug.
    let consent_unanswered = !ctx.capture.consent_accepted
        && ctx
            .scripts
            .iter()
            .any(|script| script.category == ScriptCategory::Cmp);
    let (severity, confidence) = match (scrolled, consent_unanswered) {
        (true, false) => (Severity::Critical, 0.8),
        (false, false) => (Severity::High, 0.6),
        (true, true) => (Severity::High, 0.5),
        (false, true) => (Severity::Medium, 0.4),
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
    if consent_unanswered {
        finding.description.push_str(
            " A consent platform is on the page and its banner was not accepted during this scan, so bidding \
             may instead be waiting for the user's consent choice.",
        );
        finding.recommendation.insert_str(
            0,
            "First re-scan with consent acceptance turned on: if auctions start then, bidding was waiting for \
             consent and this is expected behaviour. If they still do not start: ",
        );
    }
    finding.impact =
        format!("No header bidding demand on {device}: slots are sold without bids or stay empty");
    finding.evidence.push(timeline(
        "Auction timeline",
        format!(
            "{device} scan, viewport {}x{}: Prebid detected, 0 auctions, first interaction {}, scroll depth reached {:.0}%",
            ctx.capture.viewport.width,
            ctx.capture.viewport.height,
            interaction_at_ms.map_or("never".to_string(), ms),
            ctx.max_scroll_depth() * 100.0
        ),
    ));
    finding.affected_slots = slots_waiting_for_bids(ctx);
    estimate_impact(
        &mut finding,
        HEADER_BIDDING_REVENUE_SHARE_PCT,
        &format!("all header bidding revenue on {device}; {HEADER_BIDDING_SHARE_ASSUMPTION}"),
    );
    finding
}

fn started_after_interaction(
    ctx: &RuleContext,
    interaction_at_ms: f64,
    auction_at_ms: f64,
    in_view: Vec<String>,
) -> FindingDraft {
    let device = ctx.device_name();
    let other = ctx.other_device_name();
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
    let auctioned = if in_view.is_empty() {
        "the first Prebid auction of the page"
    } else {
        "the Prebid auction for the slots already on screen"
    };
    finding.description = format!(
        "On this **{device}** scan {auctioned} started at {}, {} after the first scroll / interaction at {}, \
         although Prebid was ready long before. Nothing is auctioned for users who do not scroll, and \
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
    finding.impact = format!(
        "No bids before the first scroll on {device}; non-scrolling users are not monetised by header bidding"
    );
    finding.evidence.push(timeline(
        "Auction timeline",
        format!(
            "{device} scan: first interaction {} -> first auction {} (page load {})",
            ms(interaction_at_ms),
            ms(auction_at_ms),
            ctx.page_load_ms().map_or("unknown".to_string(), ms),
        ),
    ));
    if !in_view.is_empty() {
        finding.evidence.push(timeline(
            "Slots in the initial viewport",
            in_view.join(", "),
        ));
    }
    finding.affected_slots = in_view;
    estimate_impact(
        &mut finding,
        HEADER_BIDDING_REVENUE_SHARE_PCT * GATED_AUCTION_HB_LOSS,
        &format!(
            "half of the header bidding revenue on {device}, taking half of its impressions to be served before \
             the first scroll; {HEADER_BIDDING_SHARE_ASSUMPTION}"
        ),
    );
    finding
}

fn slots_waiting_for_bids(ctx: &RuleContext) -> Vec<String> {
    ctx.slots()
        .iter()
        .filter(|slot| slot.outcome != SlotOutcome::Filled)
        .map(|slot| slot.element_id.clone())
        .collect()
}
