//! No capture may leave a Chrome process or a throwaway profile directory
//! behind: not when cancelled, in whatever state the visit is, not when its
//! future is dropped, not when it fails. The scan profile the user keeps is
//! the one thing that stays, on all of those paths.
//!
//! These tests find "their" browser as the only one this test binary runs, so
//! they live in their own binary and take turns. What happens when the process
//! running the capture is killed is in `abrupt_exit.rs`.
#![cfg(target_os = "linux")]

mod support;

use std::time::{Duration, Instant};

use adscan_browser::{BrowserError, capture, scan_profile_info};
use adscan_model::{ActivityKind, CaptureActivity, DeviceProfile, VisitorBehavior};
use tokio_util::sync::CancellationToken;

use support::chrome_processes::Browser;

static ONE_BROWSER_AT_A_TIME: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[tokio::test]
async fn cancelling_returns_promptly_and_leaves_no_chrome_behind() {
    let _turn = ONE_BROWSER_AT_A_TIME.lock().await;
    let site = adscan_browser::start_demo_site().await.unwrap();
    let mut request = support::request(&site.url, DeviceProfile::Desktop);
    request.observe_seconds = 60;
    request.visitor = VisitorBehavior::Scroll;
    let cancel = CancellationToken::new();
    let running = tokio::spawn(support::run_cancellable(request, cancel.clone()));

    let browser = Browser::launched_by(std::process::id()).await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(
        browser.live_processes() >= 4,
        "browser, zygote, GPU and renderer processes are running"
    );

    let cancelled_at = Instant::now();
    cancel.cancel();
    let outcome = running.await.unwrap();
    assert!(
        matches!(outcome.result, Err(BrowserError::Cancelled)),
        "{:?}",
        outcome.result.err()
    );
    assert!(
        cancelled_at.elapsed() < Duration::from_secs(3),
        "took {:?}",
        cancelled_at.elapsed()
    );
    assert!(
        outcome
            .progress
            .iter()
            .any(|line| line.starts_with("Observing")),
        "{:#?}",
        outcome.progress
    );
    // Everything is cleaned up by the time the call returns.
    browser.assert_gone(Duration::ZERO).await;
}

#[tokio::test]
async fn cancelling_during_startup_leaves_no_chrome_behind() {
    let _turn = ONE_BROWSER_AT_A_TIME.lock().await;
    let site = adscan_browser::start_demo_site().await.unwrap();
    let cancel = CancellationToken::new();
    let running = tokio::spawn(support::run_cancellable(
        support::request(&site.url, DeviceProfile::Mobile),
        cancel.clone(),
    ));

    let browser = Browser::launched_by(std::process::id()).await;
    cancel.cancel();
    let outcome = running.await.unwrap();
    assert!(
        matches!(outcome.result, Err(BrowserError::Cancelled)),
        "{:?}",
        outcome.result.err()
    );
    browser.assert_gone(Duration::ZERO).await;
}

#[tokio::test]
async fn dropping_the_capture_future_still_cleans_up() {
    let _turn = ONE_BROWSER_AT_A_TIME.lock().await;
    let site = adscan_browser::start_demo_site().await.unwrap();
    let mut request = support::request(&site.url, DeviceProfile::Desktop);
    request.observe_seconds = 60;
    let abandoned = tokio::spawn(capture(request, support::Reported::new().hooks));

    let browser = Browser::launched_by(std::process::id()).await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    abandoned.abort();
    assert!(abandoned.await.unwrap_err().is_cancelled());
    browser.assert_gone(Duration::from_secs(3)).await;
}

#[tokio::test]
async fn failed_and_finished_captures_leave_no_chrome_behind() {
    let _turn = ONE_BROWSER_AT_A_TIME.lock().await;
    let fixtures = support::Fixtures::start().await;
    for path in ["status/500", "emulation.html"] {
        let mut request = support::request(&fixtures.url(path), DeviceProfile::Desktop);
        request.observe_seconds = 1;
        let running = tokio::spawn(support::run(request));
        let browser = Browser::launched_by(std::process::id()).await;
        let outcome = running.await.unwrap();
        assert_eq!(outcome.result.is_ok(), path == "emulation.html", "{path}");
        browser.assert_gone(Duration::ZERO).await;
    }
}

