//! Scans of the built-in demo publisher site with a real browser. The demo's
//! planted bugs (see `demo-site/README.md`) must be visible in the telemetry.

mod support;

use std::collections::{BTreeMap, BTreeSet};

use adscan_model::{Capture, DeviceProfile, TrackerEventKind as Kind};
use support::{events_where, first_time};

const SCROLL_GATED_UNITS: [&str; 4] = [
    "ad-leaderboard",
    "ad-sidebar-1",
    "ad-sidebar-2",
    "ad-inarticle-1",
];
const CONFIGURED_TIMEOUT_MS: f64 = 250.0;

async fn scan(device: DeviceProfile, auto_scroll: bool) -> Capture {
    let site = adscan_browser::start_demo_site().await.unwrap();
    let mut request = support::request(&site.url, device);
    request.auto_scroll = auto_scroll;
    request.observe_seconds = if auto_scroll { 9 } else { 3 };
    let capture = support::capture_ok(request).await;
    assert_eq!(capture.final_url, site.url);
    capture
}

/// One Prebid auction as seen in the event stream.
struct SeenAuction {
    id: String,
    started_at: f64,
    timeout: Option<f64>,
    units: Vec<String>,
}

fn auctions(capture: &Capture) -> Vec<SeenAuction> {
    capture
        .events
        .iter()
        .filter_map(|event| match &event.kind {
            Kind::AuctionInit {
                auction_id,
                timeout,
                ad_units,
            } => Some(SeenAuction {
                id: auction_id.clone(),
                started_at: event.t,
                timeout: *timeout,
                units: ad_units.iter().map(|unit| unit.code.clone()).collect(),
            }),
            _ => None,
        })
        .collect()
}

fn auction_for<'a>(auctions: &'a [SeenAuction], unit: &str) -> Option<&'a SeenAuction> {
    auctions
        .iter()
        .find(|auction| auction.units.iter().any(|code| code == unit))
}

fn slot_time(capture: &Capture, element: &str, stage: &str) -> Option<f64> {
    first_time(capture, |kind| match (stage, kind) {
        ("defined", Kind::SlotDefined { element_id, .. })
        | ("requested", Kind::SlotRequested { element_id, .. })
        | ("response", Kind::SlotResponseReceived { element_id })
        | ("rendered", Kind::SlotRenderEnded { element_id, .. })
        | ("onload", Kind::SlotOnload { element_id })
        | ("viewable", Kind::ImpressionViewable { element_id }) => element_id == element,
        _ => false,
    })
}

fn phase_time(capture: &Capture, name: &str) -> Option<f64> {
    first_time(
        capture,
        |kind| matches!(kind, Kind::PageLifecycle { phase } if phase == name),
    )
}

fn render_is_empty(capture: &Capture, element: &str) -> Option<bool> {
    capture.events.iter().find_map(|event| match &event.kind {
        Kind::SlotRenderEnded {
            element_id,
            is_empty,
            ..
        } if element_id == element => Some(*is_empty),
        _ => None,
    })
}

fn requested_targeting(capture: &Capture, element: &str) -> Option<Vec<String>> {
    capture.events.iter().find_map(|event| match &event.kind {
        Kind::SlotRequested {
            element_id,
            prebid_targeting_keys,
        } if element_id == element => Some(prebid_targeting_keys.clone()),
        _ => None,
    })
}

fn assert_slot_filled_in_order(capture: &Capture, element: &str) {
    let stages = ["defined", "requested", "response", "rendered", "onload"];
    let times: Vec<f64> = stages
        .iter()
        .map(|stage| {
            slot_time(capture, element, stage)
                .unwrap_or_else(|| panic!("{element} has no {stage} event"))
        })
        .collect();
    assert!(
        times.windows(2).all(|pair| pair[0] <= pair[1]),
        "{element} lifecycle out of order: {times:?}"
    );
    assert_eq!(
        render_is_empty(capture, element),
        Some(false),
        "{element} should be filled"
    );
    let keys = requested_targeting(capture, element).unwrap();
    assert!(
        keys.contains(&"hb_adid".to_string()) && keys.contains(&"hb_pb".to_string()),
        "{element}: {keys:?}"
    );
}

