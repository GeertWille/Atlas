//! The tracker's side of the `TrackerEventKind` contract, checked in a real
//! browser against a page that exercises every code path of the script.

mod support;

use std::collections::BTreeSet;

use adscan_model::{
    Capture, DeviceProfile, PageReach, SchainNode, TrackedAdUnit, TrackedBid, TrackedBidRequest,
    TrackerEventKind as Kind, VisitorBehavior,
};
use serde_json::{Value, json};

/// Exhaustive on purpose: a new `TrackerEventKind` variant fails to compile
/// here until this test covers it.
fn variant(kind: &Kind) -> &'static str {
    match kind {
        Kind::PrebidDetected { .. } => "prebidDetected",
        Kind::PrebidConfig { .. } => "prebidConfig",
        Kind::GptDetected { .. } => "gptDetected",
        Kind::AmazonDetected {} => "amazonDetected",
        Kind::BidsRequested { .. } => "bidsRequested",
        Kind::AdUnitsAdded { .. } => "adUnitsAdded",
        Kind::AuctionInit { .. } => "auctionInit",
        Kind::BidRequested { .. } => "bidRequested",
        Kind::BidResponse { .. } => "bidResponse",
        Kind::NoBid { .. } => "noBid",
        Kind::BidTimeout { .. } => "bidTimeout",
        Kind::BidderError { .. } => "bidderError",
        Kind::AuctionEnd { .. } => "auctionEnd",
        Kind::SetTargeting { .. } => "setTargeting",
        Kind::BidWon { .. } => "bidWon",
        Kind::AdRenderFailed { .. } => "adRenderFailed",
        Kind::AdRenderSucceeded { .. } => "adRenderSucceeded",
        Kind::SlotDefined { .. } => "slotDefined",
        Kind::SlotRequested { .. } => "slotRequested",
        Kind::SlotResponseReceived { .. } => "slotResponseReceived",
        Kind::SlotRenderEnded { .. } => "slotRenderEnded",
        Kind::SlotOnload { .. } => "slotOnload",
        Kind::ImpressionViewable { .. } => "impressionViewable",
        Kind::ApsFetchBids { .. } => "apsFetchBids",
        Kind::ApsBidsReturned { .. } => "apsBidsReturned",
        Kind::PageLifecycle { .. } => "pageLifecycle",
        Kind::Scroll { .. } => "scroll",
        Kind::PrebidLog { .. } => "prebidLog",
        Kind::CmpDetected { .. } => "cmpDetected",
        Kind::TcfState { .. } => "tcfState",
        Kind::SlotsDestroyed { .. } => "slotsDestroyed",
        Kind::SlotDefinitionRejected { .. } => "slotDefinitionRejected",
        Kind::RouteChange { .. } => "routeChange",
    }
}

const ALL_VARIANTS: [&str; 31] = [
    "adRenderFailed",
    "adRenderSucceeded",
    "amazonDetected",
    "apsBidsReturned",
    "apsFetchBids",
    "auctionEnd",
    "auctionInit",
    "bidRequested",
    "bidResponse",
    "bidTimeout",
    "bidWon",
    "bidderError",
    "cmpDetected",
    "gptDetected",
    "impressionViewable",
    "noBid",
    "pageLifecycle",
    "prebidConfig",
    "prebidDetected",
    "prebidLog",
    "routeChange",
    "scroll",
    "setTargeting",
    "slotDefined",
    "slotDefinitionRejected",
    "slotOnload",
    "slotRenderEnded",
    "slotRequested",
    "slotResponseReceived",
    "slotsDestroyed",
    "tcfState",
];

fn kinds<'a>(capture: &'a Capture, name: &str) -> Vec<&'a Kind> {
    capture
        .events
        .iter()
        .map(|event| &event.kind)
        .filter(|kind| variant(kind) == name)
        .collect()
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| value.to_string()).collect()
}

