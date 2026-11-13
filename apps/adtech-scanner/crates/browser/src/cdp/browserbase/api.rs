//! Browserbase's REST API, as far as the scanner uses it, and the one
//! WebSocket to a session's browser. Every call authenticates with the
//! header `X-BB-API-Key`. Nothing here logs, and no failure carries an
//! address, a key or a body of the service.

use std::net::IpAddr;
use std::time::Duration;

use adscan_model::BrowserbaseCredentials;
use reqwest::header::HeaderValue;
use reqwest::{Method, StatusCode, Url};
use serde_json::{Value, json};

use super::failure::{self, ApiFailure, BROWSER_ENDED, KEY_NOT_ACCEPTED, UNEXPECTED_ANSWER};
use super::order::{APP_TAG, SessionOrder};
use super::time::epoch_ms;
use crate::cdp::websocket::{self, ConnectError, Keepalive, WebSocketSink, WebSocketSource};

const API_KEY_HEADER: &str = "X-BB-API-Key";
const CALL_TIMEOUT: Duration = Duration::from_secs(30);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// A session takes its client within this time (measured: 0.3 to 1.7 s).
const SOCKET_PATIENCE: Duration = Duration::from_secs(30);
const SERVICE_DOMAIN: &str = "browserbase.com";

pub(crate) const ADDRESS_NOT_ACCEPTED: &str =
    "Browserbase answered with an address the scanner does not connect to.";

/// The address of a session's browser, when the service gave one that
/// reads as an address. A credential: it has no `Debug` and no `Display`,
/// and never leaves this module but to be connected to.
#[derive(Clone)]
pub(crate) struct BrowserAddress(Option<Url>);

/// A session as the service describes it, without its credentials.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SessionState {
    pub id: String,
    /// The browser is, or is about to be, running.
    pub running: bool,
    pub started_at_ms: Option<f64>,
    pub ended_at_ms: Option<f64>,
    pub proxy_bytes: Option<f64>,
    /// `userMetadata.app` and `userMetadata.install`.
    pub app: Option<String>,
    pub install: Option<String>,
}

impl SessionState {
    fn of(session: &Value) -> Option<Self> {
        let text = |value: &Value| value.as_str().map(str::to_string);
        let instant = |name: &str| session[name].as_str().and_then(epoch_ms);
        Some(Self {
            id: text(&session["id"])?,
            running: matches!(session["status"].as_str(), Some("RUNNING" | "PENDING")),
            started_at_ms: instant("startedAt"),
            ended_at_ms: instant("endedAt"),
            proxy_bytes: session["proxyBytes"].as_f64(),
            app: text(&session["userMetadata"]["app"]),
            install: text(&session["userMetadata"]["install"]),
        })
    }

    /// How long the browser ran, once it has ended.
    pub(crate) fn seconds(&self) -> Option<f64> {
        let (started, ended) = (self.started_at_ms?, self.ended_at_ms?);
        Some(((ended - started) / 1_000.0).max(0.0))
    }

    /// Whether this app, in this installation, started the session.
    pub(crate) fn is_of(&self, install_id: &str) -> bool {
        self.app.as_deref() == Some(APP_TAG) && self.install.as_deref() == Some(install_id)
    }
}

/// One project of a key.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Project {
    pub id: String,
    pub name: String,
    pub concurrency: u32,
}

impl Project {
    fn of(project: &Value) -> Option<Self> {
        Some(Self {
            id: project["id"].as_str()?.to_string(),
            name: project["name"].as_str().unwrap_or_default().to_string(),
            concurrency: project["concurrency"]
                .as_u64()
                .and_then(|concurrency| u32::try_from(concurrency).ok())
                .unwrap_or_default(),
        })
    }
}

fn is_loopback(url: &Url) -> bool {
    match url.host() {
        Some(url::Host::Domain(name)) => name == "localhost",
        Some(url::Host::Ipv4(address)) => IpAddr::V4(address).is_loopback(),
        Some(url::Host::Ipv6(address)) => IpAddr::V6(address).is_loopback(),
        None => false,
    }
}

