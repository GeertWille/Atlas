//! The scripted visitor on pages where only one thing can happen: which link
//! it follows, how the next page view is recorded, and what it refuses to do.

mod support;

use adscan_model::{
    ActivityKind, Capture, CaptureRequest, DeviceProfile, PageReach, TrackerEventKind as Kind,
    VisitorBehavior,
};
use serde_json::json;
use support::{Fixtures, Outcome};

fn browse(url: &str, extra_pages: u32) -> CaptureRequest {
    let mut request = support::request(url, DeviceProfile::Desktop);
    request.visitor = VisitorBehavior::Browse;
    request.extra_pages = extra_pages;
    request.observe_seconds = 4;
    request.seed = 7;
    request
}

/// Runs a visit that must succeed.
async fn visit(request: CaptureRequest) -> (Capture, Outcome) {
    let mut outcome = support::run(request).await;
    let result = std::mem::replace(
        &mut outcome.result,
        Err(adscan_browser::BrowserError::Cancelled),
    );
    let capture = result.unwrap_or_else(|error| {
        panic!("capture failed: {error}\nprogress: {:#?}", outcome.progress)
    });
    (capture, outcome)
}

fn console_of(capture: &Capture, page: u32) -> Vec<&str> {
    capture
        .console
        .iter()
        .filter(|line| line.page == page)
        .map(|line| line.text.as_str())
        .collect()
}

fn says(capture: &Capture, text: &str) -> bool {
    capture.console.iter().any(|line| line.text == text)
}

fn blocked(outcome: &Outcome, reason: &str) -> usize {
    outcome
        .activities_of(ActivityKind::Blocked)
        .iter()
        .filter(|activity| activity.data.as_ref().unwrap()["reason"] == reason)
        .count()
}

fn requests_of(fixtures: &Fixtures, path: &str) -> usize {
    let requested = fixtures.requested();
    requested.iter().filter(|asked| *asked == path).count()
}

#[tokio::test]
async fn a_followed_link_opens_a_second_page_view_with_its_own_records() {
    let fixtures = Fixtures::start().await;
    let (capture, outcome) = visit(browse(&fixtures.url("browse-hard.html"), 1)).await;

    assert_eq!(capture.pages.len(), 2, "{:#?}", outcome.progress);
    let (first, second) = (&capture.pages[0], &capture.pages[1]);
    assert_eq!(capture.final_url, fixtures.url("browse-hard.html"));
    assert_eq!(first.reached_by, PageReach::Entry);
    assert_eq!(first.title.as_deref(), Some("Browse: first page"));
    assert_eq!(second.reached_by, PageReach::LinkClick);
    assert_eq!(second.url, fixtures.url("browse-hard-2.html"));
    assert_eq!(second.title.as_deref(), Some("Browse: second page"));
    let link = second.link.as_ref().expect("the link that was followed");
    assert_eq!(
        link.text,
        "The second page of this little site, a long read"
    );
    assert_eq!(link.href, fixtures.url("browse-hard-2.html"));
    assert!(
        link.reason.ends_with("chosen among the best 1"),
        "{}",
        link.reason
    );

    // One clock per page view, joined end to start.
    assert_eq!(first.started_at_ms, 0.0);
    assert_eq!(first.ended_at_ms, second.started_at_ms);
    assert!(second.started_at_ms > 4_000.0, "{}", second.started_at_ms);
    assert!(second.ended_at_ms > second.started_at_ms + 4_000.0);
    assert!(first.load_ms.is_some() && second.load_ms.is_some());
    assert!(
        second.load_ms.unwrap() < 3_000.0,
        "counted from its own start"
    );
    assert!(first.max_scroll_depth > 0.2, "the link was scrolled to");
    assert_eq!(capture.page_load_ms, first.load_ms);

    for page in [0, 1] {
        let loaded = capture.events.iter().find(|event| {
            event.page == page
                && event.kind
                    == Kind::PageLifecycle {
                        phase: "load".into(),
                    }
        });
        let loaded = loaded.unwrap_or_else(|| panic!("page {page} has no load event"));
        assert!(
            loaded.t > 0.0 && loaded.t < 3_000.0,
            "page {page}: {}",
            loaded.t
        );
    }
    let shared = capture
        .scripts
        .iter()
        .find(|script| script.url.ends_with("/browse-shared.js"))
        .unwrap();
    assert_eq!(shared.pages, [0, 1], "one body, loaded by both page views");
    assert!(shared.loaded_at_ms.unwrap() < 3_000.0);

    // Real input reached the first page, and none of it was typed.
    let first_console = console_of(&capture, 0);
    assert!(first_console.contains(&"FIRST pointermove trusted=true"));
    assert!(first_console.contains(&"FIRST wheel trusted=true"));
    assert!(first_console.contains(&"FIRST mousedown trusted=true"));
    assert!(first_console.contains(&"CLICK trusted=true on=next"));
    assert!(
        !capture.console.iter().any(|line| line.text.contains("key")
            || line.text.contains("FIRST input")
            || line.text.contains("trusted=false")),
        "{:#?}",
        capture.console
    );
    assert!(capture.events.iter().any(|event| event.page == 0
        && event.kind
            == Kind::PageLifecycle {
                phase: "firstPointerMove".into()
            }));
    assert!(
        console_of(&capture, 1).contains(&"SECOND PAGE opener=false referrer=/browse-hard.html"),
        "{:#?}",
        console_of(&capture, 1)
    );
    assert!(says(&capture, "BINDING visible=false"));
    assert_eq!(requests_of(&fixtures, "/browse-hard-2.html"), 1);

    // The audit trail says what was chosen and what was done.
    let choices = outcome.activities_of(ActivityKind::LinkChoice);
    assert_eq!(choices.len(), 1, "{:#?}", outcome.progress);
    let choice = choices[0].data.as_ref().unwrap();
    assert_eq!(choice["chosen"]["href"], fixtures.url("browse-hard-2.html"));
    assert_eq!(choice["alternatives"], json!([]));
    assert_eq!(choice["skipped"], json!({ "anchor": 1 }));
    assert_eq!(choices[0].page, Some(0));
    let views: Vec<_> = outcome
        .activities_of(ActivityKind::PageView)
        .iter()
        .map(|view| {
            let detail = view.data.as_ref().unwrap();
            (detail["index"].clone(), detail["reachedBy"].clone())
        })
        .collect();
    assert_eq!(
        views,
        [(json!(0), json!("entry")), (json!(1), json!("linkClick"))]
    );
    let actions = outcome.activities_of(ActivityKind::Action);
    assert!(
        actions
            .iter()
            .any(|action| action.data.as_ref().unwrap()["action"] == "click"),
        "{:#?}",
        outcome.progress
    );
    assert!(actions.len() <= 2 * 40 + 2, "one entry per gesture, capped");
    assert_eq!(
        blocked(&outcome, "off-site") + blocked(&outcome, "dead-link"),
        0
    );
}

