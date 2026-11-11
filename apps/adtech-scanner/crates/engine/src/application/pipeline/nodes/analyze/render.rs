//! Compact text views of the telemetry for the analysis model.

use adscan_model::{AdSlot, Auction, Capture, TrackerEventKind as Ev};
use serde_json::json;

pub fn auctions_overview(auctions: &[Auction]) -> String {
    if auctions.is_empty() {
        return "No Prebid auction started during the observation.".into();
    }
    lines(auctions.iter().map(|auction| {
        let bids = auction
            .ad_units
            .iter()
            .map(|unit| unit.bids.len())
            .sum::<usize>();
        format!(
            "auction {} start={} end={} timeout={} adUnits=[{}] bids={}",
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

pub fn auctions_detail(auctions: &[Auction]) -> String {
    if auctions.is_empty() {
        return auctions_overview(auctions);
    }
    let mut out = Vec::new();
    for auction in auctions {
        out.push(format!(
            "auction {} start={} end={} timeout={}",
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
    out.join("\n")
}

pub fn slots(slots: &[AdSlot]) -> String {
    if slots.is_empty() {
        return "No GPT slot was defined during the observation.".into();
    }
    lines(slots.iter().map(|slot| {
        format!(
            "slot {} path={} sizes=[{}] outcome={:?} inInitialViewport={} defined={} requested={} rendered={} hbTargetingOnRequest={} requestedBeforeAuctionEnd={}",
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

pub fn console(capture: &Capture) -> String {
    let prebid_logs = capture.events.iter().filter_map(|event| match &event.kind {
        Ev::PrebidLog { level, message } => {
            Some(format!("t={} prebid {level}: {message}", ms(Some(event.t))))
        }
        _ => None,
    });
    let console_lines = capture.console.iter().map(|line| {
        format!(
            "t={} console {}: {}",
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

/// Page lifecycle, scroll and library-detection events in time order.
pub fn page_events(capture: &Capture) -> String {
    let mut out = vec![format!(
        "navigation start t=0, load event {}, max scroll depth {:.0}%, consent accepted: {}",
        ms(capture.page_load_ms),
        capture.max_scroll_depth * 100.0,
        capture.consent_accepted,
    )];
    out.extend(capture.events.iter().filter_map(|event| {
        let what = match &event.kind {
            Ev::PageLifecycle { phase } => phase.clone(),
            Ev::Scroll { depth } => format!("scroll depth {:.0}%", depth * 100.0),
            Ev::PrebidDetected { .. } => "prebid detected".into(),
            Ev::GptDetected { .. } => "gpt detected".into(),
            Ev::AmazonDetected {} => "amazon apstag detected".into(),
            Ev::AuctionInit { auction_id, .. } => format!("auctionInit {}", short_id(auction_id)),
            Ev::AuctionEnd { auction_id } => format!("auctionEnd {}", short_id(auction_id)),
            Ev::SetTargeting { ad_unit_codes } => {
                format!("setTargeting [{}]", ad_unit_codes.join(", "))
            }
            Ev::SlotRequested { element_id, .. } => format!("gpt request {element_id}"),
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
