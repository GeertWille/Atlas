//! Compact text views of the telemetry for the analysis model.
//!
//! Page views are numbered from one here. Lines of a visit with more than
//! one page view say which page view they belong to; a single-page visit
//! reads as it always did.

use adscan_model::{
    AdSlot, AdsTxtStatus, Auction, BidderDiagnostics, BrowserUse, BrowserbaseProxyKind,
    BrowserbaseVisit, Capture, PageView, ScanEnvironment, TrackerEventKind as Ev,
};
use serde_json::json;

/// The page table shows at most this many page views.
const MAX_PAGE_ROWS: usize = 8;
/// A `requestBids` call that Prebid went on with this much later was held
/// by a hook; less is the time the call itself takes.
const HELD_CALL_MS: f64 = 50.0;
/// What holds a `requestBids` call. The tracker only sees that it was held,
/// not by which hook.
const CALL_HOLDERS: &str = "hooks on requestBids (such as consent management or currency)";

/// Says which records the views below cover and how they are labelled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// One page view only; nothing is labelled.
    Page(u32),
    /// Every page view, each line labelled with its page view when the
    /// visit has more than one.
    Visit { pages: usize },
}

impl Scope {
    fn covers(self, page: u32) -> bool {
        match self {
            Self::Page(only) => only == page,
            Self::Visit { .. } => true,
        }
    }

    fn label(self, page: u32) -> String {
        match self {
            Self::Visit { pages } if pages > 1 => format!("page {} ", page + 1),
            Self::Page(_) | Self::Visit { .. } => String::new(),
        }
    }
}

/// Where the browser ran when it was not on the user's machine, stated as
/// fact: a cloud address, another clock and a round trip per held request
/// all change what a page does, and none of them is a bug of the page.
fn browserbase_line(visit: &BrowserbaseVisit) -> String {
    let exit = match visit.proxy_kind {
        BrowserbaseProxyKind::None => "its cloud region (a data-centre address)".to_string(),
        BrowserbaseProxyKind::External => "the user's own proxy".to_string(),
        BrowserbaseProxyKind::Browserbase => format!(
            "a residential exit asked for in {}",
            visit.proxy_country.as_deref().unwrap_or("no named country")
        ),
    };
    let timezone = match (&visit.applied_timezone, &visit.original_timezone) {
        (Some(applied), _) => format!("{applied} (set by the scanner)"),
        (None, Some(original)) => original.clone(),
        (None, None) => "unknown".to_string(),
    };
    format!(
        "the browser ran in Browserbase's cloud, region {}, leaving through {exit}; timezone {timezone}, language {}; \
         it runs headless without saying so in its user agent; CAPTCHAs are solved automatically: {}; \
         round trip between scanner and browser {} ms, which delays every frame and document the scanner holds for a moment",
        visit.region.as_str(),
        visit.language.as_deref().unwrap_or("unknown"),
        visit.settings.solve_captchas,
        visit
            .round_trip_ms
            .map_or("unknown".to_string(), |ms| format!("{ms:.0}")),
    )
}

/// How the visit was made and what its page views were.
/// `consent` is what is known about consent dialogs that were answered late
/// or not at all (`domain::consent_wait::accounts`): stated as fact, so the
/// model does not have to guess why a page view started late.
pub fn visit(
    environment: Option<&ScanEnvironment>,
    pages: &[PageView],
    consent: &[String],
) -> String {
    let mut out = Vec::new();
    if let Some(environment) = environment {
        out.push(format!(
            "visitor: {:?} (seed {}), browser {}, {} profile, proxy {:?}",
            environment.visitor,
            environment.seed,
            if environment.headless {
                "headless"
            } else {
                "shown"
            },
            if environment.persistent_profile {
                "persistent"
            } else {
                "throwaway"
            },
            environment.proxy.kind,
        ));
        if let BrowserUse::Browserbase(visit) = &environment.browser {
            out.push(browserbase_line(visit));
        }
        out.push(format!(
            "what the pages could see: navigator.webdriver={} headless={} userAgentOverridden={} viewportOverridden={} detectionVendors=[{}]",
            flag(environment.webdriver),
            // A Browserbase browser runs headless without any sign of
            // it that a page can read.
            environment.headless && environment.browser == BrowserUse::Local,
            environment.user_agent_overridden,
            environment.viewport_overridden,
            environment
                .detectors
                .iter()
                .map(|detector| detector.vendor.as_str())
                .collect::<Vec<_>>()
                .join(", "),
        ));
    }
    out.push(format!(
        "{} page view(s). Times of auctions, slots and events count from the start of their page view.",
        pages.len()
    ));
    out.extend(pages.iter().take(MAX_PAGE_ROWS).map(|page| {
        format!(
            "page {} {} reachedBy={:?} startedAt={} dwell={} load={} maxScroll={:.0}% auctions={} slots={} emptySlots={} firstAuction={}",
            page.index + 1,
            page.url,
            page.reached_by,
            ms(Some(page.started_at_ms)),
            ms(Some(page.dwell_ms)),
            ms(page.load_ms),
            page.max_scroll_depth * 100.0,
            page.metrics.auction_count,
            page.metrics.slot_count,
            page.metrics.empty_slot_count,
            ms(page.metrics.time_to_first_auction_ms),
        )
    }));
    if pages.len() > MAX_PAGE_ROWS {
        out.push(format!(
            "... and {} more page view(s)",
            pages.len() - MAX_PAGE_ROWS
        ));
    }
    out.extend(
        consent.iter().map(|account| {
            format!("consent (established by the scanner, not a guess): {account}.")
        }),
    );
    out.join("\n")
}