#[tokio::test]
async fn the_same_seed_makes_the_same_visit() {
    let fixtures = Fixtures::start().await;
    let gestures = |outcome: &Outcome| -> Vec<String> {
        outcome
            .activities
            .iter()
            .filter(|activity| {
                matches!(
                    activity.kind,
                    ActivityKind::Action | ActivityKind::LinkChoice
                )
            })
            .map(|activity| {
                let detail = activity.data.as_ref().unwrap();
                // What was done, without how far the page happened to scroll.
                format!(
                    "{:?} {} {}",
                    activity.page, detail["action"], detail["target"]
                )
            })
            .collect()
    };
    let (_, first) = visit(browse(&fixtures.url("browse-hard.html"), 1)).await;
    let (_, again) = visit(browse(&fixtures.url("browse-hard.html"), 1)).await;
    assert_eq!(gestures(&first), gestures(&again));
    assert!(gestures(&first).len() >= 3, "{:#?}", first.progress);

    let mut other_seed = browse(&fixtures.url("browse-hard.html"), 1);
    other_seed.seed = 8;
    other_seed.observe_seconds = 12;
    let mut same_but_longer = browse(&fixtures.url("browse-hard.html"), 1);
    same_but_longer.observe_seconds = 12;
    let (_, other) = visit(other_seed).await;
    let (_, longer) = visit(same_but_longer).await;
    assert_ne!(gestures(&other), gestures(&longer));
}

#[tokio::test]
async fn a_route_change_is_a_page_view_without_a_document() {
    let fixtures = Fixtures::start().await;
    let (capture, outcome) = visit(browse(&fixtures.url("browse-spa.html"), 2)).await;

    assert_eq!(capture.pages.len(), 2, "{:#?}", outcome.progress);
    let routed = &capture.pages[1];
    assert_eq!(routed.reached_by, PageReach::RouteChange);
    assert_eq!(routed.url, fixtures.url("browse-spa/story"));
    assert_eq!(routed.title.as_deref(), Some("Browse: the routed story"));
    assert_eq!(routed.load_ms, None);
    assert_eq!(
        routed.link.as_ref().unwrap().text,
        "A story that is shown without loading a page"
    );
    assert_eq!(capture.pages[0].ended_at_ms, routed.started_at_ms);

    let route = capture
        .events
        .iter()
        .find(|event| matches!(event.kind, Kind::RouteChange { .. }))
        .unwrap();
    assert_eq!((route.page, route.t), (1, 0.0));
    assert!(console_of(&capture, 1).contains(&"AFTER ROUTE at /browse-spa/story"));
    assert!(console_of(&capture, 0).contains(&"CLICK trusted=true on=route"));
    assert_eq!(
        requests_of(&fixtures, "/browse-spa/story"),
        0,
        "no document was loaded for the route"
    );

    // The routed page has nothing to follow: the visit ends there.
    let choices = outcome.activities_of(ActivityKind::LinkChoice);
    assert_eq!(choices.len(), 2);
    assert_eq!(choices[1].message, "No link to follow");
    assert_eq!(choices[1].data.as_ref().unwrap()["chosen"], json!(null));
}

