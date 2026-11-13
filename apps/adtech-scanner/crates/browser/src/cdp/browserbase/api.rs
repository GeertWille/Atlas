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
/// The request for a browser is given longer: it is answered in under a
/// second when all is well (measured: 0.2 to 0.7 s), and an answer that is
/// given up on may leave a browser running that nobody knows the id of.
const CREATE_TIMEOUT: Duration = Duration::from_secs(60);
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
    /// `userMetadata.app`, `userMetadata.install` and `userMetadata.scan`.
    pub app: Option<String>,
    pub install: Option<String>,
    pub scan: Option<String>,
}

/// Why no session came of a request for one.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CreateFailure {
    pub failure: ApiFailure,
    /// The service may have started a browser all the same: the request
    /// reached it, and no answer that names a session or refuses one came
    /// back.
    pub session_may_exist: bool,
}

/// A call that was not answered.
struct Unanswered {
    failure: ApiFailure,
    /// Whether the service may have acted on the request.
    request_may_have_arrived: bool,
}

impl Unanswered {
    /// The request was never made.
    fn unsent(failure: ApiFailure) -> Self {
        Self {
            failure,
            request_may_have_arrived: false,
        }
    }
}

impl From<Unanswered> for ApiFailure {
    fn from(unanswered: Unanswered) -> Self {
        unanswered.failure
    }
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
            scan: text(&session["userMetadata"]["scan"]),
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

    /// Whether this app, in this installation, started the session for
    /// that scan.
    pub(crate) fn is_of_scan(&self, install_id: &str, scan_id: &str) -> bool {
        self.is_of(install_id) && self.scan.as_deref() == Some(scan_id)
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
    create_timeout: Duration,
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
        // of such a request. No redirect is followed: the API key travels
        // in a header of its own, which would go along to wherever a
        // redirect points, and a redirected upgrade would connect to a
        // host that was never checked.
        let http = reqwest::Client::builder()
            .http1_only()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(CONNECT_TIMEOUT)
            .build()
            .map_err(|_| misconfigured("no HTTP client could be made"))?;
        Ok(Self {
            http,
            base,
            browser_override,
            create_timeout: CREATE_TIMEOUT,
        })
    }

    #[cfg(test)]
    pub(crate) fn giving_a_new_browser(mut self, patience: Duration) -> Self {
        self.create_timeout = patience;
        self
    }

    /// Calls the API and returns the status and body of whatever it
    /// answers within `patience`.
    async fn send(
        &self,
        api_key: &str,
        method: Method,
        path: &[&str],
        query: &[(&str, &str)],
        body: Option<Value>,
        patience: Duration,
    ) -> Result<(StatusCode, String), Unanswered> {
        let not_understood = || Unanswered::unsent(ApiFailure::Failed(UNEXPECTED_ANSWER.into()));
        if !path.iter().all(|segment| is_an_id(segment)) {
            return Err(not_understood());
        }
        let mut url = self.base.clone();
        url.path_segments_mut()
            .map_err(|()| not_understood())?
            .pop_if_empty()
            .push("v1")
            .extend(path);
        if !query.is_empty() {
            url.query_pairs_mut().extend_pairs(query);
        }
        let mut key = HeaderValue::from_str(api_key)
            .map_err(|_| Unanswered::unsent(ApiFailure::Auth(KEY_NOT_ACCEPTED.into())))?;
        key.set_sensitive(true);
        let mut request = self
            .http
            .request(method, url)
            .timeout(patience)
            .header(API_KEY_HEADER, key);
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request.send().await.map_err(|error| Unanswered {
            // A connection that was never made carried no request.
            request_may_have_arrived: !error.is_connect(),
            failure: no_answer(error),
        })?;
        let status = response.status();
        let text = response.text().await.map_err(|error| Unanswered {
            request_may_have_arrived: true,
            failure: no_answer(error),
        })?;
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
        let (status, text) = self
            .send(api_key, method, path, query, body, CALL_TIMEOUT)
            .await?;
        if !status.is_success() {
            return Err(failure::from_response(status.as_u16(), &text, region));
        }
        serde_json::from_str(&text).map_err(|_| ApiFailure::Failed(UNEXPECTED_ANSWER.into()))
    }

