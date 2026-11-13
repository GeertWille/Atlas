//! A stand-in for Browserbase: an HTTP server with the shapes of its REST
//! API as they were measured, and behind it one local Chromium per session
//! that is reached over a WebSocket, like the cloud browser is.
//!
//! The Chromium is started so that every iframe stays in the page's
//! process, because Browserbase has no out-of-process iframes. A relay
//! between the capture and that Chromium plays the service's gateway: it
//! checks the signing key, answers 410 for a session that has ended,
//! remembers every command the capture sent, and can cut the connection.
#![allow(dead_code)]

use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, Method, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request};
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_util::sync::CancellationToken;

use super::chrome_path;

/// The secrets of the stand-in. None of them may show up in anything the
/// capture reports, returns or fails with.
pub const API_KEY: &str = "bb_live_K3y-that-must-not-leak-7d1f02";
pub const PROJECT_ID: &str = "3f8a1c52-6e0b-4b7d-9a43-5d2c8e9f1a07";
pub const OTHER_PROJECT_ID: &str = "aaaaaaaa-0000-4000-8000-000000000000";
const SIGNING_KEY_PREFIX: &str = "s1gn1ng-key-that-must-not-leak-";
const LIVE_VIEW_TOKEN_PREFIX: &str = "l1ve-view-token-that-must-not-leak-";

const BROWSER_START_PATIENCE: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Status {
    Running,
    Completed,
}

/// A command the capture sent to the browser of a session.
#[derive(Debug, Clone, PartialEq)]
pub struct Command {
    pub id: u64,
    pub method: String,
    pub params: Value,
    pub in_session: bool,
    /// How large the message was that answered it; `None` while unanswered.
    pub reply_bytes: Option<usize>,
}

struct Session {
    id: String,
    /// The body of the request that created it.
    body: Value,
    status: Status,
    created_at: f64,
    ended_at: Option<f64>,
    signing_key: String,
    live_view_token: String,
    browser: Option<StandIn>,
    /// The service's own client on the session's page. What it installed
    /// there lasts as long as it is connected.
    own_client: Option<PageClient>,
    commands: Arc<Mutex<Vec<Command>>>,
    release_requests: u32,
    reads_after_end: u32,
    cut: CancellationToken,
    connections: u32,
}

/// What a test may know about a session of the stand-in.
#[derive(Debug, Clone)]
pub struct SessionView {
    pub id: String,
    pub body: Value,
    pub running: bool,
    pub release_requests: u32,
    pub connections: u32,
    pub commands: Vec<Command>,
}

impl SessionView {
    pub fn sent(&self, method: &str) -> Vec<&Command> {
        self.commands
            .iter()
            .filter(|command| command.method == method)
            .collect()
    }
}

/// An HTTP request the stand-in received.
#[derive(Debug, Clone)]
pub struct Asked {
    pub method: String,
    pub path_and_query: String,
    pub key: Option<String>,
}

struct Service {
    sessions: Mutex<Vec<Session>>,
    /// Profile directory of each context that exists.
    contexts: Mutex<HashMap<String, TempDir>>,
    asked: Mutex<Vec<Asked>>,
    relay_port: u16,
    concurrency: AtomicU32,
    /// Sessions of "the other application" that hold a slot; each refused
    /// request frees one.
    slots_held_by_others: AtomicU32,
    /// How many reads of an ended session still report no proxy traffic.
    proxy_bytes_lag_reads: AtomicU32,
    proxy_bytes: AtomicU64,
    failing_releases: AtomicU32,
    next_id: AtomicU32,
}

/// The stand-in service. Its browsers are killed when it is dropped.
pub struct Cloud {
    pub base_url: String,
    service: Arc<Service>,
    shutdown: CancellationToken,
}

impl Cloud {
    /// `None` on a machine without a Chromium to stand in for the cloud's.
    pub async fn start() -> Option<Self> {
        chrome_path()?;
        let relay = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let api = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let service = Arc::new(Service {
            sessions: Mutex::default(),
            contexts: Mutex::default(),
            asked: Mutex::default(),
            relay_port: relay.local_addr().unwrap().port(),
            concurrency: AtomicU32::new(3),
            slots_held_by_others: AtomicU32::new(0),
            proxy_bytes_lag_reads: AtomicU32::new(0),
            proxy_bytes: AtomicU64::new(0),
            failing_releases: AtomicU32::new(0),
            next_id: AtomicU32::new(1),
        });
        let shutdown = CancellationToken::new();
        let base_url = format!("http://127.0.0.1:{}", api.local_addr().unwrap().port());
        let routes = Router::new().fallback(answer).with_state(service.clone());
        let stopped = shutdown.clone().cancelled_owned();
        tokio::spawn(async move {
            let _ = axum::serve(api, routes)
                .with_graceful_shutdown(stopped)
                .await;
        });
        tokio::spawn(relay_connections(relay, service.clone(), shutdown.clone()));
        Some(Self {
            base_url,
            service,
            shutdown,
        })
    }

    pub fn sessions(&self) -> Vec<SessionView> {
        let sessions = self.service.sessions.lock().unwrap();
        sessions
            .iter()
            .map(|session| SessionView {
                id: session.id.clone(),
                body: session.body.clone(),
                running: session.status == Status::Running,
                release_requests: session.release_requests,
                connections: session.connections,
                commands: session.commands.lock().unwrap().clone(),
            })
            .collect()
    }

    pub fn session(&self, id: &str) -> SessionView {
        self.sessions()
            .into_iter()
            .find(|session| session.id == id)
            .expect("a session with that id")
    }