fn is_at_the_service(url: &Url) -> bool {
    url.host_str()
        .is_some_and(|host| host == SERVICE_DOMAIN || host.ends_with(&format!(".{SERVICE_DOMAIN}")))
}

/// An identifier of the service, safe to put in a path.
fn is_an_id(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 128
        && text
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

#[derive(Clone)]
pub(crate) struct Api {
    http: reqwest::Client,
    base: Url,
    /// Used in place of the address every session comes with. For tests.
    browser_override: Option<Url>,
}

impl Api {
    /// `api_url` must be https, or http on this machine. `browser_override`
    /// must be `wss://`, or `ws://` on this machine.
    pub(crate) fn new(api_url: &str, browser_override: Option<&str>) -> Result<Self, ApiFailure> {
        let misconfigured =
            |what: &str| ApiFailure::Failed(format!("Browserbase is misconfigured: {what}."));
        let base = Url::parse(api_url)
            .ok()
            .filter(|url| match url.scheme() {
                "https" => url.host().is_some(),
                "http" => is_loopback(url),
                _ => false,
            })
            .ok_or_else(|| misconfigured("its API address must be https"))?;
        let browser_override = browser_override
            .map(|address| {
                Url::parse(address)
                    .ok()
                    .filter(|url| match url.scheme() {
                        "wss" => url.host().is_some(),
                        "ws" => is_loopback(url),
                        _ => false,
                    })
                    .ok_or_else(|| misconfigured("the address of its browser must be wss"))
            })
            .transpose()?;
        // HTTP/1.1 only: the WebSocket to a browser starts as an upgrade
        // of such a request.
        let http = reqwest::Client::builder()
            .http1_only()
            .connect_timeout(CONNECT_TIMEOUT)
            .build()
            .map_err(|_| misconfigured("no HTTP client could be made"))?;
        Ok(Self {
            http,
            base,
            browser_override,
        })
    }

    /// Calls the API and returns the status and body of whatever it answers.
    async fn send(
        &self,
        api_key: &str,
        method: Method,
        path: &[&str],
        query: &[(&str, &str)],
        body: Option<Value>,
    ) -> Result<(StatusCode, String), ApiFailure> {
        if !path.iter().all(|segment| is_an_id(segment)) {
            return Err(ApiFailure::Failed(UNEXPECTED_ANSWER.into()));
        }
        let mut url = self.base.clone();
        url.path_segments_mut()
            .map_err(|()| ApiFailure::Failed(UNEXPECTED_ANSWER.into()))?
            .pop_if_empty()
            .push("v1")
            .extend(path);
        if !query.is_empty() {
            url.query_pairs_mut().extend_pairs(query);
        }
        let mut key = HeaderValue::from_str(api_key)
            .map_err(|_| ApiFailure::Auth(KEY_NOT_ACCEPTED.into()))?;
        key.set_sensitive(true);
        let mut request = self
            .http
            .request(method, url)
            .timeout(CALL_TIMEOUT)
            .header(API_KEY_HEADER, key);
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request.send().await.map_err(no_answer)?;
        let status = response.status();
        let text = response.text().await.map_err(no_answer)?;
        Ok((status, text))
    }

    /// Calls the API for an answer in JSON. `region` only words a failure.
    async fn json(
        &self,
        api_key: &str,
        method: Method,
        path: &[&str],
        query: &[(&str, &str)],
        body: Option<Value>,
        region: Option<&str>,
    ) -> Result<Value, ApiFailure> {
        let (status, text) = self.send(api_key, method, path, query, body).await?;
        if !status.is_success() {
            return Err(failure::from_response(status.as_u16(), &text, region));
        }
        serde_json::from_str(&text).map_err(|_| ApiFailure::Failed(UNEXPECTED_ANSWER.into()))
    }

    /// `POST /v1/sessions`: starts a browser. The session is running when
    /// this returns.
    pub(crate) async fn create_session(
        &self,
        credentials: &BrowserbaseCredentials,
        order: &SessionOrder,
    ) -> Result<(SessionState, BrowserAddress), ApiFailure> {
        let created = self
            .json(
                &credentials.api_key,
                Method::POST,
                &["sessions"],
                &[],
                Some(order.body(&credentials.project_id)),
                Some(order.region.as_str()),
            )
            .await?;
        let state =
            SessionState::of(&created).ok_or(ApiFailure::Failed(UNEXPECTED_ANSWER.into()))?;
        Ok((state, self.browser_address(&created)))
    }

    /// The address to connect to for a session the service just described.
    /// One that is not acceptable is kept as it is and refused when it is
    /// connected to, so that the session it came with can still be ended.
    fn browser_address(&self, session: &Value) -> BrowserAddress {
        let given = session["connectUrl"]
            .as_str()
            .and_then(|address| Url::parse(address).ok());
        BrowserAddress(self.browser_override.clone().or(given))
    }

    /// The address, when it is one the scanner connects to: the service's
    /// own hosts over TLS, or this machine when the API is on this machine
    /// too (a stand-in for tests). The override was checked when it was
    /// given.
    fn accepted<'a>(&self, address: &'a BrowserAddress) -> Option<&'a Url> {
        let url = address.0.as_ref()?;
        let is_accepted = self.browser_override.as_ref() == Some(url)
            || (url.scheme() == "wss" && is_at_the_service(url))
            || (url.scheme() == "ws" && is_loopback(url) && is_loopback(&self.base));
        is_accepted.then_some(url)
    }

    /// Opens the one DevTools connection to a session's browser.
    pub(crate) async fn connect(
        &self,
        address: &BrowserAddress,
    ) -> Result<(WebSocketSource, WebSocketSink), ApiFailure> {
        let Some(address) = self.accepted(address) else {
            return Err(ApiFailure::Failed(ADDRESS_NOT_ACCEPTED.into()));
        };
        websocket::connect(
            &self.http,
            address,
            Keepalive::REMOTE_BROWSER,
            SOCKET_PATIENCE,
        )
        .await
        .map_err(|error| {
            ApiFailure::Failed(match error {
                ConnectError::Gone => {
                    "The Browserbase browser had already ended when the scanner connected to it."
                        .to_string()
                }
                error => {
                    format!("The scanner could not connect to the Browserbase browser: {error}")
                }
            })
        })
    }

    /// `GET /v1/sessions/{id}`
    pub(crate) async fn session(
        &self,
        credentials: &BrowserbaseCredentials,
        session_id: &str,
    ) -> Result<SessionState, ApiFailure> {
        let session = self
            .json(
                &credentials.api_key,
                Method::GET,
                &["sessions", session_id],
                &[],
                None,
                None,
            )
            .await?;
        SessionState::of(&session).ok_or(ApiFailure::Failed(UNEXPECTED_ANSWER.into()))
    }

    /// `POST /v1/sessions/{id}` with `REQUEST_RELEASE`: ends the browser.
    /// A session that has ended already answers the same way.
    pub(crate) async fn release(
        &self,
        credentials: &BrowserbaseCredentials,
        session_id: &str,
    ) -> Result<SessionState, ApiFailure> {
        let body = json!({ "projectId": credentials.project_id, "status": "REQUEST_RELEASE" });
        let session = self
            .json(
                &credentials.api_key,
                Method::POST,
                &["sessions", session_id],
                &[],
                Some(body),
                None,
            )
            .await?;
        SessionState::of(&session).ok_or(ApiFailure::Failed(UNEXPECTED_ANSWER.into()))
    }

    /// `GET /v1/sessions?status=RUNNING`, narrowed to the sessions whose
    /// metadata names `install_id` when one is given. The service does the
    /// narrowing; whoever acts on the answer checks each session again.
    pub(crate) async fn running_sessions(
        &self,
        credentials: &BrowserbaseCredentials,
        install_id: Option<&str>,
    ) -> Result<Vec<SessionState>, ApiFailure> {
        let narrowed;
        let mut query = vec![("status", "RUNNING")];
        if let Some(install_id) = install_id {
            if !is_an_id(install_id) {
                return Ok(Vec::new());
            }
            narrowed = format!("user_metadata['install']:'{install_id}'");
            query.push(("q", &narrowed));
        }
        let listed = self
            .json(
                &credentials.api_key,
                Method::GET,
                &["sessions"],
                &query,
                None,
                None,
            )
            .await?;
        let sessions = listed
            .as_array()
            .ok_or(ApiFailure::Failed(UNEXPECTED_ANSWER.into()))?;
        Ok(sessions.iter().filter_map(SessionState::of).collect())
    }

    /// `GET /v1/sessions/{id}/debug`: the address of the live view that
    /// fills the window. A credential.
    pub(crate) async fn live_view(
        &self,
        credentials: &BrowserbaseCredentials,
        session_id: &str,
    ) -> Result<String, ApiFailure> {
        let path = ["sessions", session_id, "debug"];
        let (status, text) = self
            .send(&credentials.api_key, Method::GET, &path, &[], None)
            .await?;
        match status {
            StatusCode::OK => {}
            StatusCode::GONE => return Err(ApiFailure::Failed(BROWSER_ENDED.into())),
            status => return Err(failure::from_response(status.as_u16(), &text, None)),
        }
        let views: Value = serde_json::from_str(&text).unwrap_or_default();
        views["debuggerFullscreenUrl"]
            .as_str()
            .filter(|address| {
                address.starts_with("https://")
                    || (address.starts_with("http://") && is_loopback(&self.base))
            })
            .map(str::to_string)
            .ok_or(ApiFailure::Failed(UNEXPECTED_ANSWER.into()))
    }

    /// `POST /v1/contexts`: a new, empty browser profile. Returns its id.
    pub(crate) async fn create_context(
        &self,
        credentials: &BrowserbaseCredentials,
    ) -> Result<String, ApiFailure> {
        let body = json!({ "projectId": credentials.project_id });
        let context = self
            .json(
                &credentials.api_key,
                Method::POST,
                &["contexts"],
                &[],
                Some(body),
                None,
            )
            .await?;
        context["id"]
            .as_str()
            .filter(|id| is_an_id(id))
            .map(str::to_string)
            .ok_or(ApiFailure::Failed(UNEXPECTED_ANSWER.into()))
    }

    /// `DELETE /v1/contexts/{id}`. A context that is gone already is fine.
    pub(crate) async fn delete_context(
        &self,
        credentials: &BrowserbaseCredentials,
        context_id: &str,
    ) -> Result<(), ApiFailure> {
        let path = ["contexts", context_id];
        let (status, text) = self
            .send(&credentials.api_key, Method::DELETE, &path, &[], None)
            .await?;
        if status.is_success() || status == StatusCode::NOT_FOUND {
            Ok(())
        } else {
            Err(failure::from_response(status.as_u16(), &text, None))
        }
    }

    /// `GET /v1/projects`: the call that costs nothing and proves a key.
    pub(crate) async fn projects(&self, api_key: &str) -> Result<Vec<Project>, ApiFailure> {
        let listed = self
            .json(api_key, Method::GET, &["projects"], &[], None, None)
            .await?;
        let projects = listed
            .as_array()
            .ok_or(ApiFailure::Failed(UNEXPECTED_ANSWER.into()))?;
        Ok(projects.iter().filter_map(Project::of).collect())
    }

    /// `GET /v1/projects/{id}`
    pub(crate) async fn project(
        &self,
        credentials: &BrowserbaseCredentials,
    ) -> Result<Project, ApiFailure> {
        let path = ["projects", credentials.project_id.as_str()];
        let project = self
            .json(&credentials.api_key, Method::GET, &path, &[], None, None)
            .await?;
        Project::of(&project).ok_or(ApiFailure::Failed(UNEXPECTED_ANSWER.into()))
    }

    /// `GET /v1/projects/{id}/usage`: `(browser minutes, proxy bytes)`,
    /// running totals of the whole project.
    pub(crate) async fn project_usage(
        &self,
        credentials: &BrowserbaseCredentials,
    ) -> Result<(f64, f64), ApiFailure> {
        let path = ["projects", credentials.project_id.as_str(), "usage"];
        let usage = self
            .json(&credentials.api_key, Method::GET, &path, &[], None, None)
            .await?;
        match (
            usage["browserMinutes"].as_f64(),
            usage["proxyBytes"].as_f64(),
        ) {
            (Some(minutes), Some(bytes)) => Ok((minutes, bytes)),
            _ => Err(ApiFailure::Failed(UNEXPECTED_ANSWER.into())),
        }
    }
}

