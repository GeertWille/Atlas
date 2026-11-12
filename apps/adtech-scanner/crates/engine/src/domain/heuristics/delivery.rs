use std::collections::BTreeSet;

use adscan_model::{
    AdSlot, AdsTxtStatus, BidStatus, BidderDiagnostics, DiagnosticArea, DiagnosticCheck,
    DiagnosticStatus, FindingCategory, Severity, SlotOutcome, TrackerEventKind as Ev,
};

use super::visit::{automation_indicators, unbid_after_navigation};
use super::{
    AuctionGate, HEADER_BIDDING_REVENUE_SHARE_PCT, HEADER_BIDDING_SHARE_ASSUMPTION, Rule,
    RuleContext, RuleScope, bidders, estimate_impact, heuristic, ms, pct, share,
};
use crate::domain::diagnostics::capitalized;
use crate::domain::finding::{FindingDraft, log_line, metric, timeline};
use crate::domain::telemetry::{auction_covers, slot_matches_code};

/// The page was scrolled far enough that every lazy slot had its chance.
const FULL_SCROLL_DEPTH: f64 = 0.9;
/// Text taken from the page is quoted in evidence as an excerpt only.
const MAX_QUOTED_CHARS: usize = 300;

fn ids(slots: &[&AdSlot]) -> Vec<String> {
    slots.iter().map(|slot| slot.element_id.clone()).collect()
}

/// GPT requested the ad while the Prebid auction for the slot was still
/// running, so the request carried no hb_* targeting. Requests that left
/// before a scroll-gated auction even began belong to that gate's finding.
pub struct AdRequestBeforeAuctionEnd;

impl Rule for AdRequestBeforeAuctionEnd {
    fn id(&self) -> &'static str {
        "ad-request-before-auction-end"
    }

    fn scope(&self) -> RuleScope {
        RuleScope::Page
    }

    fn evaluate(&self, ctx: &RuleContext) -> Vec<FindingDraft> {
        let gated_auction_at = match ctx.auction_gate() {
            Some(AuctionGate::AfterInteraction { auction_at_ms, .. }) => Some(auction_at_ms),
            _ => None,
        };
        let racing: Vec<&AdSlot> = ctx
            .slots()
            .iter()
            .filter(|slot| {
                slot.requested_before_auction_end == Some(true)
                    && slot.had_prebid_targeting == Some(false)
            })
            .filter(|slot| {
                // The gate's symptom: the request left before the slot's first
                // auction, the gated one, had even started.
                let first_auction_at = ctx
                    .auctions()
                    .iter()
                    .find(|auction| auction_covers(auction, slot))
                    .map(|auction| auction.started_at_ms);
                let waited_for_the_gate = gated_auction_at.is_some()
                    && first_auction_at == gated_auction_at
                    && slot.requested_at_ms < first_auction_at;
                !waited_for_the_gate
            })
            .collect();
        if racing.is_empty() {
            return Vec::new();
        }
        let affected_share = share(racing.len(), ctx.slots().len());
        let mut finding = heuristic(
            Severity::High,
            FindingCategory::Timing,
            0.85,
            format!(
                "{} ad request(s) sent before the Prebid auction finished",
                racing.len()
            ),
        );
        finding.description = format!(
            "For {} of {} slots the GPT ad request left before the Prebid auction for that slot had ended, and \
             the request carried no `hb_*` targeting. Google Ad Manager therefore never saw the header bids: \
             the auction ran, but its result could not win.",
            racing.len(),
            ctx.slots().len(),
        );
        finding.recommendation = "Call `googletag.pubads().refresh()` (or `display`) only from the `bidsBackHandler` \
             / failsafe timeout, after `pbjs.setTargetingForGPTAsync()`. Make sure `disableInitialLoad()` is set so \
             `display()` does not fire the request early."
            .into();
        finding.impact = "Header bids for these slots are thrown away; the ad server fills them without bid competition".into();
        for slot in &racing {
            let auction_end = ctx
                .auctions()
                .iter()
                .filter(|auction| auction_covers(auction, slot))
                .find_map(|auction| auction.ended_at_ms);
            finding.evidence.push(timeline(
                format!("Slot {}", slot.element_id),
                format!(
                    "ad request at {}, auction ended {}, hb_* targeting on request: none",
                    slot.requested_at_ms.map_or("unknown".to_string(), ms),
                    auction_end.map_or("never (still running)".to_string(), ms),
                ),
            ));
        }
        finding.affected_slots = ids(&racing);
        estimate_impact(
            &mut finding,
            affected_share * HEADER_BIDDING_REVENUE_SHARE_PCT,
            &format!(
                "the header bidding revenue of {} of {} slots, taking every slot to earn the same; \
                 {HEADER_BIDDING_SHARE_ASSUMPTION}",
                racing.len(),
                ctx.slots().len(),
            ),
        );
        vec![finding]
    }
}

