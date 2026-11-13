//! Browserbase, as the app outside a capture uses it: checking a key,
//! reading a project and its usage, finding and ending the sessions this
//! installation started, the contexts that hold a kept browser profile,
//! the probe, and the live view.

use std::sync::Arc;
use std::time::Duration;

use adscan_model::{
    BrowserbaseCredentials, BrowserbaseProbe, BrowserbaseProject, BrowserbaseRegion,
    BrowserbaseSessionSettings, RemoteSessionEvent,
};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use crate::cdp::{Api, ApiFailure, Cdp, Exit, Project, RemoteBrowser, SessionOrder, StartFailure};

pub const BROWSERBASE_API_URL: &str = "https://api.browserbase.com";

/// The `userMetadata.scan` of a session that is a probe, not a scan.
const PROBE_TAG: &str = "probe";
/// The shortest time limit the service takes: a probe is over in seconds.
const PROBE_TIMEOUT_SECONDS: u64 = 60;
/// How many commands are timed for the probe's round trip.
const PROBE_ROUND_TRIPS: usize = 9;
/// How often, and how far apart, the probe looks for the browser's page.
const FIRST_PAGE_LOOKS: u32 = 50;
const FIRST_PAGE_LOOK_PAUSE: Duration = Duration::from_millis(100);

/// Where Browserbase is reached. Not secret.
#[derive(Debug, Clone)]
pub struct BrowserbaseEndpoint {
    /// https, or http only when the host is this machine.
    pub api_url: String,
    /// For tests: used in place of the address of every session's browser.
    /// `wss://`, or `ws://` on this machine.
    pub connect_url: Option<String>,
}

impl Default for BrowserbaseEndpoint {
    fn default() -> Self {
        Self {
            api_url: BROWSERBASE_API_URL.to_string(),
            connect_url: None,
        }
    }
}

/// Why a call to Browserbase failed. The messages are whole sentences for
/// the user and never hold a key or an address.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum BrowserbaseError {
    /// The API key or the project was not accepted.
    #[error("{0}")]
    Auth(String),
    /// The project is at a limit.
    #[error("{0}")]
    Limit(String),
    #[error("{0}")]
    Failed(String),
}

impl From<ApiFailure> for BrowserbaseError {
    fn from(failure: ApiFailure) -> Self {
        match failure {
            ApiFailure::Auth(message) => Self::Auth(message),
            ApiFailure::Limit(message) => Self::Limit(message),
            ApiFailure::Failed(message) => Self::Failed(message),
        }
    }
}

/// Running totals of a project since it was made, all its users together.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BrowserbaseProjectUsage {
    pub browser_minutes: f64,
    pub proxy_bytes: f64,
}

/// What Browserbase says a session used.
#[derive(Debug, Clone, PartialEq)]
pub struct BrowserbaseSessionUsage {
    /// The browser is no longer running.
    pub ended: bool,
    /// From its start to its end; `None` while it runs.
    pub session_seconds: Option<f64>,
    /// Traffic through a Browserbase proxy. The count lags: it can read 0
    /// for half a minute after the end.
    pub proxy_bytes: Option<f64>,
}

/// What a capture needs to run its browser at Browserbase.
#[derive(Clone)]
pub struct BrowserbaseHooks {
    pub client: Browserbase,
    pub credentials: BrowserbaseCredentials,
    /// Hears of the session as it is created, ready for a person, and
    /// ended. Called from the capture's own task; must not block.
    pub on_session: Arc<dyn Fn(RemoteSessionEvent) + Send + Sync>,
}

/// A client of Browserbase. Cheap to clone. Its requests leave through
/// the system's proxy like the app's other requests.
#[derive(Clone)]
pub struct Browserbase {
    api: Api,
}

fn project_of(project: Project) -> BrowserbaseProject {
    BrowserbaseProject {
        id: project.id,
        name: project.name,
        concurrency: project.concurrency,
    }
}

impl Browserbase {
    /// Fails when `api_url` is not https (and not on this machine) or
    /// `connect_url` is not a WebSocket address the scanner connects to.
    pub fn new(endpoint: BrowserbaseEndpoint) -> Result<Self, BrowserbaseError> {
        let api = Api::new(&endpoint.api_url, endpoint.connect_url.as_deref())?;
        Ok(Self { api })
    }

    pub(crate) fn api(&self) -> &Api {
        &self.api
    }

