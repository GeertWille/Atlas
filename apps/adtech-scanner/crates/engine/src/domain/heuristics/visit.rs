//! Rules that need more than one page view, more than one ad request of a
//! slot, or the visit's environment: what a single passive page load cannot
//! show.

use std::collections::BTreeSet;

use adscan_model::{
    AdSlot, FindingCategory, PageReach, PageView, ScanEnvironment, Severity, SlotRequest,
    TrackerEvent, TrackerEventKind as Ev,
};

use super::{AUTOMATION_RULE, Rule, RuleContext, heuristic, ms, page_label};
use crate::domain::finding::{FindingDraft, metric, timeline};
use crate::domain::telemetry::{ScanTelemetry, auction_covers};

/// A page view shorter than this had no fair chance to request its ads.
const MIN_DWELL_MS: f64 = 5000.0;
const MIN_UNBID_SLOTS: usize = 2;
const MIN_LIVE_SLOTS: usize = 2;
/// An auction that starts later than this after a consent prompt came back
/// was held up by it.
const CONSENT_DELAY_MS: f64 = 2000.0;
const MIN_REQUESTS_TO_SUSPECT_FILTERING: u32 = 2;

fn events_on<'a>(
    ctx: &RuleContext<'a>,
    page: u32,
) -> impl Iterator<Item = &'a TrackerEvent> + use<'a> {
    ctx.capture
        .events
        .iter()
        .filter(move |event| event.page == page)
}

fn slots_on(scan: &ScanTelemetry, page: u32) -> &[AdSlot] {
    scan.by_page
        .get(page as usize)
        .map_or(&[], |telemetry| &telemetry.slots)
}

/// Whether an auction on the slot's own page view asked for bids for it.
fn auctioned_on_its_page(scan: &ScanTelemetry, slot: &AdSlot) -> bool {
    scan.by_page
        .get(slot.page as usize)
        .is_some_and(|telemetry| {
            telemetry
                .auctions
                .iter()
                .any(|auction| auction_covers(auction, slot))
        })
}

/// The consent prompt is on screen and nobody answered it: a wrapper that
/// holds its bids back is doing what it should.
fn consent_undecided<'a>(events: impl Iterator<Item = &'a TrackerEvent>) -> bool {
    events
        .filter_map(|event| match &event.kind {
            Ev::TcfState { event_status, .. } => Some(event_status.as_deref()),
            _ => None,
        })
        .last()
        == Some(Some("cmpuishown"))
}

fn paths_differ(page: &PageView, previous: &PageView) -> bool {
    let path = |view: &PageView| {
        url::Url::parse(&view.url)
            .ok()
            .map(|url| url.path().to_string())
    };
    path(page) != path(previous)
}

