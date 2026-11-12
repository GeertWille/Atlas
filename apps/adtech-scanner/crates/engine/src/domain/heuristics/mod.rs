//! Deterministic revenue-leak rules evaluated on captured telemetry.
//!
//! Every rule is one unit behind [`Rule`]; [`all_rules`] is the single place
//! they are registered.

mod bidders;
mod delivery;
mod lazy;
mod setup;
mod timing;
mod visit;

use std::collections::BTreeMap;

use adscan_model::{
    AdSlot, Auction, Bid, BidStatus, BidderDiagnostics, Capture, DeviceProfile, FindingCategory,
    FindingSource, PageReach, PageView, ScanEnvironment, ScriptRecord, Severity, TrackerEvent,
    TrackerEventKind as Ev,
};

use super::consent_wait::PageConsent;
use super::finding::{FindingDraft, metric, timeline};
use super::telemetry::{ScanTelemetry, Telemetry, auction_covers};

/// Share of ad revenue assumed to come from header bidding when a rule
/// estimates the impact of slots that go to the ad server without bids.
/// A planning figure, not a measurement; every estimate built on it says so.
const HEADER_BIDDING_REVENUE_SHARE_PCT: f64 = 30.0;
const HEADER_BIDDING_SHARE_ASSUMPTION: &str = "assumes header bidding brings in about 30% of ad revenue, a planning figure that was not measured";

/// What one evaluation of a rule looks at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleScope {
    /// One page view at a time: its events, auctions and slots. The rule
    /// runs once per page view and equal findings are merged.
    Page,
    /// The whole visit at once.
    Scan,
}

pub trait Rule: Send + Sync {
    fn id(&self) -> &'static str;
    fn scope(&self) -> RuleScope {
        RuleScope::Scan
    }
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
        Box::new(bidders::BidderParamsInvalid),
        Box::new(bidders::BidderTestIds),
        Box::new(bidders::BidderEndpointErrors),
        Box::new(bidders::AdsTxtMissing),
        Box::new(bidders::BiddersNotInAdsTxt),
        Box::new(bidders::AdsTxtInvalidLines),
        Box::new(bidders::SellersJsonIssues),
        Box::new(bidders::AuctionBeforeConsent),
        Box::new(bidders::BiddersWithoutConsent),
        Box::new(setup::NoPriceFloors),
        Box::new(setup::DuplicatePrebid),
        Box::new(setup::AdLibraryLoadedTwice),
        Box::new(setup::OutdatedPrebid),
        Box::new(setup::ConsentNotWired),
        Box::new(setup::ConsentTimeout),
        Box::new(visit::LaterPageViewWithoutBids),
        Box::new(visit::SpaSlotsNotReset),
        Box::new(visit::RefreshWithStaleBids),
        Box::new(visit::RefreshWithoutAuction),
        Box::new(visit::ConsentAskedAgain),
        Box::new(visit::BiddingWaitedForConsent),
        Box::new(visit::ScanIdentifiableAsAutomated),
    ]
}

/// Id of the rule that reports a visit the page could tell was automated.
pub const AUTOMATION_RULE: &str = "scan-identifiable-as-automated";

/// Id of the rule that reports page views whose bidding waited for a
/// consent choice the scan made late or not at all.
pub const CONSENT_WAIT_RULE: &str = "bidding-waited-for-consent";

pub struct RuleHit {
    pub rule: &'static str,
    pub finding: FindingDraft,
}

/// Everything the rules may look at.
pub struct RuleInput<'a> {
    pub capture: &'a Capture,
    pub telemetry: &'a ScanTelemetry,
    /// `None` when the bidder checks did not run.
    pub diagnostics: Option<&'a BidderDiagnostics>,
    pub scripts: &'a [ScriptRecord],
    /// `None` when the visit's environment was not recorded.
    pub environment: Option<&'a ScanEnvironment>,
}

