//! Adapters from the outside world to the engine's ports: the real browser
//! and the Tauri event bus.

use adscan_browser::{BrowserError, CaptureHooks};
use adscan_engine::application::ports::{CaptureError, CaptureSource, EventSink, ProgressFn};
use adscan_model::{BrowserInfo, Capture, CaptureRequest, SCAN_EVENT, ScanEvent};
use async_trait::async_trait;
use tauri::{AppHandle, Emitter, Runtime};
use tokio_util::sync::CancellationToken;

pub struct ChromeCaptureSource;

#[async_trait]
impl CaptureSource for ChromeCaptureSource {
    async fn locate_browser(&self, explicit: Option<String>) -> Option<BrowserInfo> {
        // Detection runs the browser binary to read its version.
        tokio::task::spawn_blocking(move || adscan_browser::detect_browser(explicit.as_deref()))
            .await
            .ok()
            .flatten()
    }

    async fn capture(
        &self,
        request: CaptureRequest,
        cancel: CancellationToken,
        on_progress: ProgressFn,
    ) -> Result<Capture, CaptureError> {
        let hooks = CaptureHooks {
            cancel,
            on_progress,
        };
        adscan_browser::capture(request, hooks)
            .await
            .map_err(|error| match error {
                BrowserError::NotFound => CaptureError::BrowserNotFound,
                BrowserError::Launch(reason) => CaptureError::Launch(reason),
                BrowserError::Navigation(reason) => CaptureError::Navigation(reason),
                BrowserError::Cancelled => CaptureError::Cancelled,
                BrowserError::Other(reason) => CaptureError::Other(reason),
            })
    }
}

/// Publishes scan events to every window as the `scan-event` Tauri event.
pub struct TauriEventSink<R: Runtime> {
    pub app: AppHandle<R>,
}

impl<R: Runtime> EventSink for TauriEventSink<R> {
    fn emit(&self, event: ScanEvent) {
        // A window that is closing cannot receive events; nothing to recover.
        let _ = self.app.emit(SCAN_EVENT, event);
    }
}