fn labels(pages: &[PageView], indexes: &[u32]) -> String {
    indexes
        .iter()
        .map(|index| page_label(pages, *index))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Slots of a later, freshly loaded page view that went to the ad server
/// without bids although the same containers were auctioned on an earlier
/// page view. Empty when the page view gives no ground for the conclusion.
pub(super) fn unbid_after_navigation(ctx: &RuleContext, page: u32) -> Vec<String> {
    let scan = ctx.scan;
    let Some(view) = scan.pages.get(page as usize) else {
        return Vec::new();
    };
    let loaded_a_document = page > 0 && view.reached_by != PageReach::RouteChange;
    if !loaded_a_document || view.dwell_ms < MIN_DWELL_MS || consent_undecided(events_on(ctx, page))
    {
        return Vec::new();
    }
    let auctioned_before = |element_id: &str| {
        (0..page).any(|earlier| {
            slots_on(scan, earlier)
                .iter()
                .any(|slot| slot.element_id == element_id && auctioned_on_its_page(scan, slot))
        })
    };
    let unbid: Vec<String> = slots_on(scan, page)
        .iter()
        .filter(|slot| slot.requested_at_ms.is_some())
        .filter(|slot| slot.had_prebid_targeting == Some(false))
        .filter(|slot| !auctioned_on_its_page(scan, slot))
        .filter(|slot| auctioned_before(&slot.element_id))
        .map(|slot| slot.element_id.clone())
        .collect();
    if unbid.len() >= MIN_UNBID_SLOTS {
        unbid
    } else {
        Vec::new()
    }
}

/// R1. Header bidding works on the first page of a session and is skipped
/// on the pages after it: state kept across navigations (a "last auction"
/// timestamp, a session flag) makes the wrapper think it already bid.
pub struct LaterPageViewWithoutBids;

impl Rule for LaterPageViewWithoutBids {
    fn id(&self) -> &'static str {
        "later-page-view-without-bids"
    }

    fn evaluate(&self, ctx: &RuleContext) -> Vec<FindingDraft> {
        let affected: Vec<(u32, Vec<String>)> = (1u32..)
            .zip(ctx.scan.pages.iter().skip(1))
            .map(|(page, _)| (page, unbid_after_navigation(ctx, page)))
            .filter(|(_, unbid)| !unbid.is_empty())
            .collect();
        let Some((_, first_unbid)) = affected.first() else {
            return Vec::new();
        };
        let pages: Vec<u32> = affected.iter().map(|(page, _)| *page).collect();
        let mut finding = heuristic(
            Severity::High,
            FindingCategory::LogicBug,
            0.8,
            "Later page views are sold without header bidding",
        );
        finding.description = format!(
            "On {} the page requested {} ad slot(s) without any Prebid auction for them and without `hb_*` \
             targeting, although the same slots were auctioned on an earlier page view of this visit. Header \
             bidding runs for the first page of a session and is skipped afterwards. That is the signature of \
             state that survives the navigation, such as a \"last auction\" timestamp or a \"already requested\" \
             flag in sessionStorage, localStorage or a cookie.",
            labels(&ctx.scan.pages, &pages),
            first_unbid.len(),
        );
        finding.recommendation = "Find what the wrapper stores between page views (search for `sessionStorage`, \
             `localStorage` and cookie writes next to `requestBids`) and make every page view start its own auction. \
             A guard against double auctions must be scoped to one page view, not to the session."
            .into();
        finding.impact =
            "Every page view after the first earns without header bidding competition".into();
        for (page, unbid) in &affected {
            finding.evidence.push(timeline(
                format!("Slots without bids on {}", page_label(&ctx.scan.pages, *page)),
                format!(
                    "{}: requested with no auction on this page view and no hb_* keys; auctioned earlier in the visit",
                    unbid.join(", ")
                ),
            ));
        }
        finding.evidence.push(metric(
            "Auctions per page view",
            ctx.scan
                .pages
                .iter()
                .map(|view| {
                    format!(
                        "{}: {} auction(s), {} slot(s)",
                        page_label(&ctx.scan.pages, view.index),
                        view.metrics.auction_count,
                        view.metrics.slot_count
                    )
                })
                .collect::<Vec<_>>()
                .join("\n"),
        ));
        let mut slots: Vec<String> = affected
            .iter()
            .flat_map(|(_, unbid)| unbid.iter().cloned())
            .collect();
        slots.sort();
        slots.dedup();
        finding.affected_slots = slots;
        finding.pages = pages;
        vec![finding]
    }
}

/// R2. A single-page application changed route and left the ad slots of the
/// previous view in place: GPT refuses the new definitions, or the old slots
/// are simply never asked for an ad again.
pub struct SpaSlotsNotReset;

struct StaleRoute {
    page: u32,
    slots: Vec<String>,
    rejected: bool,
}