/// Slots were defined but GPT never requested an ad for them, although they
/// were on screen. A scroll-gated auction is reported by its own rule.
pub struct SlotsNeverRequested;

impl Rule for SlotsNeverRequested {
    fn id(&self) -> &'static str {
        "slots-never-requested"
    }

    fn scope(&self) -> RuleScope {
        RuleScope::Page
    }

    fn evaluate(&self, ctx: &RuleContext) -> Vec<FindingDraft> {
        if ctx.auction_gate().is_some() {
            return Vec::new();
        }
        let page_fully_scrolled = ctx.max_scroll_depth() >= FULL_SCROLL_DEPTH;
        let idle: Vec<&AdSlot> = ctx
            .slots()
            .iter()
            .filter(|slot| slot.outcome == SlotOutcome::NeverRequested)
            .filter(|slot| slot.in_initial_viewport == Some(true) || page_fully_scrolled)
            .collect();
        if idle.is_empty() {
            return Vec::new();
        }
        let any_above_fold = idle
            .iter()
            .any(|slot| slot.in_initial_viewport == Some(true));
        let mut finding = heuristic(
            if any_above_fold {
                Severity::High
            } else {
                Severity::Medium
            },
            FindingCategory::Fill,
            0.8,
            format!("{} slot(s) defined but never requested", idle.len()),
        );
        finding.description = format!(
            "{} of {} GPT slots were defined, were on screen during the scan (scroll depth reached {}), and no ad \
             request was ever made for them. They earned nothing.",
            idle.len(),
            ctx.slots().len(),
            pct(ctx.max_scroll_depth()),
        );
        finding.recommendation = "Check that `googletag.display()` / `refresh()` is called for these slots and that \
             the lazy-load trigger (IntersectionObserver or GPT `enableLazyLoad`) actually fires for them."
            .into();
        finding.impact = "These placements never show an ad".into();
        for slot in &idle {
            finding.evidence.push(timeline(
                format!("Slot {}", slot.element_id),
                format!(
                    "defined at {}, in initial viewport: {}, ad request: never",
                    slot.defined_at_ms.map_or("unknown".to_string(), ms),
                    slot.in_initial_viewport
                        .map_or("unknown".to_string(), |v| v.to_string()),
                ),
            ));
        }
        finding.affected_slots = ids(&idle);
        estimate_impact(
            &mut finding,
            share(idle.len(), ctx.slots().len()) * 100.0,
            &format!(
                "{} of {} slots earn nothing, taking every slot to earn the same",
                idle.len(),
                ctx.slots().len(),
            ),
        );
        vec![finding]
    }
}

pub struct EmptySlots;

