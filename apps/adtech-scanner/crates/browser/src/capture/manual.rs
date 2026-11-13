//! A manual visit: a person browses in the browser window and the scanner
//! only records. Nothing is clicked, scrolled or accepted for them.

use std::time::Duration;

use adscan_model::CaptureActivity;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use crate::cdp::CdpError;

use super::error::CaptureError;
use super::page::Page;

/// A session nobody finishes ends by itself, this long after the capture was
/// asked for: the app counts down from the same moment.
pub(crate) const SESSION_LIMIT: Duration = Duration::from_secs(15 * 60);
const TICK: Duration = Duration::from_secs(1);

/// Where the person browses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Window {
    /// The window of the browser on this machine. Its connection closing
    /// is the person quitting the browser.
    OnThisMachine,
    /// Browserbase's live view of a browser in its cloud. Its connection
    /// closing is the connection lost: the person can close tabs there, but
    /// not the browser.
    LiveView,
}

/// How a manual visit ended. Either way, what was recorded is kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Ending {
    /// Finished from the app, or out of time: the browser is still there.
    BrowserOpen,
    /// The person closed the window or quit the browser: there is nobody
    /// left to ask for anything.
    BrowserGone,
}

/// Records until `finish` is triggered, the browser is closed, or `limit`
/// passes.
pub(crate) async fn record(
    page: &mut Page,
    finish: &CancellationToken,
    limit: Instant,
    window: Window,
) -> Result<Ending, CaptureError> {
    page.recorder.report(CaptureActivity::progress(format!(
        "Recording. Browse the site in {}, then finish the capture in the app (up to {} minutes)",
        match window {
            Window::OnThisMachine => "the browser window",
            Window::LiveView => "Browserbase's live view",
        },
        SESSION_LIMIT.as_secs() / 60
    )));
    while Instant::now() < limit {
        let recorded = tokio::select! {
            biased;
            _ = finish.cancelled() => return Ok(Ending::BrowserOpen),
            recorded = record_one_tick(page, limit) => recorded,
        };
        match recorded {
            Ok(()) => {}
            Err(CaptureError::TabClosed) => return Ok(Ending::BrowserGone),
            Err(CaptureError::Protocol(CdpError::Closed)) if window == Window::OnThisMachine => {
                return Ok(Ending::BrowserGone);
            }
            Err(error) => return Err(error),
        }
    }
    page.report_blocked(
        "deadline",
        "The session reached its time limit; recording stopped",
        None,
    );
    Ok(Ending::BrowserOpen)
}

async fn record_one_tick(page: &mut Page, limit: Instant) -> Result<(), CaptureError> {
    page.record_until(limit.min(Instant::now() + TICK)).await?;
    // Read while the page is there: a closed window has no title to ask for.
    page.note_title().await
}