impl Rule for SpaSlotsNotReset {
    fn id(&self) -> &'static str {
        "spa-slots-not-reset"
    }

    fn evaluate(&self, ctx: &RuleContext) -> Vec<FindingDraft> {
        let stale: Vec<StaleRoute> = (1u32..)
            .zip(ctx.scan.pages.windows(2))
            .filter(|(_, pair)| {
                pair[1].reached_by == PageReach::RouteChange && paths_differ(&pair[1], &pair[0])
            })
            .filter_map(|(page, pair)| stale_route(ctx, page, &pair[1]))
            .collect();
        if stale.is_empty() {
            return Vec::new();
        }
        let rejected = stale.iter().any(|route| route.rejected);
        let pages: Vec<u32> = stale.iter().map(|route| route.page).collect();
        let (severity, confidence) = if rejected {
            (Severity::High, 0.85)
        } else {
            (Severity::Medium, 0.6)
        };
        let mut finding = heuristic(
            severity,
            FindingCategory::LogicBug,
            confidence,
            "Ad slots are not reset when the page changes route",
        );
        finding.description = if rejected {
            format!(
                "The site changes pages without reloading (a single-page application). On {} the wrapper defined \
                 its ad slots again while the slots of the previous view still existed, and Google Publisher Tag \
                 rejected the new definitions (`defineSlot` returned null: the container already has a slot). The \
                 new page view therefore requests no ads for those containers, and any auction run for it is wasted.",
                labels(&ctx.scan.pages, &pages),
            )
        } else {
            format!(
                "The site changes pages without reloading (a single-page application). On {} an auction ran, but \
                 none of the ad slots that were already on the page was requested, destroyed or defined again. The \
                 new page view shows the previous view's ads, or none, and its bids are never used.",
                labels(&ctx.scan.pages, &pages),
            )
        };
        finding.recommendation = "On every route change call `googletag.destroySlots()` for the slots of the old \
             view and `pbjs.removeAdUnit()` for their ad units, then define the slots of the new view, run the \
             auction and request the ads. Reusing a slot is fine too, but then it must be refreshed after the new \
             auction."
            .into();
        finding.impact =
            "Page views after an in-app navigation serve no new ads; their bids are thrown away"
                .into();
        for route in &stale {
            finding.evidence.push(timeline(
                format!(
                    "Route change to {}",
                    page_label(&ctx.scan.pages, route.page)
                ),
                if route.rejected {
                    format!("defineSlot rejected for: {}", route.slots.join(", "))
                } else {
                    format!(
                        "slots left untouched for the whole page view: {}",
                        route.slots.join(", ")
                    )
                },
            ));
        }
        let mut slots: Vec<String> = stale
            .iter()
            .flat_map(|route| route.slots.iter().cloned())
            .collect();
        slots.sort();
        slots.dedup();
        finding.affected_slots = slots;
        finding.pages = pages;
        vec![finding]
    }
}

fn stale_route(ctx: &RuleContext, page: u32, view: &PageView) -> Option<StaleRoute> {
    let rejected: BTreeSet<&str> = events_on(ctx, page)
        .filter_map(|event| match &event.kind {
            Ev::SlotDefinitionRejected { element_id } => Some(element_id.as_str()),
            _ => None,
        })
        .collect();
    if !rejected.is_empty() {
        return Some(StaleRoute {
            page,
            slots: rejected.into_iter().map(str::to_string).collect(),
            rejected: true,
        });
    }

    let auction_ran = ctx
        .scan
        .by_page
        .get(page as usize)
        .is_some_and(|telemetry| !telemetry.auctions.is_empty());
    if !auction_ran || view.dwell_ms < MIN_DWELL_MS {
        return None;
    }
    let live = live_slots(ctx, page);
    let touched: BTreeSet<&str> = events_on(ctx, page)
        .flat_map(|event| match &event.kind {
            Ev::SlotDefined { element_id, .. } | Ev::SlotRequested { element_id, .. } => {
                vec![element_id.as_str()]
            }
            Ev::SlotsDestroyed { element_ids } => element_ids.iter().map(String::as_str).collect(),
            _ => Vec::new(),
        })
        .collect();
    let untouched = live.iter().all(|slot| !touched.contains(slot.as_str()));
    (live.len() >= MIN_LIVE_SLOTS && untouched).then(|| StaleRoute {
        page,
        slots: live.into_iter().collect(),
        rejected: false,
    })
}

/// Slots that exist in the document when `page` starts: defined on an
/// earlier page view of the same document and not destroyed since.
fn live_slots(ctx: &RuleContext, page: u32) -> BTreeSet<String> {
    let mut live = BTreeSet::new();
    for earlier in ctx.scan.document_of(page)..page {
        for event in events_on(ctx, earlier) {
            match &event.kind {
                Ev::SlotDefined { element_id, .. } => {
                    live.insert(element_id.clone());
                }
                Ev::SlotsDestroyed { element_ids } => {
                    for element_id in element_ids {
                        live.remove(element_id);
                    }
                }
                _ => {}
            }
        }
    }
    live
}

