//! A manual visit: a person browses in the browser window and the scanner
//! only records. Nothing is clicked, scrolled or accepted for them.

use std::time::Duration;

use adscan_model::CaptureActivity;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use crate::cdp::CdpError;

use super::error::CaptureError;
use super::page::Page;

/// A session nobody finishes ends by itself.
const SESSION_LIMIT: Duration = Duration::from_secs(15 * 60);
const TICK: Duration = Duration::from_secs(1);

/// How a manual visit ended. Either way, what was recorded is kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Ending {
    /// Finished from the app, or out of time: the browser is still there.
    BrowserOpen,
    /// The person closed the window or quit the browser: there is nobody
    /// left to ask for anything.
    BrowserGone,
}

/// Records until `finish` is triggered, the browser is closed, or the session
/// limit passes.
pub(crate) async fn record(
    page: &mut Page,
    finish: &CancellationToken,
) -> Result<Ending, CaptureError> {
    page.recorder.report(CaptureActivity::progress(format!(
        "Recording. Browse the site in the browser window, then finish the capture in the app \
         (up to {} minutes)",
        SESSION_LIMIT.as_secs() / 60
    )));
    let limit = Instant::now() + SESSION_LIMIT;
    while Instant::now() < limit {
        let recorded = tokio::select! {
            biased;
            _ = finish.cancelled() => return Ok(Ending::BrowserOpen),
            recorded = record_one_tick(page, limit) => recorded,
        };
        match recorded {
            Ok(()) => {}
            Err(CaptureError::Protocol(CdpError::Closed) | CaptureError::TabClosed) => {
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
