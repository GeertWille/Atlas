//! Adapters from the outside world to the engine's ports: the real browser
//! and the Tauri event bus.

use std::path::{Path, PathBuf};

use adscan_browser::{BrowserError, CaptureHooks};
use adscan_engine::application::ports::{CaptureError, CaptureSession, CaptureSource, EventSink};
use adscan_model::{BrowserInfo, Capture, CaptureRequest, SCAN_EVENT, ScanEvent, ScanProfileInfo};
use async_trait::async_trait;
use tauri::{AppHandle, Emitter, Runtime};

/// Directory under the app's data directory that holds the browser profile
/// persistent-profile scans share. The location is fixed here and never
/// taken from the user or the webview.
const SCAN_PROFILE_DIR: &str = "scan-profile";

pub struct ChromeCaptureSource {
    scan_profile_dir: PathBuf,
}

impl ChromeCaptureSource {
    pub fn new(data_dir: &Path) -> Self {
        Self {
            scan_profile_dir: data_dir.join(SCAN_PROFILE_DIR),
        }
    }
}

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
        session: CaptureSession,
    ) -> Result<Capture, CaptureError> {
        let hooks = CaptureHooks {
            cancel: session.cancel,
            finish: session.finish,
            on_activity: session.on_activity,
            scan_profile_dir: Some(self.scan_profile_dir.clone()),
            proxy_password: session.proxy_password,
            network: session.network,
        };
        adscan_browser::capture(request, hooks)
            .await
            .map_err(capture_error)
    }

    async fn scan_profile_info(&self) -> ScanProfileInfo {
        let dir = self.scan_profile_dir.clone();
        // Walks the profile directory to add up its size.
        tokio::task::spawn_blocking(move || adscan_browser::scan_profile_info(&dir))
            .await
            .unwrap_or(ScanProfileInfo {
                exists: false,
                size_bytes: 0.0,
                last_used_at: None,
                in_use: false,
            })
    }

    async fn clear_scan_profile(&self) -> Result<(), CaptureError> {
        let dir = self.scan_profile_dir.clone();
        tokio::task::spawn_blocking(move || adscan_browser::clear_scan_profile(&dir))
            .await
            .map_err(|error| {
                CaptureError::Other(format!("clearing the scan profile failed: {error}"))
            })?
            .map_err(capture_error)
    }
}

fn capture_error(error: BrowserError) -> CaptureError {
    match error {
        BrowserError::NotFound => CaptureError::BrowserNotFound,
        BrowserError::Launch(reason) => CaptureError::Launch(reason),
        BrowserError::Navigation(reason) => CaptureError::Navigation(reason),
        BrowserError::Cancelled => CaptureError::Cancelled,
        BrowserError::ProfileBusy => CaptureError::ProfileBusy,
        BrowserError::Other(reason) => CaptureError::Other(reason),
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