pub fn auctions_overview(auctions: &[Auction], scope: Scope) -> String {
    let mut auctions = auctions
        .iter()
        .filter(|auction| scope.covers(auction.page))
        .peekable();
    if auctions.peek().is_none() {
        return "No Prebid auction started during the observation.".into();
    }
    lines(auctions.map(|auction| {
        let bids = auction
            .ad_units
            .iter()
            .map(|unit| unit.bids.len())
            .sum::<usize>();
        format!(
            "{}auction {} start={} end={} timeout={} adUnits=[{}] bids={}",
            scope.label(auction.page),
            short_id(&auction.id),
            ms(Some(auction.started_at_ms)),
            ms(auction.ended_at_ms),
            ms(auction.timeout_ms),
            auction
                .ad_units
                .iter()
                .map(|unit| unit.code.as_str())
                .collect::<Vec<_>>()
                .join(", "),
            bids,
        )
    }))
}

pub fn auctions_detail(auctions: &[Auction], scope: Scope) -> String {
    let mut out = Vec::new();
    for auction in auctions.iter().filter(|auction| scope.covers(auction.page)) {
        out.push(format!(
            "{}auction {} start={} end={} timeout={}",
            scope.label(auction.page),
            short_id(&auction.id),
            ms(Some(auction.started_at_ms)),
            ms(auction.ended_at_ms),
            ms(auction.timeout_ms),
        ));
        for unit in &auction.ad_units {
            out.push(format!(
                "  adUnit {} sizes=[{}] winner={}",
                unit.code,
                unit.sizes.join(","),
                unit.winning_bidder.as_deref().unwrap_or("none"),
            ));
            for bid in &unit.bids {
                out.push(format!(
                    "    {} status={:?} cpm={} requestedAt={} responseTime={}",
                    bid.bidder,
                    bid.status,
                    bid.cpm.map_or("-".to_string(), |cpm| format!("{cpm:.2}")),
                    ms(bid.requested_at_ms),
                    ms(bid.response_time_ms),
                ));
            }
        }
    }
    if out.is_empty() {
        return auctions_overview(&[], scope);
    }
    out.join("\n")
}

pub fn slots(slots: &[AdSlot], scope: Scope) -> String {
    let mut slots = slots
        .iter()
        .filter(|slot| scope.covers(slot.page))
        .peekable();
    if slots.peek().is_none() {
        return "No GPT slot was defined during the observation.".into();
    }
    lines(slots.map(|slot| {
        // Only a refreshed slot has more to say than its first request.
        let refreshes = if slot.requests.len() > 1 {
            format!(
                " requests=[{}]",
                slot.requests
                    .iter()
                    .map(|request| format!(
                        "{} hb={} hb_adid={}",
                        ms(Some(request.at_ms)),
                        if request.had_prebid_targeting { "yes" } else { "no" },
                        request.prebid_ad_id.as_deref().unwrap_or("-"),
                    ))
                    .collect::<Vec<_>>()
                    .join("; ")
            )
        } else {
            String::new()
        };
        format!(
            "{}slot {} path={} sizes=[{}] outcome={:?} inInitialViewport={} defined={} requested={} rendered={} hbTargetingOnRequest={} requestedBeforeAuctionEnd={}{refreshes}",
            scope.label(slot.page),
            slot.element_id,
            slot.ad_unit_path.as_deref().unwrap_or("-"),
            slot.sizes.join(","),
            slot.outcome,
            flag(slot.in_initial_viewport),
            ms(slot.defined_at_ms),
            ms(slot.requested_at_ms),
            ms(slot.rendered_at_ms),
            flag(slot.had_prebid_targeting),
            flag(slot.requested_before_auction_end),
        )
    }))
}