    /// Every HTTP request so far, in order.
    pub fn asked(&self) -> Vec<Asked> {
        self.service.asked.lock().unwrap().clone()
    }

    pub fn context_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self
            .service
            .contexts
            .lock()
            .unwrap()
            .keys()
            .cloned()
            .collect();
        ids.sort();
        ids
    }

    /// Another application holds `slots` of the project's sessions; one is
    /// freed each time a request for a session is refused.
    pub fn others_hold(&self, slots: u32) {
        self.service
            .slots_held_by_others
            .store(slots, Ordering::Relaxed);
    }

    /// The other application's sessions never end.
    pub fn stay_full(&self) {
        self.service.concurrency.store(0, Ordering::Relaxed);
    }

    /// A session that ended reports `bytes` of proxy traffic, but only from
    /// its read number `lag_reads + 1` on.
    pub fn report_proxy_bytes(&self, bytes: u64, lag_reads: u32) {
        self.service.proxy_bytes.store(bytes, Ordering::Relaxed);
        self.service
            .proxy_bytes_lag_reads
            .store(lag_reads, Ordering::Relaxed);
    }

    /// The next `count` requests to release a session fail with a 500.
    pub fn fail_releases(&self, count: u32) {
        self.service
            .failing_releases
            .store(count, Ordering::Relaxed);
    }

    /// Cuts the connection between the capture and the browser of a
    /// session, without a closing handshake.
    pub fn cut_connection(&self, session_id: &str) {
        let sessions = self.service.sessions.lock().unwrap();
        if let Some(session) = sessions.iter().find(|session| session.id == session_id) {
            session.cut.cancel();
        }
    }

    /// The address of the session's first page in the stand-in browser
    /// itself, where the service's own client, or its live view, attaches.
    pub async fn page_socket(&self, session_id: &str) -> String {
        let port = {
            let sessions = self.service.sessions.lock().unwrap();
            let session = sessions
                .iter()
                .find(|session| session.id == session_id)
                .expect("a session with that id");
            session.browser.as_ref().expect("a running browser").port
        };
        page_socket_of(port).await
    }

    /// Every secret value the stand-in has handed out or expects.
    pub fn secrets(&self) -> Vec<String> {
        let sessions = self.service.sessions.lock().unwrap();
        let mut secrets = vec![API_KEY.to_string()];
        for session in sessions.iter() {
            secrets.push(session.signing_key.clone());
            secrets.push(session.live_view_token.clone());
        }
        secrets
    }

    /// Fails when `text` holds a secret of the stand-in, or anything that
    /// looks like the address of a browser or a live view.
    pub fn assert_no_secret_in(&self, text: &str, what: &str) {
        for secret in self.secrets() {
            assert!(!text.contains(&secret), "{what} holds a secret: {text}");
        }
        for marker in [
            SIGNING_KEY_PREFIX,
            LIVE_VIEW_TOKEN_PREFIX,
            "signingKey",
            "ws://",
            "wss://",
            "/connect/",
            "devtools-fullscreen",
            "inspector.html",
        ] {
            assert!(!text.contains(marker), "{what} holds `{marker}`: {text}");
        }
    }
}

impl Drop for Cloud {
    fn drop(&mut self) {
        self.shutdown.cancel();
        // Dropping the sessions kills their browsers; the contexts are the
        // profiles some of them ran on.
        self.service.sessions.lock().unwrap().clear();
        self.service.contexts.lock().unwrap().clear();
    }
}

// ------------------------------------------------------------- the browser

/// The DevTools address of the first page of the browser on `port`. The
/// page is there a moment after the browser listens.
async fn page_socket_of(port: u16) -> String {
    for _ in 0..100 {
        let listed = devtools_get(port, "/json/list").await;
        let pages: Value = serde_json::from_str(&listed).expect("the list of pages");
        let page = pages
            .as_array()
            .and_then(|pages| pages.iter().find(|page| page["type"] == "page"))
            .and_then(|page| page["webSocketDebuggerUrl"].as_str());
        if let Some(page) = page {
            return page.to_string();
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("the stand-in browser opened no page");
}

/// The body of a GET to the HTTP side of a browser's DevTools port, which
/// keeps its connections open: read to the length it states.
async fn devtools_get(port: u16, path: &str) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let mut connection = TcpStream::connect((Ipv4Addr::LOCALHOST, port))
        .await
        .unwrap();
    let request = format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n");
    connection.write_all(request.as_bytes()).await.unwrap();
    let mut received = Vec::new();
    loop {
        let mut chunk = [0_u8; 4096];
        let read = connection.read(&mut chunk).await.unwrap();
        assert!(read > 0, "the browser closed its DevTools port mid-answer");
        received.extend_from_slice(&chunk[..read]);
        let text = String::from_utf8_lossy(&received);
        let Some((head, body)) = text.split_once("\r\n\r\n") else {
            continue;
        };
        let length: usize = head
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse().ok())?
            })
            .expect("a content length");
        if body.len() >= length {
            return body[..length].to_string();
        }
    }
}

/// A local Chromium that takes DevTools clients on a loopback port.
struct StandIn {
    child: tokio::process::Child,
    group: Option<u32>,
    port: u16,
    socket: String,
    /// `None` when the profile belongs to a context and outlives the browser.
    throwaway_profile: Option<TempDir>,
    /// The directory in the temp dir that holds the browser's singleton
    /// socket. A killed Chromium leaves it there.
    singleton_directory: Option<PathBuf>,
}