    /// The projects of a key. Costs nothing, and is what proves a key.
    pub async fn projects(
        &self,
        api_key: &str,
    ) -> Result<Vec<BrowserbaseProject>, BrowserbaseError> {
        let projects = self.api.projects(api_key).await?;
        Ok(projects.into_iter().map(project_of).collect())
    }

    pub async fn project(
        &self,
        credentials: &BrowserbaseCredentials,
    ) -> Result<BrowserbaseProject, BrowserbaseError> {
        Ok(project_of(self.api.project(credentials).await?))
    }

    pub async fn project_usage(
        &self,
        credentials: &BrowserbaseCredentials,
    ) -> Result<BrowserbaseProjectUsage, BrowserbaseError> {
        let (browser_minutes, proxy_bytes) = self.api.project_usage(credentials).await?;
        Ok(BrowserbaseProjectUsage {
            browser_minutes,
            proxy_bytes,
        })
    }

    /// How many browsers run in the project right now, whoever started
    /// them.
    pub async fn running_session_count(
        &self,
        credentials: &BrowserbaseCredentials,
    ) -> Result<u32, BrowserbaseError> {
        let running = self.api.running_sessions(credentials, None).await?;
        Ok(running.len() as u32)
    }

    /// The ids of the running sessions this installation of this app
    /// started: those whose metadata says so, checked here again whatever
    /// the service's own filter answered. A project can be shared with
    /// other applications; their sessions are never returned.
    pub async fn running_sessions_of(
        &self,
        credentials: &BrowserbaseCredentials,
        install_id: &str,
    ) -> Result<Vec<String>, BrowserbaseError> {
        let listed = self
            .api
            .running_sessions(credentials, Some(install_id))
            .await?;
        Ok(listed
            .into_iter()
            .filter(|session| session.running && session.is_of(install_id))
            .map(|session| session.id)
            .collect())
    }

    /// What a session used, as the service counts it now.
    pub async fn session_usage(
        &self,
        credentials: &BrowserbaseCredentials,
        session_id: &str,
    ) -> Result<BrowserbaseSessionUsage, BrowserbaseError> {
        let session = self.api.session(credentials, session_id).await?;
        Ok(BrowserbaseSessionUsage {
            ended: !session.running,
            session_seconds: session.seconds(),
            proxy_bytes: session.proxy_bytes,
        })
    }

    /// Ends a session. One that has ended already is fine.
    pub async fn release_session(
        &self,
        credentials: &BrowserbaseCredentials,
        session_id: &str,
    ) -> Result<(), BrowserbaseError> {
        self.api.release(credentials, session_id).await?;
        Ok(())
    }

    /// Makes a context, the place a kept browser profile lives in, and
    /// returns its id.
    pub async fn create_context(
        &self,
        credentials: &BrowserbaseCredentials,
    ) -> Result<String, BrowserbaseError> {
        Ok(self.api.create_context(credentials).await?)
    }

    /// Deletes a context. One that is gone already is fine.
    pub async fn delete_context(
        &self,
        credentials: &BrowserbaseCredentials,
        context_id: &str,
    ) -> Result<(), BrowserbaseError> {
        Ok(self.api.delete_context(credentials, context_id).await?)
    }

    /// Starts one browser with the standard settings, reads what its blank
    /// page reports, and ends it. Nothing is navigated to. The browser is
    /// ended whether the reading worked or not. Costs a billed minute.
    pub async fn probe(
        &self,
        credentials: &BrowserbaseCredentials,
        region: BrowserbaseRegion,
        install_id: &str,
    ) -> Result<BrowserbaseProbe, BrowserbaseError> {
        let order = SessionOrder {
            region,
            timeout_seconds: PROBE_TIMEOUT_SECONDS,
            settings: BrowserbaseSessionSettings::STANDARD,
            exit: Exit::Region,
            context_id: None,
            install_id: install_id.to_string(),
            scan_id: PROBE_TAG.to_string(),
        };
        let never_cancelled = CancellationToken::new();
        let browser =
            RemoteBrowser::start(&self.api, credentials, &order, &never_cancelled, |_, _| {})
                .await
                .map_err(|failure| match failure {
                    StartFailure::Refused(failure) => BrowserbaseError::from(failure),
                    StartFailure::Cancelled => {
                        BrowserbaseError::Failed("The probe was cancelled.".into())
                    }
                })?;
        // Its own task: the browser is ended even when the caller stops
        // waiting for the answer.
        let probing = tokio::spawn(async move {
            let connected = browser.connect().await;
            let read = match &connected {
                Ok((cdp, _events)) => read_blank_page(cdp).await,
                Err(failure) => Err(failure.clone()),
            };
            let ended = browser
                .end(connected.as_ref().ok().map(|(cdp, _)| cdp))
                .await;
            read.map(|page| BrowserbaseProbe {
                region,
                browser_version: page.browser_version,
                user_agent: page.user_agent,
                webdriver: page.webdriver,
                timezone: page.timezone,
                language: page.language,
                round_trip_ms: page.round_trip_ms,
                session_seconds: ended.and_then(|ended| ended.seconds()),
            })
        });
        match probing.await {
            Ok(probed) => Ok(probed?),
            Err(_) => Err(BrowserbaseError::Failed(
                "The probe of the Browserbase browser failed unexpectedly.".into(),
            )),
        }
    }

