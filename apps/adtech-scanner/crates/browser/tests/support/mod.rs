//! Shared helpers for the integration tests: running captures against a real
//! Chromium and serving fixture pages.
#![allow(dead_code)]

#[cfg(target_os = "linux")]
pub mod chrome_processes;

use std::io::Write;
use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use adscan_browser::{BrowserError, CaptureHooks, capture};
use adscan_model::{
    ActivityKind, BodyAdmission, BodyOffer, BodySkip, Capture, CaptureActivity, CaptureProxy,
    CaptureRequest, CapturedBody, DeviceProfile, NetworkRecord, NetworkSink, RawHeader,
    ResourceKind, TrackerEvent, TrackerEventKind, VisitorBehavior,
};
use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use flate2::Compression;
use flate2::write::GzEncoder;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
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
        visitor: VisitorBehavior::None,
        extra_pages: 0,
        seed: 1,
        accept_consent: false,
        headless: true,
        persistent_profile: false,
        proxy: CaptureProxy::System,
        chrome_path: chrome_path(),
    }
}

/// Everything a capture reported, whether it succeeded or not.
pub struct Outcome {
    pub result: Result<Capture, BrowserError>,
    /// The message of every activity, in order.
    pub progress: Vec<String>,
    pub activities: Vec<CaptureActivity>,
}

impl Outcome {
    pub fn activities_of(&self, kind: ActivityKind) -> Vec<&CaptureActivity> {
        self.activities
            .iter()
            .filter(|activity| activity.kind == kind)
            .collect()
    }
}

/// Hooks that collect what the capture reports, and where they collect it.
pub struct Reported {
    pub hooks: CaptureHooks,
    activities: Arc<Mutex<Vec<CaptureActivity>>>,
}

impl Reported {
    pub fn new() -> Self {
        let activities = Arc::new(Mutex::new(Vec::new()));
        let sink = activities.clone();
        Self {
            hooks: CaptureHooks {
                cancel: CancellationToken::new(),
                finish: CancellationToken::new(),
                on_activity: Arc::new(move |activity| sink.lock().unwrap().push(activity)),
                scan_profile_dir: None,
                proxy_password: None,
                network: None,
            },
            activities,
        }
    }

    pub fn so_far(&self) -> Vec<CaptureActivity> {
        self.activities.lock().unwrap().clone()
    }

    /// What the capture has reported, to watch while it runs.
    pub fn activities(&self) -> Arc<Mutex<Vec<CaptureActivity>>> {
        self.activities.clone()
    }

    pub async fn run(self, request: CaptureRequest) -> Outcome {
        let result = capture(request, self.hooks.clone()).await;
        let activities = self.so_far();
        Outcome {
            result,
            progress: activities
                .iter()
                .map(|activity| activity.message.clone())
                .collect(),
            activities,
        }
    }
}

pub async fn run(request: CaptureRequest) -> Outcome {
    Reported::new().run(request).await
}

/// The per-body cap of a sink that keeps text, as the app's archive has it.
pub const KEPT_BODY_BYTES: usize = 5 * 1024 * 1024;

type Admit = Box<dyn Fn(&BodyOffer<'_>) -> BodyAdmission + Send + Sync>;

/// A network sink that keeps what the capture hands over.
pub struct Requests {
    admit: Admit,
    /// `seq` and expected bytes of every body that was offered.
    offered: Mutex<Vec<(u32, u64)>>,
    recorded: Mutex<Vec<NetworkRecord>>,
}

impl Requests {
    /// Admits what the app's archive keeps: documents, scripts, styles and
    /// what scripts fetch, when it is text; never images, fonts or media.
    pub fn keeping_text() -> Arc<Self> {
        Self::admitting(|offer| {
            let is_text = offer.mime_type.is_none_or(|mime| {
                mime.starts_with("text/")
                    || mime.contains("json")
                    || mime.contains("javascript")
                    || mime.contains("xml")
            });
            let kept_kind = matches!(
                offer.kind,
                ResourceKind::Document
                    | ResourceKind::Script
                    | ResourceKind::Stylesheet
                    | ResourceKind::Xhr
                    | ResourceKind::Fetch
                    | ResourceKind::Other
            );
            if kept_kind && is_text {
                BodyAdmission::Read {
                    max_bytes: KEPT_BODY_BYTES,
                }
            } else {
                BodyAdmission::Skip(BodySkip::Type)
            }
        })
    }

    pub fn admitting(
        admit: impl Fn(&BodyOffer<'_>) -> BodyAdmission + Send + Sync + 'static,
    ) -> Arc<Self> {
        Arc::new(Self {
            admit: Box::new(admit),
            offered: Mutex::new(Vec::new()),
            recorded: Mutex::new(Vec::new()),
        })
    }

    /// Every record so far, in the order of `seq`.
    pub fn records(&self) -> Vec<NetworkRecord> {
        let mut records = self.recorded.lock().unwrap().clone();
        records.sort_by_key(|record| record.seq);
        records
    }

    pub fn count(&self) -> usize {
        self.recorded.lock().unwrap().len()
    }

    pub fn offered(&self) -> Vec<(u32, u64)> {
        self.offered.lock().unwrap().clone()
    }
}

impl NetworkSink for Requests {
    fn admit_body(&self, offer: &BodyOffer<'_>) -> BodyAdmission {
        self.offered
            .lock()
            .unwrap()
            .push((offer.seq, offer.expected_bytes));
        (self.admit)(offer)
    }

    fn record(&self, record: NetworkRecord) {
        self.recorded.lock().unwrap().push(record);
    }
}

/// Runs a capture that must succeed, with its requests recorded.
pub async fn capture_with_requests(request: CaptureRequest) -> (Capture, Vec<NetworkRecord>) {
    let requests = Requests::keeping_text();
    let mut reported = Reported::new();
    reported.hooks.network = Some(requests.clone());
    let outcome = reported.run(request).await;
    let capture = outcome.result.unwrap_or_else(|error| {
        panic!("capture failed: {error}\nprogress: {:#?}", outcome.progress)
    });
    (capture, requests.records())
}

pub fn header_value<'a>(headers: &'a [RawHeader], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|header| header.name.eq_ignore_ascii_case(name))
        .map(|header| header.value.as_str())
}

