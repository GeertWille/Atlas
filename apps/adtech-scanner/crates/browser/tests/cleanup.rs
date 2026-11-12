//! No capture may leave a Chrome process or a profile directory behind:
//! not when cancelled, not when its future is dropped, not when it fails.
//!
//! These tests find "their" browser as the only one this test binary runs, so
//! they live in their own binary and take turns. What happens when the process
//! running the capture is killed is in `abrupt_exit.rs`.
#![cfg(target_os = "linux")]

mod support;

use std::time::{Duration, Instant};

use adscan_browser::{BrowserError, capture};
use adscan_model::{DeviceProfile, VisitorBehavior};
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