async fn kitchen_sink() -> Capture {
    let fixtures = support::Fixtures::start().await;
    let mut request = support::request(&fixtures.url("kitchen-sink.html"), DeviceProfile::Desktop);
    request.visitor = VisitorBehavior::Scroll;
    request.observe_seconds = 4;
    support::capture_ok(request).await
}

#[tokio::test]
async fn every_event_the_tracker_emits_fits_the_contract() {
    let capture = kitchen_sink().await;

    let seen: BTreeSet<&str> = capture
        .events
        .iter()
        .map(|event| variant(&event.kind))
        .collect();
    assert_eq!(seen.into_iter().collect::<Vec<_>>(), ALL_VARIANTS);
    for event in &capture.events {
        assert!(
            event.t >= 0.0 && event.t < 60_000.0,
            "implausible timestamp in {event:?}"
        );
        let round_trip = serde_json::to_value(event).unwrap();
        assert_eq!(round_trip["type"], variant(&event.kind));
    }

    // -- Prebid under a custom global, attached late: history replayed once.
    assert_eq!(
        kinds(&capture, "prebidDetected"),
        [&Kind::PrebidDetected {
            global_name: "acmeHb".into(),
            version: Some("v9.1.0".into()),
            modules: strings(&["fooBidAdapter", "barBidAdapter"]),
        }]
    );
    let auctions = kinds(&capture, "auctionInit");
    assert_eq!(
        auctions[0],
        &Kind::AuctionInit {
            auction_id: "auction-1".into(),
            timeout: Some(1500.0),
            ad_units: vec![
                TrackedAdUnit {
                    code: "unit-a".into(),
                    sizes: strings(&["300x250", "336x280"]),
                    bidders: strings(&["foo", "bar"]),
                    bids: vec![
                        TrackedBid {
                            bidder: "foo".into(),
                            params: json!({ "placementId": 12345, "keywords": { "section": ["news"] } }),
                        },
                        TrackedBid {
                            bidder: "bar".into(),
                            params: json!({ "siteId": "9" }),
                        },
                        TrackedBid {
                            bidder: "foo".into(),
                            params: json!({}),
                        },
                    ],
                },
                TrackedAdUnit {
                    code: "unit-b".into(),
                    sizes: strings(&["728x90", "640x480"]),
                    bidders: strings(&["bar"]),
                    bids: vec![],
                },
                TrackedAdUnit {
                    code: "unit-c".into(),
                    sizes: strings(&["160x600"]),
                    bidders: vec![],
                    bids: vec![],
                },
            ],
        }
    );
    assert_eq!(
        auctions[1],
        &Kind::AuctionInit {
            auction_id: "auction-2".into(),
            timeout: None,
            ad_units: vec![]
        }
    );
    // Params are copied whole or not at all, and never at the page's expense.
    let self_reference = json!({ "placementId": 7, "self": "[…]" });
    let mut circular = self_reference;
    for _ in 0..3 {
        circular = json!({ "placementId": 7, "self": circular });
    }
    assert_eq!(
        auctions[2],
        &Kind::AuctionInit {
            auction_id: "auction-3".into(),
            timeout: None,
            ad_units: vec![TrackedAdUnit {
                code: "unit-h".into(),
                sizes: strings(&["300x250"]),
                bidders: strings(&["loop", "wide", "trap", "ice", "list", "nil"]),
                bids: vec![
                    TrackedBid {
                        bidder: "loop".into(),
                        params: circular,
                    },
                    TrackedBid {
                        bidder: "ice".into(),
                        params: json!({ "zoneId": 3, "nested": { "a": [1] } }),
                    },
                ],
            }],
        }
    );
    assert_eq!(auctions.len(), 3);
    assert_eq!(
        kinds(&capture, "bidRequested"),
        [
            &Kind::BidRequested {
                auction_id: "auction-1".into(),
                bidder: "foo".into(),
                ad_unit_codes: strings(&["unit-a"]),
                gdpr_applies: Some(true),
                has_consent_string: Some(true),
                schain: vec![
                    SchainNode {
                        asi: "exchange.example".into(),
                        sid: "pub-1".into(),
                        hp: Some(true),
                    },
                    SchainNode {
                        asi: "reseller.example".into(),
                        sid: "77".into(),
                        hp: Some(false),
                    },
                ],
                bidder_request_id: Some("request-foo".into()),
                bids: vec![
                    TrackedBidRequest {
                        bid_id: "bid-1".into(),
                        ad_unit_code: "unit-a".into(),
                    },
                    TrackedBidRequest {
                        bid_id: "bid-2".into(),
                        ad_unit_code: "unit-a".into(),
                    },
                ],
            },
            &Kind::BidRequested {
                auction_id: "auction-1".into(),
                bidder: "bar".into(),
                ad_unit_codes: strings(&["unit-a", "unit-b"]),
                gdpr_applies: Some(true),
                has_consent_string: Some(false),
                schain: vec![SchainNode {
                    asi: "legacy.example".into(),
                    sid: "s-2".into(),
                    hp: None,
                }],
                bidder_request_id: None,
                bids: Vec::new(),
            },
        ]
    );

    // -- Consent: the open banner, then the reader's choice, reported once.
    // The TC string itself never leaves the page.
    assert_eq!(
        kinds(&capture, "cmpDetected"),
        [&Kind::CmpDetected { api: "tcf".into() }]
    );
    assert_eq!(
        kinds(&capture, "tcfState"),
        [
            &Kind::TcfState {
                event_status: Some("cmpuishown".into()),
                gdpr_applies: Some(true),
                has_tc_string: false,
                purpose_consents: vec![],
                vendor_consents: vec![],
            },
            &Kind::TcfState {
                event_status: Some("useractioncomplete".into()),
                gdpr_applies: Some(true),
                has_tc_string: true,
                purpose_consents: vec![1, 7],
                vendor_consents: vec![32, 76],
            },
        ]
    );
    assert!(
        !serde_json::to_string(&capture.events)
            .unwrap()
            .contains("CPsecretTcString"),
        "the consent string must not be captured"
    );
    let replayed = capture
        .events
        .iter()
        .find(|event| variant(&event.kind) == "auctionInit")
        .unwrap();
    let detected = capture
        .events
        .iter()
        .find(|event| variant(&event.kind) == "prebidDetected")
        .unwrap();
    assert!(
        replayed.t < detected.t,
        "replayed history keeps the time it originally happened"
    );

    assert_eq!(
        kinds(&capture, "bidResponse"),
        [
            &Kind::BidResponse {
                auction_id: "auction-1".into(),
                bidder: "foo".into(),
                ad_unit_code: "unit-a".into(),
                cpm: 1.5,
                currency: Some("EUR".into()),
                size: Some("300x250".into()),
                time_to_respond: Some(187.0),
                deal_id: Some("deal-7".into()),
            },
            &Kind::BidResponse {
                auction_id: "auction-1".into(),
                bidder: "bar".into(),
                ad_unit_code: "unit-b".into(),
                cpm: 0.42,
                currency: None,
                size: Some("728x90".into()),
                time_to_respond: None,
                deal_id: None,
            },
            &Kind::BidResponse {
                auction_id: "unknown".into(),
                bidder: "unknown".into(),
                ad_unit_code: "unknown".into(),
                cpm: 0.0,
                currency: None,
                size: None,
                time_to_respond: None,
                deal_id: None,
            },
        ]
    );
    assert_eq!(
        kinds(&capture, "noBid"),
        [&Kind::NoBid {
            auction_id: "auction-1".into(),
            bidder: "bar".into(),
            ad_unit_code: "unit-a".into()
        }]
    );
    let timeout = |auction: &str, bidder: &str, unit: &str| Kind::BidTimeout {
        auction_id: auction.into(),
        bidder: bidder.into(),
        ad_unit_code: unit.into(),
    };
    assert_eq!(
        kinds(&capture, "bidTimeout"),
        [
            &timeout("auction-1", "slow", "unit-a"),
            &timeout("auction-1", "slow", "unit-b"),
            &timeout("unknown", "lonely", "unknown"),
        ]
    );
    assert_eq!(
        kinds(&capture, "bidderError"),
        [
            &Kind::BidderError {
                auction_id: Some("auction-1".into()),
                bidder: "broken".into(),
                message: Some("HTTP 503".into())
            },
            &Kind::BidderError {
                auction_id: None,
                bidder: "worse".into(),
                message: Some("adapter exploded".into())
            },
            &Kind::BidderError {
                auction_id: Some("auction-1".into()),
                bidder: "stalled".into(),
                message: Some("request timed out".into())
            },
            // The status says more than an error page's markup.
            &Kind::BidderError {
                auction_id: Some("auction-1".into()),
                bidder: "verbose".into(),
                message: Some("HTTP 500".into())
            },
        ]
    );
    assert_eq!(
        kinds(&capture, "auctionEnd"),
        [&Kind::AuctionEnd {
            auction_id: "auction-1".into()
        }]
    );
    assert_eq!(
        kinds(&capture, "setTargeting"),
        [
            &Kind::SetTargeting {
                ad_unit_codes: strings(&["unit-a", "unit-b"])
            },
            &Kind::SetTargeting {
                ad_unit_codes: vec![]
            },
        ]
    );
    assert_eq!(
        kinds(&capture, "bidWon"),
        [
            &Kind::BidWon {
                auction_id: Some("auction-1".into()),
                bidder: "foo".into(),
                ad_unit_code: "unit-a".into(),
                cpm: 1.5
            },
            &Kind::BidWon {
                auction_id: None,
                bidder: "unknown".into(),
                ad_unit_code: "unknown".into(),
                cpm: 0.0
            },
        ]
    );
    assert_eq!(
        kinds(&capture, "adRenderSucceeded"),
        [&Kind::AdRenderSucceeded {
            ad_unit_code: Some("unit-a".into()),
            bidder: Some("foo".into())
        }]
    );
    assert_eq!(
        kinds(&capture, "adRenderFailed"),
        [&Kind::AdRenderFailed {
            ad_unit_code: None,
            bidder: None,
            reason: Some("cannotFindAd".into())
        }]
    );

    // -- Prebid log lines: warnings and errors once (auctionDebug), the rest from the console.
    let log = |level: &str, message: &str| Kind::PrebidLog {
        level: level.into(),
        message: message.into(),
    };
    assert_eq!(
        kinds(&capture, "prebidLog"),
        [
            &log("warning", r#"Slow bidder {"bidder":"slow","ms":3000}"#),
            &log("error", "boom"),
            &log("info", r#"Auction finished {"winners":1}"#),
            &log("log", "adapter line"),
        ]
    );

    // -- GPT: one slot existed before the tracker saw the library, three were
    //    defined through the wrapped factories.
    let defined =
        |id: &str, path: &str, sizes: &[&str], in_viewport: Option<bool>| Kind::SlotDefined {
            element_id: id.into(),
            ad_unit_path: Some(path.into()),
            sizes: strings(sizes),
            in_viewport,
        };
    assert_eq!(
        kinds(&capture, "gptDetected"),
        [&Kind::GptDetected {
            version: Some("20261101".into())
        }]
    );
    assert_eq!(
        kinds(&capture, "slotDefined"),
        [
            &defined("unit-a", "/1/site/early", &["300x250", "fluid"], Some(true)),
            &defined("unit-b", "/1/site/below", &["728x90"], Some(false)),
            &defined("unit-hidden", "/1/site/hidden", &["1x1"], Some(false)),
            &defined("unit-missing", "/1/site/interstitial", &[], None),
        ]
    );
    let requested = |id: &str, keys: &[&str], ad_id: Option<&str>| Kind::SlotRequested {
        element_id: id.into(),
        prebid_targeting_keys: strings(keys),
        prebid_ad_id: ad_id.map(str::to_string),
    };
    assert_eq!(
        kinds(&capture, "slotRequested"),
        [
            &requested("unit-a", &["hb_bidder", "hb_pb"], None),
            &requested("unit-b", &[], None),
            &requested("unit-missing", &["hb_adid"], Some("ad-9")),
        ]
    );
    assert_eq!(
        kinds(&capture, "slotRenderEnded"),
        [
            &Kind::SlotRenderEnded {
                element_id: "unit-a".into(),
                is_empty: false,
                size: Some("300x250".into()),
                advertiser_id: Some("123".into()),
                line_item_id: Some("456".into()),
            },
            &Kind::SlotRenderEnded {
                element_id: "unit-b".into(),
                is_empty: true,
                size: None,
                advertiser_id: None,
                line_item_id: None,
            },
            &Kind::SlotRenderEnded {
                element_id: "unit-missing".into(),
                is_empty: true,
                size: Some("fluid".into()),
                advertiser_id: None,
                line_item_id: Some("789".into()),
            },
        ]
    );
    for single in ["slotResponseReceived", "slotOnload", "impressionViewable"] {
        assert_eq!(kinds(&capture, single).len(), 1, "{single}");
    }

    // -- Amazon: the fetch queued by the stub is reported once, not again when
    //    the real library replays it.
    assert_eq!(kinds(&capture, "amazonDetected").len(), 1);
    assert_eq!(
        kinds(&capture, "apsFetchBids"),
        [
            &Kind::ApsFetchBids {
                slot_ids: strings(&["unit-a", "unit-b"]),
                timeout: Some(900.0)
            },
            &Kind::ApsFetchBids {
                slot_ids: strings(&["unit-c"]),
                timeout: None
            },
        ]
    );
    let returned = kinds(&capture, "apsBidsReturned");
    assert_eq!(returned.len(), 2);
    assert!(
        matches!(returned[0], Kind::ApsBidsReturned { slot_ids, duration: Some(duration) }
            if slot_ids == &strings(&["unit-a"]) && (150.0..2_000.0).contains(duration)),
        "{:?}",
        returned[0]
    );

    // -- Page lifecycle.
    let phases: Vec<&str> = kinds(&capture, "pageLifecycle")
        .into_iter()
        .map(|kind| match kind {
            Kind::PageLifecycle { phase } => phase.as_str(),
            _ => unreachable!(),
        })
        .collect();
    assert_eq!(phases[..2], ["domContentLoaded", "load"]);
    assert_eq!(
        phases[2..].iter().copied().collect::<BTreeSet<_>>(),
        BTreeSet::from(["firstInteraction", "firstScroll"])
    );
    assert!(
        capture.max_scroll_depth > 0.3,
        "depth {}",
        capture.max_scroll_depth
    );
}

#[tokio::test]
async fn slot_removal_refusal_and_route_changes_are_reported() {
    let capture = kitchen_sink().await;

    assert_eq!(
        kinds(&capture, "slotDefinitionRejected"),
        [&Kind::SlotDefinitionRejected {
            element_id: "unit-b".into()
        }]
    );
    assert_eq!(
        kinds(&capture, "slotsDestroyed"),
        [&Kind::SlotsDestroyed {
            element_ids: strings(&["unit-hidden", "unit-missing"])
        }]
    );

    let page = capture.final_url.clone();
    let route = |query: &str, navigation_type: &str| Kind::RouteChange {
        url: format!("{page}{query}"),
        navigation_type: navigation_type.into(),
    };
    assert_eq!(
        kinds(&capture, "routeChange"),
        [
            &route("?route=2", "push"),
            &route("?route=3", "replace"),
            &route("?route=3#section", "push"),
        ]
    );

    // The pushed route is a page view; tidying its address and jumping
    // within it are not.
    assert_eq!(capture.pages.len(), 2, "{:#?}", capture.pages);
    let (entry, routed) = (&capture.pages[0], &capture.pages[1]);
    assert_eq!(entry.reached_by, PageReach::Entry);
    assert!(entry.load_ms.is_some());
    assert_eq!(routed.reached_by, PageReach::RouteChange);
    assert_eq!(routed.url, format!("{page}?route=3"));
    assert_eq!(routed.load_ms, None);
    assert_eq!(routed.link, None);
    assert_eq!(entry.ended_at_ms, routed.started_at_ms);
    assert!(routed.started_at_ms > 400.0 && routed.ended_at_ms > routed.started_at_ms);

    let first_on_route = capture.events.iter().find(|event| event.page == 1).unwrap();
    assert_eq!(first_on_route.kind, route("?route=2", "push"));
    assert_eq!(
        first_on_route.t, 0.0,
        "the route change starts its page view"
    );
    assert!(
        capture
            .events
            .iter()
            .filter(|event| event.page == 1)
            .count()
            > 3,
        "what happens after the route change belongs to its page view"
    );
    assert!(
        capture
            .events
            .iter()
            .all(|event| event.page <= 1 && event.t >= 0.0)
    );
    let in_order = capture
        .events
        .windows(2)
        .all(|pair| (pair[0].page, pair[0].t) <= (pair[1].page, pair[1].t));
    assert!(in_order, "events are sorted by page view, then time");
}

#[tokio::test]
async fn prebid_config_snapshot_is_bounded_and_json_safe() {
    let capture = kitchen_sink().await;
    let configs = kinds(&capture, "prebidConfig");
    let [
        Kind::PrebidConfig {
            bidder_timeout,
            price_granularity,
            has_floors,
            has_user_sync,
            has_consent_management,
            currency,
            raw,
        },
    ] = configs[..]
    else {
        panic!("expected one config snapshot, got {configs:?}");
    };
    assert_eq!(*bidder_timeout, Some(1500.0));
    assert_eq!(price_granularity.as_deref(), Some("custom"));
    assert!(*has_floors && *has_consent_management && !*has_user_sync);
    assert_eq!(currency.as_deref(), Some("EUR"));

    assert_eq!(raw["bidderTimeout"], 1500);
    assert_eq!(raw["consentManagement"]["gdpr"]["timeout"], 8000);
    assert_eq!(raw["s2sConfig"]["bidders"][0], "foo");
    assert_eq!(
        raw["s2sConfig"].get("onTimeout"),
        None,
        "functions are stripped"
    );
    assert_eq!(raw["realTimeData"], "[omitted: too large]");
    assert_eq!(
        raw.get("debug"),
        None,
        "debug is the tracker's own switch, not the publisher's"
    );
    assert!(raw.to_string().len() < 20_000);
    let config_event = capture
        .events
        .iter()
        .position(|event| variant(&event.kind) == "prebidConfig")
        .unwrap();
    assert_eq!(
        variant(&capture.events[config_event + 1].kind),
        "auctionInit"
    );
}

#[tokio::test]
async fn the_tracker_stays_out_of_the_pages_way() {
    let capture = kitchen_sink().await;
    let state = capture
        .console
        .iter()
        .find_map(|line| line.text.strip_prefix("PAGE_STATE "))
        .expect("the page reports its state");
    let state: Value = serde_json::from_str(state).unwrap();
    assert_eq!(
        state["debugRequested"], true,
        "Prebid debug logging is switched on"
    );
    assert_eq!(
        state["apsCallbacks"], 2,
        "the page's own callbacks still run exactly once each"
    );
    assert_eq!(state["apsFirstBids"], 1);
    assert_eq!(
        state["defineSlotLooksNative"], true,
        "wrapped functions print as the originals"
    );
    assert_eq!(state["definedSlotsAreUntouched"], true);
    assert_eq!(state["consoleLooksNative"], true);
    assert_eq!(
        state["bindingVisible"], false,
        "the transport binding is hidden from the page"
    );
    assert_eq!(
        state["bindingVisibleInFrame"], false,
        "and from the frames the page creates"
    );
    assert_eq!(
        state["pbjsCreated"], false,
        "no globals are created for libraries the page does not use"
    );
    assert_eq!(
        state["hostileParamsUntouched"], true,
        "reading params leaves them as the page made them"
    );
    assert_eq!(
        state["consentListeners"], 1,
        "the consent platform is listened to once"
    );
    assert_eq!(
        state["secondSlotForOneContainer"],
        Value::Null,
        "a refused slot definition still returns what the library returned"
    );
    assert_eq!(state["destroyedSome"], true);
    assert_eq!(
        state["slotsLeft"], 2,
        "the page's slots are destroyed as asked"
    );
    assert_eq!(state["destroySlotsLooksNative"], true);
    assert_eq!(
        state["historyLooksNative"], true,
        "route changes are observed without touching history"
    );
    assert!(
        !capture.console.iter().any(|line| line.level == "error"),
        "the tracker must not cause page errors: {:#?}",
        capture.console
    );

    // Scripts: both inline blocks of the page, the tag-manager style injected
    // script, but neither the tracker itself nor one-line evals.
    let bodies: Vec<&str> = capture
        .scripts
        .iter()
        .map(|script| script.body.as_str())
        .collect();
    assert!(capture.scripts.iter().all(|script| script.inline));
    assert!(
        bodies
            .iter()
            .any(|body| body.contains("window._pbjsGlobals = ['acmeHb']"))
    );
    assert!(
        bodies
            .iter()
            .any(|body| body.starts_with("window.injectedByTagManager"))
    );
    assert!(
        !bodies
            .iter()
            .any(|body| body.contains("TINY_EVAL_MARKER\"") && body.len() < 100)
    );
    assert!(!bodies.iter().any(|body| body.contains("Ad-stack tracker")));
    let injected = capture
        .scripts
        .iter()
        .find(|script| script.body.starts_with("window.injectedByTagManager"))
        .unwrap();
    assert_eq!(
        injected.initiator.as_deref(),
        Some(capture.final_url.as_str())
    );
}

/// A wrapper that asks Prebid for bids with no ad unit to auction: once
/// before the tracker could subscribe, once after.
#[tokio::test]
async fn what_the_page_asks_prebid_for_is_reported_and_the_call_goes_on_as_made() {
    let fixtures = support::Fixtures::start().await;
    let mut request = support::request(
        &fixtures.url("prebid-requests.html"),
        DeviceProfile::Desktop,
    );
    request.visitor = VisitorBehavior::None;
    request.observe_seconds = 3;
    let capture = support::capture_ok(request).await;

    let asked: Vec<&Kind> = kinds(&capture, "bidsRequested");
    assert_eq!(
        asked,
        [
            // From Prebid's history: that it happened, not with what.
            &Kind::BidsRequested {
                ad_unit_codes: vec![],
                ad_unit_count: None,
                timeout: None,
            },
            &Kind::BidsRequested {
                ad_unit_codes: strings(&["top", "side"]),
                ad_unit_count: Some(0),
                timeout: Some(900.0),
            },
        ]
    );
    assert_eq!(
        kinds(&capture, "adUnitsAdded"),
        [
            &Kind::AdUnitsAdded { registered: None },
            &Kind::AdUnitsAdded {
                registered: Some(0)
            },
        ]
    );
    assert!(kinds(&capture, "auctionInit").is_empty());
    // The first request is timed when it happened, not when it was replayed.
    let times: Vec<f64> = capture
        .events
        .iter()
        .filter(|event| matches!(event.kind, Kind::BidsRequested { .. }))
        .map(|event| event.t)
        .collect();
    assert!(times[0] < 300.0 && times[1] >= 600.0, "{times:?}");

    // Both calls reached Prebid once each, with the arguments and the
    // receiver the page gave them.
    let made: Vec<&str> = capture
        .console
        .iter()
        .filter_map(|line| line.text.strip_prefix("REQUEST "))
        .collect();
    let as_made = r#"this=true {"adUnitCodes":["top","side"],"adUnits":[],"timeout":900}"#;
    assert_eq!(made, [as_made, as_made]);
}