#[tokio::test]
async fn desktop_without_scrolling_never_starts_the_gated_auction() {
    let capture = scan(DeviceProfile::Desktop, false).await;
    let auctions = auctions(&capture);

    // Planted bug (a): the scroll gate meant for phones applies to desktop.
    for unit in SCROLL_GATED_UNITS {
        assert!(
            auction_for(&auctions, unit).is_none(),
            "{unit} should not be auctioned before a scroll"
        );
    }
    assert_eq!(phase_time(&capture, "firstScroll"), None);
    assert_eq!(capture.max_scroll_depth, 0.0);
    for element in [
        "ad-leaderboard",
        "ad-sidebar-1",
        "ad-inarticle-1",
        "ad-inarticle-2",
    ] {
        assert!(
            slot_time(&capture, element, "defined").is_some(),
            "{element} is defined"
        );
        assert_eq!(
            slot_time(&capture, element, "requested"),
            None,
            "{element} is never requested"
        );
    }
    let leaderboard_in_viewport = capture.events.iter().find_map(|event| match &event.kind {
        Kind::SlotDefined {
            element_id,
            in_viewport,
            ..
        } if element_id == "ad-leaderboard" => *in_viewport,
        _ => None,
    });
    assert_eq!(
        leaderboard_in_viewport,
        Some(true),
        "the empty slot is one the visitor is looking at"
    );

    // The libraries are there and the page finished loading: nothing else explains the silence.
    assert!(first_time(&capture, |kind| matches!(kind, Kind::PrebidDetected { .. })).is_some());
    assert!(first_time(&capture, |kind| matches!(kind, Kind::GptDetected { .. })).is_some());
    assert!(
        capture
            .page_load_ms
            .is_some_and(|ms| ms > 0.0 && ms < 10_000.0)
    );

    // The anchor slot has its own (ungated) path, and its own bug (b).
    assert_eq!(auctions.len(), 1);
    assert_eq!(auctions[0].units, ["ad-sticky-footer"]);
    assert_eq!(render_is_empty(&capture, "ad-sticky-footer"), Some(true));
}

