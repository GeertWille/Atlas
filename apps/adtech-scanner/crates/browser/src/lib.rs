//! Browser capture: drives a locally installed Chrome/Chromium, or a browser
//! at Browserbase, over the DevTools protocol, injects the tracker script, and
//! returns an in-memory [`Capture`].
//!
//! The public API below is a fixed contract consumed by `src-tauri`.

mod browserbase;
mod capture;
mod cdp;
mod demo;
mod detect;
mod device;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::UNIX_EPOCH;

use adscan_model::{
    BrowserInfo, Capture, CaptureActivity, CaptureRequest, NetworkSink, ScanProfileInfo,
};
use tokio_util::sync::CancellationToken;

pub use browserbase::{
    BROWSERBASE_API_URL, Browserbase, BrowserbaseEndpoint, BrowserbaseError, BrowserbaseHooks,
    BrowserbaseProjectUsage, BrowserbaseSessionStanding, BrowserbaseSessionUsage,
};

#[derive(Debug, thiserror::Error)]
pub enum BrowserError {
    #[error("No Chrome or Chromium installation was found")]
    NotFound,
    #[error("The browser failed to start: {0}")]
    Launch(String),
    #[error("The page could not be loaded: {0}")]
    Navigation(String),
    #[error("The scan was cancelled")]
    Cancelled,
    #[error("The browser profile is in use by another scan")]
    ProfileBusy,
    #[error("{0}")]
    Other(String),
    /// Browserbase did not accept the API key or the project.
    #[error("{0}")]
    RemoteAuth(String),
    /// Browserbase is at a limit of the project, after waiting for it.
    #[error("{0}")]
    RemoteLimit(String),
    /// Anything else that went wrong with the browser at Browserbase.
    #[error("{0}")]
    Remote(String),
}

/// Callbacks and cancellation for one capture.
#[derive(Clone)]
pub struct CaptureHooks {
    pub cancel: CancellationToken,
    /// Manual visits: stop recording and keep the data. Ignored by the
    /// scripted behaviours.
    pub finish: CancellationToken,
    /// Everything the capture does and refuses to do, as it happens.
    pub on_activity: Arc<dyn Fn(CaptureActivity) + Send + Sync>,
    /// Root of the browser profile that is kept between scans; the profile of
    /// each device is a directory below it. Needed when the request asks for
    /// the persistent profile.
    pub scan_profile_dir: Option<PathBuf>,
    /// Password of a custom proxy, when one is stored. Given to the proxy
    /// when it asks, over the DevTools connection. Never logged, never put on
    /// a command line.
    pub proxy_password: Option<String>,
    /// Where every request of the visit is handed over while it happens.
    /// `None`: the scan keeps no network data and no body is read for it.
    pub network: Option<Arc<dyn NetworkSink>>,
    /// How Browserbase is reached and who is asking. Needed when the
    /// request's browser is Browserbase.
    pub browserbase: Option<BrowserbaseHooks>,
}

/// Find a usable browser. `explicit` is the user's configured path, if any.
pub fn detect_browser(explicit: Option<&str>) -> Option<BrowserInfo> {
    detect::detect_browser(explicit)
}

/// Visit the page with the tracker injected, as the request's visitor would,
/// and return everything captured.
pub async fn capture(
    request: CaptureRequest,
    hooks: CaptureHooks,
) -> Result<Capture, BrowserError> {
    // The capture runs as its own task so that Chrome is shut down and its
    // profile removed even when the caller drops this future instead of
    // cancelling through `hooks.cancel`.
    let cancel = hooks.cancel.child_token();
    let cancel_if_dropped = cancel.clone().drop_guard();
    let hooks = CaptureHooks { cancel, ..hooks };
    let outcome = tokio::spawn(capture::run(request, hooks)).await;
    cancel_if_dropped.disarm();
    outcome.unwrap_or_else(|panic| {
        Err(BrowserError::Other(format!(
            "the capture task failed: {panic}"
        )))
    })
}

/// What is on disk for the kept browser profile under `root`: the profiles of
/// both devices together. Reads the disk; call it off the async runtime.
pub fn scan_profile_info(root: &Path) -> ScanProfileInfo {
    let profiles = capture::scan_profile_directories(root)
        .map(|profile| cdp::inspect_persistent_profile(&profile));
    let last_used = profiles
        .iter()
        .filter_map(|profile| profile.last_used)
        .max();
    ScanProfileInfo {
        exists: profiles.iter().any(|profile| profile.exists),
        size_bytes: profiles
            .iter()
            .map(|profile| profile.size_bytes)
            .sum::<u64>() as f64,
        last_used_at: last_used
            .and_then(|at| at.duration_since(UNIX_EPOCH).ok())
            .map(|since_epoch| since_epoch.as_secs_f64() * 1_000.0),
        in_use: profiles.iter().any(|profile| profile.in_use),
    }
}

/// Deletes the kept browser profile under `root`, for both devices. While a
/// scan uses either, nothing is deleted and the error is
/// [`BrowserError::ProfileBusy`]. Blocking.
pub fn clear_scan_profile(root: &Path) -> Result<(), BrowserError> {
    cdp::clear_persistent_profile(&capture::scan_profile_directories(root)).map_err(|error| {
        match error {
            cdp::PersistentProfileError::Busy => BrowserError::ProfileBusy,
            cdp::PersistentProfileError::Io(error) => {
                BrowserError::Other(format!("The browser profile could not be deleted: {error}"))
            }
        }
    })
}

/// Removes the throwaway browser profiles that scans which crashed, or were
/// killed, left in the temporary directory, and returns how many. Profiles
/// of scans that are running, here or in another instance of the app, and
/// the kept browser profile are never touched. Blocking; meant for app start.
pub fn sweep_stale_profiles() -> usize {
    cdp::remove_abandoned_profiles(&std::env::temp_dir())
}

/// Handle to the built-in demo publisher site. The server stops when dropped.
pub struct DemoSite {
    /// e.g. "http://127.0.0.1:49152/"
    pub url: String,
    pub shutdown: CancellationToken,
}

impl Drop for DemoSite {
    fn drop(&mut self) {
        self.shutdown.cancel();
    }
}

/// Serve the bundled demo publisher page (with planted ad-stack bugs) on a random
/// loopback port, with its stand-in ad-tech partners on two more. Works fully
/// offline.
pub async fn start_demo_site() -> std::io::Result<DemoSite> {
    demo::start().await
}
