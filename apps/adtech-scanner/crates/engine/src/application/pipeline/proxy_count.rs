//! Browserbase counts a session's proxy traffic with a delay: right after
//! the browser has ended the figure can still read zero. It is read once
//! more a little later, after the scan itself has written its last word, so
//! no report waits for it and nothing writes the summary after it.

use std::time::Duration;

use adscan_model::{BrowserUse, BrowserbaseCredentials, ScanEvent};

use super::PipelineDeps;

/// The session whose proxy traffic is still being counted.
pub struct EndedSession {
    pub scan_id: String,
    pub session_id: String,
    /// Epoch ms.
    pub ended_at: f64,
    pub credentials: BrowserbaseCredentials,
}

/// Reads the count again once `deps.proxy_count_delay` has passed since the
/// session ended, and writes it to the scan if it moved. Best effort: a scan
/// that was deleted meanwhile, an unreachable Browserbase or a closed app
/// leave the first reading in place.
pub fn read_again_later(deps: PipelineDeps, session: EndedSession) {
    tokio::spawn(async move {
        let waited =
            Duration::from_secs_f64(((deps.clock.now_ms() - session.ended_at) / 1_000.0).max(0.0));
        tokio::time::sleep(deps.proxy_count_delay.saturating_sub(waited)).await;
        let Ok(usage) = deps
            .browserbase
            .session_usage(&session.credentials, &session.session_id)
            .await
        else {
            return;
        };
        let Some(proxy_bytes) = usage.proxy_bytes else {
            return;
        };
        let Ok(Some(detail)) = deps.scans.detail(&session.scan_id).await else {
            return;
        };
        let mut summary = detail.summary;
        let Some(remote) = summary.remote.as_mut() else {
            return;
        };
        if remote.proxy_bytes == Some(proxy_bytes) {
            return;
        }
        remote.proxy_bytes = Some(proxy_bytes);
        // The environment first: whoever hears of the summary reads both.
        if let Some(mut environment) = detail.environment
            && let BrowserUse::Browserbase(visit) = &mut environment.browser
        {
            visit.proxy_bytes = Some(proxy_bytes);
            let _ = deps
                .scans
                .save_environment(&session.scan_id, &environment)
                .await;
        }
        if deps.scans.save_summary(&summary).await.is_err() {
            return;
        }
        deps.events.emit(ScanEvent::ScanUpdated { summary });
    });
}