#[tokio::test]
async fn desktop_starts_the_gated_auction_on_first_scroll_and_shows_every_planted_bug() {
    let capture = scan(DeviceProfile::Desktop, true).await;
    let auctions = auctions(&capture);

    // -- Libraries and configuration ------------------------------------------
    let prebid = events_where(&capture, |kind| matches!(kind, Kind::PrebidDetected { .. }))
        .next()
        .unwrap();
    let Kind::PrebidDetected {
        global_name,
        version,
        modules,
    } = &prebid.kind
    else {
        unreachable!()
    };
    assert_eq!(
        (global_name.as_str(), version.as_deref()),
        ("pbjs", Some("v10.14.0"))
    );
    assert!(
        modules.contains(&"appnexusBidAdapter".to_string()) && modules.len() == 12,
        "{modules:?}"
    );
    assert!(
        events_where(&capture, |kind| matches!(
            kind,
            Kind::GptDetected { version: Some(_) }
        ))
        .count()
            == 1
    );
    assert!(events_where(&capture, |kind| matches!(kind, Kind::AmazonDetected {})).count() == 1);

    // Planted bug (c): the wrapper reads the wrong config key and falls back to 250 ms.
    let config = events_where(&capture, |kind| matches!(kind, Kind::PrebidConfig { .. }))
        .next()
        .unwrap();
    let Kind::PrebidConfig {
        bidder_timeout,
        price_granularity,
        currency,
        raw,
        ..
    } = &config.kind
    else {
        unreachable!()
    };
    assert_eq!(*bidder_timeout, Some(CONFIGURED_TIMEOUT_MS));
    assert_eq!(price_granularity.as_deref(), Some("medium"));
    assert_eq!(currency.as_deref(), Some("USD"));
    assert_eq!(raw["userSync"]["syncDelay"], 3000);

    // -- Planted bug (a): nothing is auctioned until the first scroll ----------
    let first_scroll = phase_time(&capture, "firstScroll").expect("the scan scrolls");
    let main =
        auction_for(&auctions, "ad-leaderboard").expect("scrolling starts the gated auction");
    assert_eq!(main.units, SCROLL_GATED_UNITS);
    assert!(
        first_scroll > 2_000.0,
        "the scan idles before scrolling, first scroll at {first_scroll}"
    );
    assert!(
        main.started_at >= first_scroll && main.started_at - first_scroll < 500.0,
        "auction at {} should follow the first scroll at {first_scroll}",
        main.started_at
    );
    let load = capture.page_load_ms.unwrap();
    assert!(
        main.started_at - load > 2_000.0,
        "the auction waited long after load ({load})"
    );

    // -- Auctions --------------------------------------------------------------
    let lazy =
        auction_for(&auctions, "ad-inarticle-2").expect("scrolling near the lazy slot auctions it");
    assert_eq!(lazy.units, ["ad-inarticle-2"]);
    assert!(lazy.started_at > main.started_at);
    assert_eq!(auctions.len(), 3, "anchor, gated and lazy auction");
    for auction in &auctions {
        assert_eq!(auction.timeout, Some(CONFIGURED_TIMEOUT_MS));
        let ended = first_time(
            &capture,
            |kind| matches!(kind, Kind::AuctionEnd { auction_id } if *auction_id == auction.id),
        )
        .unwrap_or_else(|| panic!("auction {} never ended", auction.id));
        let duration = ended - auction.started_at;
        assert!(
            (50.0..1_000.0).contains(&duration),
            "auction lasted {duration} ms"
        );
    }

    // -- Bids of the gated auction ----------------------------------------------
    let requested: BTreeMap<&str, &Vec<String>> = capture
        .events
        .iter()
        .filter_map(|event| match &event.kind {
            Kind::BidRequested {
                auction_id,
                bidder,
                ad_unit_codes,
                ..
            } if *auction_id == main.id => Some((bidder.as_str(), ad_unit_codes)),
            _ => None,
        })
        .collect();
    assert_eq!(
        requested.keys().copied().collect::<Vec<_>>(),
        [
            "appnexus",
            "criteo",
            "ix",
            "openx",
            "pubmatic",
            "rubicon",
            "triplelift"
        ]
    );
    assert_eq!(requested["appnexus"], &SCROLL_GATED_UNITS);

    let mut best_bid: BTreeMap<&str, f64> = BTreeMap::new();
    let mut responders = BTreeSet::new();
    for event in &capture.events {
        let Kind::BidResponse {
            auction_id,
            bidder,
            ad_unit_code,
            cpm,
            currency,
            size,
            time_to_respond,
            ..
        } = &event.kind
        else {
            continue;
        };
        assert!(*cpm > 0.0 && *cpm < 20.0, "cpm {cpm}");
        assert_eq!(currency.as_deref(), Some("USD"));
        assert!(
            size.as_deref().is_some_and(|size| size.contains('x')),
            "{size:?}"
        );
        let latency = time_to_respond.expect("bids report their latency");
        assert!(
            latency > 0.0 && latency <= CONFIGURED_TIMEOUT_MS + 50.0,
            "{bidder} answered in {latency} ms"
        );
        if *auction_id == main.id {
            responders.insert(bidder.as_str());
            let best = best_bid.entry(ad_unit_code.as_str()).or_default();
            *best = best.max(*cpm);
        }
    }
    assert!(
        responders.contains("appnexus") && responders.contains("rubicon"),
        "{responders:?}"
    );
    let no_bids = events_where(
        &capture,
        |kind| matches!(kind, Kind::NoBid { auction_id, .. } if *auction_id == main.id),
    );
    assert!(no_bids.count() >= 1, "some bidder passes on some unit");

    // Planted bug (c), observed: the two bidders slower than 250 ms always time out.
    let timed_out: BTreeSet<(&str, &str)> = capture
        .events
        .iter()
        .filter_map(|event| match &event.kind {
            Kind::BidTimeout {
                auction_id,
                bidder,
                ad_unit_code,
            } if *auction_id == main.id => Some((bidder.as_str(), ad_unit_code.as_str())),
            _ => None,
        })
        .collect();
    for slow in ["pubmatic", "criteo"] {
        assert!(
            !responders.contains(slow),
            "{slow} cannot answer within the configured timeout"
        );
        for unit in requested[slow] {
            assert!(
                timed_out.contains(&(slow, unit.as_str())),
                "{slow} should time out on {unit}"
            );
        }
    }
    assert_eq!(
        timed_out.len(),
        requested["pubmatic"].len() + requested["criteo"].len()
    );
    let late_response_logged = capture.events.iter().any(|event| {
        matches!(&event.kind, Kind::PrebidLog { level, message } if level == "warning" && message.contains("Late response from pubmatic"))
    });
    assert!(
        late_response_logged,
        "Prebid's warning about the late bid is captured"
    );

    // -- Winners and slot lifecycle ----------------------------------------------
    for element in [
        "ad-leaderboard",
        "ad-sidebar-1",
        "ad-inarticle-1",
        "ad-inarticle-2",
    ] {
        assert_slot_filled_in_order(&capture, element);
        let auction = auction_for(&auctions, element).unwrap();
        let ended = first_time(
            &capture,
            |kind| matches!(kind, Kind::AuctionEnd { auction_id } if *auction_id == auction.id),
        );
        assert!(
            slot_time(&capture, element, "requested") >= ended,
            "{element} waits for its auction"
        );
    }
    for (unit, best) in best_bid.iter().filter(|(unit, _)| **unit != "ad-sidebar-2") {
        let won = capture.events.iter().find_map(|event| match &event.kind {
            Kind::BidWon {
                auction_id,
                ad_unit_code,
                cpm,
                ..
            } if ad_unit_code == unit => Some((auction_id.as_deref(), *cpm)),
            _ => None,
        });
        assert_eq!(
            won,
            Some((Some(main.id.as_str()), *best)),
            "the highest bid wins {unit}"
        );
        let rendered = events_where(
            &capture,
            |kind| matches!(kind, Kind::AdRenderSucceeded { ad_unit_code: Some(code), bidder: Some(_) } if code == unit),
        );
        assert_eq!(rendered.count(), 1, "{unit} renders its winner once");
    }
    assert!(
        events_where(&capture, |kind| matches!(
            kind,
            Kind::ImpressionViewable { .. }
        ))
        .count()
            >= 1,
        "at least one rendered ad becomes viewable while scrolling"
    );
    let render = events_where(&capture, |kind| {
        matches!(kind, Kind::SlotRenderEnded { element_id, .. } if element_id == "ad-leaderboard")
    })
    .next()
    .unwrap();
    let Kind::SlotRenderEnded {
        size,
        advertiser_id,
        line_item_id,
        ..
    } = &render.kind
    else {
        unreachable!()
    };
    assert!(
        matches!(size.as_deref(), Some("970x250" | "728x90")),
        "{size:?}"
    );
    assert!(advertiser_id.is_some() && line_item_id.is_some());

    // -- Planted bug (b): the anchor slot is requested before its auction ends ---
    let anchor = auction_for(&auctions, "ad-sticky-footer").unwrap();
    let anchor_requested = slot_time(&capture, "ad-sticky-footer", "requested").unwrap();
    let anchor_auction_ended = first_time(
        &capture,
        |kind| matches!(kind, Kind::AuctionEnd { auction_id } if *auction_id == anchor.id),
    )
    .unwrap();
    assert!(anchor_requested >= anchor.started_at && anchor_requested < anchor_auction_ended);
    assert_eq!(
        requested_targeting(&capture, "ad-sticky-footer"),
        Some(vec![])
    );
    assert_eq!(render_is_empty(&capture, "ad-sticky-footer"), Some(true));
    let anchor_had_bids = capture.events.iter().any(|event| {
        matches!(&event.kind, Kind::BidResponse { auction_id, .. } if *auction_id == anchor.id)
    });
    assert!(
        anchor_had_bids,
        "bids existed for the slot that went out without them"
    );

    // -- Planted bug (d): an ad unit without a slot ------------------------------
    assert!(
        best_bid.contains_key("ad-sidebar-2"),
        "bidders are asked, and answer, for ad-sidebar-2"
    );
    assert_eq!(slot_time(&capture, "ad-sidebar-2", "defined"), None);
    assert_eq!(slot_time(&capture, "ad-sidebar-2", "requested"), None);

    // -- Amazon TAM and page level -----------------------------------------------
    let fetches: Vec<&Vec<String>> = capture
        .events
        .iter()
        .filter_map(|event| match &event.kind {
            Kind::ApsFetchBids {
                slot_ids,
                timeout: Some(800.0),
            } => Some(slot_ids),
            _ => None,
        })
        .collect();
    assert_eq!(
        fetches,
        [
            &SCROLL_GATED_UNITS.map(String::from).to_vec(),
            &vec!["ad-inarticle-2".to_string()]
        ]
    );
    let returns = events_where(
        &capture,
        |kind| matches!(kind, Kind::ApsBidsReturned { duration: Some(duration), .. } if (150.0..600.0).contains(duration)),
    );
    assert_eq!(returns.count(), 2);
    assert!(
        capture.max_scroll_depth > 0.5,
        "depth {}",
        capture.max_scroll_depth
    );
    assert_eq!(
        capture.page_title.as_deref(),
        Some("The tide-powered town that sells its nights | The Meridian Post")
    );
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as f64;
    assert!(now - capture.navigation_start > 9_000.0 && now - capture.navigation_start < 120_000.0);
    assert!(
        capture
            .events
            .iter()
            .all(|event| event.t >= 0.0 && event.t < 60_000.0)
    );
}