/// Whether an auction for the slot ended after `after` and no later than
/// `until`, both ms into the slot's page view.
fn auction_ended_between(scan: &ScanTelemetry, slot: &AdSlot, after: f64, until: f64) -> bool {
    let document = scan.document_of(slot.page);
    let window = scan.absolute_ms(slot.page, after)..=scan.absolute_ms(slot.page, until);
    scan.total
        .auctions
        .iter()
        .filter(|auction| scan.document_of(auction.page) == document)
        .filter(|auction| auction_covers(auction, slot))
        .filter_map(|auction| Some(scan.absolute_ms(auction.page, auction.ended_at_ms?)))
        .any(|ended_at| ended_at > *window.start() && window.contains(&ended_at))
}

/// Pairs of consecutive ad requests of each slot, with the slot.
fn refreshes(scan: &ScanTelemetry) -> impl Iterator<Item = (&AdSlot, &SlotRequest, &SlotRequest)> {
    scan.total.slots.iter().flat_map(|slot| {
        slot.requests
            .windows(2)
            .map(move |pair| (slot, &pair[0], &pair[1]))
    })
}

fn slot_label(scan: &ScanTelemetry, slot: &AdSlot) -> String {
    if scan.pages.len() > 1 {
        format!(
            "Slot {} on {}",
            slot.element_id,
            page_label(&scan.pages, slot.page)
        )
    } else {
        format!("Slot {}", slot.element_id)
    }
}

fn pages_and_slots(slots: &[&AdSlot]) -> (Vec<u32>, Vec<String>) {
    let pages: BTreeSet<u32> = slots.iter().map(|slot| slot.page).collect();
    let ids: BTreeSet<String> = slots.iter().map(|slot| slot.element_id.clone()).collect();
    (pages.into_iter().collect(), ids.into_iter().collect())
}

/// R3. A slot is refreshed with the targeting of a bid that was already
/// used: Prebid renders a bid once, so the second impression has no ad.
pub struct RefreshWithStaleBids;

impl Rule for RefreshWithStaleBids {
    fn id(&self) -> &'static str {
        "refresh-with-stale-bids"
    }

    fn evaluate(&self, ctx: &RuleContext) -> Vec<FindingDraft> {
        let scan = ctx.scan;
        let stale: Vec<(&AdSlot, &SlotRequest, &SlotRequest)> = refreshes(scan)
            .filter(|(_, previous, refresh)| {
                previous.prebid_ad_id.is_some() && previous.prebid_ad_id == refresh.prebid_ad_id
            })
            .filter(|(slot, previous, refresh)| {
                !auction_ended_between(scan, slot, previous.at_ms, refresh.at_ms)
            })
            .collect();
        if stale.is_empty() {
            return Vec::new();
        }
        let slots: Vec<&AdSlot> = stale.iter().map(|(slot, _, _)| *slot).collect();
        let mut finding = heuristic(
            Severity::High,
            FindingCategory::LogicBug,
            0.9,
            "Slots are refreshed with bids that were already used",
        );
        finding.description = format!(
            "{} ad refresh(es) went to the ad server carrying the same Prebid bid (`hb_adid`) as the request \
             before, with no new auction in between. A Prebid bid can be rendered once. When the ad server picks \
             the header bidding line item again, the creative cannot render and the refreshed impression stays \
             empty; when it does not, the stale price still distorts the ad server's decision.",
            stale.len(),
        );
        finding.recommendation = "Refresh through the auction: call `pbjs.requestBids` for the slot's ad unit and \
             call `googletag.pubads().refresh([slot])` from its `bidsBackHandler`, after \
             `pbjs.setTargetingForGPTAsync`. Clear the old keys first (`slot.clearTargeting()` or Prebid's \
             `targetingControls`)."
            .into();
        finding.impact =
            "Refreshed impressions are sold with dead bids: empty ads and lost refresh revenue"
                .into();
        for (slot, previous, refresh) in &stale {
            finding.evidence.push(timeline(
                slot_label(scan, slot),
                format!(
                    "requested at {} and again at {} with the same hb_adid {}; no auction for it ended in between",
                    ms(previous.at_ms),
                    ms(refresh.at_ms),
                    refresh.prebid_ad_id.as_deref().unwrap_or_default(),
                ),
            ));
        }
        (finding.pages, finding.affected_slots) = pages_and_slots(&slots);
        vec![finding]
    }
}

/// R4. A slot that was sold with header bids the first time is refreshed
/// straight from the ad server, with no auction and no bids.
pub struct RefreshWithoutAuction;