pub fn prebid_config(capture: &Capture) -> String {
    let entries: Vec<String> = capture
        .events
        .iter()
        .filter_map(|event| match &event.kind {
            Ev::PrebidDetected {
                global_name,
                version,
                modules,
            } => Some(format!(
                "t={} prebid global `{global_name}` version={} modules=[{}]",
                ms(Some(event.t)),
                version.as_deref().unwrap_or("unknown"),
                modules.join(", "),
            )),
            Ev::PrebidConfig {
                bidder_timeout,
                price_granularity,
                has_floors,
                has_user_sync,
                has_consent_management,
                currency,
                raw,
            } => Some(format!(
                "t={} getConfig: {}\nraw: {raw}",
                ms(Some(event.t)),
                json!({
                    "bidderTimeout": bidder_timeout,
                    "priceGranularity": price_granularity,
                    "hasFloors": has_floors,
                    "hasUserSync": has_user_sync,
                    "hasConsentManagement": has_consent_management,
                    "currency": currency,
                }),
            )),
            _ => None,
        })
        .collect();
    if entries.is_empty() {
        "Prebid was not detected on the page.".into()
    } else {
        entries.join("\n")
    }
}

pub fn console(capture: &Capture, scope: Scope) -> String {
    let prebid_logs = capture
        .events
        .iter()
        .filter(|event| scope.covers(event.page))
        .filter_map(|event| match &event.kind {
            Ev::PrebidLog { level, message } => Some(format!(
                "{}t={} prebid {level}: {message}",
                scope.label(event.page),
                ms(Some(event.t))
            )),
            _ => None,
        });
    let console_lines = capture
        .console
        .iter()
        .filter(|line| scope.covers(line.page))
        .map(|line| {
            format!(
                "{}t={} console {}: {}",
                scope.label(line.page),
                ms(Some(line.t)),
                line.level,
                line.text
            )
        });
    let all: Vec<String> = prebid_logs.chain(console_lines).collect();
    if all.is_empty() {
        "No console output or Prebid log lines were captured.".into()
    } else {
        all.join("\n")
    }
}

/// Page lifecycle, scroll and library-detection events of one page view in
/// time order.
pub fn page_events(capture: &Capture, page: u32) -> String {
    let (load_ms, max_scroll_depth) = capture
        .pages
        .get(page as usize)
        .map_or((capture.page_load_ms, capture.max_scroll_depth), |view| {
            (view.load_ms, view.max_scroll_depth)
        });
    let mut out = vec![format!(
        "navigation start t=0, load event {}, max scroll depth {:.0}%, consent accepted: {}",
        ms(load_ms),
        max_scroll_depth * 100.0,
        capture.consent_accepted,
    )];
    let events = capture.events.iter().filter(|event| event.page == page);
    out.extend(events.filter_map(|event| {
        let what = match &event.kind {
            Ev::PageLifecycle { phase } => phase.clone(),
            Ev::Scroll { depth } => format!("scroll depth {:.0}%", depth * 100.0),
            Ev::PrebidDetected { .. } => "prebid detected".into(),
            Ev::GptDetected { .. } => "gpt detected".into(),
            Ev::AmazonDetected {} => "amazon apstag detected".into(),
            Ev::BidsRequested {
                ad_unit_codes,
                ad_unit_count,
                timeout,
                released_at,
                still_held,
            } => format!(
                "pbjs.requestBids called: adUnitCodes=[{}] adUnitsToAuction={} timeout={}{}",
                ad_unit_codes.join(", "),
                ad_unit_count.map_or("unknown".to_string(), |count| count.to_string()),
                ms(*timeout),
                match (released_at, still_held) {
                    (Some(at), _) if *at - event.t >= HELD_CALL_MS => format!(
                        "; {CALL_HOLDERS} held it until t={} ({} later)",
                        ms(Some(*at)),
                        ms(Some(*at - event.t)),
                    ),
                    (None, true) =>
                        format!("; still held by {CALL_HOLDERS} when the page view ended"),
                    // Let through at once, or not known (a scan stored
                    // before releases were recorded).
                    _ => String::new(),
                },
            ),
            Ev::AdUnitsAdded { registered } => format!(
                "pbjs.addAdUnits called: adUnitsRegisteredAfter={}",
                registered.map_or("unknown".to_string(), |count| count.to_string()),
            ),
            Ev::AuctionInit { auction_id, .. } => format!("auctionInit {}", short_id(auction_id)),
            Ev::AuctionEnd { auction_id } => format!("auctionEnd {}", short_id(auction_id)),
            Ev::SetTargeting { ad_unit_codes } => {
                format!("setTargeting [{}]", ad_unit_codes.join(", "))
            }
            Ev::SlotRequested { element_id, .. } => format!("gpt request {element_id}"),
            Ev::RouteChange {
                url,
                navigation_type,
            } => format!("route change ({navigation_type}) to {url}"),
            Ev::SlotsDestroyed { element_ids } => {
                format!("gpt destroySlots [{}]", element_ids.join(", "))
            }
            Ev::SlotDefinitionRejected { element_id } => {
                format!(
                    "gpt defineSlot rejected for {element_id}: the container already has a slot"
                )
            }
            Ev::ApsFetchBids { slot_ids, .. } => {
                format!("apstag.fetchBids [{}]", slot_ids.join(", "))
            }
            Ev::ApsBidsReturned { slot_ids, .. } => {
                format!("apstag bids back [{}]", slot_ids.join(", "))
            }
            _ => return None,
        };
        Some(format!("t={} {what}", ms(Some(event.t))))
    }));
    out.join("\n")
}