impl Rule for EmptySlots {
    fn id(&self) -> &'static str {
        "empty-slots"
    }

    fn scope(&self) -> RuleScope {
        RuleScope::Page
    }

    fn evaluate(&self, ctx: &RuleContext) -> Vec<FindingDraft> {
        let empty: Vec<&AdSlot> = ctx
            .slots()
            .iter()
            .filter(|slot| slot.outcome == SlotOutcome::Empty)
            .collect();
        if empty.is_empty() {
            return Vec::new();
        }
        let requested = ctx
            .slots()
            .iter()
            .filter(|slot| slot.requested_at_ms.is_some())
            .count();
        let empty_share = share(empty.len(), requested);
        let severity = if empty_share >= 0.5 {
            Severity::High
        } else if empty_share >= 0.2 {
            Severity::Medium
        } else {
            Severity::Low
        };
        let without_bids: Vec<&&AdSlot> = empty
            .iter()
            .filter(|slot| slot.had_prebid_targeting != Some(true))
            .collect();
        let mut finding = heuristic(
            severity,
            FindingCategory::Fill,
            0.6,
            format!(
                "{} of {} requested slots came back empty",
                empty.len(),
                requested
            ),
        );
        finding.description = format!(
            "The ad server answered {} of {} ad requests with no creative (fill rate {}). {} of the empty slots \
             went out without Prebid targeting, so no header bid could fill them.",
            empty.len(),
            requested,
            pct(ctx.telemetry.metrics.fill_rate),
            without_bids.len(),
        );
        finding.recommendation = "For the empty slots, check that their sizes match active line items, that a \
             house or AdX/Open Bidding fallback exists, and that Prebid targeting is set before the request so \
             header bids can compete."
            .into();
        finding.impact = "Rendered page space with no ad".into();
        for slot in &empty {
            finding.evidence.push(metric(
                format!("Slot {}", slot.element_id),
                format!(
                    "sizes {}, requested at {}, hb_* targeting: {}, result: empty",
                    slot.sizes.join("/"),
                    slot.requested_at_ms.map_or("unknown".to_string(), ms),
                    slot.had_prebid_targeting
                        .map_or("unknown".to_string(), |v| v.to_string()),
                ),
            ));
        }
        finding.affected_slots = ids(&empty);
        vec![finding]
    }
}

/// Prebid auctions ad units for which no GPT slot exists: the bids have
/// nowhere to go.
pub struct AdUnitsWithoutSlot;

impl Rule for AdUnitsWithoutSlot {
    fn id(&self) -> &'static str {
        "ad-units-without-slot"
    }

    fn scope(&self) -> RuleScope {
        RuleScope::Page
    }

    fn evaluate(&self, ctx: &RuleContext) -> Vec<FindingDraft> {
        if ctx.slots().is_empty() {
            return Vec::new();
        }
        let orphaned: BTreeSet<&str> = ctx
            .auctions()
            .iter()
            .flat_map(|auction| &auction.ad_units)
            .filter(|unit| {
                !ctx.slots()
                    .iter()
                    .any(|slot| slot_matches_code(slot, &unit.code))
            })
            .map(|unit| unit.code.as_str())
            .collect();
        if orphaned.is_empty() {
            return Vec::new();
        }
        let codes: Vec<String> = orphaned.iter().map(|code| code.to_string()).collect();
        let mut finding = heuristic(
            Severity::Medium,
            FindingCategory::Configuration,
            0.75,
            format!(
                "{} Prebid ad unit(s) have no matching GPT slot",
                codes.len()
            ),
        );
        finding.description = format!(
            "Prebid requested bids for ad unit code(s) `{}` but no GPT slot with that element id or ad unit path \
             was defined. `setTargetingForGPTAsync` cannot attach these bids to any slot, so they can never win; \
             the bid requests are wasted.",
            codes.join("`, `"),
        );
        finding.recommendation = "Make each Prebid ad unit `code` equal to the GPT slot's div id (or its ad unit \
             path), or pass a `customSlotMatching` function to `setTargetingForGPTAsync`. Remove ad units for \
             placements that no longer exist."
            .into();
        finding.impact = "Bids for these ad units are never delivered to the ad server".into();
        finding.evidence.push(metric(
            "Unmatched ad unit codes",
            format!(
                "{} | GPT slots on page: {}",
                codes.join(", "),
                ctx.slots()
                    .iter()
                    .map(|slot| slot.element_id.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
            ),
        ));
        finding.affected_slots = codes;
        vec![finding]
    }
}

