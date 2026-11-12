//! The browser profile that is kept between scans: what it remembers, who may
//! use it, and when it can be deleted.

mod support;

use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use adscan_browser::{BrowserError, clear_scan_profile, scan_profile_info};
use adscan_model::{Capture, CaptureRequest, DeviceProfile};
use support::{Fixtures, Reported};

fn on_kept_profile(url: &str, device: DeviceProfile, root: &Path) -> (CaptureRequest, Reported) {
    let mut request = support::request(url, device);
    request.persistent_profile = true;
    request.observe_seconds = 1;
    let mut reported = Reported::new();
    reported.hooks.scan_profile_dir = Some(root.to_path_buf());
    (request, reported)
}

async fn scan_on_kept_profile(url: &str, device: DeviceProfile, root: &Path) -> Capture {
    let (request, reported) = on_kept_profile(url, device, root);
    let outcome = reported.run(request).await;
    outcome.result.unwrap_or_else(|error| {
        panic!("capture failed: {error}\nprogress: {:#?}", outcome.progress)
    })
}

/// What the page found in its cookie jar when it loaded.
fn cookies_found(capture: &Capture) -> &str {
    capture
        .console
        .iter()
        .find_map(|line| line.text.strip_prefix("COOKIE before="))
        .expect("the page reports its cookies")
}

#[tokio::test]
async fn the_kept_profile_remembers_a_visitor_and_a_throwaway_one_does_not() {
    let fixtures = Fixtures::start().await;
    let root = tempfile::tempdir().unwrap();
    let root = root.path().join("scan-profile");
    let page = fixtures.url("cookie.html");
    assert!(!scan_profile_info(&root).exists);

    let first = scan_on_kept_profile(&page, DeviceProfile::Desktop, &root).await;
    assert_eq!(cookies_found(&first), "", "a new profile knows nobody");
    assert!(first.environment.persistent_profile);

    let second = scan_on_kept_profile(&page, DeviceProfile::Desktop, &root).await;
    assert_eq!(cookies_found(&second), "returning=yes");

    // A phone is another visitor, and so is a scan that keeps nothing.
    let phone = scan_on_kept_profile(&page, DeviceProfile::Mobile, &root).await;
    assert_eq!(cookies_found(&phone), "");
    let mut throwaway = support::request(&page, DeviceProfile::Desktop);
    throwaway.observe_seconds = 1;
    let throwaway = support::capture_ok(throwaway).await;
    assert_eq!(cookies_found(&throwaway), "");
    assert!(!throwaway.environment.persistent_profile);

    assert!(root.join("desktop/Default").is_dir() && root.join("mobile/Default").is_dir());
    let info = scan_profile_info(&root);
    assert!(info.exists && !info.in_use);
    assert!(info.size_bytes > 10_000.0, "{}", info.size_bytes);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
        * 1_000.0;
    let last_used = info.last_used_at.expect("it was just used");
    assert!(
        (now - 60_000.0..=now).contains(&last_used),
        "{last_used} vs {now}"
    );

    clear_scan_profile(&root).unwrap();
    let cleared = scan_profile_info(&root);
    assert!(!cleared.exists && cleared.size_bytes == 0.0 && cleared.last_used_at.is_none());
    let afresh = scan_on_kept_profile(&page, DeviceProfile::Desktop, &root).await;
    assert_eq!(cookies_found(&afresh), "", "cleared means forgotten");
}

#[tokio::test]
async fn one_scan_at_a_time_uses_the_kept_profile_and_it_is_not_cleared_under_a_scan() {
    let fixtures = Fixtures::start().await;
    let root = tempfile::tempdir().unwrap();
    let root = root.path().join("scan-profile");
    let page = fixtures.url("cookie.html");

    let (mut long_scan, reported) = on_kept_profile(&page, DeviceProfile::Desktop, &root);
    long_scan.observe_seconds = 8;
    let running = tokio::spawn(reported.run(long_scan));
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    while !scan_profile_info(&root).in_use {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the scan never started"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    let (second_scan, reported) = on_kept_profile(&page, DeviceProfile::Desktop, &root);
    let refused = reported.run(second_scan).await;
    assert!(
        matches!(refused.result, Err(BrowserError::ProfileBusy)),
        "{:?}",
        refused.result.err()
    );
    assert_eq!(
        BrowserError::ProfileBusy.to_string(),
        "The browser profile is in use by another scan"
    );
    assert!(matches!(
        clear_scan_profile(&root),
        Err(BrowserError::ProfileBusy)
    ));
    // The other device's profile is free, and still not cleared on its own.
    assert!(root.join("desktop").is_dir());

    let first = running.await.unwrap();
    assert!(first.result.is_ok(), "{:?}", first.result.err());
    assert!(!scan_profile_info(&root).in_use);
    clear_scan_profile(&root).unwrap();
    assert!(!root.join("desktop").exists());
    clear_scan_profile(&root).unwrap();
}

#[tokio::test]
async fn the_kept_profile_cannot_be_asked_for_without_a_place_to_keep_it() {
    let fixtures = Fixtures::start().await;
    let mut request = support::request(&fixtures.url("cookie.html"), DeviceProfile::Desktop);
    request.persistent_profile = true;
    let outcome = support::run(request).await;
    assert!(
        matches!(&outcome.result, Err(BrowserError::Other(message)) if message.contains("browser profile")),
        "{:?}",
        outcome.result.err()
    );
    assert!(fixtures.requested().is_empty(), "no browser was started");
}