/// A call that got no answer, in the HTTP client's words without the
/// address.
fn no_answer(error: reqwest::Error) -> ApiFailure {
    let reason = if error.is_timeout() {
        "it did not answer in time".to_string()
    } else {
        let error = error.without_url();
        let mut reason = error.to_string();
        let mut source = std::error::Error::source(&error);
        while let Some(cause) = source {
            reason = cause.to_string();
            source = cause.source();
        }
        reason
    };
    failure::unreachable(&reason)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn failed(message: &str) -> ApiFailure {
        ApiFailure::Failed(message.to_string())
    }

    #[test]
    fn the_api_is_reached_over_tls_or_on_this_machine_and_nowhere_else() {
        for accepted in [
            "https://api.browserbase.com",
            "https://api.browserbase.com/",
            "http://127.0.0.1:4010",
            "http://localhost:4010",
            "http://[::1]:4010",
        ] {
            assert!(Api::new(accepted, None).is_ok(), "{accepted}");
        }
        for refused in [
            "http://api.browserbase.com",
            "http://192.168.1.20:4010",
            "ftp://api.browserbase.com",
            "api.browserbase.com",
            "",
        ] {
            assert_eq!(
                Api::new(refused, None).err(),
                Some(failed(
                    "Browserbase is misconfigured: its API address must be https."
                )),
                "{refused}"
            );
        }
        assert!(Api::new("https://api.browserbase.com", Some("ws://127.0.0.1:9222/x")).is_ok());
        assert!(Api::new("https://api.browserbase.com", Some("wss://relay.example/x")).is_ok());
        for refused in [
            "ws://relay.example/x",
            "https://relay.example/x",
            "nonsense",
        ] {
            assert!(
                Api::new("https://api.browserbase.com", Some(refused)).is_err(),
                "{refused}"
            );
        }
    }

    fn address_of(api: &Api, connect_url: &str) -> BrowserAddress {
        api.browser_address(&json!({ "connectUrl": connect_url }))
    }

    #[test]
    fn only_the_services_own_browsers_are_connected_to() {
        let real = Api::new("https://api.browserbase.com", None).unwrap();
        for accepted in [
            "wss://connect.browserbase.com/?signingKey=k",
            "wss://connect.usw2.browserbase.com/?signingKey=k",
            "wss://browserbase.com/s",
        ] {
            assert!(
                real.accepted(&address_of(&real, accepted)).is_some(),
                "{accepted}"
            );
        }
        for refused in [
            "ws://connect.browserbase.com/?signingKey=k",
            "wss://connect.browserbase.com.evil.example/",
            "wss://notbrowserbase.com/",
            "ws://127.0.0.1:9222/devtools/browser/1",
            "https://connect.browserbase.com/",
            "not an address",
        ] {
            assert!(
                real.accepted(&address_of(&real, refused)).is_none(),
                "{refused}"
            );
        }
        // A session that came without an address at all.
        assert!(real.accepted(&real.browser_address(&json!({}))).is_none());
    }

    #[test]
    fn a_stand_in_on_this_machine_may_hand_out_browsers_on_this_machine() {
        let stand_in = Api::new("http://127.0.0.1:4010", None).unwrap();
        assert!(
            stand_in
                .accepted(&address_of(
                    &stand_in,
                    "ws://127.0.0.1:9222/devtools/browser/1"
                ))
                .is_some()
        );
        assert!(
            stand_in
                .accepted(&address_of(&stand_in, "ws://localhost:9222/x"))
                .is_some()
        );
        assert!(
            stand_in
                .accepted(&address_of(&stand_in, "ws://10.0.0.7:9222/x"))
                .is_none()
        );
        assert!(
            stand_in
                .accepted(&address_of(&stand_in, "ws://example.com/x"))
                .is_none()
        );
    }

    #[test]
    fn the_override_replaces_every_address_the_service_hands_out() {
        let overridden = Api::new(
            "https://api.browserbase.com",
            Some("ws://127.0.0.1:9222/devtools/browser/1"),
        )
        .unwrap();
        let address = address_of(&overridden, "wss://connect.browserbase.com/?signingKey=k");

        assert_eq!(
            overridden.accepted(&address).map(Url::as_str),
            Some("ws://127.0.0.1:9222/devtools/browser/1")
        );
    }

    #[tokio::test]
    async fn an_address_that_is_not_accepted_is_never_connected_to() {
        let real = Api::new("https://api.browserbase.com", None).unwrap();
        let elsewhere = address_of(&real, "wss://attacker.example/?signingKey=s3cret");

        let refused = real.connect(&elsewhere).await.err();

        assert_eq!(refused, Some(failed(ADDRESS_NOT_ACCEPTED)));
    }

    #[test]
    fn a_session_is_read_without_its_credentials() {
        let session = json!({
            "id": "5e551001-0000-4000-8000-000000000001",
            "status": "COMPLETED",
            "startedAt": "2026-11-13T08:26:07.000Z",
            "endedAt": "2026-11-13T08:26:48.500Z",
            "proxyBytes": 131_844,
            "connectUrl": "wss://connect.browserbase.com/?signingKey=s3cret",
            "signingKey": "s3cret",
            "userMetadata": { "app": "adscan", "install": "abc", "scan": "probe" },
        });

        let state = SessionState::of(&session).unwrap();

        assert!(!state.running);
        assert_eq!(state.seconds(), Some(41.5));
        assert_eq!(state.proxy_bytes, Some(131_844.0));
        assert!(state.is_of("abc"));
        assert!(!state.is_of("abd"));
        assert!(!format!("{state:?}").contains("s3cret"));

        let running = SessionState::of(&json!({ "id": "x", "status": "RUNNING" })).unwrap();
        assert!(running.running);
        assert_eq!(running.seconds(), None);
        assert!(!running.is_of(""));
        assert_eq!(SessionState::of(&json!({ "status": "RUNNING" })), None);
    }

    #[test]
    fn a_session_of_another_application_is_not_this_installations() {
        let of = |metadata: Value| {
            SessionState::of(&json!({ "id": "x", "status": "RUNNING", "userMetadata": metadata }))
                .unwrap()
        };
        assert!(of(json!({ "app": "adscan", "install": "abc" })).is_of("abc"));
        assert!(!of(json!({ "app": "other", "install": "abc" })).is_of("abc"));
        assert!(!of(json!({ "install": "abc" })).is_of("abc"));
        assert!(!of(json!({ "scanId": "1", "auditId": "2", "organizationId": "3" })).is_of("abc"));
        assert!(!of(json!({ "app": "adscan" })).is_of("abc"));
    }

    #[test]
    fn what_goes_into_a_path_is_an_identifier() {
        assert!(is_an_id("5e551001-0000-4000-8000-000000000001"));
        assert!(is_an_id("0123456789abcdef0123456789abcdef"));
        for not_an_id in ["", "a/b", "..", "a b", "a?b", "a'b", "a%2Fb"] {
            assert!(!is_an_id(not_an_id), "{not_an_id}");
        }
    }
}