#[tokio::test]
async fn a_tab_the_click_opens_becomes_the_recorded_tab() {
    let fixtures = Fixtures::start().await;
    let (capture, outcome) = visit(browse(&fixtures.url("browse-newtab.html"), 1)).await;

    assert_eq!(capture.pages.len(), 2, "{:#?}", outcome.progress);
    let second = &capture.pages[1];
    assert_eq!(second.reached_by, PageReach::LinkClick);
    assert_eq!(second.url, fixtures.url("browse-hard-2.html"));
    assert_eq!(second.title.as_deref(), Some("Browse: second page"));
    assert!(second.load_ms.is_some());

    // The tracker was in the new tab before the page's first script.
    let new_tab = console_of(&capture, 1);
    assert!(new_tab.contains(&"BINDING visible=false"), "{new_tab:#?}");
    assert!(new_tab.iter().any(|line| line.starts_with("SECOND PAGE")));
    assert!(capture.events.iter().any(|event| event.page == 1
        && event.kind
            == Kind::PageLifecycle {
                phase: "domContentLoaded".into()
            }));
    assert_eq!(blocked(&outcome, "popup"), 0);
}

#[tokio::test]
async fn a_link_that_redirects_off_the_site_is_stopped_before_the_other_site_is_asked() {
    let fixtures = Fixtures::start().await;
    let elsewhere = Fixtures::start().await;
    let other_site = elsewhere.url("emulation.html");
    let page = format!(
        "{}?{other_site}",
        fixtures.url("browse-offsite-redirect.html")
    );
    let (capture, outcome) = visit(browse(&page, 2)).await;

    assert_eq!(capture.pages.len(), 1, "{:#?}", outcome.progress);
    assert_eq!(
        elsewhere.requested(),
        Vec::<String>::new(),
        "nothing was requested from the other site"
    );
    assert_eq!(
        requests_of(&fixtures, &format!("/redirect?{other_site}")),
        1,
        "the link itself is on the site and was followed"
    );
    assert_eq!(blocked(&outcome, "off-site"), 1, "{:#?}", outcome.progress);
    let refusal = outcome
        .activities_of(ActivityKind::Blocked)
        .into_iter()
        .find(|activity| activity.data.as_ref().unwrap()["reason"] == "off-site")
        .unwrap();
    assert_eq!(
        refusal.data,
        Some(json!({ "reason": "off-site", "url": other_site, "host": "127.0.0.1" }))
    );

    // The plain link to the other site was never a candidate, and after the
    // refusal the visit follows nothing more.
    let choices = outcome.activities_of(ActivityKind::LinkChoice);
    assert_eq!(choices.len(), 1);
    assert_eq!(
        choices[0].data.as_ref().unwrap()["skipped"],
        json!({ "off-site": 1 })
    );
    assert!(says(&capture, "CLICK trusted=true on=redirecting"));
    assert_eq!(capture.final_url, page, "the visit is still on its page");
}

#[tokio::test]
async fn links_that_must_not_be_followed_are_never_requested() {
    let fixtures = Fixtures::start().await;
    let (capture, outcome) = visit(browse(&fixtures.url("browse-traps.html"), 1)).await;

    assert_eq!(capture.pages.len(), 2, "{:#?}", outcome.progress);
    assert_eq!(capture.pages[1].url, fixtures.url("browse-hard-2.html"));
    let trapped: Vec<String> = fixtures
        .requested()
        .into_iter()
        .filter(|asked| asked.starts_with("/trap/"))
        .collect();
    assert_eq!(trapped, Vec::<String>::new());

    let choice = outcome.activities_of(ActivityKind::LinkChoice)[0]
        .data
        .clone()
        .unwrap();
    assert_eq!(
        choice["skipped"],
        json!({
            "hidden": 1,
            "inside-ad": 1,
            "scheme": 1,
            "anchor": 1,
            "off-site": 1,
            "file": 1,
            "rel": 1,
            "sensitive": 1,
            "control": 2,
        })
    );
    assert_eq!(
        choice["chosen"]["text"],
        "The one link of this page that a visitor may follow"
    );
    assert_eq!(choice["alternatives"], json!([]));
}