    /// The address of Browserbase's live view of a running session, the
    /// one that fills the window. A credential: open it and drop it.
    pub async fn live_view_url(
        &self,
        credentials: &BrowserbaseCredentials,
        session_id: &str,
    ) -> Result<String, BrowserbaseError> {
        Ok(self.api.live_view(credentials, session_id).await?)
    }
}

const BLANK_PAGE_FACTS: &str = "[navigator.webdriver, navigator.userAgent, \
     Intl.DateTimeFormat().resolvedOptions().timeZone, navigator.language]";

/// What the page a browser starts with reports.
struct BlankPage {
    browser_version: Option<String>,
    user_agent: Option<String>,
    webdriver: Option<bool>,
    timezone: Option<String>,
    language: Option<String>,
    /// The median of a few commands.
    round_trip_ms: Option<f64>,
}

/// Reads what the page the browser starts with reports, without creating
/// or navigating anything.
async fn read_blank_page(cdp: &Cdp) -> Result<BlankPage, ApiFailure> {
    let unread = |error: crate::cdp::CdpError| {
        ApiFailure::Failed(format!(
            "The Browserbase browser could not be asked what a page sees: {error}"
        ))
    };
    let version = cdp
        .send(None, "Browser.getVersion", json!({}))
        .await
        .map_err(unread)?;
    // The page the browser starts with: there at once at Browserbase, a
    // moment after the first answer in a browser that has only just started.
    let mut page = None;
    for _ in 0..FIRST_PAGE_LOOKS {
        let targets = cdp
            .send(None, "Target.getTargets", json!({}))
            .await
            .map_err(unread)?;
        page = targets["targetInfos"]
            .as_array()
            .and_then(|targets| targets.iter().find(|target| target["type"] == "page"))
            .and_then(|page| page["targetId"].as_str().map(str::to_string));
        if page.is_some() {
            break;
        }
        tokio::time::sleep(FIRST_PAGE_LOOK_PAUSE).await;
    }
    let page = page.ok_or_else(|| {
        ApiFailure::Failed("The Browserbase browser did not open a page to read.".into())
    })?;
    let attached = cdp
        .send(
            None,
            "Target.attachToTarget",
            json!({ "targetId": page, "flatten": true }),
        )
        .await
        .map_err(unread)?;
    let session = attached["sessionId"].as_str().unwrap_or_default();
    let evaluate = |expression: &str| {
        cdp.send(
            Some(session),
            "Runtime.evaluate",
            json!({ "expression": expression, "returnByValue": true }),
        )
    };
    let seen = evaluate(BLANK_PAGE_FACTS).await.map_err(unread)?;
    let seen = &seen["result"]["value"];
    let mut round_trips_ms = Vec::with_capacity(PROBE_ROUND_TRIPS);
    for _ in 0..PROBE_ROUND_TRIPS {
        let asked = tokio::time::Instant::now();
        evaluate("1").await.map_err(unread)?;
        round_trips_ms.push(asked.elapsed().as_secs_f64() * 1_000.0);
    }
    round_trips_ms.sort_by(f64::total_cmp);
    let text = |value: &Value| value.as_str().map(str::to_string);
    Ok(BlankPage {
        browser_version: text(&version["product"]),
        user_agent: text(&seen[1]),
        webdriver: seen[0].as_bool(),
        timezone: text(&seen[2]),
        language: text(&seen[3]),
        round_trip_ms: round_trips_ms.get(PROBE_ROUND_TRIPS / 2).copied(),
    })
}
