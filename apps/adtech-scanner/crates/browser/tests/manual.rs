//! Visits with a visible browser: a person browsing while the scanner only
//! records, and the scripted visitor in a window of its own.
//!
//! A window needs a display. Where there is none (Linux without `DISPLAY`),
//! each test runs itself again under `xvfb-run`; see `support::has_a_display`.
//! The tests find "their" browser as the only one this process runs, so they
//! take turns.
#![cfg(unix)]

mod support;

use std::time::{Duration, Instant};

use adscan_browser::BrowserError;
use adscan_model::{
    ActivityKind, CaptureRequest, DeviceProfile, PageReach, TrackerEventKind as Kind,
    VisitorBehavior,
};
use serde_json::json;
use support::{Fixtures, Reported};

static ONE_BROWSER_AT_A_TIME: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn manual(url: &str) -> CaptureRequest {
    let mut request = support::request(url, DeviceProfile::Desktop);
    request.visitor = VisitorBehavior::Manual;
    request.headless = false;
    request
}

fn console_has(capture: &adscan_model::Capture, start: &str) -> bool {
    capture
        .console
        .iter()
        .any(|line| line.text.starts_with(start))
}

#[tokio::test]
async fn a_manual_visit_records_where_the_person_goes_until_it_is_finished() {
    if !support::has_a_display("a_manual_visit_records_where_the_person_goes_until_it_is_finished")
    {
        return;
    }
    let _turn = ONE_BROWSER_AT_A_TIME.lock().await;
    let fixtures = Fixtures::start().await;
    let elsewhere = Fixtures::start().await;
    // The page stands in for the person: it moves on to a second page of
    // the site and from there to another site.
    let other_site = elsewhere.url("emulation.html");
    let entry = format!("{}?elsewhere={other_site}", fixtures.url("manual.html"));
    let reported = Reported::new();
    let finish = reported.hooks.finish.clone();
    let started = Instant::now();
    let running = tokio::spawn(reported.run(manual(&entry)));

    tokio::time::sleep(Duration::from_secs(7)).await;
    assert!(
        !running.is_finished(),
        "a manual visit waits to be finished"
    );
    finish.cancel();
    let outcome = running.await.unwrap();
    let capture = outcome.result.as_ref().unwrap_or_else(|error| {
        panic!("capture failed: {error}\nprogress: {:#?}", outcome.progress)
    });
    assert!(
        started.elapsed() < Duration::from_secs(20),
        "finishing is prompt: {:?}",
        started.elapsed()
    );

    // Two pages of the site, as the person reached them. The other site is
    // theirs to visit and nothing of it is recorded.
    assert_eq!(capture.pages.len(), 2, "{:#?}", outcome.progress);
    assert_eq!(capture.pages[0].reached_by, PageReach::Entry);
    assert_eq!(capture.pages[1].reached_by, PageReach::UserNavigation);
    assert!(capture.pages[1].url.contains("/manual.html?step=1"));
    assert_eq!(
        capture.pages[1].title.as_deref(),
        Some("A page a person browses")
    );
    assert_eq!(elsewhere.requested(), ["/emulation.html"]);
    assert!(
        !console_has(capture, "DEVICE "),
        "the other site's console is not recorded"
    );
    assert!(
        capture
            .scripts
            .iter()
            .all(|script| script.body.contains("A person moving on")),
        "nor are its scripts"
    );
    let left: Vec<_> = outcome
        .activities_of(ActivityKind::Blocked)
        .iter()
        .map(|activity| (activity.message.clone(), activity.data.clone()))
        .collect();
    assert_eq!(
        left,
        [(
            "Left the scanned site (127.0.0.1): not recorded".to_string(),
            Some(json!({ "reason": "off-site", "url": other_site, "host": "127.0.0.1" }))
        )]
    );

    // The scanner did nothing on the page. (The display's own pointer may
    // rest over the window; that one is the person's.)
    for input in ["wheel", "mousedown", "touchstart", "keydown", "scroll"] {
        assert!(
            !console_has(capture, &format!("INPUT {input}")),
            "{input}: {:#?}",
            capture.console
        );
    }
    assert!(outcome.activities_of(ActivityKind::Action).is_empty());
    assert!(outcome.activities_of(ActivityKind::LinkChoice).is_empty());
    assert!(!capture.consent_accepted);
    assert!(!capture.events.iter().any(|event| matches!(
        &event.kind,
        Kind::PageLifecycle { phase } if phase == "firstInteraction" || phase == "firstScroll"
    )));

    // A browser a person drives is not flagged as automated, and it is the
    // browser as it is: its own window, its own user agent.
    assert!(console_has(capture, "WEBDRIVER false ua="));
    assert_eq!(capture.environment.webdriver, Some(false));
    assert!(!capture.environment.headless);
    assert!(!capture.environment.user_agent_overridden);
    assert!(!capture.environment.viewport_overridden);
    assert!(
        !capture.user_agent.contains("Headless"),
        "{}",
        capture.user_agent
    );
    let window = format!(
        "VIEWPORT {}x{}",
        capture.viewport.width, capture.viewport.height
    );
    assert!(
        console_has(capture, &window),
        "{window}: {:#?}",
        capture.console
    );
    let environment = outcome.activities_of(ActivityKind::Environment);
    assert!(
        environment[0]
            .message
            .starts_with("This visit is not flagged as automated"),
        "{}",
        environment[0].message
    );
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn quitting_the_browser_ends_a_manual_visit_and_keeps_what_was_recorded() {
    if !support::has_a_display(
        "quitting_the_browser_ends_a_manual_visit_and_keeps_what_was_recorded",
    ) {
        return;
    }
    use support::chrome_processes::Browser;

    let _turn = ONE_BROWSER_AT_A_TIME.lock().await;
    let fixtures = Fixtures::start().await;
    let running = tokio::spawn(Reported::new().run(manual(&fixtures.url("browse-hard.html"))));
    let browser = Browser::launched_by(std::process::id()).await;
    tokio::time::sleep(Duration::from_secs(4)).await;

    // What the person's "Quit" does.
    nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(browser.group.cast_signed()),
        nix::sys::signal::Signal::SIGTERM,
    )
    .unwrap();
    let outcome = tokio::time::timeout(Duration::from_secs(15), running)
        .await
        .expect("the capture notices that the browser is gone")
        .unwrap();

    let capture = outcome.result.unwrap_or_else(|error| {
        panic!("capture failed: {error}\nprogress: {:#?}", outcome.progress)
    });
    assert_eq!(capture.pages.len(), 1);
    assert_eq!(
        capture.pages[0].title.as_deref(),
        Some("Browse: first page")
    );
    assert!(capture.pages[0].ended_at_ms > 3_000.0);
    assert!(capture.page_load_ms.is_some());
    assert!(
        capture
            .scripts
            .iter()
            .any(|script| script.url.ends_with("/browse-shared.js"))
    );
    assert!(console_has(&capture, "BINDING visible=false"));
    browser.assert_gone(Duration::from_secs(5)).await;
}

