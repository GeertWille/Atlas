use adscan_model::{FindingCategory, Severity, TrackerEventKind as Ev};

use super::{Rule, RuleContext, heuristic, median, ms, pct};
use crate::domain::finding::{FindingDraft, metric, timeline};

const LATE_AUCTION_MS: f64 = 3000.0;
const VERY_LATE_AUCTION_MS: f64 = 6000.0;
const SLOW_GPT_MS: f64 = 3000.0;
const VERY_SLOW_GPT_MS: f64 = 6000.0;

/// Several bidders hit the timeout: the configured timeout is below what the
/// demand partners need on this page.
pub struct AuctionTimeoutTooShort;

impl Rule for AuctionTimeoutTooShort {
    fn id(&self) -> &'static str {
        "auction-timeout-too-short"
    }

    fn evaluate(&self, ctx: &RuleContext) -> Vec<FindingDraft> {
        if !ctx.timeouts_are_systemic() {
            return Vec::new();
        }
        let metrics = &ctx.telemetry.metrics;
        let stats = ctx.bidder_stats();
        let timed_out: Vec<String> = stats
            .iter()
            .filter(|(_, stats)| stats.timeouts > 0)
            .map(|(bidder, _)| bidder.to_string())
            .collect();
        let responded: Vec<f64> = stats
            .values()
            .flat_map(|stats| stats.response_times_ms.iter().copied())
            .collect();
        let timeout = metrics.prebid_timeout_ms;
        let severity = if metrics.timeout_rate >= 0.5 {
            Severity::High
        } else {
            Severity::Medium
        };
        let mut finding = heuristic(
            severity,
            FindingCategory::Timing,
            0.8,
            match timeout {
                Some(timeout) => format!(
                    "Auction timeout of {} cuts off {} of bid requests",
                    ms(timeout),
                    pct(metrics.timeout_rate)
                ),
                None => format!("{} of bid requests time out", pct(metrics.timeout_rate)),
            },
        );
        finding.description = format!(
            "{} of {} bid requests ended in a timeout across {} bidders ({}). When several partners miss the \
             deadline at once, the auction timeout is shorter than their real latency on this page, so bids that \
             would have competed are discarded.",
            metrics.bid_timeout_count,
            metrics.bid_request_count,
            timed_out.len(),
            timed_out.join(", "),
        );
        finding.recommendation = "Raise `bidderTimeout` (and the wrapper's failsafe timeout) to cover the slower \
             partners, start the auction earlier in the page lifecycle, and make sure bidder endpoints are \
             preconnected. Re-measure the timeout rate after each change."
            .into();
        finding.impact =
            "Bids arriving after the deadline are dropped, lowering auction pressure on every slot"
                .into();
        finding.estimated_revenue_impact_pct =
            Some(metrics.timeout_rate * super::HEADER_BIDDING_REVENUE_SHARE_PCT);
        finding.evidence.push(metric(
            "Timeout rate",
            format!(
                "{} / {} bid requests timed out ({}); configured timeout {}",
                metrics.bid_timeout_count,
                metrics.bid_request_count,
                pct(metrics.timeout_rate),
                timeout.map_or("unknown".to_string(), ms),
            ),
        ));
        if let Some(median_response) = median(&responded) {
            finding.evidence.push(metric(
                "Latency of bidders that did respond",
                format!(
                    "median {}, slowest {}",
                    ms(median_response),
                    ms(responded.iter().copied().fold(0.0, f64::max))
                ),
            ));
        }
        finding.affected_bidders = timed_out;
        vec![finding]
    }
}

/// One bidder keeps timing out while the rest of the auction is fine.
pub struct BidderTimeouts;