pub fn evaluate(input: &RuleInput) -> Vec<RuleHit> {
    let all_events: Vec<&TrackerEvent> = input.capture.events.iter().collect();
    let events_by_page: Vec<Vec<&TrackerEvent>> = (0u32..)
        .zip(&input.telemetry.pages)
        .map(|(index, _)| {
            all_events
                .iter()
                .copied()
                .filter(|event| event.page == index)
                .collect()
        })
        .collect();
    let scan_ctx = RuleContext {
        capture: input.capture,
        telemetry: &input.telemetry.total,
        events: &all_events,
        page: None,
        scan: input.telemetry,
        diagnostics: input.diagnostics,
        scripts: input.scripts,
        environment: input.environment,
    };
    let page_count = input.telemetry.pages.len();
    let mut hits = Vec::new();
    for rule in all_rules() {
        let findings = match rule.scope() {
            RuleScope::Scan => rule.evaluate(&scan_ctx),
            RuleScope::Page => {
                let per_page = (0..page_count)
                    .map(|index| {
                        let ctx = RuleContext {
                            telemetry: &input.telemetry.by_page[index],
                            events: &events_by_page[index],
                            page: Some(&input.telemetry.pages[index]),
                            ..scan_ctx
                        };
                        rule.evaluate(&ctx)
                    })
                    .collect();
                merge_page_findings(per_page, &input.telemetry.pages)
            }
        };
        let id = rule.id();
        hits.extend(
            findings
                .into_iter()
                .map(|finding| RuleHit { rule: id, finding }),
        );
    }
    hits
}

/// The same problem on several page views is one finding. Two findings of
/// one rule are the same problem when they concern the same bidders and come
/// from different page views; a rule that reports several things about one
/// page view keeps them apart. The text is that of the page view where it
/// was most severe, the earliest on a tie.
fn merge_page_findings(per_page: Vec<Vec<FindingDraft>>, pages: &[PageView]) -> Vec<FindingDraft> {
    struct Merged {
        best: FindingDraft,
        seen: Vec<(u32, String)>,
    }
    let mut merged: Vec<Merged> = Vec::new();
    for (page, drafts) in (0u32..).zip(per_page) {
        for mut draft in drafts {
            draft.affected_bidders.sort();
            let same_problem = merged.iter_mut().find(|entry| {
                entry.best.affected_bidders == draft.affected_bidders
                    && !entry.seen.iter().any(|(seen_on, _)| *seen_on == page)
            });
            match same_problem {
                Some(entry) => {
                    entry.seen.push((page, draft.title.clone()));
                    let mut slots = std::mem::take(&mut entry.best.affected_slots);
                    for slot in std::mem::take(&mut draft.affected_slots) {
                        if !slots.contains(&slot) {
                            slots.push(slot);
                        }
                    }
                    if draft.severity > entry.best.severity {
                        entry.best = draft;
                    }
                    entry.best.affected_slots = slots;
                }
                None => merged.push(Merged {
                    seen: vec![(page, draft.title.clone())],
                    best: draft,
                }),
            }
        }
    }
    merged
        .into_iter()
        .map(|Merged { mut best, seen }| {
            best.pages = seen.iter().map(|(page, _)| *page).collect();
            if seen.len() > 1 {
                let lines: Vec<String> = seen
                    .iter()
                    .map(|(page, title)| format!("{}: {title}", page_label(pages, *page)))
                    .collect();
                best.evidence
                    .push(timeline("Per page view", lines.join("\n")));
            }
            best
        })
        .collect()
}

/// "page 2 (/sport/final)": page views are numbered from one for people.
pub(crate) fn page_label(pages: &[PageView], page: u32) -> String {
    let path = pages
        .get(page as usize)
        .and_then(|view| url::Url::parse(&view.url).ok())
        .map(|url| url.path().to_string());
    match path {
        Some(path) => format!("page {} ({path})", page + 1),
        None => format!("page {}", page + 1),
    }
}