/// Display slots that are requested but take part in no Prebid auction.
pub struct SlotsOutsideHeaderBidding;

impl Rule for SlotsOutsideHeaderBidding {
    fn id(&self) -> &'static str {
        "slots-outside-header-bidding"
    }

    fn scope(&self) -> RuleScope {
        RuleScope::Page
    }

    fn evaluate(&self, ctx: &RuleContext) -> Vec<FindingDraft> {
        if ctx.auctions().is_empty() {
            return Vec::new();
        }
        let is_display = |slot: &AdSlot| {
            slot.sizes
                .iter()
                .any(|size| size != "1x1" && size != "fluid")
        };
        // Slots that were auctioned on an earlier page view and lost their
        // bids after a navigation are that rule's finding: they do have a
        // Prebid ad unit.
        let lost_after_navigation = ctx
            .page
            .map(|page| unbid_after_navigation(ctx, page.index))
            .unwrap_or_default();
        let outside: Vec<&AdSlot> = ctx
            .slots()
            .iter()
            .filter(|slot| slot.requested_at_ms.is_some() && is_display(slot))
            .filter(|slot| !lost_after_navigation.contains(&slot.element_id))
            .filter(|slot| {
                !ctx.auctions()
                    .iter()
                    .any(|auction| auction_covers(auction, slot))
            })
            .collect();
        if outside.is_empty() {
            return Vec::new();
        }
        let mut finding = heuristic(
            Severity::Medium,
            FindingCategory::Configuration,
            0.7,
            format!(
                "{} slot(s) are not part of any Prebid auction",
                outside.len()
            ),
        );
        finding.description = format!(
            "{} of {} display slots were requested from the ad server but no Prebid ad unit matches them, so they \
             are sold without header bidding competition.",
            outside.len(),
            ctx.slots().len(),
        );
        finding.recommendation =
            "Add Prebid ad units for these placements, or fix the ad unit `code` so it matches \
             the slot's div id / ad unit path."
                .into();
        finding.impact = "No header bidding demand on these placements".into();
        finding.evidence.push(metric(
            "Slots without a Prebid ad unit",
            outside
                .iter()
                .map(|slot| {
                    format!(
                        "{} ({})",
                        slot.element_id,
                        slot.ad_unit_path.as_deref().unwrap_or("no path")
                    )
                })
                .collect::<Vec<_>>()
                .join(", "),
        ));
        finding.affected_slots = ids(&outside);
        estimate_impact(
            &mut finding,
            share(outside.len(), ctx.slots().len()) * HEADER_BIDDING_REVENUE_SHARE_PCT,
            &format!(
                "the header bidding revenue of {} of {} slots, taking every slot to earn the same; \
                 {HEADER_BIDDING_SHARE_ASSUMPTION}",
                outside.len(),
                ctx.slots().len(),
            ),
        );
        vec![finding]
    }
}

pub struct WonBidNotRendered;

