//! Pages that cannot be opened and pages that misbehave.

mod support;

use std::time::{Duration, Instant};

use adscan_browser::BrowserError;
use adscan_model::DeviceProfile;

async fn navigation_error(url: &str) -> String {
    let outcome = support::run(support::request(url, DeviceProfile::Desktop)).await;
    match outcome.result {
        Err(BrowserError::Navigation(message)) => message,
        other => panic!(
            "expected a navigation error for {url}, got {:?}",
            other.map(|capture| capture.final_url)
        ),
    }
}

#[tokio::test]
async fn unreachable_address_is_a_navigation_error_in_plain_language() {
    let closed_port = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().port()
    };
    let message = navigation_error(&format!("http://127.0.0.1:{closed_port}/")).await;
    assert_eq!(
        message,
        "127.0.0.1 refused the connection. Check that the site is running and the port is correct"
    );
}

#[tokio::test]
async fn error_pages_are_navigation_errors_that_name_the_status() {
    let fixtures = support::Fixtures::start().await;
    let not_found = navigation_error(&fixtures.url("status/404")).await;
    assert!(
        not_found.contains("HTTP 404 Not Found") && not_found.contains("127.0.0.1"),
        "{not_found}"
    );
    let unavailable = navigation_error(&fixtures.url("status/503")).await;
    assert!(
        unavailable.contains("HTTP 503") && unavailable.contains("try again later"),
        "{unavailable}"
    );
}

#[tokio::test]
async fn addresses_that_are_not_web_pages_are_rejected_before_launching() {
    let started = Instant::now();
    let message = navigation_error("meridianpost.example/article").await;
    assert!(
        message.contains("not a web address") && message.contains("https://"),
        "{message}"
    );
    assert!(
        started.elapsed() < Duration::from_millis(500),
        "no browser should start"
    );
}

#[tokio::test]
async fn a_page_that_never_finishes_loading_is_still_captured() {
    let fixtures = support::Fixtures::start().await;
    let mut request = support::request(&fixtures.url("never-loads.html"), DeviceProfile::Desktop);
    request.observe_seconds = 1;
    let outcome = support::run(request).await;
    let capture = outcome.result.expect("a stuck page is scanned, not failed");

    assert!(
        outcome
            .progress
            .iter()
            .any(|line| line.contains("still loading")),
        "{:#?}",
        outcome.progress
    );
    assert_eq!(capture.page_load_ms, None);
    assert_eq!(
        capture.page_title.as_deref(),
        Some("Never finishes loading")
    );
    assert!(capture.console.iter().any(|line| line.text == "INLINE ran"));
    assert!(
        capture
            .scripts
            .iter()
            .any(|script| script.inline && script.body.contains("INLINE ran"))
    );
}

#[tokio::test]
async fn dialogs_and_debugger_statements_do_not_stall_the_scan() {
    let fixtures = support::Fixtures::start().await;
    let mut request = support::request(&fixtures.url("hostile.html"), DeviceProfile::Desktop);
    request.observe_seconds = 1;
    let started = Instant::now();
    let capture = support::capture_ok(request).await;
    assert!(
        capture
            .console
            .iter()
            .any(|line| line.text == "SURVIVED dialog and debugger")
    );
    // An alert is closed; a question is never answered with "yes".
    let said = |text: &str| capture.console.iter().any(|line| line.text == text);
    assert!(said("CONFIRMED false"), "{:#?}", capture.console);
    assert!(said("PROMPTED null"), "{:#?}", capture.console);
    assert!(capture.page_load_ms.is_some());
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "took {:?}",
        started.elapsed()
    );
}

#[tokio::test]
async fn scripts_from_data_and_blob_urls_are_captured_without_payload_urls() {
    let fixtures = support::Fixtures::start().await;
    let mut request = support::request(&fixtures.url("odd-scripts.html"), DeviceProfile::Desktop);
    request.observe_seconds = 1;
    let capture = support::capture_ok(request).await;

    let script_with = |marker: &str| {
        capture
            .scripts
            .iter()
            .find(|script| !script.inline && script.body.contains(marker))
            .unwrap_or_else(|| panic!("no script contains {marker}"))
    };
    let from_data = script_with("DATA_URL ran");
    assert!(from_data.body.len() > 8_000);
    assert!(
        from_data.url.starts_with("data:text/javascript,[") && from_data.url.len() < 80,
        "{}",
        from_data.url
    );
    assert!(script_with("BLOB_URL ran").url.starts_with("blob:http://"));
}

#[tokio::test]
async fn a_page_cannot_save_files_to_the_users_disk() {
    fn saved_files() -> Vec<std::path::PathBuf> {
        let downloads = std::env::var_os("HOME")
            .map(|home| std::path::PathBuf::from(home).join("Downloads"))
            .and_then(|directory| std::fs::read_dir(directory).ok());
        downloads
            .into_iter()
            .flatten()
            .filter_map(|entry| Some(entry.ok()?.path()))
            .collect()
    }

    let before = saved_files();
    let fixtures = support::Fixtures::start().await;
    let mut request = support::request(&fixtures.url("downloads.html"), DeviceProfile::Desktop);
    request.observe_seconds = 2;
    let capture = support::capture_ok(request).await;

    assert_eq!(
        capture.page_title.as_deref(),
        Some("A page that pushes a file at the visitor")
    );
    assert_eq!(saved_files(), before, "the download must be refused");
}