impl Rule for RefreshWithoutAuction {
    fn id(&self) -> &'static str {
        "refresh-without-auction"
    }

    fn evaluate(&self, ctx: &RuleContext) -> Vec<FindingDraft> {
        let scan = ctx.scan;
        let bare: Vec<(&AdSlot, &SlotRequest, &SlotRequest)> = refreshes(scan)
            .filter(|(slot, _, _)| slot.requests[0].had_prebid_targeting)
            .filter(|(_, _, refresh)| !refresh.had_prebid_targeting)
            .filter(|(slot, previous, refresh)| {
                !auction_ended_between(scan, slot, previous.at_ms, refresh.at_ms)
            })
            .collect();
        if bare.is_empty() {
            return Vec::new();
        }
        let slots: Vec<&AdSlot> = bare.iter().map(|(slot, _, _)| *slot).collect();
        let mut finding = heuristic(
            Severity::Medium,
            FindingCategory::Configuration,
            0.7,
            "Slots are refreshed without a new auction",
        );
        finding.description = format!(
            "{} ad refresh(es) were requested without `hb_*` targeting and without a Prebid auction since the \
             request before, on slots whose first request did carry header bids. Only the first impression of \
             these slots competes with header bidding; every refresh is filled by the ad server alone.",
            bare.len(),
        );
        finding.recommendation = "Run a Prebid auction for the slot before each refresh and refresh from the \
             `bidsBackHandler`. If an ad-refresh library does the refreshing, connect it to Prebid (most have a \
             Prebid hook) instead of calling `pubads().refresh()` directly."
            .into();
        finding.impact = "Refreshed impressions earn without header bidding competition".into();
        for (slot, previous, refresh) in &bare {
            finding.evidence.push(timeline(
                slot_label(scan, slot),
                format!(
                    "requested at {} with hb_* keys: {}; refreshed at {} with none and no auction in between",
                    ms(previous.at_ms),
                    if previous.had_prebid_targeting { "yes" } else { "no" },
                    ms(refresh.at_ms),
                ),
            ));
        }
        (finding.pages, finding.affected_slots) = pages_and_slots(&slots);
        vec![finding]
    }
}

/// R5. The consent prompt comes back on a later page view although the
/// choice was already made in this visit: the choice is not stored, and
/// every page view waits for consent again.
pub struct ConsentAskedAgain;

impl Rule for ConsentAskedAgain {
    fn id(&self) -> &'static str {
        "consent-asked-again"
    }

    fn evaluate(&self, ctx: &RuleContext) -> Vec<FindingDraft> {
        let status_on = |page: u32, wanted: &str| {
            events_on(ctx, page).any(|event| {
                matches!(&event.kind, Ev::TcfState { event_status, .. }
                    if event_status.as_deref() == Some(wanted))
            })
        };
        let mut decided = ctx.capture.consent_accepted;
        let mut asked_again: Vec<u32> = Vec::new();
        for (page, _) in (0u32..).zip(&ctx.scan.pages) {
            if page > 0 && decided && status_on(page, "cmpuishown") {
                asked_again.push(page);
            }
            decided |= status_on(page, "useractioncomplete");
        }
        if asked_again.is_empty() {
            return Vec::new();
        }
        let held_up: Vec<u32> = asked_again
            .iter()
            .copied()
            .filter(|page| {
                ctx.scan.pages[*page as usize]
                    .metrics
                    .time_to_first_auction_ms
                    .is_none_or(|started_at| started_at > CONSENT_DELAY_MS)
            })
            .collect();
        let mut finding = heuristic(
            if held_up.is_empty() {
                Severity::Medium
            } else {
                Severity::High
            },
            FindingCategory::Consent,
            0.75,
            "The consent prompt comes back on later page views",
        );
        finding.description = format!(
            "The consent choice was made earlier in this visit, and the consent platform showed its prompt again \
             on {}. The choice is not being stored or not being read back, so every page view starts undecided.{}",
            labels(&ctx.scan.pages, &asked_again),
            if held_up.is_empty() {
                String::new()
            } else {
                format!(
                    " On {} bidding started more than {} into the page view or not at all.",
                    labels(&ctx.scan.pages, &held_up),
                    ms(CONSENT_DELAY_MS),
                )
            },
        );
        finding.recommendation = "Check that the consent cookie is written for the whole site (domain and path, \
             `SameSite`, lifetime) and that the consent platform is configured to read it on every page. This \
             holds within one visit, so it is not an effect of a fresh browser profile."
            .into();
        finding.impact =
            "Visitors are asked again on every page and bidding waits each time".into();
        finding.evidence.push(timeline(
            "Consent prompt per page view",
            (0u32..)
                .zip(&ctx.scan.pages)
                .map(|(page, view)| {
                    format!(
                        "{}: prompt {}, first auction {}",
                        page_label(&ctx.scan.pages, page),
                        if status_on(page, "cmpuishown") {
                            "shown"
                        } else {
                            "not shown"
                        },
                        view.metrics
                            .time_to_first_auction_ms
                            .map_or("never".to_string(), ms),
                    )
                })
                .collect::<Vec<_>>()
                .join("\n"),
        ));
        finding.pages = asked_again;
        vec![finding]
    }
}