#[tokio::test]
async fn desktop_scan_shows_the_planted_bidder_setup_bugs() {
    let capture = scan(DeviceProfile::Desktop, true).await;
    let requested_units = |name: &str| -> Vec<&str> {
        capture
            .events
            .iter()
            .filter_map(|event| match &event.kind {
                Kind::BidRequested {
                    bidder,
                    ad_unit_codes,
                    ..
                } if bidder == name => Some(ad_unit_codes),
                _ => None,
            })
            .flatten()
            .map(String::as_str)
            .collect()
    };
    let bids_from = |name: &str| {
        events_where(
            &capture,
            |kind| matches!(kind, Kind::BidResponse { bidder, .. } if bidder == name),
        )
        .count()
    };
    let passes_from = |name: &str| {
        events_where(
            &capture,
            |kind| matches!(kind, Kind::NoBid { bidder, .. } if bidder == name),
        )
        .count()
    };

    // -- Planted bug (e): ix is called but never bids -----------------------------
    assert_eq!(
        requested_units("ix"),
        ["ad-sticky-footer", "ad-leaderboard", "ad-inarticle-1"]
    );
    assert_eq!((bids_from("ix"), passes_from("ix")), (0, 3));

    // -- Planted bug (f): triplelift has its inventory code on one ad unit only ---
    let mut inventory_codes: BTreeMap<&str, Option<&str>> = BTreeMap::new();
    for event in &capture.events {
        let Kind::AuctionInit { ad_units, .. } = &event.kind else {
            continue;
        };
        for unit in ad_units {
            for bid in unit.bids.iter().filter(|bid| bid.bidder == "triplelift") {
                inventory_codes.insert(&unit.code, bid.params["inventoryCode"].as_str());
            }
        }
    }
    assert_eq!(
        inventory_codes,
        BTreeMap::from([
            ("ad-inarticle-1", None),
            ("ad-inarticle-2", Some("MeridianPost_article2_HDX")),
            ("ad-sidebar-1", None),
        ])
    );
    // As in real Prebid, a bid the adapter rejects is announced, never sent,
    // and ends as a no-bid.
    assert_eq!(
        requested_units("triplelift"),
        ["ad-sidebar-1", "ad-inarticle-1", "ad-inarticle-2"]
    );
    assert_eq!(
        (bids_from("triplelift"), passes_from("triplelift")),
        (1, 2),
        "it bids where its params are complete"
    );
    let dropped = events_where(&capture, |kind| {
        matches!(kind, Kind::PrebidLog { level, message }
            if level == "warning" && message.starts_with("Invalid bid sent to bidder triplelift"))
    });
    assert_eq!(dropped.count(), 2);
    // Every other bid carries the params the wrapper configured.
    let appnexus_placement = capture.events.iter().find_map(|event| match &event.kind {
        Kind::AuctionInit { ad_units, .. } => ad_units
            .iter()
            .flat_map(|unit| &unit.bids)
            .find(|bid| bid.bidder == "appnexus")
            .map(|bid| bid.params["placementId"].clone()),
        _ => None,
    });
    assert!(
        appnexus_placement.is_some_and(|id| id.is_number()),
        "appnexus bids carry a numeric placementId"
    );

    // -- Planted bug (g): the OpenX exchange answers with a no-bid reason ----------
    assert_eq!(
        requested_units("openx"),
        ["ad-leaderboard", "ad-sidebar-1", "ad-inarticle-1"]
    );
    assert_eq!((bids_from("openx"), passes_from("openx")), (0, 3));
    assert_eq!(
        capture.bid_responses.len(),
        1,
        "one request for the three ad units: {:#?}",
        capture.bid_responses
    );
    for answer in &capture.bid_responses {
        assert_eq!(
            (
                answer.bidder.as_str(),
                answer.host.as_str(),
                answer.status,
                answer.reason.as_deref()
            ),
            (
                "openx",
                "127.0.0.1",
                Some(200),
                Some("no-bid reason 7 (blocked publisher or site)")
            )
        );
        assert!(answer.t > 0.0 && answer.t < 60_000.0);
    }
    // The demo has no consent platform.
    assert_eq!(
        events_where(&capture, |kind| matches!(
            kind,
            Kind::CmpDetected { .. } | Kind::TcfState { .. }
        ))
        .count(),
        0
    );
}

