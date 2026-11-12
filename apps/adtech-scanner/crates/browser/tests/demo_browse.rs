//! A visitor browsing the built-in demo publisher site: it must act with real
//! input, follow only the links a visitor may follow, and its further page
//! views must show the bugs planted for them (see `demo-site/README.md`,
//! bugs (h), (i) and (j)).

mod support;

use std::collections::{BTreeMap, BTreeSet};

use adscan_model::{
    ActivityKind, Capture, CaptureRequest, DeviceProfile, PageReach, TrackerEvent,
    TrackerEventKind as Kind, VisitorBehavior,
};
use serde_json::Value;
use support::Outcome;

/// Paths the demo pages link to that no visitor of the scanner may request,
/// and the click-through address of the ads.
const TRAPS: [&str; 6] = [
    "/account/login",
    "/logout",
    "/files/media-kit.pdf",
    "/sponsored/partner-story",
    "/sponsored/offer",
    "/ads/click",
];
/// Where the demo's bidders ask their stand-in exchanges.
const EXCHANGES: [&str; 7] = [
    "/ut/v3/prebid",
    "/a/api/fastlane.json",
    "/openrtb/pbjs",
    "/translator",
    "/openrtb_2_5/pbjs/auction/request",
    "/header/auction",
    "/openrtbb/prebidjs",
];
/// The containers every article page has; `ad-sidebar-2` is an ad unit
/// without one.
const SLOTS: [&str; 5] = [
    "ad-leaderboard",
    "ad-sidebar-1",
    "ad-inarticle-1",
    "ad-inarticle-2",
    "ad-sticky-footer",
];

struct Visit {
    capture: Capture,
    outcome: Outcome,
    /// How often the demo site was asked for each path.
    hits: BTreeMap<String, u32>,
    origin: String,
}

async fn browse(device: DeviceProfile, seed: u32, extra_pages: u32, observe_seconds: u32) -> Visit {
    let site = adscan_browser::start_demo_site().await.unwrap();
    let mut request: CaptureRequest = support::request(&site.url, device);
    request.visitor = VisitorBehavior::Browse;
    request.extra_pages = extra_pages;
    request.observe_seconds = observe_seconds;
    request.seed = seed;
    let mut outcome = support::run(request).await;
    let result = std::mem::replace(
        &mut outcome.result,
        Err(adscan_browser::BrowserError::Cancelled),
    );
    let capture = result.unwrap_or_else(|error| {
        panic!("capture failed: {error}\nprogress: {:#?}", outcome.progress)
    });
    assert!(
        !outcome
            .progress
            .iter()
            .any(|line| line.contains("malformed")),
        "{:#?}",
        outcome.progress
    );
    let hits = support::http_get(&format!("{}__demo/hits", site.url)).await;
    Visit {
        capture,
        outcome,
        hits: serde_json::from_str(&hits).unwrap(),
        origin: site.url.trim_end_matches('/').to_string(),
    }
}

impl Visit {
    fn on_page(&self, page: usize) -> impl Iterator<Item = &TrackerEvent> {
        self.capture
            .events
            .iter()
            .filter(move |event| event.page as usize == page)
    }

    /// The ad units of each auction that started on a page view.
    fn auctions_on(&self, page: usize) -> Vec<Vec<&str>> {
        self.on_page(page)
            .filter_map(|event| match &event.kind {
                Kind::AuctionInit { ad_units, .. } => {
                    Some(ad_units.iter().map(|unit| unit.code.as_str()).collect())
                }
                _ => None,
            })
            .collect()
    }

    /// Time, targeting keys and Prebid ad id of every ad request of a slot
    /// on a page view.
    fn requests_of(&self, page: usize, slot: &str) -> Vec<(f64, usize, Option<&str>)> {
        self.on_page(page)
            .filter_map(|event| match &event.kind {
                Kind::SlotRequested {
                    element_id,
                    prebid_targeting_keys,
                    prebid_ad_id,
                } if element_id == slot => Some((
                    event.t,
                    prebid_targeting_keys.len(),
                    prebid_ad_id.as_deref(),
                )),
                _ => None,
            })
            .collect()
    }

    fn renders_of(&self, page: usize, slot: &str) -> Vec<bool> {
        self.on_page(page)
            .filter_map(|event| match &event.kind {
                Kind::SlotRenderEnded {
                    element_id,
                    is_empty,
                    ..
                } if element_id == slot => Some(!is_empty),
                _ => None,
            })
            .collect()
    }

    fn assert_no_trap_was_requested(&self) {
        for path in self.hits.keys() {
            assert!(
                !TRAPS.iter().any(|trap| path.starts_with(trap)),
                "{path} was requested: {:#?}",
                self.outcome.progress
            );
            let is_known = path == "/"
                || [
                    "/climate/",
                    "/fragments/",
                    "/static/",
                    "/tag/",
                    "/usersync/",
                ]
                .iter()
                .any(|prefix| path.starts_with(prefix))
                || [
                    "/collect",
                    "/collect/sampling.json",
                    "/api/archive-index.json",
                    "/ads.txt",
                ]
                .contains(&path.as_str())
                || EXCHANGES.contains(&path.as_str());
            assert!(is_known, "{path} was requested");
        }
        for page in &self.capture.pages {
            let path = page.url.trim_start_matches(&self.origin);
            assert!(
                path == "/" || path.starts_with("/climate/"),
                "the visit went to {}",
                page.url
            );
        }
    }