impl Rule for BidderTimeouts {
    fn id(&self) -> &'static str {
        "bidder-timeouts"
    }

    fn evaluate(&self, ctx: &RuleContext) -> Vec<FindingDraft> {
        if ctx.timeouts_are_systemic() {
            return Vec::new();
        }
        ctx.bidder_stats()
            .into_iter()
            .filter(|(_, stats)| stats.requests >= 2 && stats.timeouts * 2 >= stats.requests)
            .map(|(bidder, stats)| {
                let mut finding = heuristic(
                    Severity::Medium,
                    FindingCategory::Timing,
                    0.7,
                    format!("Bidder {bidder} timed out on {} of {} requests", stats.timeouts, stats.requests),
                );
                finding.description = format!(
                    "`{bidder}` did not answer before the auction timeout in {} of its {} bid requests, while other \
                     bidders responded in time. Its demand is effectively absent from these auctions and it still \
                     costs a network request per ad unit.",
                    stats.timeouts, stats.requests,
                );
                finding.recommendation = format!(
                    "Check the `{bidder}` adapter parameters and endpoint region, and ask the partner about latency. \
                     If it stays slow, move it server-side (Prebid Server) or remove it from the affected ad units."
                );
                finding.impact = format!("{bidder} demand rarely takes part in the auction");
                finding.evidence.push(metric(
                    "Bidder outcome",
                    format!(
                        "{bidder}: {} requests, {} timeouts, {} bids; auction timeout {}",
                        stats.requests,
                        stats.timeouts,
                        stats.bids,
                        ctx.telemetry.metrics.prebid_timeout_ms.map_or("unknown".to_string(), ms),
                    ),
                ));
                finding.affected_bidders = vec![bidder.to_string()];
                finding
            })
            .collect()
    }
}

/// The first auction starts seconds into the page view without waiting for an
/// interaction (that case belongs to `AuctionGatedOnInteraction`).
pub struct LateFirstAuction;

impl Rule for LateFirstAuction {
    fn id(&self) -> &'static str {
        "late-first-auction"
    }

    fn evaluate(&self, ctx: &RuleContext) -> Vec<FindingDraft> {
        let Some(started_at) = ctx.telemetry.metrics.time_to_first_auction_ms else {
            return Vec::new();
        };
        if started_at < LATE_AUCTION_MS || ctx.auction_gate().is_some() {
            return Vec::new();
        }
        let severity = if started_at >= VERY_LATE_AUCTION_MS {
            Severity::High
        } else {
            Severity::Medium
        };
        let mut finding = heuristic(
            severity,
            FindingCategory::Timing,
            0.7,
            format!("First auction starts {} after navigation", ms(started_at)),
        );
        finding.description = format!(
            "The first Prebid auction began {} after navigation start{}. Until it completes no slot can carry \
             header bidding demand, so early impressions are lost to short sessions and viewability suffers.",
            ms(started_at),
            ctx.capture
                .page_load_ms
                .map_or(String::new(), |load| format!(
                    " (page load event at {})",
                    ms(load)
                )),
        );
        finding.recommendation = "Load Prebid and the wrapper with high priority (async in the head, not behind a \
             tag manager or consent callback chain that waits for unrelated work) and call `requestBids` as soon as \
             ad units and consent are available."
            .into();
        finding.impact =
            "Late bids mean late ads: lower viewability and missed impressions on short visits"
                .into();
        finding.evidence.push(timeline(
            "Time to first auction",
            format!("auctionInit at {}", ms(started_at)),
        ));
        vec![finding]
    }
}

pub struct SlowGptLoad;

impl Rule for SlowGptLoad {
    fn id(&self) -> &'static str {
        "slow-gpt-load"
    }

    fn evaluate(&self, ctx: &RuleContext) -> Vec<FindingDraft> {
        let Some(ready_at) = ctx.first_event_at(|kind| matches!(kind, Ev::GptDetected { .. }))
        else {
            return Vec::new();
        };
        if ready_at < SLOW_GPT_MS || ctx.auction_gate().is_some() {
            return Vec::new();
        }
        let severity = if ready_at >= VERY_SLOW_GPT_MS {
            Severity::Medium
        } else {
            Severity::Low
        };
        let mut finding = heuristic(
            severity,
            FindingCategory::Timing,
            0.7,
            format!("Google Publisher Tag is ready only after {}", ms(ready_at)),
        );
        finding.description = format!(
            "`googletag` became available {} after navigation start. No ad request can leave before that, \
             whatever the auction does.",
            ms(ready_at)
        );
        finding.recommendation = "Load `gpt.js` asynchronously from the document head with a preconnect to \
             `securepubads.g.doubleclick.net`, and do not chain it behind other third-party scripts."
            .into();
        finding.impact = "Every ad request on the page is delayed".into();
        finding.evidence.push(timeline(
            "GPT ready",
            format!("googletag detected at {}", ms(ready_at)),
        ));
        vec![finding]
    }
}