#[tokio::test]
async fn a_link_under_something_else_is_not_clicked() {
    let fixtures = Fixtures::start().await;
    let (capture, outcome) = visit(browse(&fixtures.url("browse-covered.html"), 1)).await;

    assert_eq!(capture.pages.len(), 1, "{:#?}", outcome.progress);
    assert_eq!(blocked(&outcome, "hit-test"), 1, "{:#?}", outcome.progress);
    assert_eq!(requests_of(&fixtures, "/browse-hard-2.html"), 0);
    assert!(
        !capture
            .console
            .iter()
            .any(|line| line.text.starts_with("CLICK")),
        "nothing was clicked: {:#?}",
        capture.console
    );
    // The link was tried once and is not offered again.
    let choices = outcome.activities_of(ActivityKind::LinkChoice);
    assert_eq!(choices.len(), 2);
    assert_eq!(
        choices[1].data.as_ref().unwrap()["skipped"]["visited"],
        json!(1)
    );
}

#[tokio::test]
async fn a_phone_visitor_swipes_and_taps() {
    let fixtures = Fixtures::start().await;
    let mut request = browse(&fixtures.url("browse-hard.html"), 1);
    request.device = DeviceProfile::Mobile;
    let (capture, outcome) = visit(request).await;

    assert_eq!(capture.pages.len(), 2, "{:#?}", outcome.progress);
    assert_eq!(capture.pages[1].reached_by, PageReach::LinkClick);
    let first_console = console_of(&capture, 0);
    assert!(first_console.contains(&"FIRST touchstart trusted=true"));
    assert!(first_console.contains(&"CLICK trusted=true on=next"));
    assert!(!first_console.contains(&"FIRST wheel trusted=true"));
    let done: Vec<String> = outcome
        .activities_of(ActivityKind::Action)
        .iter()
        .map(|action| action.data.as_ref().unwrap()["action"].to_string())
        .collect();
    assert!(done.contains(&"\"tap\"".to_string()), "{done:?}");
    assert!(!done.contains(&"\"click\"".to_string()), "{done:?}");
    assert!(capture.environment.user_agent_overridden && capture.environment.viewport_overridden);
}

#[tokio::test]
async fn watching_only_sends_no_input_at_all() {
    let fixtures = Fixtures::start().await;
    let mut request = support::request(&fixtures.url("browse-hard.html"), DeviceProfile::Desktop);
    request.visitor = VisitorBehavior::None;
    request.observe_seconds = 3;
    let (capture, outcome) = visit(request).await;

    assert_eq!(capture.pages.len(), 1);
    assert!(
        !capture
            .console
            .iter()
            .any(|line| line.text.starts_with("FIRST") || line.text.starts_with("CLICK")),
        "{:#?}",
        capture.console
    );
    assert_eq!(capture.max_scroll_depth, 0.0);
    assert!(outcome.activities_of(ActivityKind::Action).is_empty());
    assert!(outcome.activities_of(ActivityKind::LinkChoice).is_empty());
    assert_eq!(requests_of(&fixtures, "/browse-hard-2.html"), 0);
}

#[tokio::test]
async fn the_environment_is_reported_once_and_as_the_page_itself_reads_it() {
    let fixtures = Fixtures::start().await;
    let mut request = support::request(&fixtures.url("emulation.html"), DeviceProfile::Desktop);
    request.observe_seconds = 1;
    let (capture, outcome) = visit(request).await;

    let reports = outcome.activities_of(ActivityKind::Environment);
    assert_eq!(reports.len(), 1, "{:#?}", outcome.progress);
    assert_eq!(
        reports[0].message,
        "This visit is identifiable as automated: navigator.webdriver is true, headless"
    );
    assert_eq!(
        reports[0].data,
        Some(json!({
            "webdriver": true,
            "headless": true,
            "persistentProfile": false,
            "userAgentOverridden": false,
            "viewportOverridden": true,
        }))
    );
    // The page reads the value in its own world, the scanner in another.
    let seen_by_the_page = capture
        .console
        .iter()
        .find_map(|line| line.text.strip_prefix("DEVICE "))
        .map(|seen| serde_json::from_str::<serde_json::Value>(seen).unwrap())
        .expect("the page reports what it sees");
    assert_eq!(
        json!(capture.environment.webdriver),
        seen_by_the_page["webdriver"]
    );
    assert_eq!(json!(capture.user_agent), seen_by_the_page["userAgent"]);
    assert_eq!(
        capture.environment.request_hosts,
        Vec::<String>::new(),
        "the page asked nobody else for anything"
    );
}