/// The moments of a browsing visit at which it is cancelled below, each
/// recognised by what the visit has reported by then.
const STATES: [(&str, fn(&CaptureActivity) -> bool); 5] = [
    ("settling", |activity| {
        activity.message.starts_with("Browsing like a visitor")
    }),
    ("engaging", |activity| activity.kind == ActivityKind::Action),
    ("choosing a link", |activity| {
        activity.kind == ActivityKind::LinkChoice
    }),
    ("awaiting the navigation", |activity| {
        activity.message.starts_with("Clicked the link")
    }),
    ("on the second page", |activity| {
        activity.kind == ActivityKind::PageView && activity.page == Some(1)
    }),
];

#[tokio::test]
async fn cancelling_a_browsing_visit_is_prompt_in_every_state_and_leaves_nothing() {
    let _turn = ONE_BROWSER_AT_A_TIME.lock().await;
    let site = adscan_browser::start_demo_site().await.unwrap();
    for (state, reached) in STATES {
        let mut request = support::request(&site.url, DeviceProfile::Desktop);
        request.visitor = VisitorBehavior::Browse;
        request.extra_pages = 2;
        request.observe_seconds = 6;
        request.seed = 11;
        let reported = support::Reported::new();
        let cancel = reported.hooks.cancel.clone();
        let activities = reported.activities();
        let running = tokio::spawn(reported.run(request));
        let browser = Browser::launched_by(std::process::id()).await;

        let deadline = Instant::now() + Duration::from_secs(60);
        while !activities.lock().unwrap().iter().any(reached) {
            assert!(Instant::now() < deadline, "the visit never got to: {state}");
            assert!(!running.is_finished(), "the visit ended before: {state}");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let cancelled_at = Instant::now();
        cancel.cancel();
        let outcome = running.await.unwrap();
        assert!(
            matches!(outcome.result, Err(BrowserError::Cancelled)),
            "{state}: {:?}",
            outcome.result.err()
        );
        assert!(
            cancelled_at.elapsed() < Duration::from_secs(3),
            "{state}: took {:?}",
            cancelled_at.elapsed()
        );
        browser.assert_gone(Duration::ZERO).await;
    }
}

#[tokio::test]
async fn the_kept_scan_profile_stays_when_a_scan_is_cancelled_dropped_or_fails() {
    let _turn = ONE_BROWSER_AT_A_TIME.lock().await;
    let site = adscan_browser::start_demo_site().await.unwrap();
    let fixtures = support::Fixtures::start().await;
    let root = tempfile::tempdir().unwrap();
    let profile = root.path().join("desktop");
    let on_kept_profile = |url: &str| {
        let mut request = support::request(url, DeviceProfile::Desktop);
        request.persistent_profile = true;
        request.observe_seconds = 60;
        let mut reported = support::Reported::new();
        reported.hooks.scan_profile_dir = Some(root.path().to_path_buf());
        (request, reported)
    };
    let assert_kept_and_free = |how: &str| {
        assert!(
            profile.join("Default").is_dir(),
            "{how}: the profile is gone"
        );
        let info = scan_profile_info(root.path());
        assert!(info.exists && !info.in_use, "{how}: {info:?}");
    };

    let (request, reported) = on_kept_profile(&site.url);
    let cancel = reported.hooks.cancel.clone();
    let running = tokio::spawn(reported.run(request));
    let browser = Browser::launched_on_kept_profile(std::process::id(), &profile).await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    let cancelled_at = Instant::now();
    cancel.cancel();
    let outcome = running.await.unwrap();
    assert!(matches!(outcome.result, Err(BrowserError::Cancelled)));
    assert!(
        cancelled_at.elapsed() < Duration::from_secs(6),
        "took {:?}",
        cancelled_at.elapsed()
    );
    browser.assert_exited(Duration::ZERO).await;
    assert_kept_and_free("cancelled");

    let (request, reported) = on_kept_profile(&site.url);
    let abandoned = tokio::spawn(capture(request, reported.hooks));
    let browser = Browser::launched_on_kept_profile(std::process::id(), &profile).await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    abandoned.abort();
    assert!(abandoned.await.unwrap_err().is_cancelled());
    browser.assert_exited(Duration::from_secs(8)).await;
    // The lock is let go once the abandoned capture has shut its browser down.
    let deadline = Instant::now() + Duration::from_secs(3);
    while scan_profile_info(root.path()).in_use && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_kept_and_free("dropped");

    let (request, reported) = on_kept_profile(&fixtures.url("status/500"));
    let running = tokio::spawn(reported.run(request));
    let browser = Browser::launched_on_kept_profile(std::process::id(), &profile).await;
    let outcome = running.await.unwrap();
    assert!(matches!(outcome.result, Err(BrowserError::Navigation(_))));
    browser.assert_exited(Duration::ZERO).await;
    assert_kept_and_free("failed");
}