/// What the scanner verified about each bidder, one line per check.
pub fn bidder_checks(diagnostics: Option<&BidderDiagnostics>) -> String {
    let Some(diagnostics) = diagnostics else {
        return "The bidder checks did not run for this scan.".into();
    };
    let ads_txt = &diagnostics.ads_txt;
    let mut out = vec![
        format!(
            "ads.txt {} status={} records={} invalidLines={}{}",
            ads_txt.url,
            match ads_txt.status {
                AdsTxtStatus::Found => "found",
                AdsTxtStatus::Missing => "missing",
                AdsTxtStatus::Unreadable => "unreadable",
            },
            ads_txt.record_count,
            ads_txt.invalid_line_count,
            ads_txt
                .detail
                .as_ref()
                .map_or(String::new(), |detail| format!(" ({detail})")),
        ),
        format!(
            "sellers.json lookups: {}",
            if diagnostics.sellers_json_checked {
                "on"
            } else {
                "off (user setting)"
            }
        ),
        format!(
            "consent: cmp={} prebidConsentManagement={} gdprApplies={} consentKnownAt={} auctionsBeforeConsent={}/{}",
            diagnostics.consent.cmp_present,
            flag(diagnostics.consent.consent_management_configured),
            flag(diagnostics.consent.gdpr_applies),
            ms(diagnostics.consent.resolved_at_ms),
            diagnostics.consent.auctions_before_consent,
            diagnostics.consent.auction_count,
        ),
    ];
    for bidder in &diagnostics.bidders {
        out.push(format!(
            "bidder {} requests={} bids={}",
            bidder.bidder, bidder.requests, bidder.bids
        ));
        for check in &bidder.checks {
            out.push(format!(
                "  {:?}/{:?} {}: {}{}",
                check.area,
                check.status,
                check.code,
                check.summary,
                check
                    .excerpt
                    .as_ref()
                    .map_or(String::new(), |excerpt| format!(" | {excerpt}")),
            ));
        }
    }
    out.join("\n")
}

fn lines(items: impl Iterator<Item = String>) -> String {
    items.collect::<Vec<_>>().join("\n")
}

fn ms(value: Option<f64>) -> String {
    value.map_or("-".to_string(), |value| format!("{value:.0}ms"))
}

fn flag(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "yes",
        Some(false) => "no",
        None => "unknown",
    }
}

fn short_id(id: &str) -> &str {
    id.char_indices().nth(8).map_or(id, |(cut, _)| &id[..cut])
}

#[cfg(test)]
mod tests {
    use super::*;
    use adscan_model::TrackerEvent;

