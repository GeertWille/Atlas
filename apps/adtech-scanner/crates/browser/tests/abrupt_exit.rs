//! A browser must not outlive the process that launched it, however that
//! process ends, and what a killed process leaves on disk must not stay.
//!
//! The one thing a killed process may leave is the scan profile the user
//! keeps, which nothing but the user removes.
//!
//! The process that gets killed is this test binary, started a second time to
//! run only [`capture_in_a_process_of_its_own`]. Every test gives its captures
//! a temporary directory of their own, so that the profiles it counts are its.
#![cfg(target_os = "linux")]

mod support;

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use adscan_browser::DemoSite;
use adscan_model::{DeviceProfile, VisitorBehavior};
use nix::sys::signal::{Signal, kill};
use nix::unistd::Pid;
use support::chrome_processes::{Browser, children_named, command_line};
use tokio::process::{Child, Command};

const URL_ENV: &str = "ADSCAN_TEST_CAPTURE_URL";
const OBSERVE_SECONDS_ENV: &str = "ADSCAN_TEST_CAPTURE_OBSERVE_SECONDS";
/// Root of the kept scan profile to scan on; a throwaway profile when unset.
const SCAN_PROFILE_ENV: &str = "ADSCAN_TEST_CAPTURE_SCAN_PROFILE";

/// Not a test of its own: the body of the process the tests below start, and
/// kill. It does nothing when run with the other tests.
#[tokio::test]
#[ignore = "runs in the process started by the tests of this file"]
async fn capture_in_a_process_of_its_own() {
    let Ok(url) = std::env::var(URL_ENV) else {
        return;
    };
    let mut request = support::request(&url, DeviceProfile::Desktop);
    request.observe_seconds = std::env::var(OBSERVE_SECONDS_ENV).unwrap().parse().unwrap();
    request.visitor = VisitorBehavior::Scroll;
    let mut reported = support::Reported::new();
    if let Some(scan_profile) = std::env::var_os(SCAN_PROFILE_ENV) {
        request.persistent_profile = true;
        reported.hooks.scan_profile_dir = Some(scan_profile.into());
    }
    let outcome = reported.run(request).await;
    outcome.result.unwrap_or_else(|error| {
        panic!("capture failed: {error}\nprogress: {:#?}", outcome.progress)
    });
}

/// A stand-in for the app: a process that scans the demo site.
struct App {
    process: Child,
    id: u32,
}

impl App {
    fn start(site: &DemoSite, temporary_files: &Path, observe_seconds: u32) -> Self {
        Self::start_on(site, temporary_files, observe_seconds, None)
    }

    fn start_on(
        site: &DemoSite,
        temporary_files: &Path,
        observe_seconds: u32,
        scan_profile: Option<&Path>,
    ) -> Self {
        let mut command = Command::new(std::env::current_exe().unwrap());
        if let Some(scan_profile) = scan_profile {
            command.env(SCAN_PROFILE_ENV, scan_profile);
        }
        let process = command
            .args(["capture_in_a_process_of_its_own", "--exact", "--ignored"])
            .env(URL_ENV, &site.url)
            .env(OBSERVE_SECONDS_ENV, observe_seconds.to_string())
            .env("TMPDIR", temporary_files)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let id = process.id().unwrap();
        Self { process, id }
    }

    /// Starts a scan that lasts far longer than the test and waits until it
    /// is well under way.
    async fn scanning(site: &DemoSite, temporary_files: &Path) -> (Self, Browser) {
        let app = Self::start(site, temporary_files, 120);
        let browser = Browser::launched_by(app.id).await;
        let deadline = Instant::now() + Duration::from_secs(20);
        while browser.live_processes() < 4 {
            assert!(Instant::now() < deadline, "the browser did not get going");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
        (app, browser)
    }

    /// What a crash, a force quit or `kill -9` does: the process is gone
    /// without having run a single line of its own.
    async fn kill(mut self) {
        self.process.start_kill().unwrap();
        self.process.wait().await.unwrap();
    }

    async fn finish(mut self) {
        let status = tokio::time::timeout(Duration::from_secs(60), self.process.wait())
            .await
            .expect("the scan finished")
            .unwrap();
        assert!(status.success(), "{status}");
    }
}

fn profiles_in(temporary_files: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(temporary_files)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.to_string_lossy().contains("adscan-"))
        .collect()
}