    /// What the page's own analytics counted of the visitor's input, per
    /// report it made.
    fn engagement(&self) -> Vec<Value> {
        self.capture
            .console
            .iter()
            .filter_map(|line| line.text.strip_prefix("ENGAGEMENT "))
            .map(|counted| serde_json::from_str(counted).unwrap())
            .collect()
    }

    /// Bug (i): a page view reached by loading a document, in a session that
    /// auctioned moments ago, asks the ad server without asking the bidders.
    /// The recirculation unit, which has its own code path, is not affected.
    fn assert_later_document_never_bids(&self, page: usize) {
        assert_eq!(
            self.auctions_on(page),
            [["ad-recirc"]],
            "only the recirculation unit is auctioned on page {page}"
        );
        let mut requested = 0;
        for slot in SLOTS {
            for (_, hb_keys, ad_id) in self.requests_of(page, slot) {
                requested += 1;
                assert_eq!((hb_keys, ad_id), (0, None), "{slot} on page {page}");
            }
            assert!(
                self.renders_of(page, slot).iter().all(|filled| !filled),
                "{slot} on page {page} cannot be filled"
            );
        }
        assert!(requested >= 2, "only {requested} slots were requested");
        let recirculation = self.requests_of(page, "ad-recirc");
        assert_eq!(recirculation.len(), 1);
        assert!(recirculation[0].1 >= 4 && recirculation[0].2.is_some());
        assert_eq!(self.renders_of(page, "ad-recirc"), [true]);
    }

