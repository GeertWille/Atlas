//! Adapters from the outside world to the engine's ports: the real browser,
//! Browserbase and the Tauri event bus.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use adscan_browser::{BrowserError, Browserbase, BrowserbaseHooks, CaptureHooks};
use adscan_engine::application::ports::{
    BrowserbaseApi, BrowserbaseError, CaptureError, CaptureSession, CaptureSource, EventSink,
    ProjectUsage, RemoteSessionUsage,
};
use adscan_model::{
    BrowserInfo, BrowserbaseCredentials, BrowserbaseProbe, BrowserbaseProject, BrowserbaseRegion,
    Capture, CaptureRequest, SCAN_EVENT, ScanEvent, ScanProfileInfo,
};
use async_trait::async_trait;
use tauri::{AppHandle, Emitter, Runtime};

/// Directory under the app's data directory that holds the browser profile
/// persistent-profile scans share. The location is fixed here and never
/// taken from the user or the webview.
const SCAN_PROFILE_DIR: &str = "scan-profile";

/// Captures with Chrome on this machine or, when the request says so, with
/// a browser at Browserbase.
pub struct ChromeCaptureSource {
    scan_profile_dir: PathBuf,
    browserbase: Browserbase,
}

impl ChromeCaptureSource {
    pub fn new(data_dir: &Path, browserbase: Browserbase) -> Self {
        Self {
            scan_profile_dir: data_dir.join(SCAN_PROFILE_DIR),
            browserbase,
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
            browserbase: session.browserbase.map(|access| BrowserbaseHooks {
                client: self.browserbase.clone(),
                credentials: access.credentials,
                on_session: access.on_session,
            }),
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
                CaptureError::Other(format!("clearing the browser profile failed: {error}"))
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
        BrowserError::RemoteAuth(reason) => CaptureError::RemoteAuth(reason),
        BrowserError::RemoteLimit(reason) => CaptureError::RemoteLimit(reason),
        BrowserError::Remote(reason) => CaptureError::Remote(reason),
        BrowserError::Other(reason) => CaptureError::Other(reason),
    }
}

/// Shows an address in the user's own browser.
pub type OpenUrl = Arc<dyn Fn(&str) -> Result<(), String> + Send + Sync>;

/// Browserbase's API for everything but the browser a capture drives.
pub struct BrowserbaseAdapter {
    client: Browserbase,
    open_url: OpenUrl,
}

impl BrowserbaseAdapter {
    /// `open_url` is how a live view reaches the user's browser: the
    /// operating system's opener in the app, a recorder in tests.
    pub fn new(client: Browserbase, open_url: OpenUrl) -> Self {
        Self { client, open_url }
    }
}

fn browserbase_error(error: adscan_browser::BrowserbaseError) -> BrowserbaseError {
    match error {
        adscan_browser::BrowserbaseError::Auth(message) => BrowserbaseError::Auth(message),
        adscan_browser::BrowserbaseError::Limit(message) => BrowserbaseError::Limit(message),
        adscan_browser::BrowserbaseError::Failed(message) => BrowserbaseError::Failed(message),
    }
}

#[async_trait]
impl BrowserbaseApi for BrowserbaseAdapter {
    async fn projects(&self, api_key: &str) -> Result<Vec<BrowserbaseProject>, BrowserbaseError> {
        self.client
            .projects(api_key)
            .await
            .map_err(browserbase_error)
    }

    async fn project(
        &self,
        credentials: &BrowserbaseCredentials,
    ) -> Result<BrowserbaseProject, BrowserbaseError> {
        self.client
            .project(credentials)
            .await
            .map_err(browserbase_error)
    }

    async fn project_usage(
        &self,
        credentials: &BrowserbaseCredentials,
    ) -> Result<ProjectUsage, BrowserbaseError> {
        let usage = self
            .client
            .project_usage(credentials)
            .await
            .map_err(browserbase_error)?;
        Ok(ProjectUsage {
            browser_minutes: usage.browser_minutes,
            proxy_bytes: usage.proxy_bytes,
        })
    }

    async fn running_session_count(
        &self,
        credentials: &BrowserbaseCredentials,
    ) -> Result<u32, BrowserbaseError> {
        self.client
            .running_session_count(credentials)
            .await
            .map_err(browserbase_error)
    }

    async fn running_sessions_of(
        &self,
        credentials: &BrowserbaseCredentials,
        install_id: &str,
    ) -> Result<Vec<String>, BrowserbaseError> {
        self.client
            .running_sessions_of(credentials, install_id)
            .await
            .map_err(browserbase_error)
    }

    async fn session_usage(
        &self,
        credentials: &BrowserbaseCredentials,
        session_id: &str,
    ) -> Result<RemoteSessionUsage, BrowserbaseError> {
        let usage = self
            .client
            .session_usage(credentials, session_id)
            .await
            .map_err(browserbase_error)?;
        Ok(RemoteSessionUsage {
            ended: usage.ended,
            session_seconds: usage.session_seconds,
            proxy_bytes: usage.proxy_bytes,
        })
    }

    async fn release_session(
        &self,
        credentials: &BrowserbaseCredentials,
        session_id: &str,
    ) -> Result<(), BrowserbaseError> {
        self.client
            .release_session(credentials, session_id)
            .await
            .map_err(browserbase_error)
    }

    async fn create_context(
        &self,
        credentials: &BrowserbaseCredentials,
    ) -> Result<String, BrowserbaseError> {
        self.client
            .create_context(credentials)
            .await
            .map_err(browserbase_error)
    }

    async fn delete_context(
        &self,
        credentials: &BrowserbaseCredentials,
        context_id: &str,
    ) -> Result<(), BrowserbaseError> {
        self.client
            .delete_context(credentials, context_id)
            .await
            .map_err(browserbase_error)
    }

    async fn probe(
        &self,
        credentials: &BrowserbaseCredentials,
        region: BrowserbaseRegion,
        install_id: &str,
    ) -> Result<BrowserbaseProbe, BrowserbaseError> {
        self.client
            .probe(credentials, region, install_id)
            .await
            .map_err(browserbase_error)
    }

    async fn open_live_view(
        &self,
        credentials: &BrowserbaseCredentials,
        session_id: &str,
    ) -> Result<(), BrowserbaseError> {
        // The address lets whoever holds it drive the browser. It lives in
        // this function only, and the opener's own failure text, which may
        // quote it, is not passed on.
        let address = self
            .client
            .live_view_url(credentials, session_id)
            .await
            .map_err(browserbase_error)?;
        (self.open_url)(&address).map_err(|_| {
            BrowserbaseError::Failed(
                "The live view could not be opened in your browser.".to_string(),
            )
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