impl StandIn {
    async fn start(kept_profile: Option<&Path>) -> Self {
        let executable = chrome_path().expect("a Chromium for the stand-in");
        let throwaway_profile = match kept_profile {
            Some(_) => None,
            None => Some(
                tempfile::Builder::new()
                    .prefix("adscan-standin-")
                    .tempdir()
                    .unwrap(),
            ),
        };
        let profile = kept_profile
            .map(Path::to_path_buf)
            .or_else(|| {
                throwaway_profile
                    .as_ref()
                    .map(|dir| dir.path().to_path_buf())
            })
            .unwrap();
        // A browser killed on this profile left its lock behind.
        for name in ["SingletonLock", "SingletonSocket", "SingletonCookie"] {
            let _ = std::fs::remove_file(profile.join(name));
        }
        let mut command = tokio::process::Command::new(executable);
        command
            .arg("--headless=new")
            .arg("--remote-debugging-port=0")
            .arg("--remote-debugging-address=127.0.0.1")
            .arg(format!("--user-data-dir={}", profile.display()))
            .args([
                "--no-first-run",
                "--no-default-browser-check",
                "--disable-search-engine-choice-screen",
                "--disable-default-apps",
                "--disable-dev-shm-usage",
                // No traffic of the browser's own, as for a local scan.
                "--disable-background-networking",
                "--disable-client-side-phishing-detection",
                // Not `--gaia-url`, which a local scan has: Chromium gives
                // the sign-in site a process of its own, and with it every
                // frame of another site below a page on 127.0.0.1.
                "--google-base-url=http://127.0.0.1:9/",
                "--disable-extensions",
                "--disable-component-update",
                "--disable-sync",
                "--disable-background-timer-throttling",
                "--disable-backgrounding-occluded-windows",
                "--disable-renderer-backgrounding",
                "--password-store=basic",
                "--use-mock-keychain",
                "--mute-audio",
                "--blink-settings=primaryHoverType=2,availableHoverTypes=2,\
                 primaryPointerType=4,availablePointerTypes=4",
                // As Browserbase starts its browsers.
                "--disable-blink-features=AutomationControlled",
                // Browserbase has no out-of-process iframes.
                "--disable-site-isolation-trials",
                "--disable-features=IsolateOrigins,site-per-process,Translate,MediaRouter,\
                 OptimizationHints,DialMediaRouteProvider,NetworkTimeServiceQuerying",
                "--window-size=2560,1440",
                // Names that look public and lead to this machine.
                "--host-resolver-rules=MAP *.adscan-fixture.example 127.0.0.1",
            ]);
        if nix::unistd::Uid::effective().is_root() {
            command.arg("--no-sandbox");
        }
        command
            .arg("about:blank")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .process_group(0);
        let mut child = command.spawn().expect("the stand-in browser starts");
        let group = child.id();
        let mut lines = BufReader::new(child.stderr.take().unwrap()).lines();
        let listening = async {
            while let Ok(Some(line)) = lines.next_line().await {
                if let Some(socket) = line.strip_prefix("DevTools listening on ") {
                    return socket.trim().to_string();
                }
            }
            panic!("the stand-in browser exited before it listened");
        };
        let socket = tokio::time::timeout(BROWSER_START_PATIENCE, listening)
            .await
            .expect("the stand-in browser listens");
        // Chromium blocks when nobody reads what it prints.
        tokio::spawn(async move { while let Ok(Some(_)) = lines.next_line().await {} });
        let port = socket
            .strip_prefix("ws://127.0.0.1:")
            .and_then(|rest| rest.split('/').next())
            .and_then(|port| port.parse().ok())
            .expect("a loopback DevTools address");
        let singleton_directory = std::fs::read_link(profile.join("SingletonSocket"))
            .ok()
            .and_then(|socket| socket.parent().map(Path::to_path_buf))
            .filter(|directory| directory.is_absolute());
        Self {
            child,
            group,
            port,
            socket,
            throwaway_profile,
            singleton_directory,
        }
    }
}