/// The indicators by which a page could tell this visit was automated, as a
/// clause: "navigator.webdriver was true, the browser ran headless".
pub(super) fn automation_indicators(environment: Option<&ScanEnvironment>) -> Option<String> {
    let environment = environment?;
    let mut indicators = Vec::new();
    if environment.webdriver == Some(true) {
        indicators.push("navigator.webdriver was true");
    }
    if environment.headless {
        indicators.push("the browser ran headless");
    }
    (!indicators.is_empty()).then(|| indicators.join(", "))
}

/// R6. Not a fault of the page: the visit was recognisable as automated,
/// and something on the page had a reason to act on that. A bidder that
/// stays silent may be filtering the scan, not broken.
pub struct ScanIdentifiableAsAutomated;

impl Rule for ScanIdentifiableAsAutomated {
    fn id(&self) -> &'static str {
        AUTOMATION_RULE
    }

    fn evaluate(&self, ctx: &RuleContext) -> Vec<FindingDraft> {
        let Some(indicators) = automation_indicators(ctx.environment) else {
            return Vec::new();
        };
        let detectors: Vec<&str> = ctx
            .environment
            .map(|environment| {
                environment
                    .detectors
                    .iter()
                    .map(|detector| detector.vendor.as_str())
                    .collect()
            })
            .unwrap_or_default();
        let silent: Vec<&str> = ctx
            .bidder_stats()
            .into_iter()
            .filter(|(_, stats)| {
                stats.requests >= MIN_REQUESTS_TO_SUSPECT_FILTERING && stats.bids == 0
            })
            .map(|(bidder, _)| bidder)
            .collect();
        if detectors.is_empty() && silent.is_empty() {
            return Vec::new();
        }
        let mut reasons = vec![indicators.clone()];
        if !detectors.is_empty() {
            reasons.push(format!(
                "{} {} on the page",
                detectors.join(" and "),
                if detectors.len() == 1 { "is" } else { "are" }
            ));
        }
        let mut finding = heuristic(
            Severity::Info,
            FindingCategory::Configuration,
            0.5,
            "This scan was identifiable as automated traffic",
        );
        finding.description = format!(
            "Bidders may be filtering this scan as automated traffic: {}. Repeat as \"I'll browse myself\" or \
             with the browser shown before concluding a bidder is broken.",
            reasons.join(", "),
        );
        finding.recommendation =
            "Nothing to fix on the site. Treat missing bids in this scan with caution and \
             confirm them with a visit a person drives."
                .into();
        finding.impact =
            "Bid counts and fill in this scan may be lower than what real visitors get".into();
        finding
            .evidence
            .push(metric("What the page could see", indicators));
        if let Some(environment) = ctx.environment
            && !environment.detectors.is_empty()
        {
            finding.evidence.push(metric(
                "Detection vendors on the page",
                environment
                    .detectors
                    .iter()
                    .map(|detector| format!("{} ({})", detector.vendor, detector.host))
                    .collect::<Vec<_>>()
                    .join("\n"),
            ));
        }
        if !silent.is_empty() {
            finding.evidence.push(metric(
                "Bidders that were asked and never bid",
                silent.join(", "),
            ));
            finding.affected_bidders = silent.into_iter().map(str::to_string).collect();
        }
        vec![finding]
    }
}