/// The body a record carries, as text.
pub fn body_text(record: &NetworkRecord) -> Option<String> {
    match &record.body {
        CapturedBody::Read { bytes, .. } => Some(String::from_utf8_lossy(bytes).into_owned()),
        _ => None,
    }
}

pub async fn run_cancellable(request: CaptureRequest, cancel: CancellationToken) -> Outcome {
    let mut reported = Reported::new();
    reported.hooks.cancel = cancel;
    reported.run(request).await
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
/// It remembers what was asked of it.
pub struct Fixtures {
    pub port: u16,
    requested: Arc<Mutex<Vec<String>>>,
    shutdown: CancellationToken,
}

impl Fixtures {
    pub async fn start() -> Self {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let shutdown = CancellationToken::new();
        let stopped = shutdown.clone().cancelled_owned();
        let requested = Arc::new(Mutex::new(Vec::new()));
        let routes = Router::new()
            .fallback(respond)
            .with_state(requested.clone());
        tokio::spawn(async move {
            let _ = axum::serve(listener, routes)
                .with_graceful_shutdown(stopped)
                .await;
        });
        Self {
            port,
            requested,
            shutdown,
        }
    }

    pub fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}/{path}", self.port)
    }

    /// Path and query of every request so far, in order.
    pub fn requested(&self) -> Vec<String> {
        self.requested.lock().unwrap().clone()
    }
}

impl Drop for Fixtures {
    fn drop(&mut self) {
        self.shutdown.cancel();
    }
}

async fn respond(
    State(requested): State<Arc<Mutex<Vec<String>>>>,
    uri: Uri,
    headers: HeaderMap,
) -> Response {
    let asked = uri
        .path_and_query()
        .map_or(uri.path(), |asked| asked.as_str());
    requested.lock().unwrap().push(asked.to_string());
    let path = uri.path().trim_start_matches('/');
    if let Some(code) = path.strip_prefix("status/") {
        return status(code.parse().unwrap());
    }
    match path {
        // A page of a site that asks for a login of its own. Any login it is
        // given is remembered as a request of its own, to be found.
        "members-only" => {
            for name in [header::AUTHORIZATION, header::PROXY_AUTHORIZATION] {
                if let Some(login) = headers.get(&name) {
                    requested
                        .lock()
                        .unwrap()
                        .push(format!("LOGIN {name}: {login:?}"));
                }
            }
            (
                StatusCode::UNAUTHORIZED,
                [(header::WWW_AUTHENTICATE, "Basic realm=\"members\"")],
                "<h1>Members only</h1>",
            )
                .into_response()
        }
        // Sends the browser on to the address given as the query.
        "redirect" => (
            StatusCode::FOUND,
            [(header::LOCATION, uri.query().unwrap_or_default())],
        )
            .into_response(),
        // Never answers, so a page that needs it never fires `load`.
        "never" => std::future::pending().await,
        "download" => download(),
        "events" => event_stream(),
        "large.json" => large_json(),
        "payload" => payload_response(uri.query().unwrap_or_default()).await,
        _ => fixture(path).await,
    }
}