impl Drop for StandIn {
    fn drop(&mut self) {
        if let Some(group) = self.group {
            let group = nix::unistd::Pid::from_raw(group as i32);
            let _ = nix::sys::signal::killpg(group, nix::sys::signal::Signal::SIGKILL);
        }
        let _ = self.child.start_kill();
        if let Some(directory) = &self.singleton_directory {
            for name in ["SingletonSocket", "SingletonCookie"] {
                let _ = std::fs::remove_file(directory.join(name));
            }
            let _ = std::fs::remove_dir(directory);
        }
        if let Some(profile) = self.throwaway_profile.take() {
            // The processes may still be writing as they die.
            for _ in 0..50 {
                if std::fs::remove_dir_all(profile.path()).is_ok() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }
}

// --------------------------------------------------------------- the relay

async fn relay_connections(
    listener: TcpListener,
    service: Arc<Service>,
    shutdown: CancellationToken,
) {
    loop {
        let connection = tokio::select! {
            _ = shutdown.cancelled() => return,
            connection = listener.accept() => connection,
        };
        if let Ok((connection, _)) = connection {
            tokio::spawn(relay(connection, service.clone()));
        }
    }
}

fn refusal(status: StatusCode) -> ErrorResponse {
    let mut response = ErrorResponse::new(None);
    *response.status_mut() = status;
    response
}

/// What the gateway makes of an upgrade request: the session it is for.
/// The refusal is the type the WebSocket handshake takes.
#[allow(clippy::result_large_err)]
fn admitted(service: &Service, request: &Request) -> Result<String, ErrorResponse> {
    let uri = request.uri();
    let session_id = uri
        .path()
        .strip_prefix("/connect/")
        .ok_or_else(|| refusal(StatusCode::NOT_FOUND))?;
    let key = uri
        .query()
        .and_then(|query| query.strip_prefix("signingKey="))
        .unwrap_or_default();
    let mut sessions = service.sessions.lock().unwrap();
    let session = sessions
        .iter_mut()
        .find(|session| session.id == session_id)
        .ok_or_else(|| refusal(StatusCode::NOT_FOUND))?;
    if session.signing_key != key {
        return Err(refusal(StatusCode::FORBIDDEN));
    }
    if session.status != Status::Running {
        return Err(refusal(StatusCode::GONE));
    }
    session.connections += 1;
    Ok(session_id.to_string())
}

fn unlimited() -> Option<WebSocketConfig> {
    Some(
        WebSocketConfig::default()
            .max_message_size(None)
            .max_frame_size(None),
    )
}

#[allow(clippy::result_large_err)]
async fn relay(connection: TcpStream, service: Arc<Service>) {
    let mut session_id = None;
    let admitting = |request: &Request, response| {
        session_id = Some(admitted(&service, request)?);
        Ok(response)
    };
    let Ok(mut capture) =
        tokio_tungstenite::accept_hdr_async_with_config(connection, admitting, unlimited()).await
    else {
        return;
    };
    let session_id = session_id.expect("an admitted session");
    let (browser_socket, commands, cut) = {
        let sessions = service.sessions.lock().unwrap();
        let session = sessions
            .iter()
            .find(|session| session.id == session_id)
            .unwrap();
        (
            session
                .browser
                .as_ref()
                .map(|browser| browser.socket.clone()),
            session.commands.clone(),
            session.cut.clone(),
        )
    };
    let Some(browser_socket) = browser_socket else {
        return;
    };
    let port = browser_socket
        .strip_prefix("ws://")
        .and_then(|rest| rest.split('/').next())
        .unwrap()
        .to_string();
    let to_browser = TcpStream::connect(&port).await.unwrap();
    let (mut browser, _) = tokio_tungstenite::client_async_with_config(
        browser_socket.as_str(),
        to_browser,
        unlimited(),
    )
    .await
    .expect("the stand-in browser takes the gateway's connection");
    loop {
        tokio::select! {
            biased;
            // Cut: both ends are dropped as they are.
            _ = cut.cancelled() => break,
            sent = capture.next() => match sent {
                Some(Ok(Message::Text(text))) => {
                    if let Ok(command) = serde_json::from_str::<Value>(text.as_str()) {
                        commands.lock().unwrap().push(Command {
                            id: command["id"].as_u64().unwrap_or_default(),
                            method: command["method"].as_str().unwrap_or_default().to_string(),
                            params: command["params"].clone(),
                            in_session: command["sessionId"].is_string(),
                            reply_bytes: None,
                        });
                    }
                    if browser.send(Message::Text(text)).await.is_err() {
                        break;
                    }
                }
                Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
                _ => break,
            },
            answered = browser.next() => match answered {
                Some(Ok(message @ (Message::Text(_) | Message::Binary(_)))) => {
                    if let Some(id) = answered_id(&message) {
                        let mut commands = commands.lock().unwrap();
                        if let Some(command) = commands.iter_mut().rev().find(|command| command.id == id) {
                            command.reply_bytes = Some(message.len());
                        }
                    }
                    if capture.send(message).await.is_err() {
                        break;
                    }
                }
                Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
                _ => break,
            },
        }
    }
    // Without `keepAlive` a session ends with its connection.
    end(&service, &session_id).await;
}

/// The id of the command a message of the browser answers. Chromium
/// writes the id first.
fn answered_id(message: &Message) -> Option<u64> {
    let text = message.to_text().ok()?;
    let digits: String = text
        .strip_prefix("{\"id\":")?
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse().ok()
}

/// How long a browser that was asked to close is given to write its
/// profile, which a context keeps, before it is killed.
const EXIT_PATIENCE: Duration = Duration::from_secs(5);

async fn end(service: &Service, session_id: &str) {
    let browser = {
        let mut sessions = service.sessions.lock().unwrap();
        let Some(session) = sessions.iter_mut().find(|session| session.id == session_id) else {
            return;
        };
        if session.status == Status::Running {
            session.status = Status::Completed;
            session.ended_at = Some(epoch_seconds());
        }
        session.own_client = None;
        session.browser.take()
    };
    if let Some(mut browser) = browser {
        let _ = tokio::time::timeout(EXIT_PATIENCE, browser.child.wait()).await;
    }
}

// ----------------------------------------------------------------- the API

fn epoch_seconds() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
}

/// An instant the way the service writes it: `2026-11-13T08:26:07.123Z`.
fn timestamp(epoch_seconds: f64) -> String {
    let millis = (epoch_seconds * 1000.0).round() as i64;
    let (days, of_day) = (millis.div_euclid(86_400_000), millis.rem_euclid(86_400_000));
    // Days since the epoch to a civil date (Howard Hinnant's algorithm).
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z",
        of_day / 3_600_000,
        of_day / 60_000 % 60,
        of_day / 1_000 % 60,
        of_day % 1_000
    )
}

fn failure(status: StatusCode, message: &str) -> Response {
    let body = json!({
        "statusCode": status.as_u16(),
        "error": status.canonical_reason().unwrap_or_default(),
        "message": message,
    });
    reply(status, &body)
}

fn validation_failure(message: &str) -> Response {
    let body = json!({
        "statusCode": 400,
        "code": "FST_ERR_VALIDATION",
        "error": "Bad Request",
        "message": message,
    });
    reply(StatusCode::BAD_REQUEST, &body)
}