async fn assert_no_processes(browser: &Browser, within: Duration) {
    let deadline = Instant::now() + within;
    while browser.live_processes() > 0 {
        assert!(
            Instant::now() < deadline,
            "{} processes of the browser outlived the app",
            browser.live_processes()
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn signal(process: u32, signal: Signal) {
    kill(Pid::from_raw(process.cast_signed()), signal).unwrap();
}

/// The watchdog of the app's browser, taken out of the picture.
async fn kill_watchdog(app: &App) {
    let watchdogs = children_named(app.id, "sh");
    assert_eq!(watchdogs.len(), 1, "{watchdogs:?}");
    signal(watchdogs[0], Signal::SIGKILL);
    let deadline = Instant::now() + Duration::from_secs(5);
    while !children_named(app.id, "sh").is_empty() {
        assert!(Instant::now() < deadline, "the watchdog is still there");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn killing_the_app_mid_scan_leaves_no_browser_and_no_profile() {
    let site = adscan_browser::start_demo_site().await.unwrap();
    let temporary_files = tempfile::tempdir().unwrap();
    let (app, browser) = App::scanning(&site, temporary_files.path()).await;
    assert!(browser.profile.starts_with(temporary_files.path()));

    app.kill().await;

    browser.assert_gone(Duration::from_secs(5)).await;
    assert_eq!(profiles_in(temporary_files.path()), Vec::<PathBuf>::new());
}

/// Without the watchdog, the closing DevTools pipe alone makes the browser
/// exit, and the profile it leaves is removed when the app next scans.
#[tokio::test]
async fn the_browser_exits_by_itself_and_the_next_start_removes_its_profile() {
    let site = adscan_browser::start_demo_site().await.unwrap();
    let temporary_files = tempfile::tempdir().unwrap();
    let (app, browser) = App::scanning(&site, temporary_files.path()).await;
    kill_watchdog(&app).await;

    app.kill().await;

    assert_no_processes(&browser, Duration::from_secs(5)).await;
    assert!(browser.profile.join("Default").exists());

    App::start(&site, temporary_files.path(), 1).finish().await;

    assert!(!browser.profile.exists());
    assert_eq!(profiles_in(temporary_files.path()), Vec::<PathBuf>::new());
}

/// A browser that hangs does not notice its DevTools pipe closing.
#[tokio::test]
async fn a_browser_that_hangs_is_killed_when_the_app_dies() {
    let site = adscan_browser::start_demo_site().await.unwrap();
    let temporary_files = tempfile::tempdir().unwrap();
    let (app, browser) = App::scanning(&site, temporary_files.path()).await;
    signal(browser.group, Signal::SIGSTOP);

    app.kill().await;

    browser.assert_gone(Duration::from_secs(5)).await;
}

#[tokio::test]
async fn a_second_instance_leaves_the_profile_of_a_running_scan_alone() {
    let site = adscan_browser::start_demo_site().await.unwrap();
    let temporary_files = tempfile::tempdir().unwrap();
    let (mut app, browser) = App::scanning(&site, temporary_files.path()).await;

    App::start(&site, temporary_files.path(), 1).finish().await;

    assert_eq!(
        profiles_in(temporary_files.path()),
        std::slice::from_ref(&browser.profile)
    );
    assert!(browser.profile.join("Default").exists());
    assert!(browser.live_processes() >= 4);
    assert!(
        app.process.try_wait().unwrap().is_none(),
        "the first scan is still running"
    );

    app.kill().await;
    browser.assert_gone(Duration::from_secs(5)).await;
}

/// Any local process can drive a browser that listens on a debugging port.
#[tokio::test]
async fn the_browser_is_driven_over_a_private_pipe_not_a_network_port() {
    let site = adscan_browser::start_demo_site().await.unwrap();
    let temporary_files = tempfile::tempdir().unwrap();
    let (app, browser) = App::scanning(&site, temporary_files.path()).await;

    let arguments = command_line(browser.group);
    assert!(arguments.contains(&"--remote-debugging-pipe".to_string()));
    assert!(
        !arguments
            .iter()
            .any(|argument| argument.contains("debugging-port")),
        "{arguments:?}"
    );
    assert!(!browser.profile.join("DevToolsActivePort").exists());

    app.kill().await;
    browser.assert_gone(Duration::from_secs(5)).await;
}

/// The profile the user keeps is not the app's to delete, however the app
/// ends: the browser goes, the profile stays and can be used again.
#[tokio::test]
async fn killing_the_app_leaves_no_browser_and_keeps_the_scan_profile() {
    let site = adscan_browser::start_demo_site().await.unwrap();
    let temporary_files = tempfile::tempdir().unwrap();
    let scan_profile = tempfile::tempdir().unwrap();
    let kept = scan_profile.path().join("desktop");
    let app = App::start_on(
        &site,
        temporary_files.path(),
        120,
        Some(scan_profile.path()),
    );
    let browser = Browser::launched_on_kept_profile(app.id, &kept).await;
    let deadline = Instant::now() + Duration::from_secs(20);
    while browser.live_processes() < 4 {
        assert!(Instant::now() < deadline, "the browser did not get going");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(adscan_browser::scan_profile_info(scan_profile.path()).in_use);

    app.kill().await;

    browser.assert_exited(Duration::from_secs(5)).await;
    // Longer than the watchdog takes to remove a throwaway profile.
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(
        kept.join("Default").is_dir(),
        "the kept profile was removed"
    );
    assert_eq!(profiles_in(temporary_files.path()), Vec::<PathBuf>::new());
    let left = adscan_browser::scan_profile_info(scan_profile.path());
    assert!(left.exists && !left.in_use, "{left:?}");

    // What the killed browser left in the profile does not stop the next scan.
    App::start_on(&site, temporary_files.path(), 1, Some(scan_profile.path()))
        .finish()
        .await;
    assert!(kept.join("Default").is_dir());
}

/// A hung browser on the kept profile is killed like any other.
#[tokio::test]
async fn a_hung_browser_on_the_scan_profile_is_killed_and_the_profile_kept() {
    let site = adscan_browser::start_demo_site().await.unwrap();
    let temporary_files = tempfile::tempdir().unwrap();
    let scan_profile = tempfile::tempdir().unwrap();
    let kept = scan_profile.path().join("desktop");
    let app = App::start_on(
        &site,
        temporary_files.path(),
        120,
        Some(scan_profile.path()),
    );
    let browser = Browser::launched_on_kept_profile(app.id, &kept).await;
    let deadline = Instant::now() + Duration::from_secs(20);
    while browser.live_processes() < 4 || !kept.join("Default").is_dir() {
        assert!(Instant::now() < deadline, "the browser did not get going");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    signal(browser.group, Signal::SIGSTOP);

    app.kill().await;

    browser.assert_exited(Duration::from_secs(5)).await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(
        kept.join("Default").is_dir(),
        "the kept profile was removed"
    );
}
