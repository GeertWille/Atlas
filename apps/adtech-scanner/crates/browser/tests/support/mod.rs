//! Shared helpers for the integration tests: running captures against a real
//! Chromium and serving fixture pages.
#![allow(dead_code)]

use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use adscan_browser::{BrowserError, CaptureHooks, capture};
use adscan_model::{Capture, CaptureRequest, DeviceProfile, TrackerEvent, TrackerEventKind};
use axum::Router;
use axum::extract::Path;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

/// The Chromium this machine provides for tests, used when `ADSCAN_CHROME_PATH`
/// is not set. With neither, the normal auto-detection applies.
const TEST_CHROMIUM: &str = "/opt/pw-browsers/chromium-1243/chrome-linux64/chrome";

pub fn chrome_path() -> Option<String> {
    std::env::var("ADSCAN_CHROME_PATH").ok().or_else(|| {
        std::path::Path::new(TEST_CHROMIUM)
            .exists()
            .then(|| TEST_CHROMIUM.to_string())
    })
}

pub fn request(url: &str, device: DeviceProfile) -> CaptureRequest {
    CaptureRequest {
        url: url.to_string(),
        device,
        observe_seconds: 3,
        auto_scroll: false,
        accept_consent: false,
        headless: true,
        chrome_path: chrome_path(),
    }
}

/// Everything a capture reported, whether it succeeded or not.
pub struct Outcome {
    pub result: Result<Capture, BrowserError>,
    pub progress: Vec<String>,
}

pub async fn run(request: CaptureRequest) -> Outcome {
    run_cancellable(request, CancellationToken::new()).await
}

pub async fn run_cancellable(request: CaptureRequest, cancel: CancellationToken) -> Outcome {
    let lines = Arc::new(Mutex::new(Vec::new()));
    let sink = lines.clone();
    let hooks = CaptureHooks {
        cancel,
        on_progress: Arc::new(move |line| sink.lock().unwrap().push(line)),
    };
    let result = capture(request, hooks).await;
    let progress = lines.lock().unwrap().clone();
    Outcome { result, progress }
}

/// Runs a capture that must succeed and must not have dropped tracker events.
pub async fn capture_ok(request: CaptureRequest) -> Capture {
    let outcome = run(request).await;
    assert!(
        !outcome
            .progress
            .iter()
            .any(|line| line.contains("malformed")),
        "the tracker emitted events outside the contract: {:#?}",
        outcome.progress
    );
    outcome.result.unwrap_or_else(|error| {
        panic!("capture failed: {error}\nprogress: {:#?}", outcome.progress)
    })
}

pub fn events_where<'a>(
    capture: &'a Capture,
    matches: impl Fn(&TrackerEventKind) -> bool + 'a,
) -> impl Iterator<Item = &'a TrackerEvent> {
    capture
        .events
        .iter()
        .filter(move |event| matches(&event.kind))
}

pub fn first_time(capture: &Capture, matches: impl Fn(&TrackerEventKind) -> bool) -> Option<f64> {
    capture
        .events
        .iter()
        .find(|event| matches(&event.kind))
        .map(|event| event.t)
}

/// A local server for the pages in `tests/fixtures`, stopped when dropped.
pub struct Fixtures {
    pub port: u16,
    shutdown: CancellationToken,
}

impl Fixtures {
    pub async fn start() -> Self {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let shutdown = CancellationToken::new();
        let stopped = shutdown.clone().cancelled_owned();
        let routes = Router::new()
            .route("/status/{code}", get(status))
            .route("/never", get(never))
            .route("/{file}", get(fixture));
        tokio::spawn(async move {
            let _ = axum::serve(listener, routes)
                .with_graceful_shutdown(stopped)
                .await;
        });
        Self { port, shutdown }
    }

    pub fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}/{path}", self.port)
    }
}

impl Drop for Fixtures {
    fn drop(&mut self) {
        self.shutdown.cancel();
    }
}

async fn fixture(Path(file): Path<String>) -> Response {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(&file);
    let content_type = if file.ends_with(".js") {
        "text/javascript"
    } else {
        "text/html; charset=utf-8"
    };
    match tokio::fs::read(path).await {
        Ok(body) => ([(header::CONTENT_TYPE, content_type)], body).into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn status(Path(code): Path<u16>) -> Response {
    let status = StatusCode::from_u16(code).unwrap();
    (
        status,
        [(header::CONTENT_TYPE, "text/html")],
        format!("<h1>{status}</h1>"),
    )
        .into_response()
}

/// A resource that never answers, so a page that needs it never fires `load`.
async fn never() -> Response {
    std::future::pending().await
}