#[tokio::test]
async fn a_manual_visit_can_be_cancelled_and_then_keeps_nothing() {
    if !support::has_a_display("a_manual_visit_can_be_cancelled_and_then_keeps_nothing") {
        return;
    }
    let _turn = ONE_BROWSER_AT_A_TIME.lock().await;
    let fixtures = Fixtures::start().await;
    let reported = Reported::new();
    let cancel = reported.hooks.cancel.clone();
    let running = tokio::spawn(reported.run(manual(&fixtures.url("browse-hard.html"))));

    tokio::time::sleep(Duration::from_secs(4)).await;
    let cancelled_at = Instant::now();
    cancel.cancel();
    let outcome = running.await.unwrap();
    assert!(
        matches!(outcome.result, Err(BrowserError::Cancelled)),
        "{:?}",
        outcome.result.err()
    );
    assert!(cancelled_at.elapsed() < Duration::from_secs(3));
}

#[tokio::test]
async fn a_visible_browser_browsed_by_the_script_keeps_its_own_window_and_says_what_it_is() {
    if !support::has_a_display(
        "a_visible_browser_browsed_by_the_script_keeps_its_own_window_and_says_what_it_is",
    ) {
        return;
    }
    let _turn = ONE_BROWSER_AT_A_TIME.lock().await;
    let fixtures = Fixtures::start().await;
    let mut request = support::request(&fixtures.url("manual.html"), DeviceProfile::Desktop);
    request.visitor = VisitorBehavior::Browse;
    request.headless = false;
    request.observe_seconds = 4;
    let outcome = support::run(request).await;
    let capture = outcome.result.as_ref().unwrap_or_else(|error| {
        panic!("capture failed: {error}\nprogress: {:#?}", outcome.progress)
    });

    assert!(
        console_has(capture, "WEBDRIVER true ua="),
        "the scanner is the visitor, and the browser says so: {:#?}",
        capture.console
    );
    assert_eq!(capture.environment.webdriver, Some(true));
    assert!(!capture.environment.headless && !capture.environment.viewport_overridden);
    assert!(!capture.user_agent.contains("Headless"));
    let window = format!(
        "VIEWPORT {}x{}",
        capture.viewport.width, capture.viewport.height
    );
    assert!(
        console_has(capture, &window),
        "{window}: {:#?}",
        capture.console
    );
    assert!(
        console_has(capture, "INPUT wheel") || console_has(capture, "INPUT pointermove"),
        "the scripted visitor acts in a visible browser too"
    );
    let environment = outcome.activities_of(ActivityKind::Environment);
    assert_eq!(
        environment[0].message,
        "This visit is identifiable as automated: navigator.webdriver is true"
    );
}