#[tokio::test]
async fn every_script_of_the_demo_page_is_captured_with_its_body() {
    let capture = scan(DeviceProfile::Desktop, false).await;
    let origin = capture.final_url.trim_end_matches('/');
    let by_path: BTreeMap<&str, _> = capture
        .scripts
        .iter()
        .map(|script| {
            (
                script.url.strip_prefix(origin).unwrap_or(&script.url),
                script,
            )
        })
        .collect();
    assert_eq!(
        by_path.keys().copied().collect::<Vec<_>>(),
        [
            "/static/ads/apstag.js",
            "/static/ads/hb-wrapper.min.js",
            "/static/ads/prebid.js",
            "/static/js/analytics.js",
            "/static/js/app.9b3e7f.js",
            "/static/js/vendors-ui.4c1d2a.chunk.js",
            "/tag/js/gpt.js",
            "/tag/js/gpt.js?v=adstack-4.2.1",
            "inline:1",
        ]
    );
    assert_eq!(
        by_path.len(),
        capture.scripts.len(),
        "no script is stored twice"
    );

    let page = Some(capture.final_url.clone());
    let wrapper = Some(format!("{origin}/static/ads/hb-wrapper.min.js"));
    for (path, script) in &by_path {
        assert!(
            script.body.len() > 500,
            "{path} has a body of {} bytes",
            script.body.len()
        );
        assert_eq!(script.inline, *path == "inline:1");
        let loaded_at = script
            .loaded_at_ms
            .unwrap_or_else(|| panic!("{path} has no load time"));
        assert!(
            (0.0..5_000.0).contains(&loaded_at),
            "{path} loaded at {loaded_at}"
        );
        let injected_by_wrapper = matches!(
            *path,
            "/static/ads/apstag.js" | "/tag/js/gpt.js?v=adstack-4.2.1"
        );
        assert_eq!(
            script.initiator,
            if injected_by_wrapper {
                wrapper.clone()
            } else {
                page.clone()
            },
            "{path}"
        );
    }
    assert!(by_path["inline:1"].body.contains("window.__MERIDIAN__"));
    assert!(
        by_path["/static/ads/prebid.js"]
            .body
            .contains("pbjs.requestBids = function")
    );
    assert!(
        by_path["/static/js/vendors-ui.4c1d2a.chunk.js"]
            .body
            .contains("tinykit-ui")
    );
    let wrapper_body = &by_path["/static/ads/hb-wrapper.min.js"].body;
    assert!(
        wrapper_body.lines().count() <= 3 && wrapper_body.contains("(min-width:"),
        "served minified"
    );

    // Planted bug (d): the wrapper loads GPT a second time under another URL.
    assert_eq!(
        by_path["/tag/js/gpt.js"].body,
        by_path["/tag/js/gpt.js?v=adstack-4.2.1"].body
    );
    assert!(
        capture
            .console
            .iter()
            .any(|line| line.level == "warning" && line.text.contains("loaded more than once")),
        "{:#?}",
        capture.console
    );
    assert!(capture.console.iter().all(|line| line.t >= 0.0));
}