    /// Bug (j): after a route change every slot is defined again and refused,
    /// and nothing is requested from the ad server for the new article.
    fn assert_route_change_requests_no_ads(&self, page: usize) {
        let refused: BTreeSet<&str> = self
            .on_page(page)
            .filter_map(|event| match &event.kind {
                Kind::SlotDefinitionRejected { element_id } => Some(element_id.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(refused, BTreeSet::from(SLOTS), "page {page}");
        for slot in SLOTS {
            assert_eq!(self.requests_of(page, slot), [], "{slot} on page {page}");
        }
        assert!(
            !self
                .on_page(page)
                .any(|event| matches!(event.kind, Kind::SlotsDestroyed { .. })),
            "the wrapper never destroys the slots of the article it leaves"
        );
    }
}

#[tokio::test]
async fn a_browsing_visitor_acts_with_real_input_and_takes_no_trap() {
    let visit = browse(DeviceProfile::Desktop, 3, 2, 14).await;
    let capture = &visit.capture;

    assert!(capture.pages.len() >= 2, "{:#?}", visit.outcome.progress);
    visit.assert_no_trap_was_requested();
    assert_eq!(visit.hits["/"], 1);
    assert_eq!(
        capture.environment.request_hosts,
        ["localhost"],
        "the demo site asks its partners' stand-ins on this machine and nobody else"
    );

    // The page's own analytics saw a reader: pointer and wheel, all of it
    // input a person's devices would cause.
    let engagement = visit.engagement();
    assert!(!engagement.is_empty(), "{:#?}", capture.console);
    assert!(engagement.iter().all(|report| report["trusted"] == true));
    assert!(engagement.iter().all(|report| report["touches"] == 0));
    assert!(
        engagement.iter().any(|report| {
            report["pointerMoves"].as_u64() > Some(0) && report["wheels"].as_u64() > Some(0)
        }),
        "{engagement:?}"
    );
    assert!(capture.pages[0].max_scroll_depth > 0.0);
    assert!(visit.on_page(0).any(|event| event.kind
        == Kind::PageLifecycle {
            phase: "firstPointerMove".into()
        }));

    // Every choice was among articles, and what was passed over says why.
    let choices = visit.outcome.activities_of(ActivityKind::LinkChoice);
    assert!(!choices.is_empty());
    for choice in &choices {
        let detail = choice.data.as_ref().unwrap();
        if let Some(href) = detail["chosen"]["href"].as_str() {
            assert!(
                href.starts_with(&format!("{}/climate/", visit.origin)),
                "{href}"
            );
        }
        let skipped = &detail["skipped"];
        for (reason, at_least) in [
            ("sensitive", 1),
            ("file", 1),
            ("rel", 1),
            ("scheme", 1),
            ("off-site", 1),
            ("inside-ad", 1),
        ] {
            assert!(
                skipped[reason].as_u64() >= Some(at_least),
                "{reason}: {skipped}"
            );
        }
    }

    // Bug (h): the rail ad a reader keeps in view is refreshed without a
    // new auction, so the ad server is offered the bid it already used.
    let rail = visit.requests_of(0, "ad-sidebar-1");
    assert_eq!(rail.len(), 2, "{rail:?}");
    let (first, again) = (rail[0], rail[1]);
    assert!(first.2.is_some() && first.2 == again.2, "{rail:?}");
    assert!(again.0 > first.0 + 5_000.0, "{rail:?}");
    let rail_auctions = visit
        .auctions_on(0)
        .iter()
        .filter(|units| units.contains(&"ad-sidebar-1"))
        .count();
    assert_eq!(rail_auctions, 1, "no auction for the refresh");
    let failed: Vec<f64> = visit
        .on_page(0)
        .filter(|event| {
            matches!(&event.kind, Kind::AdRenderFailed { ad_unit_code, .. }
                if ad_unit_code.as_deref() == Some("ad-sidebar-1"))
        })
        .map(|event| event.t)
        .collect();
    assert_eq!(failed.len(), 1);
    assert!(failed[0] > again.0);

    // The later page views show the bug planted for the way they were reached.
    for (index, page) in capture.pages.iter().enumerate().skip(1) {
        let opened_in_a_new_tab = page
            .link
            .as_ref()
            .is_some_and(|link| link.href.ends_with("/climate/white-roofs"))
            && page.reached_by == PageReach::LinkClick;
        match page.reached_by {
            // A new tab starts a session of its own, where nothing is wrong.
            PageReach::LinkClick if opened_in_a_new_tab => {}
            PageReach::LinkClick => visit.assert_later_document_never_bids(index),
            PageReach::RouteChange => visit.assert_route_change_requests_no_ads(index),
            other => panic!("page {index} was reached by {other:?}"),
        }
        assert!(page.link.is_some() && page.title.is_some(), "{page:#?}");
    }
    let wrapper = capture
        .scripts
        .iter()
        .find(|script| script.url.ends_with("/static/ads/hb-wrapper.min.js"))
        .unwrap();
    let documents: Vec<u32> = capture
        .pages
        .iter()
        .enumerate()
        .filter(|(_, page)| page.reached_by != PageReach::RouteChange)
        .map(|(index, _)| index as u32)
        .collect();
    assert_eq!(
        wrapper.pages, documents,
        "a script is listed by every page view that loaded it"
    );
}

#[tokio::test]
async fn the_planted_bugs_of_later_page_views_show_for_both_ways_of_getting_there() {
    // With these seeds the first link followed loads a document (3) and
    // changes the route in place (6). The visit is planned from the seed, so
    // this holds from run to run.
    let loaded = browse(DeviceProfile::Desktop, 3, 1, 8).await;
    assert_eq!(
        loaded.capture.pages[1].reached_by,
        PageReach::LinkClick,
        "{:#?}",
        loaded.outcome.progress
    );
    loaded.assert_no_trap_was_requested();
    loaded.assert_later_document_never_bids(1);
    assert_eq!(loaded.capture.pages.len(), 2);

    let routed = browse(DeviceProfile::Desktop, 6, 1, 6).await;
    assert_eq!(
        routed.capture.pages[1].reached_by,
        PageReach::RouteChange,
        "{:#?}",
        routed.outcome.progress
    );
    routed.assert_no_trap_was_requested();
    routed.assert_route_change_requests_no_ads(1);
    // On the entry document nothing holds the auction back: it runs, with
    // every ad unit in it twice, and its bids are never used.
    let auctions = routed.auctions_on(1);
    assert_eq!(auctions.len(), 1, "{auctions:?}");
    let leaderboards = auctions[0]
        .iter()
        .filter(|unit| **unit == "ad-leaderboard")
        .count();
    assert_eq!(leaderboards, 2, "{auctions:?}");
    assert!(
        routed
            .on_page(1)
            .any(|event| matches!(event.kind, Kind::BidResponse { .. }))
    );
    let article = routed.capture.pages[1]
        .url
        .trim_start_matches(&routed.origin)
        .to_string();
    assert_eq!(
        routed.hits.get(&article),
        None,
        "the article was shown without loading its page"
    );
    let fragment = article.replace("/climate/", "/fragments/") + ".html";
    assert_eq!(routed.hits.get(&fragment), Some(&1));
    assert_eq!(routed.capture.pages[1].load_ms, None);
}

#[tokio::test]
async fn a_phone_visitor_browses_the_demo_site_by_touch() {
    let visit = browse(DeviceProfile::Mobile, 3, 1, 8).await;

    assert_eq!(
        visit.capture.pages.len(),
        2,
        "{:#?}",
        visit.outcome.progress
    );
    visit.assert_no_trap_was_requested();
    let engagement = visit.engagement();
    assert!(
        engagement
            .iter()
            .any(|report| report["touches"].as_u64() > Some(0)),
        "{engagement:?}"
    );
    assert!(
        engagement
            .iter()
            .all(|report| report["wheels"] == 0 && report["trusted"] == true),
        "{engagement:?}"
    );
    let tapped = visit
        .outcome
        .activities_of(ActivityKind::Action)
        .iter()
        .any(|action| action.data.as_ref().unwrap()["action"] == "tap");
    assert!(tapped, "{:#?}", visit.outcome.progress);
}