    #[test]
    fn the_page_timeline_times_a_bid_request_by_its_call_and_says_how_long_prebid_held_it() {
        let asked = |t: f64, released_at: Option<f64>, still_held: bool| TrackerEvent {
            page: 0,
            t,
            kind: Ev::BidsRequested {
                ad_unit_codes: vec!["top".into()],
                ad_unit_count: Some(1),
                timeout: Some(900.0),
                released_at,
                still_held,
            },
        };
        // As a scan stored before releases were recorded holds it.
        let stored_before: TrackerEvent = serde_json::from_value(json!({
            "page": 0, "t": 3_500.0, "type": "bidsRequested",
            "adUnitCodes": ["top"], "adUnitCount": 1, "timeout": 900.0,
        }))
        .unwrap();
        let capture = Capture::single_page(
            "https://news.example/",
            vec![
                asked(400.0, Some(1_500.0), false),
                asked(2_500.0, Some(2_500.4), false),
                asked(3_000.0, None, true),
                stored_before,
            ],
        );
        let timeline = page_events(&capture, 0);
        let lines: Vec<&str> = timeline.lines().skip(1).collect();
        let call = "pbjs.requestBids called: adUnitCodes=[top] adUnitsToAuction=1 timeout=900ms";
        assert_eq!(
            lines,
            [
                format!(
                    "t=400ms {call}; hooks on requestBids (such as consent management or currency) held \
                     it until t=1500ms (1100ms later)"
                ),
                format!("t=2500ms {call}"),
                format!(
                    "t=3000ms {call}; still held by hooks on requestBids (such as consent management or \
                     currency) when the page view ended"
                ),
                // Not known: nothing is said about it.
                format!("t=3500ms {call}"),
            ]
        );
    }

    #[test]
    fn the_visit_block_states_what_is_known_about_consent_as_fact() {
        let account =
            "On page 1 a consent dialog was showing from 0.4 s and the scanner did not accept it. \
                       Consent was given by hand in the browser window 27.0 s into the page view"
                .to_string();
        let block = visit(None, &[], std::slice::from_ref(&account));
        assert!(
            block.ends_with(&format!(
                "consent (established by the scanner, not a guess): {account}."
            )),
            "{block}"
        );
        assert!(!visit(None, &[], &[]).contains("consent"));
    }

    fn environment(browser: BrowserUse) -> ScanEnvironment {
        ScanEnvironment {
            visitor: adscan_model::VisitorBehavior::Scroll,
            seed: 7,
            headless: true,
            persistent_profile: false,
            webdriver: Some(false),
            user_agent_overridden: false,
            viewport_overridden: true,
            emulated_mobile: false,
            detectors: Vec::new(),
            proxy: adscan_model::ProxyUse {
                kind: adscan_model::ProxyKind::None,
                name: None,
                host: None,
            },
            browser,
        }
    }

    #[test]
    fn the_visit_block_says_where_the_browser_ran_when_that_was_not_this_machine() {
        let local = visit(Some(&environment(BrowserUse::Local)), &[], &[]);
        assert!(!local.contains("Browserbase"), "{local}");
        assert!(local.contains("navigator.webdriver=no headless=true"));

        let cloud = visit(
            Some(&environment(BrowserUse::Browserbase(Box::new(
                BrowserbaseVisit {
                    region: adscan_model::BrowserbaseRegion::EuCentral1,
                    session_id: "session-1".into(),
                    browser_version: None,
                    user_agent: None,
                    proxy_kind: BrowserbaseProxyKind::Browserbase,
                    proxy_country: Some("BE".into()),
                    settings: adscan_model::BrowserbaseSessionSettings::STANDARD,
                    original_timezone: Some("Europe/Brussels".into()),
                    applied_timezone: None,
                    language: Some("en-US".into()),
                    round_trip_ms: Some(118.6),
                    session_seconds: None,
                    proxy_bytes: None,
                },
            )))),
            &[],
            &[],
        );
        assert!(
            cloud.contains(
                "the browser ran in Browserbase's cloud, region eu-central-1, leaving through a residential \
                 exit asked for in BE; timezone Europe/Brussels, language en-US; it runs headless without \
                 saying so in its user agent; CAPTCHAs are solved automatically: true; round trip between \
                 scanner and browser 119 ms"
            ),
            "{cloud}"
        );
        // What the page could read is another matter than how the browser ran.
        assert!(cloud.contains("navigator.webdriver=no headless=false"));
        assert!(
            !cloud.contains("session-1"),
            "an id is of no use to the model"
        );
    }
}