impl Rule for WonBidNotRendered {
    fn id(&self) -> &'static str {
        "won-bid-not-rendered"
    }

    fn scope(&self) -> RuleScope {
        RuleScope::Page
    }

    fn evaluate(&self, ctx: &RuleContext) -> Vec<FindingDraft> {
        let failures: Vec<_> = ctx
            .capture
            .events
            .iter()
            .filter_map(|event| match &event.kind {
                Ev::AdRenderFailed {
                    ad_unit_code,
                    bidder,
                    reason,
                } => Some((event.t, ad_unit_code, bidder, reason)),
                _ => None,
            })
            .collect();
        if failures.is_empty() {
            return Vec::new();
        }
        let mut finding = heuristic(
            Severity::High,
            FindingCategory::Fill,
            0.9,
            format!("{} winning bid(s) failed to render", failures.len()),
        );
        finding.description =
            "Prebid reported `adRenderFailed`: a bid won the ad server auction but its creative \
             could not be rendered. The impression was lost after it had already been sold."
                .into();
        finding.recommendation = "Check the Prebid creative in the ad server (it must call `pbjs.renderAd` with the \
             `hb_adid` of the winning bid, or use the Prebid Universal Creative), that bids are not expired or \
             already used when rendered, and that safeframe settings match the creative."
            .into();
        finding.impact = "Sold impressions are not delivered and not paid".into();
        for (t, code, bidder, reason) in failures {
            finding.evidence.push(log_line(
                format!("adRenderFailed at {}", ms(t)),
                format!(
                    "ad unit {}, bidder {}: {}",
                    code.as_deref().unwrap_or("unknown"),
                    bidder.as_deref().unwrap_or("unknown"),
                    reason
                        .as_deref()
                        .map_or("no reason given".to_string(), |reason| reason
                            .chars()
                            .take(MAX_QUOTED_CHARS)
                            .collect()),
                ),
            ));
            finding.affected_slots.extend(code.clone());
            finding.affected_bidders.extend(bidder.clone());
        }
        finding.affected_slots.sort();
        finding.affected_slots.dedup();
        finding.affected_bidders.sort();
        finding.affected_bidders.dedup();
        vec![finding]
    }
}

/// Bidders that answered every request with "no bid". Timeouts are reported by
/// the timing rules.
pub struct BiddersNeverBid;

const MIN_REQUESTS_TO_JUDGE_BIDDER: u32 = 3;

/// Bidders with enough requests to judge and not one bid, with their request count.
pub(super) fn silent_bidders<'a>(ctx: &RuleContext<'a>) -> Vec<(&'a str, u32)> {
    ctx.bidder_stats()
        .into_iter()
        .filter(|(_, stats)| {
            stats.requests >= MIN_REQUESTS_TO_JUDGE_BIDDER && stats.bids == 0 && stats.timeouts == 0
        })
        .map(|(bidder, stats)| (bidder, stats.requests))
        .collect()
}