/// What one evaluation of a rule sees.
#[derive(Clone, Copy)]
pub struct RuleContext<'a> {
    pub capture: &'a Capture,
    /// Auctions, slots and metrics of the rule's scope.
    pub telemetry: &'a Telemetry,
    /// Tracker events of the rule's scope, in time order.
    pub events: &'a [&'a TrackerEvent],
    /// The page view being evaluated; `None` in scan scope.
    pub page: Option<&'a PageView>,
    /// The whole visit, whatever the scope.
    pub scan: &'a ScanTelemetry,
    /// `None` when the bidder checks did not run.
    pub diagnostics: Option<&'a BidderDiagnostics>,
    pub scripts: &'a [ScriptRecord],
    pub environment: Option<&'a ScanEnvironment>,
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
/// Prebid has to have been on the page, with consent given, for this long
/// before "no auction ever started" says anything about it.
const GATE_MIN_WATCH_MS: f64 = 5000.0;

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

    /// Load time of the page view in scope; of the entry page in scan scope.
    pub(crate) fn page_load_ms(&self) -> Option<f64> {
        match self.page {
            Some(page) => page.load_ms,
            None => self.capture.page_load_ms,
        }
    }

    /// Scroll depth reached on the page view in scope; the deepest of the
    /// visit in scan scope.
    pub(crate) fn max_scroll_depth(&self) -> f64 {
        match self.page {
            Some(page) => page.max_scroll_depth,
            None => self.capture.max_scroll_depth,
        }
    }

    /// Whether the page view in scope came from a route change, that is,
    /// without loading a new document.
    pub(crate) fn is_route_change(&self) -> bool {
        self.page
            .is_some_and(|page| page.reached_by == PageReach::RouteChange)
    }

    /// What became of the consent dialog of the page view in scope.
    pub(crate) fn page_consent(&self) -> Option<PageConsent> {
        self.page
            .map(|page| PageConsent::of(self.capture, page.index))
    }

    /// When consent was given on the page view in scope, where that was late
    /// enough to have held its ad stack up. Until then a wait is not the
    /// site's doing.
    pub(crate) fn held_up_by_consent_until(&self) -> Option<f64> {
        self.page_consent()?.held_up_until_ms()
    }

    pub(crate) fn first_event_at(&self, matches: impl Fn(&Ev) -> bool) -> Option<f64> {
        self.events
            .iter()
            .find(|event| matches(&event.kind))
            .map(|event| event.t)
    }

    /// A pointer that merely moved counts: wrappers that wait for "a real
    /// user" start bidding on the first mouse move as well.
    pub(crate) fn first_interaction_at(&self) -> Option<f64> {
        self.first_event_at(|kind| match kind {
            Ev::PageLifecycle { phase } => {
                phase == "firstScroll" || phase == "firstInteraction" || phase == "firstPointerMove"
            }
            Ev::Scroll { depth } => *depth > 0.0,
            _ => false,
        })
    }

    pub(crate) fn prebid_config(&self) -> Option<PrebidConfig> {
        self.events.iter().find_map(|event| match &event.kind {
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
    ///
    /// A page view reached by a route change has no gate of its own: the
    /// document, and Prebid in it, were loaded pages ago.
    pub(crate) fn auction_gate(&self) -> Option<AuctionGate> {
        if self.is_route_change() {
            return None;
        }
        let prebid_ready_at =
            self.first_event_at(|kind| matches!(kind, Ev::PrebidDetected { .. }))?;
        let interaction_at_ms = self.first_interaction_at();
        let consent_at = self.held_up_by_consent_until();
        if self.auctions().is_empty() {
            // Prebid that arrived, or was allowed to start, moments before
            // the page view ended had no chance to run an auction.
            let could_start_at = prebid_ready_at.max(consent_at.unwrap_or(0.0));
            let watched_ms = self
                .page
                .map_or(f64::INFINITY, |page| page.dwell_ms - could_start_at);
            return (watched_ms >= GATE_MIN_WATCH_MS)
                .then_some(AuctionGate::NeverStarted { interaction_at_ms });
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
                // An auction right after consent was given waited for that.
                let reacted_to_consent = consent_at.is_some_and(|consent_at| {
                    (0.0..=GATE_REACTION_WINDOW_MS).contains(&(auction_at_ms - consent_at))
                });
                if !reacted_to_interaction || !prebid_sat_idle || reacted_to_consent {
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
        pages: Vec::new(),
    }
}

/// Attaches a revenue-impact estimate together with what it rests on. One
/// page view cannot support more precision than a whole percentage.
pub(crate) fn estimate_impact(finding: &mut FindingDraft, pct: f64, basis: &str) {
    let pct = pct.clamp(0.0, 100.0).round();
    finding.estimated_revenue_impact_pct = Some(pct);
    finding.evidence.push(metric(
        "How the revenue impact was estimated",
        format!("About {pct:.0}% of ad revenue at risk: {basis}. An estimate from one page view, not a measurement."),
    ));
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
