//! Browser capture: drives a locally installed Chrome/Chromium over the DevTools
//! protocol, injects the tracker script, and returns an in-memory [`Capture`].
//!
//! The public API below is a fixed contract consumed by `src-tauri`.

mod capture;
mod cdp;
mod demo;
mod detect;
mod device;

use std::sync::Arc;

use adscan_model::{BrowserInfo, Capture, CaptureRequest};
use tokio_util::sync::CancellationToken;

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
    #[error("{0}")]
    Other(String),
}

/// Callbacks and cancellation for one capture.
#[derive(Clone)]
pub struct CaptureHooks {
    pub cancel: CancellationToken,
    /// Human-readable progress lines ("Launching Chrome 141", "Page loaded in 1.8s",
    /// "Scrolling (40%)", "Auction 2 started with 6 bidders").
    pub on_progress: Arc<dyn Fn(String) + Send + Sync>,
}

/// Find a usable browser. `explicit` is the user's configured path, if any.
pub fn detect_browser(explicit: Option<&str>) -> Option<BrowserInfo> {
    detect::detect_browser(explicit)
}

/// Load the page with the tracker injected, observe it, and return everything captured.
pub async fn capture(request: CaptureRequest, hooks: CaptureHooks) -> Result<Capture, BrowserError> {
    // The capture runs as its own task so that Chrome is shut down and its
    // profile removed even when the caller drops this future instead of
    // cancelling through `hooks.cancel`.
    let cancel = hooks.cancel.child_token();
    let cancel_if_dropped = cancel.clone().drop_guard();
    let hooks = CaptureHooks { cancel, on_progress: hooks.on_progress };
    let outcome = tokio::spawn(capture::run(request, hooks)).await;
    cancel_if_dropped.disarm();
    outcome.unwrap_or_else(|panic| Err(BrowserError::Other(format!("the capture task failed: {panic}"))))
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
/// loopback port. Works fully offline.
pub async fn start_demo_site() -> std::io::Result<DemoSite> {
    demo::start().await
}