impl Rule for BiddersNeverBid {
    fn id(&self) -> &'static str {
        "bidders-never-bid"
    }

    fn evaluate(&self, ctx: &RuleContext) -> Vec<FindingDraft> {
        let silent = silent_bidders(ctx);
        if silent.is_empty() {
            return Vec::new();
        }
        let errored = ctx
            .bids()
            .filter(|bid| bid.status == BidStatus::Error)
            .count();
        let names: Vec<String> = silent
            .iter()
            .map(|(bidder, _)| bidder.to_string())
            .collect();
        // What the bidder checks found wrong for each silent bidder, most
        // certain first.
        let causes: Vec<(&str, Vec<&DiagnosticCheck>)> = silent
            .iter()
            .map(|(bidder, _)| (*bidder, likely_causes(ctx.diagnostics, bidder)))
            .collect();
        let explained = causes.iter().filter(|(_, found)| !found.is_empty()).count();
        let found = || causes.iter().flat_map(|(_, found)| found);
        let has_problem = found().any(|check| check.status == DiagnosticStatus::Problem);
        // What the adapter rejects and what the endpoint itself says are
        // observed, not inferred.
        let has_observed_cause = found().any(|check| {
            check.status == DiagnosticStatus::Problem
                && matches!(
                    check.area,
                    DiagnosticArea::Params | DiagnosticArea::Response
                )
        });

        let mut finding = heuristic(
            if has_problem {
                Severity::Medium
            } else {
                Severity::Low
            },
            FindingCategory::Configuration,
            if has_observed_cause {
                0.85
            } else if has_problem {
                0.7
            } else {
                0.4
            },
            format!("{} bidder(s) never returned a bid", names.len()),
        );
        finding.description = format!(
            "`{}` answered every bid request in this scan without a bid. One page view is a small sample, so \
             read the causes below as leads to confirm, not as a verdict.",
            names.join("`, `"),
        );
        let site_has_no_ads_txt = ctx
            .diagnostics
            .is_some_and(|diagnostics| diagnostics.ads_txt.status == AdsTxtStatus::Missing);
        if site_has_no_ads_txt {
            finding.description.push_str(
                " The site has no ads.txt at all, which on its own keeps many buyers from bidding.",
            );
        }
        let mut advice: Vec<&str> = Vec::new();
        let mut unexplained: Vec<&str> = Vec::new();
        for (bidder, found) in &causes {
            let Some((most_likely, others)) = found.split_first() else {
                unexplained.push(bidder);
                continue;
            };
            finding
                .description
                .push_str(&format!("\n\n- `{bidder}`: {}.", most_likely.summary));
            if !others.is_empty() {
                finding.description.push_str(&format!(
                    " Also: {}.",
                    others
                        .iter()
                        .map(|check| check.summary.as_str())
                        .collect::<Vec<_>>()
                        .join("; ")
                ));
            }
            for check in found {
                if let Some(line) =
                    bidders::advice(&check.code).filter(|line| !advice.contains(line))
                {
                    advice.push(line);
                }
                bidders::add_evidence(&mut finding, &[(*bidder, *check)]);
            }
        }
        if !unexplained.is_empty() {
            let checked = if ctx.diagnostics.is_some() {
                "the checks the scanner can run on its own (adapter params, the endpoint's answers, ads.txt, consent) found nothing wrong"
            } else {
                "the bidder checks did not run for this scan"
            };
            finding.description.push_str(&format!(
                "\n\n- `{}`: {checked}. A bidder that never bids usually has wrong placement parameters, an \
                 unapproved domain (ads.txt / sellers.json) or missing consent.",
                unexplained.join("`, `"),
            ));
        }
        finding.recommendation = advice
            .iter()
            .enumerate()
            .map(|(index, line)| format!("{}. {}.", index + 1, capitalized(line)))
            .collect::<Vec<_>>()
            .join("\n");
        if !unexplained.is_empty() {
            if !finding.recommendation.is_empty() {
                finding.recommendation.push_str("\n\n");
            }
            finding.recommendation.push_str(&format!(
                "For `{}`: verify the adapter params (placement / site / zone ids) with the partner, check the \
                 domain is approved in their platform and listed in ads.txt, and compare with their own reporting.",
                unexplained.join("`, `"),
            ));
        }
        finding.impact = "Requests are sent to partners that add no auction pressure".into();
        finding.evidence.push(metric(
            "Bidders without bids",
            format!(
                "{}; bidder errors in scan: {errored}; causes found for {explained} of {}",
                silent
                    .iter()
                    .map(|(bidder, requests)| format!("{bidder}: 0 bids / {requests} requests"))
                    .collect::<Vec<_>>()
                    .join(", "),
                silent.len(),
            ),
        ));
        // A page that can tell the visit is automated may get no bids for
        // that reason alone.
        if let Some(indicators) = automation_indicators(ctx.environment) {
            finding.evidence.push(metric(
                "Caveat: this scan was identifiable as automated",
                format!(
                    "{indicators}. Bidders may have declined for that reason; confirm with a visit a person drives."
                ),
            ));
        }
        finding.affected_bidders = names;
        vec![finding]
    }
}

/// The failed checks of one bidder, in the order the diagnostics rank them.
fn likely_causes<'a>(
    diagnostics: Option<&'a BidderDiagnostics>,
    bidder: &str,
) -> Vec<&'a DiagnosticCheck> {
    diagnostics
        .into_iter()
        .flat_map(|diagnostics| &diagnostics.bidders)
        .filter(|diagnosis| diagnosis.bidder == bidder)
        .flat_map(|diagnosis| &diagnosis.checks)
        .filter(|check| {
            matches!(
                check.status,
                DiagnosticStatus::Problem | DiagnosticStatus::Notice
            )
        })
        .collect()
}