fn reply(status: StatusCode, body: &Value) -> Response {
    (
        status,
        [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
        body.to_string(),
    )
        .into_response()
}

fn project() -> Value {
    json!({
        "id": PROJECT_ID,
        "createdAt": "2026-10-01T10:00:00.000Z",
        "updatedAt": "2026-10-01T10:04:00.000Z",
        "name": "Local",
        "ownerId": "owner_1",
        "defaultTimeout": 1800,
        "concurrency": 3,
    })
}

impl Service {
    fn session_json(&self, session: &mut Session) -> Value {
        let mut shown = json!({
            "id": session.id,
            "createdAt": timestamp(session.created_at),
            "updatedAt": timestamp(session.ended_at.unwrap_or(session.created_at)),
            "projectId": PROJECT_ID,
            "startedAt": timestamp(session.created_at),
            "expiresAt": timestamp(
                session.created_at + session.body["timeout"].as_f64().unwrap_or(1_800.0)
            ),
            "status": if session.status == Status::Running { "RUNNING" } else { "COMPLETED" },
            "proxyBytes": 0,
            "keepAlive": session.body["keepAlive"].as_bool().unwrap_or(false),
            "region": session.body["region"].as_str().unwrap_or("us-west-2"),
            "userMetadata": session.body.get("userMetadata").cloned().unwrap_or(json!({})),
        });
        if let Some(context) = session.body["browserSettings"]["context"]["id"].as_str() {
            shown["contextId"] = json!(context);
        }
        match session.ended_at {
            Some(ended_at) => {
                shown["endedAt"] = json!(timestamp(ended_at));
                session.reads_after_end += 1;
                if session.reads_after_end > self.proxy_bytes_lag_reads.load(Ordering::Relaxed) {
                    shown["proxyBytes"] = json!(self.proxy_bytes.load(Ordering::Relaxed));
                }
            }
            None => {
                shown["connectUrl"] = json!(format!(
                    "ws://127.0.0.1:{}/connect/{}?signingKey={}",
                    self.relay_port, session.id, session.signing_key
                ));
                shown["seleniumRemoteUrl"] = json!(format!(
                    "http://127.0.0.1:{}/webdriver/session",
                    self.relay_port
                ));
                shown["signingKey"] = json!(session.signing_key);
            }
        }
        shown
    }

    /// The reason a request for a session is refused, if it is.
    fn refuses(&self, body: &Value) -> Option<Response> {
        const KNOWN: [&str; 8] = [
            "projectId",
            "region",
            "timeout",
            "keepAlive",
            "browserSettings",
            "proxies",
            "userMetadata",
            "extensionId",
        ];
        let fields = body.as_object()?;
        if let Some(unknown) = fields.keys().find(|key| !KNOWN.contains(&key.as_str())) {
            return Some(failure(
                StatusCode::BAD_REQUEST,
                &format!("Validation error: Unrecognized key(s) in object: '{unknown}'"),
            ));
        }
        match body["projectId"].as_str() {
            Some(PROJECT_ID) => {}
            Some(id) if id.len() == 36 => {
                return Some(failure(StatusCode::UNAUTHORIZED, "Unauthorized Project ID"));
            }
            _ => return Some(failure(StatusCode::BAD_REQUEST, "Invalid Project ID")),
        }
        const REGIONS: [&str; 4] = ["us-west-2", "us-east-1", "eu-central-1", "ap-southeast-1"];
        if let Some(region) = body.get("region")
            && !region
                .as_str()
                .is_some_and(|region| REGIONS.contains(&region))
        {
            return Some(validation_failure(
                "body/region must be equal to one of the allowed values",
            ));
        }
        if let Some(timeout) = body.get("timeout") {
            match timeout.as_u64() {
                Some(seconds) if seconds < 60 => {
                    return Some(validation_failure("body/timeout must be >= 60"));
                }
                Some(seconds) if seconds > 21_600 => {
                    return Some(validation_failure("body/timeout must be <= 21600"));
                }
                Some(_) => {}
                None => return Some(validation_failure("body/timeout must be integer")),
            }
        }
        let settings = &body["browserSettings"];
        if settings.get("verified").is_some() || settings.get("advancedStealth").is_some() {
            return Some(failure(
                StatusCode::FORBIDDEN,
                "Verified mode is only available on the Enterprise plan",
            ));
        }
        if settings.get("os").is_some() {
            return Some(failure(
                StatusCode::BAD_REQUEST,
                "mac OS is only available for verified users, which is only available on the \
                 Enterprise plan. By default, we only support Linux.",
            ));
        }
        if let Some(context) = settings["context"]["id"].as_str()
            && !self.contexts.lock().unwrap().contains_key(context)
        {
            return Some(failure(StatusCode::BAD_REQUEST, "Context ID not found"));
        }
        for (index, proxy) in body["proxies"].as_array().into_iter().flatten().enumerate() {
            match proxy["type"].as_str() {
                Some("browserbase") => {
                    let country = proxy["geolocation"]["country"].as_str().unwrap_or("US");
                    if country == "XX" {
                        return Some(failure(
                            StatusCode::BAD_REQUEST,
                            &format!(
                                "Validation error: Invalid country code at \
                                 \"proxies[{index}].geolocation.country\""
                            ),
                        ));
                    }
                }
                Some("external") => {
                    let server = proxy["server"].as_str().unwrap_or_default();
                    if !(server.starts_with("http://") || server.starts_with("https://")) {
                        return Some(failure(
                            StatusCode::BAD_REQUEST,
                            &format!(
                                "Validation error: Invalid url at \"proxies[{index}].server\""
                            ),
                        ));
                    }
                }
                _ => {
                    return Some(validation_failure(&format!(
                        "body/proxies/{index}/type must be equal to constant, \
                         body/proxies/{index} must have required property 'server'"
                    )));
                }
            }
        }
        if let Some(metadata) = body.get("userMetadata") {
            if let Some(not_text) = metadata
                .as_object()
                .into_iter()
                .flatten()
                .map(|(_, value)| value)
                .find(|value| !value.is_string())
            {
                return Some(failure(
                    StatusCode::BAD_REQUEST,
                    &format!("Value is not a string: {not_text}"),
                ));
            }
            let size = metadata.to_string().len();
            if size > 512 {
                return Some(failure(
                    StatusCode::BAD_REQUEST,
                    &format!(
                        "User metadata must be at most 512 characters, current size is {size} \
                         characters"
                    ),
                ));
            }
        }
        None
    }

    /// Whether the project is at its limit, counting the other
    /// application's sessions. A refusal frees one of those.
    fn is_full(&self) -> Option<Response> {
        let limit = self.concurrency.load(Ordering::Relaxed);
        let own = self
            .sessions
            .lock()
            .unwrap()
            .iter()
            .filter(|session| session.status == Status::Running)
            .count() as u32;
        let others = self.slots_held_by_others.load(Ordering::Relaxed);
        if limit > 0 && own + others.min(limit) < limit {
            return None;
        }
        if others > 0 {
            self.slots_held_by_others
                .store(others - 1, Ordering::Relaxed);
        }
        let limit = limit.max(3);
        let mut refused = failure(
            StatusCode::TOO_MANY_REQUESTS,
            &format!(
                "You've exceeded your max concurrent sessions limit (limit {limit}, currently \
                 {limit}). Please contact support to increase your limit."
            ),
        );
        // No `retry-after`: only these.
        let headers = refused.headers_mut();
        headers.insert("ratelimit-limit", "50".parse().unwrap());
        headers.insert("ratelimit-remaining", "49".parse().unwrap());
        headers.insert("ratelimit-reset", "60".parse().unwrap());
        Some(refused)
    }

    async fn create_session(&self, body: Value) -> Response {
        if let Some(refused) = self.refuses(&body).or_else(|| self.is_full()) {
            return refused;
        }
        let number = self.next_id.fetch_add(1, Ordering::Relaxed);
        let kept_profile = body["browserSettings"]["context"]["id"]
            .as_str()
            .and_then(|id| {
                let contexts = self.contexts.lock().unwrap();
                contexts.get(id).map(|profile| profile.path().to_path_buf())
            });
        let browser = StandIn::start(kept_profile.as_deref()).await;
        let own_client =
            PageClient::attach_as(&page_socket_of(browser.port).await, Role::TheService).await;
        let mut session = Session {
            id: format!("5e55100{number:01}-0000-4000-8000-{number:012}"),
            body,
            status: Status::Running,
            created_at: epoch_seconds(),
            ended_at: None,
            signing_key: format!("{SIGNING_KEY_PREFIX}{number}"),
            live_view_token: format!("{LIVE_VIEW_TOKEN_PREFIX}{number}"),
            browser: Some(browser),
            own_client: Some(own_client),
            commands: Arc::default(),
            release_requests: 0,
            reads_after_end: 0,
            cut: CancellationToken::new(),
            connections: 0,
        };
        let created = self.session_json(&mut session);
        self.sessions.lock().unwrap().push(session);
        reply(StatusCode::CREATED, &created)
    }

    fn with_session(&self, id: &str, then: impl FnOnce(&mut Session) -> Response) -> Response {
        if id.len() != 36 {
            return failure(StatusCode::BAD_REQUEST, "Invalid Session ID");
        }
        let mut sessions = self.sessions.lock().unwrap();
        match sessions.iter_mut().find(|session| session.id == id) {
            Some(session) => then(session),
            None => failure(StatusCode::NOT_FOUND, "Not Found"),
        }
    }

    fn update_session(&self, id: &str, body: &Value) -> Response {
        if body["status"] != "REQUEST_RELEASE" {
            return validation_failure("body/status must be equal to one of the allowed values");
        }
        let failing = self.failing_releases.load(Ordering::Relaxed);
        self.with_session(id, |session| {
            session.release_requests += 1;
            if failing > 0 {
                self.failing_releases.store(failing - 1, Ordering::Relaxed);
                return failure(StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error");
            }
            if session.status == Status::Running {
                session.status = Status::Completed;
                session.ended_at = Some(epoch_seconds());
                session.cut.cancel();
                session.own_client = None;
                session.browser = None;
            }
            reply(StatusCode::OK, &self.session_json(session))
        })
    }

    fn list_sessions(&self, query: &str) -> Response {
        let wanted_status = query_value(query, "status");
        // `user_metadata['install']:'<id>'`
        let wanted_metadata = query_value(query, "q").and_then(|q| {
            let (key, value) = q.strip_prefix("user_metadata['")?.split_once("']:'")?;
            Some((key.to_string(), value.strip_suffix('\'')?.to_string()))
        });
        let mut sessions = self.sessions.lock().unwrap();
        let listed: Vec<Value> = sessions
            .iter_mut()
            .filter(|session| match wanted_status.as_deref() {
                Some("RUNNING") => session.status == Status::Running,
                Some("COMPLETED") => session.status == Status::Completed,
                _ => true,
            })
            .filter(|session| {
                wanted_metadata.as_ref().is_none_or(|(key, value)| {
                    session.body["userMetadata"][key.as_str()] == value.as_str()
                })
            })
            .map(|session| {
                let mut listed = self.session_json(session);
                for credential in ["connectUrl", "seleniumRemoteUrl", "signingKey"] {
                    listed.as_object_mut().unwrap().remove(credential);
                }
                listed
            })
            .collect();
        reply(StatusCode::OK, &json!(listed))
    }

    fn live_view(&self, id: &str) -> Response {
        self.with_session(id, |session| {
            if session.status != Status::Running {
                return failure(StatusCode::GONE, "Session stopped");
            }
            let token = &session.live_view_token;
            let socket = format!(
                "connect.usw2.browserbase.com/debug/{}/devtools/page/PAGE1?debug=true&token={token}",
                session.id
            );
            let inspector = "https://www.browserbase.com/devtools/inspector.html";
            let fullscreen = "https://www.browserbase.com/devtools-fullscreen/inspector.html";
            reply(
                StatusCode::OK,
                &json!({
                    "debuggerFullscreenUrl": format!("{fullscreen}?wss={socket}"),
                    "debuggerUrl": format!("{inspector}?wss={socket}"),
                    "wsUrl": format!("wss://{socket}"),
                    "pages": [{
                        "id": "PAGE1",
                        "url": "about:blank",
                        "title": "",
                        "faviconUrl": "",
                        "debuggerUrl": format!("{inspector}?wss={socket}"),
                        "debuggerFullscreenUrl": format!("{fullscreen}?wss={socket}"),
                    }],
                }),
            )
        })
    }

    fn contexts(&self, method: &Method, id: Option<&str>, body: &Value) -> Response {
        let mut contexts = self.contexts.lock().unwrap();
        match (method.as_str(), id) {
            ("POST", None) => {
                if body["projectId"] != PROJECT_ID {
                    return failure(StatusCode::UNAUTHORIZED, "Unauthorized Project ID");
                }
                let number = self.next_id.fetch_add(1, Ordering::Relaxed);
                let id = format!("c0c0c0c{number:01}-0000-4000-8000-{number:012}");
                let profile = tempfile::Builder::new()
                    .prefix("adscan-standin-context-")
                    .tempdir()
                    .unwrap();
                contexts.insert(id.clone(), profile);
                reply(
                    StatusCode::CREATED,
                    &json!({
                        "id": id,
                        "uploadUrl": "https://uploads.invalid/context?X-Amz-Signature=abc",
                        "publicKey": "-----BEGIN PUBLIC KEY-----\nMIIB\n-----END PUBLIC KEY-----\n",
                        "cipherAlgorithm": "AES-256-CBC",
                        "initializationVectorSize": 16,
                    }),
                )
            }
            ("GET", Some(id)) if contexts.contains_key(id) => reply(
                StatusCode::OK,
                &json!({
                    "id": id,
                    "createdAt": "2026-11-13T08:00:00.000Z",
                    "updatedAt": "2026-11-13T08:00:00.000Z",
                    "projectId": PROJECT_ID,
                }),
            ),
            ("DELETE", Some(id)) if contexts.remove(id).is_some() => {
                StatusCode::NO_CONTENT.into_response()
            }
            _ => failure(StatusCode::NOT_FOUND, "Not Found"),
        }
    }
}

/// The percent-decoded value of `name` in a query string.
fn query_value(query: &str, name: &str) -> Option<String> {
    url::form_urlencoded::parse(query.as_bytes())
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.into_owned())
}

async fn answer(
    State(service): State<Arc<Service>>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let key = headers
        .get("x-bb-api-key")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    service.asked.lock().unwrap().push(Asked {
        method: method.to_string(),
        path_and_query: uri
            .path_and_query()
            .map_or_else(|| uri.path().to_string(), |asked| asked.to_string()),
        key: key.clone(),
    });
    if key.as_deref() != Some(API_KEY) {
        return failure(StatusCode::UNAUTHORIZED, "Unauthorized");
    }
    let body = if body.is_empty() {
        Value::Null
    } else {
        match serde_json::from_slice(&body) {
            Ok(body) => body,
            Err(_) => {
                return reply(
                    StatusCode::BAD_REQUEST,
                    &json!({
                        "statusCode": 400,
                        "code": "FST_ERR_CTP_INVALID_JSON_BODY",
                        "error": "Bad Request",
                        "message": "Body is not valid JSON",
                    }),
                );
            }
        }
    };
    let parts: Vec<&str> = uri.path().trim_matches('/').split('/').collect();
    let query = uri.query().unwrap_or_default();
    match (method.as_str(), parts.as_slice()) {
        ("POST", ["v1", "sessions"]) => service.create_session(body).await,
        ("GET", ["v1", "sessions"]) => service.list_sessions(query),
        ("GET", ["v1", "sessions", id]) => service.with_session(id, |session| {
            reply(StatusCode::OK, &service.session_json(session))
        }),
        ("POST", ["v1", "sessions", id]) => service.update_session(id, &body),
        ("GET", ["v1", "sessions", id, "debug"]) => service.live_view(id),
        (_, ["v1", "contexts"]) => service.contexts(&method, None, &body),
        (_, ["v1", "contexts", id]) => service.contexts(&method, Some(id), &body),
        ("GET", ["v1", "projects"]) => reply(StatusCode::OK, &json!([project()])),
        ("GET", ["v1", "projects", PROJECT_ID]) => reply(StatusCode::OK, &project()),
        ("GET", ["v1", "projects", PROJECT_ID, "usage"]) => reply(
            StatusCode::OK,
            &json!({ "browserMinutes": 483, "proxyBytes": 1_998_349_077_u64 }),
        ),
        ("GET", ["v1", "projects", id, ..]) if id.len() != 36 => {
            failure(StatusCode::BAD_REQUEST, "Invalid Project ID")
        }
        ("GET", ["v1", "projects", ..]) => failure(StatusCode::NOT_FOUND, "Project not found"),
        _ => failure(StatusCode::NOT_FOUND, "Not Found"),
    }
}

// ------------------------------------------------------- a second client

/// A DevTools client on one page of a stand-in browser, next to the
/// capture's: what Browserbase's own client and its live view are.
pub struct PageClient {
    commands: mpsc::UnboundedSender<(Value, oneshot::Sender<Value>)>,
    next_id: AtomicU64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    /// Does what it is told and nothing else.
    Person,
    /// Runs the service's own scripts in the page's world of every
    /// document the tab shows, as Browserbase does.
    TheService,
}

/// What Browserbase runs in the page's own world: its heartbeat, and a
/// script that starts the way its own do.
fn the_services_scripts() -> [String; 2] {
    let heartbeat = "(() => {\n\
         const info = console.info.bind(console);\n\
         setInterval(() => info('Browserbase keeping connection alive'), 1500);\n\
         //# sourceURL=./dist/scripts/keepalive.js\n    })();"
        .to_string();
    let of_the_service = format!(
        "(() => {{\n      /* browserbase:stealth-init-script */\n\"use strict\";\n\n\
         // src/captchas/solve-markers.ts\n\
         var SOLVE_STARTED = \"browserbase-solving-started\";\n\
         var SOLVE_FINISHED = \"browserbase-solving-finished\";\n{}\n    }})();",
        "var standIn = 'for a script of the service';\n".repeat(8)
    );
    [heartbeat, of_the_service]
}

/// Command ids of what a client does by itself, apart from what it is
/// told to do.
const OWN_COMMAND_IDS_FROM: u64 = 1_000_000_000;

impl PageClient {
    pub async fn attach(page_socket: &str) -> Self {
        Self::attach_as(page_socket, Role::Person).await
    }

    async fn attach_as(page_socket: &str, role: Role) -> Self {
        let authority = page_socket
            .strip_prefix("ws://")
            .and_then(|rest| rest.split('/').next())
            .expect("a loopback page address");
        let connection = TcpStream::connect(authority).await.unwrap();
        let (mut socket, _) =
            tokio_tungstenite::client_async_with_config(page_socket, connection, unlimited())
                .await
                .expect("the page takes a second client");
        let (commands, mut queue) = mpsc::unbounded_channel::<(Value, oneshot::Sender<Value>)>();
        // The main frame of a page carries the id of its target, which is
        // the last part of the page's address.
        let main_frame = page_socket
            .rsplit('/')
            .next()
            .unwrap_or_default()
            .to_string();
        tokio::spawn(async move {
            let mut waiting: HashMap<u64, oneshot::Sender<Value>> = HashMap::new();
            let mut own_id = OWN_COMMAND_IDS_FROM;
            if role == Role::TheService {
                let watch = json!({ "id": own_id, "method": "Runtime.enable", "params": {} });
                if socket.send(Message::text(watch.to_string())).await.is_err() {
                    return;
                }
            }
            loop {
                tokio::select! {
                    command = queue.recv() => {
                        let Some((command, reply)) = command else { break };
                        waiting.insert(command["id"].as_u64().unwrap(), reply);
                        if socket.send(Message::text(command.to_string())).await.is_err() {
                            break;
                        }
                    }
                    message = socket.next() => {
                        let Some(Ok(Message::Text(text))) = message else { break };
                        let message: Value = serde_json::from_str(text.as_str()).unwrap();
                        if let Some(reply) = message["id"].as_u64().and_then(|id| waiting.remove(&id)) {
                            let _ = reply.send(message);
                            continue;
                        }
                        // The page's own world of a document the tab shows.
                        let context = &message["params"]["context"];
                        let is_a_new_document = message["method"] == "Runtime.executionContextCreated"
                            && context["auxData"]["isDefault"] == true
                            && context["auxData"]["frameId"] == main_frame.as_str();
                        if role == Role::TheService && is_a_new_document {
                            for script in the_services_scripts() {
                                own_id += 1;
                                let run = json!({ "id": own_id, "method": "Runtime.evaluate",
                                    "params": { "expression": script, "contextId": context["id"] } });
                                if socket.send(Message::text(run.to_string())).await.is_err() {
                                    return;
                                }
                            }
                        }
                    }
                }
            }
        });
        Self {
            commands,
            next_id: AtomicU64::new(1),
        }
    }

    pub async fn call(&self, method: &str, params: Value) -> Value {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (reply, answer) = oneshot::channel();
        self.commands
            .send((
                json!({ "id": id, "method": method, "params": params }),
                reply,
            ))
            .expect("the second client is connected");
        let mut answer = answer.await.expect("the page answers the second client");
        answer["result"].take()
    }

    /// Sends a command the page will not live to answer.
    pub fn send(&self, method: &str, params: Value) {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (reply, _never_answered) = oneshot::channel();
        self.commands
            .send((
                json!({ "id": id, "method": method, "params": params }),
                reply,
            ))
            .expect("the second client is connected");
    }

    /// Runs `script` in the page's own world.
    pub async fn evaluate(&self, script: &str) -> Value {
        let mut result = self
            .call(
                "Runtime.evaluate",
                json!({ "expression": script, "returnByValue": true, "awaitPromise": true }),
            )
            .await;
        result["result"]["value"].take()
    }

    /// The lines Browserbase writes to the console of a page it hosts, as
    /// they were measured.
    pub async fn log_like_the_service(&self) {
        self.evaluate(
            "console.info('Browserbase keeping connection alive')\n\
             //# sourceURL=./dist/scripts/keepalive.js",
        )
        .await;
        self.evaluate(
            "console.log('browserbase-solving-started'); \
             console.log(JSON.stringify({ key: 'browserbase-captcha-event', value: 'solved' })); \
             console.log('browserbase-solving-finished')",
        )
        .await;
    }
}
