//! No capture may leave a Chrome process or a profile directory behind:
//! not when cancelled, not when its future is dropped, not when it fails.
//!
//! These tests find "their" browser as the only one this test binary runs, so
//! they live in their own binary and take turns.
#![cfg(target_os = "linux")]

mod support;

use std::time::{Duration, Instant};

use adscan_browser::{BrowserError, CaptureHooks, capture};
use adscan_model::DeviceProfile;
use tokio_util::sync::CancellationToken;

/// Finds the browser a capture in this test process started, and everything
/// that browser spawned, by what they leave in `/proc`.
mod chrome_processes {
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    const PROFILE_FLAG: &str = "--user-data-dir=";

    struct Process {
        pid: u32,
        parent: u32,
        group: u32,
        is_zombie: bool,
    }

    fn processes() -> Vec<Process> {
        let Ok(entries) = std::fs::read_dir("/proc") else {
            return Vec::new();
        };
        entries
            .filter_map(|entry| {
                let pid: u32 = entry.ok()?.file_name().to_str()?.parse().ok()?;
                let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
                // "pid (comm) state ppid pgrp ..."; comm may contain spaces.
                let mut fields = stat.rsplit_once(')')?.1.split_whitespace();
                let is_zombie = fields.next()? == "Z";
                Some(Process {
                    pid,
                    parent: fields.next()?.parse().ok()?,
                    group: fields.next()?.parse().ok()?,
                    is_zombie,
                })
            })
            .collect()
    }

    fn profile_of(pid: u32) -> Option<PathBuf> {
        let command_line = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
        command_line.split(|byte| *byte == 0).find_map(|argument| {
            String::from_utf8_lossy(argument)
                .strip_prefix(PROFILE_FLAG)
                .map(PathBuf::from)
        })
    }

    fn mentions(pid: u32, text: &str) -> bool {
        std::fs::read(format!("/proc/{pid}/cmdline"))
            .is_ok_and(|command_line| String::from_utf8_lossy(&command_line).contains(text))
    }

    /// A browser launched by this test process: its process group and profile.
    pub struct Browser {
        group: u32,
        pub profile: PathBuf,
    }

    impl Browser {
        /// Waits for a capture running in this process to launch its browser.
        pub async fn launched() -> Self {
            for _ in 0..500 {
                let own = processes()
                    .into_iter()
                    .filter(|process| process.parent == std::process::id());
                if let Some(browser) = own
                    .filter_map(|process| Some((process.pid, profile_of(process.pid)?)))
                    .next()
                {
                    assert!(
                        browser.1.to_string_lossy().contains("adscan-profile-"),
                        "{:?}",
                        browser.1
                    );
                    return Self {
                        group: browser.0,
                        profile: browser.1,
                    };
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            panic!("no browser was launched");
        }

        /// Live processes of the browser: its process group (browser, zygotes,
        /// GPU, renderers) plus anything else started with its profile path.
        pub fn live_processes(&self) -> usize {
            let profile = self.profile.to_string_lossy();
            processes()
                .iter()
                .filter(|process| !process.is_zombie)
                .filter(|process| process.group == self.group || mentions(process.pid, &profile))
                .count()
        }

        pub async fn assert_gone(&self, within: Duration) {
            let deadline = Instant::now() + within;
            loop {
                let live = self.live_processes();
                if live == 0 && !self.profile.exists() {
                    return;
                }
                assert!(
                    Instant::now() < deadline,
                    "{live} Chrome processes are still alive, profile {} exists: {}",
                    self.profile.display(),
                    self.profile.exists()
                );
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }
    }
}

use chrome_processes::Browser;

static ONE_BROWSER_AT_A_TIME: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[tokio::test]
async fn cancelling_returns_promptly_and_leaves_no_chrome_behind() {
    let _turn = ONE_BROWSER_AT_A_TIME.lock().await;
    let site = adscan_browser::start_demo_site().await.unwrap();
    let mut request = support::request(&site.url, DeviceProfile::Desktop);
    request.observe_seconds = 60;
    request.auto_scroll = true;
    let cancel = CancellationToken::new();
    let running = tokio::spawn(support::run_cancellable(request, cancel.clone()));

    let browser = Browser::launched().await;
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

    let browser = Browser::launched().await;
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
    let hooks = CaptureHooks {
        cancel: CancellationToken::new(),
        on_progress: std::sync::Arc::new(|_| {}),
    };
    let abandoned = tokio::spawn(capture(request, hooks));

    let browser = Browser::launched().await;
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
        let browser = Browser::launched().await;
        let outcome = running.await.unwrap();
        assert_eq!(outcome.result.is_ok(), path == "emulation.html", "{path}");
        browser.assert_gone(Duration::ZERO).await;
    }
}