    /// `POST /v1/sessions`: starts a browser. The session is running when
    /// this returns. A failure says whether a browser may have been
    /// started all the same.
    pub(crate) async fn create_session(
        &self,
        credentials: &BrowserbaseCredentials,
        order: &SessionOrder,
    ) -> Result<(SessionState, BrowserAddress), CreateFailure> {
        let (status, text) = self
            .send(
                &credentials.api_key,
                Method::POST,
                &["sessions"],
                &[],
                Some(order.body(&credentials.project_id)),
                self.create_timeout,
            )
            .await
            .map_err(|unanswered| CreateFailure {
                session_may_exist: unanswered.request_may_have_arrived,
                failure: unanswered.failure,
            })?;
        if !status.is_success() {
            return Err(CreateFailure {
                failure: failure::from_response(
                    status.as_u16(),
                    &text,
                    Some(order.region.as_str()),
                ),
                // A refusal is the service's own word that there is no
                // session. A failure of the service, or of a gateway in
                // front of it, is not.
                session_may_exist: !status.is_client_error(),
            });
        }
        let created: Value = serde_json::from_str(&text).unwrap_or_default();
        match SessionState::of(&created) {
            Some(state) => Ok((state, self.browser_address(&created))),
            None => Err(CreateFailure {
                failure: ApiFailure::Failed(UNEXPECTED_ANSWER.into()),
                session_may_exist: true,
            }),
        }
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

    /// `GET /v1/sessions/{id}`, for a session that may not exist: `None`
    /// when the service knows no session of that id for this key.
    pub(crate) async fn find_session(
        &self,
        credentials: &BrowserbaseCredentials,
        session_id: &str,
    ) -> Result<Option<SessionState>, ApiFailure> {
        let path = ["sessions", session_id];
        let (status, text) = self
            .send(
                &credentials.api_key,
                Method::GET,
                &path,
                &[],
                None,
                CALL_TIMEOUT,
            )
            .await?;
        if status == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !status.is_success() {
            return Err(failure::from_response(status.as_u16(), &text, None));
        }
        serde_json::from_str::<Value>(&text)
            .ok()
            .as_ref()
            .and_then(SessionState::of)
            .map(Some)
            .ok_or(ApiFailure::Failed(UNEXPECTED_ANSWER.into()))
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
    /// metadata names `install_id`. The service does the narrowing;
    /// whoever acts on the answer checks each session again.
    pub(crate) async fn running_sessions(
        &self,
        credentials: &BrowserbaseCredentials,
        install_id: &str,
    ) -> Result<Vec<SessionState>, ApiFailure> {
        if !is_an_id(install_id) {
            return Ok(Vec::new());
        }
        let narrowed = format!("user_metadata['install']:'{install_id}'");
        let listed = self
            .json(
                &credentials.api_key,
                Method::GET,
                &["sessions"],
                &[("status", "RUNNING"), ("q", &narrowed)],
                None,
                None,
            )
            .await?;
        let sessions = listed
            .as_array()
            .ok_or(ApiFailure::Failed(UNEXPECTED_ANSWER.into()))?;
        Ok(sessions.iter().filter_map(SessionState::of).collect())
    }

    /// `GET /v1/sessions?status=RUNNING`: how many browsers run in the
    /// project, whoever started them. The sessions of other applications
    /// are counted and nothing else: not one field of them is read.
    pub(crate) async fn running_session_count(
        &self,
        credentials: &BrowserbaseCredentials,
    ) -> Result<u32, ApiFailure> {
        let (status, text) = self
            .send(
                &credentials.api_key,
                Method::GET,
                &["sessions"],
                &[("status", "RUNNING")],
                None,
                CALL_TIMEOUT,
            )
            .await?;
        if !status.is_success() {
            return Err(failure::from_response(status.as_u16(), &text, None));
        }
        serde_json::from_str::<Vec<serde::de::IgnoredAny>>(&text)
            .map(|running| running.len() as u32)
            .map_err(|_| ApiFailure::Failed(UNEXPECTED_ANSWER.into()))
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
            .send(
                &credentials.api_key,
                Method::GET,
                &path,
                &[],
                None,
                CALL_TIMEOUT,
            )
            .await?;
        match status {
            StatusCode::OK => {}
            StatusCode::GONE => return Err(ApiFailure::Failed(BROWSER_ENDED.into())),
            status => return Err(failure::from_response(status.as_u16(), &text, None)),
        }
        let views: Value = serde_json::from_str(&text).unwrap_or_default();
        views["debuggerFullscreenUrl"]
            .as_str()
            .filter(|address| self.is_a_live_view(address))
            .map(str::to_string)
            .ok_or(ApiFailure::Failed(UNEXPECTED_ANSWER.into()))
    }

    /// Whether `address` is one the user's browser is sent to: a page of
    /// the service's own over TLS, or one on this machine when the API is
    /// on this machine too (a stand-in for tests). The same rule as for
    /// the address of a browser.
    fn is_a_live_view(&self, address: &str) -> bool {
        let Ok(url) = Url::parse(address) else {
            return false;
        };
        match url.scheme() {
            "https" if is_at_the_service(&url) => true,
            "http" | "https" => is_loopback(&url) && is_loopback(&self.base),
            _ => false,
        }
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
            .send(
                &credentials.api_key,
                Method::DELETE,
                &path,
                &[],
                None,
                CALL_TIMEOUT,
            )
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
        websocket::innermost_cause(error)
    };
    failure::unreachable(&reason)
}

#[cfg(test)]
mod tests {
    use super::super::stand_in::{Answer, StandIn};
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
    fn a_session_says_which_scan_of_which_installation_it_was_started_for() {
        let of = |metadata: Value| {
            SessionState::of(&json!({ "id": "x", "status": "RUNNING", "userMetadata": metadata }))
                .unwrap()
        };
        let ours = of(json!({ "app": "adscan", "install": "abc", "scan": "scan-1" }));
        assert!(ours.is_of_scan("abc", "scan-1"));
        assert!(!ours.is_of_scan("abc", "scan-2"));
        assert!(!ours.is_of_scan("abd", "scan-1"));
        assert!(
            !of(json!({ "app": "other", "install": "abc", "scan": "scan-1" }))
                .is_of_scan("abc", "scan-1")
        );
        assert!(!of(json!({ "app": "adscan", "install": "abc" })).is_of_scan("abc", "scan-1"));
        assert!(!of(json!({ "scanId": "scan-1" })).is_of_scan("abc", "scan-1"));
        // A probe is tagged as one.
        assert!(
            of(json!({ "app": "adscan", "install": "abc", "scan": "probe" }))
                .is_of_scan("abc", "probe")
        );
    }

    fn credentials() -> BrowserbaseCredentials {
        BrowserbaseCredentials {
            api_key: "bb_live_K3y-that-must-not-travel".into(),
            project_id: "3f8a1c52-6e0b-4b7d-9a43-5d2c8e9f1a07".into(),
        }
    }

    fn order() -> SessionOrder {
        SessionOrder {
            region: adscan_model::BrowserbaseRegion::EuCentral1,
            timeout_seconds: 240,
            settings: adscan_model::BrowserbaseSessionSettings::STANDARD,
            exit: super::super::order::Exit::Region,
            context_id: None,
            install_id: "0123456789abcdef0123456789abcdef".into(),
            scan_id: "7c9e6679-7425-40de-944b-e07fc1f90ae7".into(),
        }
    }

    /// The API key travels in a header of its own, which an HTTP client
    /// does not take off when a redirect leads to another host. So no
    /// redirect is followed, neither by a call nor by the upgrade to the
    /// socket of a browser, whose target was checked before.
    #[tokio::test]
    async fn a_redirect_is_never_followed() {
        let elsewhere = StandIn::start(|_, _| Answer::Json(200, json!([]))).await;
        let target = format!("{}/v1/projects", elsewhere.url);
        let service = StandIn::start(move |_, _| Answer::Redirect(target.clone())).await;
        let api = Api::new(&service.url, None).unwrap();

        let called = api.projects(&credentials().api_key).await;
        let created = api.create_session(&credentials(), &order()).await;
        let browser = api.browser_address(&json!({
            "connectUrl": format!("ws://127.0.0.1:{}/connect/1?signingKey=s3cret", service.port),
        }));
        let connected = api.connect(&browser).await.err();

        assert_eq!(called, Err(failed(UNEXPECTED_ANSWER)));
        assert_eq!(
            created.err().map(|refused| refused.failure),
            Some(failed(UNEXPECTED_ANSWER))
        );
        assert_eq!(
            connected,
            Some(failed(
                "The scanner could not connect to the Browserbase browser: the browser refused \
                 the connection (HTTP 302)"
            ))
        );
        assert_eq!(service.asked().len(), 3);
        assert!(
            elsewhere.asked().is_empty(),
            "a redirect was followed: {:?}",
            elsewhere.asked()
        );
    }

    #[test]
    fn a_live_view_is_only_opened_at_the_service_itself() {
        let real = Api::new("https://api.browserbase.com", None).unwrap();
        for accepted in [
            "https://www.browserbase.com/devtools-fullscreen/inspector.html?wss=connect.browserbase.com/debug/1",
            "https://browserbase.com/live",
        ] {
            assert!(real.is_a_live_view(accepted), "{accepted}");
        }
        for refused in [
            "https://attacker.example/devtools-fullscreen/inspector.html?wss=connect.browserbase.com",
            "https://www.browserbase.com.attacker.example/",
            "https://notbrowserbase.com/",
            "http://www.browserbase.com/devtools-fullscreen/inspector.html",
            "http://127.0.0.1:9222/",
            "https://127.0.0.1:9222/",
            "file:///Applications/Calculator.app",
            "javascript:alert(1)",
            "www.browserbase.com/devtools-fullscreen/inspector.html",
            "",
        ] {
            assert!(!real.is_a_live_view(refused), "{refused}");
        }

        // A stand-in on this machine may show a page on this machine.
        let stand_in = Api::new("http://127.0.0.1:4010", None).unwrap();
        assert!(stand_in.is_a_live_view("http://127.0.0.1:4010/live/1"));
        assert!(stand_in.is_a_live_view("http://localhost:9222/"));
        assert!(stand_in.is_a_live_view("https://www.browserbase.com/devtools-fullscreen/x"));
        assert!(!stand_in.is_a_live_view("http://attacker.example/"));
        assert!(!stand_in.is_a_live_view("https://attacker.example/"));
        assert!(!stand_in.is_a_live_view("http://192.168.1.20/"));
    }

    #[tokio::test]
    async fn a_live_view_somewhere_else_is_not_handed_out() {
        let session = "5e551001-0000-4000-8000-000000000001";
        let service = StandIn::start(|_, before| {
            let address = match before {
                0 => "https://attacker.example/devtools-fullscreen/inspector.html?token=t0ken",
                _ => "http://127.0.0.1:9/live?token=t0ken",
            };
            Answer::Json(200, json!({ "debuggerFullscreenUrl": address }))
        })
        .await;
        let api = Api::new(&service.url, None).unwrap();

        let elsewhere = api.live_view(&credentials(), session).await;
        let here = api.live_view(&credentials(), session).await;

        assert_eq!(elsewhere, Err(failed(UNEXPECTED_ANSWER)));
        assert_eq!(here.as_deref(), Ok("http://127.0.0.1:9/live?token=t0ken"));
    }

    /// Whether a browser may run after a request for one failed decides
    /// whether it is looked for.
    #[tokio::test]
    async fn a_failed_request_for_a_browser_says_whether_one_may_have_been_started() {
        let refusal = |status: u16| json!({ "statusCode": status, "message": "No" });
        for (status, answer, may_exist) in [
            (400, refusal(400), false),
            (401, refusal(401), false),
            (429, refusal(429), false),
            (500, refusal(500), true),
            (503, json!("upstream connect error"), true),
            (200, json!({ "status": "RUNNING" }), true),
            (201, json!([]), true),
        ] {
            let service = StandIn::start(move |_, _| Answer::Json(status, answer.clone())).await;
            let api = Api::new(&service.url, None).unwrap();

            let refused = api.create_session(&credentials(), &order()).await.err();

            assert_eq!(
                refused.map(|refused| refused.session_may_exist),
                Some(may_exist),
                "{status}"
            );
        }

        let slow = StandIn::start(|_, _| {
            Answer::Late(Duration::from_millis(500), 201, json!({ "id": "x" }))
        })
        .await;
        let impatient = Api::new(&slow.url, None)
            .unwrap()
            .giving_a_new_browser(Duration::from_millis(100));
        let given_up = impatient
            .create_session(&credentials(), &order())
            .await
            .err()
            .unwrap();
        assert!(given_up.session_may_exist);
        assert_eq!(
            given_up.failure,
            failed("Browserbase could not be reached: it did not answer in time")
        );
    }

    /// How many browsers run is a number: the sessions of whoever else
    /// uses the project are counted, and nothing of them is read.
    #[tokio::test]
    async fn the_running_browsers_are_counted_without_being_read() {
        let service = StandIn::start(|asked, _| {
            assert_eq!(asked.query, "status=RUNNING");
            Answer::Json(
                200,
                json!([
                    { "id": "a", "status": "RUNNING", "userMetadata": { "organizationId": "3" } },
                    { "unexpected": true },
                    17,
                ]),
            )
        })
        .await;
        let api = Api::new(&service.url, None).unwrap();

        assert_eq!(api.running_session_count(&credentials()).await, Ok(3));

        let odd = StandIn::start(|_, _| Answer::Json(200, json!({ "sessions": [] }))).await;
        let api = Api::new(&odd.url, None).unwrap();
        assert_eq!(
            api.running_session_count(&credentials()).await,
            Err(failed(UNEXPECTED_ANSWER))
        );
    }

    #[tokio::test]
    async fn a_session_the_service_does_not_know_is_told_apart_from_a_failure() {
        let service = StandIn::start(|asked, _| match asked.path.as_str() {
            "/v1/sessions/known" => {
                Answer::Json(200, json!({ "id": "known", "status": "RUNNING" }))
            }
            "/v1/sessions/unknown" => {
                Answer::Json(404, json!({ "statusCode": 404, "message": "Not Found" }))
            }
            _ => Answer::Json(503, json!({})),
        })
        .await;
        let api = Api::new(&service.url, None).unwrap();

        let known = api.find_session(&credentials(), "known").await.unwrap();
        assert!(known.is_some_and(|session| session.running));
        assert_eq!(api.find_session(&credentials(), "unknown").await, Ok(None));
        assert_eq!(
            api.find_session(&credentials(), "broken").await,
            Err(failed("Browserbase is not available right now (HTTP 503)."))
        );
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