#[tokio::test]
async fn mobile_bids_at_page_load_without_any_scroll() {
    let capture = scan(DeviceProfile::Mobile, false).await;
    let auctions = auctions(&capture);

    // The other side of planted bug (a): phones, the intended target of the
    // scroll gate, are the ones that skip it.
    assert_eq!(phase_time(&capture, "firstScroll"), None);
    let main = auction_for(&auctions, "ad-leaderboard").expect("mobile auctions without a scroll");
    assert_eq!(main.units, SCROLL_GATED_UNITS);
    let load = capture.page_load_ms.unwrap();
    assert!(
        main.started_at < load + 500.0,
        "auction at {} ms, load at {load} ms",
        main.started_at
    );

    for element in ["ad-leaderboard", "ad-sidebar-1", "ad-inarticle-1"] {
        assert_slot_filled_in_order(&capture, element);
    }
    assert!(
        slot_time(&capture, "ad-leaderboard", "viewable").is_some(),
        "the top banner is seen"
    );
    assert_eq!(
        slot_time(&capture, "ad-inarticle-2", "requested"),
        None,
        "the lazy slot waits for a scroll"
    );

    let leaderboard_sizes = capture.events.iter().find_map(|event| match &event.kind {
        Kind::SlotDefined {
            element_id, sizes, ..
        } if element_id == "ad-leaderboard" => Some(sizes.clone()),
        _ => None,
    });
    assert_eq!(
        leaderboard_sizes,
        Some(vec!["320x100".to_string(), "320x50".to_string()])
    );
    assert!(capture.user_agent.contains("Android"));
    assert_eq!(capture.device, DeviceProfile::Mobile);
    assert_eq!(
        render_is_empty(&capture, "ad-sticky-footer"),
        Some(true),
        "bug (b) is device independent"
    );
}

#[tokio::test]
async fn mobile_scrolling_by_touch_triggers_the_lazy_slot() {
    let capture = scan(DeviceProfile::Mobile, true).await;
    let auctions = auctions(&capture);

    let first_scroll = phase_time(&capture, "firstScroll").expect("touch swipes scroll the page");
    assert!(
        phase_time(&capture, "firstInteraction").is_some_and(|touched| touched <= first_scroll)
    );
    let lazy = auction_for(&auctions, "ad-inarticle-2")
        .expect("the lazy slot is auctioned once it comes near");
    assert!(lazy.started_at > first_scroll);
    assert_slot_filled_in_order(&capture, "ad-inarticle-2");
    assert!(
        capture.max_scroll_depth > 0.3,
        "depth {}",
        capture.max_scroll_depth
    );
    let last_depth = capture
        .events
        .iter()
        .rev()
        .find_map(|event| match event.kind {
            Kind::Scroll { depth } => Some(depth),
            _ => None,
        });
    assert!(
        last_depth.is_some_and(|depth| depth < 0.05),
        "the scan returns to the top, ended at {last_depth:?}"
    );
}