async fn fixture(file: &str) -> Response {
    // Fixture pages link to each other with and without the extension.
    let name = if file.contains('.') {
        file.to_string()
    } else {
        format!("{file}.html")
    };
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(&name);
    let content_type = if name.ends_with(".js") {
        "text/javascript"
    } else {
        "text/html; charset=utf-8"
    };
    match tokio::fs::read(path).await {
        Ok(body) => ([(header::CONTENT_TYPE, content_type)], body).into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

fn status(code: u16) -> Response {
    let status = StatusCode::from_u16(code).unwrap();
    (
        status,
        [(header::CONTENT_TYPE, "text/html")],
        format!("<h1>{status}</h1>"),
    )
        .into_response()
}

/// How large the body of `large.json` is: more than a sink keeps of one body.
pub const LARGE_JSON_BYTES: usize = 8 * 1024 * 1024;

fn large_json() -> Response {
    let filler = "x".repeat(LARGE_JSON_BYTES - r#"{"pad":""}"#.len());
    (
        [(header::CONTENT_TYPE, "application/json")],
        format!(r#"{{"pad":"{filler}"}}"#),
    )
        .into_response()
}

/// The value of `name` in a query like `size=100&by=worker`.
fn query_value<'a>(query: &'a str, name: &str) -> Option<&'a str> {
    query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .find_map(|(key, value)| (key == name).then_some(value))
}

/// What `payload?<query>` answers with: `size=<bytes>` of JSON that names
/// the query, so that no two answers are the same.
pub fn payload(query: &str) -> Vec<u8> {
    let size: usize = query_value(query, "size").unwrap().parse().unwrap();
    let mut body = format!(r#"{{"asked":"{query}","pad":""#).into_bytes();
    // Letters without a pattern, so that compressing them is real work.
    let mut state = 0x2545_f491_u32;
    while body.len() + 2 < size {
        state = state.wrapping_mul(1_103_515_245).wrapping_add(12_345);
        body.push(b'a' + ((state >> 16) % 26) as u8);
    }
    body.extend_from_slice(br#""}"#);
    assert_eq!(body.len(), size, "{query} is longer than its size");
    body
}

/// [`payload`], as the query wants it sent: `wait=<ms>` that much later, as
/// from a server that is not on this machine, `encoding=gzip` compressed,
/// `cache=1` to be kept by the browser, `stall=1` half of it and then
/// nothing more, on a connection that stays open.
async fn payload_response(query: &str) -> Response {
    if let Some(wait) = query_value(query, "wait") {
        tokio::time::sleep(Duration::from_millis(wait.parse().unwrap())).await;
    }
    let mut body = payload(query);
    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, "application/json".parse().unwrap());
    if query_value(query, "encoding") == Some("gzip") {
        let mut compressed = GzEncoder::new(Vec::new(), Compression::default());
        compressed.write_all(&body).unwrap();
        body = compressed.finish().unwrap();
        headers.insert(header::CONTENT_ENCODING, "gzip".parse().unwrap());
    }
    if query_value(query, "cache") == Some("1") {
        headers.insert(header::CACHE_CONTROL, "max-age=600".parse().unwrap());
    }
    if query_value(query, "stall") == Some("1") {
        body.truncate(body.len() / 2);
        let first_half =
            futures_util::stream::once(async move { Ok::<_, std::convert::Infallible>(body) });
        let never_ends = axum::body::Body::from_stream(futures_util::StreamExt::chain(
            first_half,
            futures_util::stream::pending(),
        ));
        return (headers, never_ends).into_response();
    }
    (headers, body).into_response()
}

/// A server-sent event stream that sends one event and never ends.
fn event_stream() -> Response {
    let first =
        futures_util::stream::once(async { Ok::<_, std::convert::Infallible>("data: tick\n\n") });
    let body = axum::body::Body::from_stream(futures_util::StreamExt::chain(
        first,
        futures_util::stream::pending(),
    ));
    ([(header::CONTENT_TYPE, "text/event-stream")], body).into_response()
}

/// A response the browser saves to disk instead of showing.
fn download() -> Response {
    (
        [
            (header::CONTENT_TYPE, "application/octet-stream"),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=\"adscan-download-probe.bin\"",
            ),
        ],
        "not a page",
    )
        .into_response()
}

/// The body of a plain GET, for asking the test servers what they saw.
pub async fn http_get(url: &str) -> String {
    let address = url.strip_prefix("http://").expect("a local http address");
    let (authority, path) = address.split_once('/').unwrap_or((address, ""));
    let mut connection = TcpStream::connect(authority).await.unwrap();
    let request = format!("GET /{path} HTTP/1.1\r\nHost: {authority}\r\nConnection: close\r\n\r\n");
    connection.write_all(request.as_bytes()).await.unwrap();
    let mut response = String::new();
    connection.read_to_string(&mut response).await.unwrap();
    response
        .split_once("\r\n\r\n")
        .map(|(_, body)| body.to_string())
        .unwrap_or_default()
}

/// Whether a test that shows a browser window can run in this process. On
/// Linux a window needs a display: without one the test runs itself again
/// under `xvfb-run` and this returns `false` when that run has passed.
///
/// ```ignore
/// if !support::has_a_display("name_of_this_test") { return; }
/// ```
pub fn has_a_display(test: &str) -> bool {
    if !cfg!(target_os = "linux") || std::env::var_os("DISPLAY").is_some() {
        return true;
    }
    let run_again = std::process::Command::new("xvfb-run")
        .arg("--auto-servernum")
        .arg(std::env::current_exe().unwrap())
        .args([test, "--exact", "--nocapture", "--test-threads=1"])
        .status();
    match run_again {
        Ok(outcome) => assert!(outcome.success(), "{test} failed under xvfb-run"),
        Err(error) => eprintln!("{test} was skipped: no display and no xvfb-run ({error})"),
    }
    false
}
