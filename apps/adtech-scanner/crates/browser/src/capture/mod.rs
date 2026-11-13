//! One capture, start to finish: launch, emulate, visit, collect.

mod bid_endpoints;
mod clock;
mod consent;
mod console;
mod driver;
mod environment;
mod error;
mod injected;
mod input;
mod interception;
mod manual;
mod navigation;
mod network;
mod noise;
mod page;
mod pages;
mod patience;
mod progress;
mod recorder;
mod remote;
mod scenario;
mod scripts;
mod tabs;
mod tracker_log;
mod visitor;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use adscan_model::{
    BrowserbaseCapture, Capture, CaptureActivity, CaptureBrowser, CaptureEnvironment, CaptureProxy,
    CaptureRequest, CapturedPage, ConsentNote, DeviceProfile, ProxyScheme, RemoteSessionEvent,
    VisitorBehavior,
};
use serde_json::{Value, json};
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::time::Instant;
use url::Url;

use crate::cdp::{
    ApiFailure, Backend, Cdp, CdpError, CdpEvent, Chrome, LaunchOptions, ProfileChoice,
    ProxyServer, RemoteBrowser, SessionState, StartNote,
};
use crate::device::{emulation_for, window_size};
use crate::{BrowserError, BrowserbaseHooks, CaptureHooks, detect};

pub(crate) use clock::{CLOCK_QUESTION, CLOCK_SAMPLES, ClockSample, distance, machine_epoch_ms};
pub(crate) use remote::BLANK_PAGE_FACTS;

use environment::Observed;
use error::CaptureError;
use interception::{Interception, ProxyLogin};
use manual::{Ending, Window};
use navigation::parse_address;
use network::reply::ReplyLimit;
use page::{Page, TabSetup};
use pages::{PageLog, ViewFacts};
use recorder::{BrowserLink, Recorder};

const NOT_SET_UP: &str = "Browserbase is not set up. Add the API key in Settings.";
const CONNECTION_LOST: &str = "The connection to the Browserbase browser was lost. The browser \
                               has ended. Nothing was kept.";
const TIME_LIMIT_REACHED: &str =
    "Browserbase ended the browser at its time limit. Nothing was kept.";

/// Page views of one visit. A person browsing gets more than a script.
const MAX_PAGE_VIEWS: usize = 8;
const MAX_PAGE_VIEWS_MANUAL: usize = 25;

/// The directory of the kept browser profile of a device, under the root the
/// app chose. A phone and a desktop browser never share cookies.
pub(crate) fn scan_profile_directories(root: &Path) -> [PathBuf; 2] {
    [DeviceProfile::Desktop, DeviceProfile::Mobile].map(|device| scan_profile_of(root, device))
}

fn scan_profile_of(root: &Path, device: DeviceProfile) -> PathBuf {
    root.join(match device {
        DeviceProfile::Desktop => "desktop",
        DeviceProfile::Mobile => "mobile",
    })
}

/// What a visit has to know about the browser it runs in.
struct Venue<'a> {
    /// The login of the scan's own proxy, when this machine's browser has
    /// to give it. A browser at Browserbase is signed in by the service.
    proxy_login: Option<ProxyLogin>,
    /// The scan's own proxy by name, for the words of a failure.
    proxy_host: Option<String>,
    at_browserbase: Option<RemoteVenue<'a>>,
}

struct RemoteVenue<'a> {
    capture: &'a BrowserbaseCapture,
    hooks: &'a BrowserbaseHooks,
    session_id: String,
}

pub(crate) async fn run(
    request: CaptureRequest,
    hooks: CaptureHooks,
) -> Result<Capture, BrowserError> {
    let started = Instant::now();
    let address = parse_address(&request.url).map_err(CaptureError::from)?;
    let (mut backend, venue) = match &request.browser {
        CaptureBrowser::Local => launch_local(&request, &hooks).await?,
        CaptureBrowser::Browserbase(capture) => launch_remote(&request, capture, &hooks).await?,
    };
    let connected = tokio::select! {
        biased;
        _ = hooks.cancel.cancelled() => Err(CaptureError::Cancelled),
        connected = backend.connect() => connected.map_err(CaptureError::from),
    };
    let (cdp, outcome) = match connected {
        Ok((cdp, events)) => {
            let cdp = Arc::new(cdp);
            let visit = Visit {
                request: &request,
                address: &address,
                hooks: &hooks,
                venue,
                started,
            };
            let outcome = tokio::select! {
                biased;
                _ = hooks.cancel.cancelled() => Err(CaptureError::Cancelled),
                outcome = visit.scan(cdp.clone(), events) => outcome,
            };
            (Some(cdp), outcome)
        }
        Err(error) => (None, Err(error)),
    };
    let outcome = match (outcome, backend.remote()) {
        (Err(error), Some(browser)) if is_a_lost_connection(&error) => {
            (hooks.on_activity)(remote::connection_lost());
            Err(connection_lost(browser))
        }
        (outcome, _) => outcome,
    };
    let remote_session = backend
        .remote()
        .map(|browser| browser.session_id().to_string());
    let ended = backend.close(cdp.as_deref()).await;
    if let Some(session_id) = remote_session {
        report_remote_end(&hooks, session_id, ended);
    }
    Ok(outcome?)
}

/// Whether a capture in a remote browser ended because the connection to
/// that browser is gone. A connection can die without closing: commands
/// then go unanswered, and the first of them gives up before the silence
/// on the socket is long enough to be taken for its end.
fn is_a_lost_connection(error: &CaptureError) -> bool {
    matches!(
        error,
        CaptureError::Protocol(CdpError::Closed | CdpError::Timeout(_))
    )
}

/// Why a capture ends when the connection to its Browserbase browser is
/// gone. The service ends a browser at the time limit it was started with
/// by closing the connection.
fn connection_lost(browser: &RemoteBrowser) -> CaptureError {
    let message = if browser.has_reached_its_time_limit() {
        TIME_LIMIT_REACHED
    } else {
        CONNECTION_LOST
    };
    CaptureError::Remote(ApiFailure::Failed(message.into()))
}

/// Tells the audit trail and the engine how a Browserbase session ended.
/// The engine only hears of an end that the service confirmed: it keeps
/// its record of the session otherwise, and releases it at its next start.
fn report_remote_end(hooks: &CaptureHooks, session_id: String, ended: Option<SessionState>) {
    let Some(ended) = ended else {
        return (hooks.on_activity)(remote::release_unconfirmed());
    };
    (hooks.on_activity)(remote::released(&ended));
    if let Some(browserbase) = &hooks.browserbase {
        (browserbase.on_session)(RemoteSessionEvent::Released {
            session_id,
            session_seconds: ended.seconds(),
            proxy_bytes: ended.proxy_bytes,
        });
    }
}

/// Starts Chrome on this machine.
async fn launch_local<'a>(
    request: &CaptureRequest,
    hooks: &CaptureHooks,
) -> Result<(Backend, Venue<'a>), BrowserError> {
    let (proxy, proxy_login) = proxy_of(request, hooks)?;
    let kept_profile = match (&hooks.scan_profile_dir, request.persistent_profile) {
        (Some(root), true) => Some(scan_profile_of(root, request.device)),
        (None, true) => {
            return Err(BrowserError::Other(
                "No directory was given for the browser profile".into(),
            ));
        }
        (_, false) => None,
    };
    // Detection runs the browser for its version, which blocks.
    let configured_path = request.chrome_path.clone();
    let browser =
        tokio::task::spawn_blocking(move || detect::detect_browser(configured_path.as_deref()))
            .await
            .ok()
            .flatten()
            .ok_or(BrowserError::NotFound)?;
    (hooks.on_activity)(CaptureActivity::progress(format!(
        "Launching {} ({}{}{})",
        browser.version,
        if request.headless {
            "headless"
        } else {
            "visible"
        },
        if kept_profile.is_some() {
            ", browser profile"
        } else {
            ""
        },
        match &proxy {
            ProxyServer::System => String::new(),
            ProxyServer::Direct => ", no proxy".to_string(),
            ProxyServer::Custom { host, .. } => format!(", through the proxy {host}"),
        },
    )));

    let chrome = Chrome::spawn(LaunchOptions {
        executable: Path::new(&browser.path),
        headless: request.headless,
        window_size: window_size(request.device),
        profile: kept_profile
            .as_deref()
            .map_or(ProfileChoice::Throwaway, ProfileChoice::Persistent),
        proxy: &proxy,
    })
    .map_err(CaptureError::from)?;
    let venue = Venue {
        proxy_host: proxy_login.as_ref().map(|proxy| proxy.host.clone()),
        proxy_login,
        at_browserbase: None,
    };
    Ok((Backend::Local(chrome), venue))
}

/// Asks Browserbase for a browser. The engine hears of the session the
/// moment it exists, before anything else is done with it.
async fn launch_remote<'a>(
    request: &CaptureRequest,
    capture: &'a BrowserbaseCapture,
    hooks: &'a CaptureHooks,
) -> Result<(Backend, Venue<'a>), BrowserError> {
    let Some(browserbase) = &hooks.browserbase else {
        return Err(BrowserError::RemoteAuth(NOT_SET_UP.into()));
    };
    let order = remote::session_order(request, capture, hooks.proxy_password.as_deref())?;
    (hooks.on_activity)(CaptureActivity::progress(format!(
        "Asking Browserbase for a browser in {}",
        capture.region.as_str()
    )));
    let browser = RemoteBrowser::start(
        browserbase.client.api(),
        &browserbase.credentials,
        &order,
        &hooks.cancel,
        |note| {
            (hooks.on_activity)(match note {
                StartNote::Waiting { attempt, pause } => remote::rate_limited(attempt, pause),
                StartNote::StraysReleased(count) => remote::strays_released(count),
            })
        },
    )
    .await
    .map_err(CaptureError::from)?;
    let session_id = browser.session_id().to_string();
    (browserbase.on_session)(RemoteSessionEvent::Created {
        session_id: session_id.clone(),
    });
    (hooks.on_activity)(remote::session_created(&session_id, &order));
    let venue = Venue {
        proxy_login: None,
        proxy_host: match &request.proxy {
            CaptureProxy::Custom { host, .. } => Some(host.clone()),
            CaptureProxy::System | CaptureProxy::None => None,
        },
        at_browserbase: Some(RemoteVenue {
            capture,
            hooks: browserbase,
            session_id,
        }),
    };
    Ok((Backend::Browserbase(browser), venue))
}

/// How the browser reaches the network, and what it may tell a proxy that
/// asks who is there.
fn proxy_of(
    request: &CaptureRequest,
    hooks: &CaptureHooks,
) -> Result<(ProxyServer, Option<ProxyLogin>), CaptureError> {
    match &request.proxy {
        CaptureProxy::System => Ok((ProxyServer::System, None)),
        CaptureProxy::None => Ok((ProxyServer::Direct, None)),
        CaptureProxy::Custom {
            scheme: ProxyScheme::Socks5,
            username: Some(_),
            ..
        } => Err(CaptureError::Unsupported(
            "Chrome cannot sign in to a SOCKS5 proxy. Remove the user name, or use an HTTP or HTTPS proxy",
        )),
        CaptureProxy::Custom {
            scheme,
            host,
            port,
            username,
        } => {
            let scheme = match scheme {
                ProxyScheme::Http => "http",
                ProxyScheme::Https => "https",
                ProxyScheme::Socks5 => "socks5",
            };
            let server = ProxyServer::Custom {
                scheme: scheme.to_string(),
                host: host.clone(),
                port: *port,
            };
            let login = ProxyLogin {
                host: host.clone(),
                username: username.clone(),
                password: hooks.proxy_password.clone(),
            };
            Ok((server, Some(login)))
        }
    }
}

/// A visible desktop browser that is browsed like a visitor would keeps its
/// own window as the viewport. Everything else gets the profile's size: a
/// phone is emulated, and a headless browser has no window of its own.
fn overrides_viewport(request: &CaptureRequest) -> bool {
    let browsed = matches!(
        request.visitor,
        VisitorBehavior::Browse | VisitorBehavior::Manual
    );
    !(request.device == DeviceProfile::Desktop && !request.headless && browsed)
}

struct Visit<'a> {
    request: &'a CaptureRequest,
    address: &'a Url,
    hooks: &'a CaptureHooks,
    venue: Venue<'a>,
    /// When the capture was asked for.
    started: Instant,
}

impl Visit<'_> {
    async fn scan(
        self,
        cdp: Arc<Cdp>,
        events: UnboundedReceiver<CdpEvent>,
    ) -> Result<Capture, CaptureError> {
        let request = self.request;
        let on_activity = &self.hooks.on_activity;
        let Venue {
            proxy_login,
            proxy_host,
            at_browserbase,
        } = self.venue;
        let link = match &at_browserbase {
            Some(remote) => BrowserLink {
                hosted_by_service: true,
                reply_limit: ReplyLimit::Remote,
                behind_service_proxy: remote.capture.location.is_some(),
            },
            None => BrowserLink::LOCAL,
        };
        let (mut page, version) =
            attach(cdp, events, request, proxy_login, link, self.hooks).await?;
        let arrival = match &at_browserbase {
            Some(remote) => {
                let arrival = page
                    .arrive_at_remote_browser(remote.capture, &version)
                    .await?;
                let facts = arrival.facts(&remote.session_id, version["product"].as_str());
                on_activity(remote::connected(&facts));
                on_activity(remote::timezone(&facts));
                Some((arrival.seen, facts))
            }
            None => None,
        };
        on_activity(CaptureActivity::progress(format!(
            "Opening {} as {}",
            self.address,
            if page.overrides_viewport() {
                page.emulation().label.as_str()
            } else {
                "desktop (the browser's own window)"
            }
        )));
        driver::open(&mut page, self.address, proxy_host.as_deref()).await?;
        page.confine_to_site().await?;
        let observed = page.observe_environment().await?;
        let (mut seen, facts) = arrival.unzip();
        if let Some(seen) = seen.as_mut().filter(|seen| seen.brands.is_empty()) {
            // A blank page has no client hints to read; the loaded page has.
            seen.brands.clone_from(&observed.brands);
        }
        let environment = CaptureEnvironment {
            // A browser at Browserbase runs headless, whatever was asked for.
            headless: request.headless || at_browserbase.is_some(),
            persistent_profile: match &at_browserbase {
                Some(remote) => remote.capture.context_id.is_some(),
                None => request.persistent_profile,
            },
            webdriver: observed.webdriver,
            user_agent_overridden: page.emulation().user_agent_override().is_some(),
            viewport_overridden: page.overrides_viewport(),
            request_hosts: Vec::new(),
            browserbase: facts,
        };
        page.recorder
            .report(environment::activity(&environment, seen.as_ref()));

        let (consent_accepted, ending) = match request.visitor {
            VisitorBehavior::Manual => {
                let window = match &at_browserbase {
                    Some(remote) => {
                        // The page is recording: the person can take over.
                        (remote.hooks.on_session)(RemoteSessionEvent::LiveViewReady {
                            session_id: remote.session_id.clone(),
                        });
                        on_activity(remote::live_view());
                        Window::LiveView
                    }
                    None => Window::OnThisMachine,
                };
                let limit = self.started + manual::SESSION_LIMIT;
                let ending = manual::record(&mut page, &self.hooks.finish, limit, window).await?;
                (false, ending)
            }
            VisitorBehavior::Browse => (
                driver::browse(&mut page, request).await?,
                Ending::BrowserOpen,
            ),
            VisitorBehavior::None | VisitorBehavior::Scroll => (
                driver::observe(&mut page, request).await?,
                Ending::BrowserOpen,
            ),
        };
        let capture = collect(
            page,
            request,
            observed,
            environment,
            consent_accepted,
            ending,
        )
        .await?;
        on_activity(CaptureActivity::progress(format!(
            "Captured {} scripts, {} tracker events and {} console lines on {}",
            capture.scripts.len(),
            capture.events.len(),
            capture.console.len(),
            match capture.pages.len() {
                1 => "1 page view".to_string(),
                views => format!("{views} page views"),
            }
        )));
        Ok(capture)
    }
}

/// Sets the browser up for the visit and attaches to its first tab. Also
/// returns what the browser says of itself (`Browser.getVersion`).
async fn attach(
    cdp: Arc<Cdp>,
    events: UnboundedReceiver<CdpEvent>,
    request: &CaptureRequest,
    proxy_login: Option<ProxyLogin>,
    link: BrowserLink,
    hooks: &CaptureHooks,
) -> Result<(Page, Value), CaptureError> {
    let remote = request.browser != CaptureBrowser::Local;
    // A scanned page must not be able to put files in the user's Downloads.
    cdp.send(
        None,
        "Browser.setDownloadBehavior",
        json!({ "behavior": "deny", "eventsEnabled": true }),
    )
    .await?;
    let version = cdp.send(None, "Browser.getVersion", json!({})).await?;
    let setup = TabSetup {
        // A remote browser's own window is whatever its host gave it.
        override_viewport: remote || overrides_viewport(request),
        emulation: emulation_for(
            request.device,
            version["userAgent"].as_str().unwrap_or_default(),
            version["product"].as_str().unwrap_or_default(),
        ),
        visitor: request.visitor,
        hosted_by_service: remote,
        timezone: None,
    };
    let follows_links = request.visitor == VisitorBehavior::Browse;
    let interception = Interception::new(proxy_login, follows_links);
    let manual = request.visitor == VisitorBehavior::Manual;
    let page_views = if manual {
        MAX_PAGE_VIEWS_MANUAL
    } else {
        MAX_PAGE_VIEWS
    };
    let on_activity = hooks.on_activity.clone();
    let network = hooks.network.clone();
    let mut page = Page::attach(cdp, events, setup, interception, |main_frame_id| {
        let pages = PageLog::new(page_views, manual);
        Recorder::new(main_frame_id, pages, on_activity, network).linked_by(link)
    })
    .await?;
    if !request.headless && !remote {
        // Once: the window opens behind the app otherwise. A scan never
        // takes the focus again after that. A remote browser has no window
        // on this machine.
        page.call("Page.bringToFront", json!({})).await?;
    }
    Ok((page, version))
}

/// Turns what was recorded into the capture. With the browser gone, that is
/// what had arrived by then.
async fn collect(
    mut page: Page,
    request: &CaptureRequest,
    observed: Observed,
    mut environment: CaptureEnvironment,
    consent_accepted: bool,
    ending: Ending,
) -> Result<Capture, CaptureError> {
    if ending == Ending::BrowserOpen {
        page.flush_tracker().await?;
        page.note_title().await?;
        page.settle_bodies(page.patience.bodies).await?;
    }
    page.recorder.finish_requests();
    let service_scripts = page.recorder.service_scripts();
    if service_scripts.count > 0 {
        page.recorder.report(remote::service_scripts(
            service_scripts.count,
            service_scripts.chars,
        ));
    }
    let too_large = page.recorder.too_large_bodies();
    if too_large.count > 0 {
        page.recorder.report(remote::bodies_too_large(
            too_large.count,
            too_large.largest_bytes,
        ));
    }
    let requests = page.recorder.request_counts();
    if page.recorder.keeps_requests() {
        page.recorder.report(CaptureActivity::progress(format!(
            "Recorded {} requests, {} of them in iframes and workers with a process of their own",
            requests.total, requests.in_child_targets
        )));
    }
    let ended_at = page.recorder.clock.now_ms();
    let emulated = page.emulation().clone();
    let viewport = if page.overrides_viewport() {
        emulated.viewport
    } else {
        observed.viewport.unwrap_or(emulated.viewport)
    };
    let recorder = page.into_recorder();
    environment.request_hosts = recorder.third_party_hosts();
    let Recorder {
        tracker,
        pages,
        scripts,
        console,
        bid_responses,
        consent_notes,
        ..
    } = recorder;

    let facts: Vec<ViewFacts> = (0..pages.len())
        .map(|page| tracker.facts(page as u32))
        .collect();
    let navigation_start = pages.navigation_start().unwrap_or(ended_at);
    let place = |epoch_ms: f64| pages.place_at(epoch_ms);
    let scripts = scripts.into_captured(place);
    let console = console.into_lines(place);
    let bid_responses = bid_responses.into_observed(place);
    let consent_notes = consent_notes
        .into_iter()
        .map(|note| {
            let page = note.page.unwrap_or(0);
            ConsentNote {
                page,
                t: pages
                    .started_at_epoch_ms(page)
                    .map_or(0.0, |started_at| (note.at_epoch_ms - started_at).max(0.0)),
                kind: note.kind,
                banner: note.banner,
            }
        })
        .collect();
    let mut pages = pages.into_captured(ended_at, |page| facts[page as usize]);
    if pages.is_empty() {
        pages.push(CapturedPage::entry(request.url.clone()));
    }
    let entry = &pages[0];
    Ok(Capture {
        final_url: entry.url.clone(),
        page_title: entry.title.clone(),
        device: request.device,
        viewport,
        user_agent: observed.user_agent.unwrap_or(emulated.user_agent),
        navigation_start,
        page_load_ms: entry.load_ms,
        max_scroll_depth: pages
            .iter()
            .map(|page| page.max_scroll_depth)
            .fold(0.0, f64::max),
        scripts,
        console,
        events: tracker.into_events(),
        consent_accepted,
        consent_notes,
        bid_responses,
        pages,
        environment,
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::time::Duration;

    use adscan_model::{
        BodyAdmission, BodyOffer, NetworkRecord, NetworkSink, PageReach, RequestEnd, ResourceKind,
    };

    use super::*;

    /// A sink that keeps every record and reads every body.
    #[derive(Default)]
    struct Kept(Mutex<Vec<NetworkRecord>>);

    impl NetworkSink for Kept {
        fn admit_body(&self, _: &BodyOffer<'_>) -> BodyAdmission {
            BodyAdmission::Read { max_bytes: 4_096 }
        }

        fn record(&self, record: NetworkRecord) {
            self.0.lock().unwrap().push(record);
        }
    }

    /// A connection that died without closing shows as commands that are
    /// never answered. For a remote browser that is a lost connection, like
    /// a closed one; a browser that refuses a command is not.
    #[test]
    fn an_unanswered_command_counts_as_a_lost_connection_like_a_closed_one() {
        assert!(is_a_lost_connection(&CaptureError::Protocol(
            CdpError::Closed
        )));
        assert!(is_a_lost_connection(&CaptureError::Protocol(
            CdpError::Timeout("Runtime.evaluate")
        )));
        for other in [
            CaptureError::Protocol(CdpError::Rejected {
                method: "Page.navigate",
                message: "Cannot navigate to invalid URL".into(),
            }),
            CaptureError::Cancelled,
            CaptureError::PageCrashed,
            CaptureError::TabClosed,
            CaptureError::NoPage,
        ] {
            assert!(!is_a_lost_connection(&other), "{other}");
        }
    }

    /// A page cannot close the tab it was opened in, so what a person does
    /// when they close the window is done here over the DevTools connection,
    /// which only code inside this crate can reach. With the browser gone
    /// there is nobody to ask for anything: the requests it reported are
    /// handed over as they stood.
    #[tokio::test]
    async fn closing_the_last_tab_ends_a_manual_visit_and_keeps_what_was_recorded() {
        let Some(browser) = detect::detect_browser(None) else {
            return;
        };
        let site = crate::demo::start().await.unwrap();
        let mut chrome = Chrome::spawn(LaunchOptions {
            executable: Path::new(&browser.path),
            headless: true,
            window_size: window_size(DeviceProfile::Desktop),
            profile: ProfileChoice::Throwaway,
            proxy: &ProxyServer::System,
        })
        .unwrap();
        let (cdp, events) = crate::cdp::connect(&mut chrome).await.unwrap();
        let cdp = Arc::new(cdp);
        let emulation = emulation_for(DeviceProfile::Desktop, "Chrome/1.0", "Chrome/1.0");
        let setup = TabSetup {
            emulation,
            override_viewport: true,
            visitor: VisitorBehavior::Manual,
            hosted_by_service: false,
            timezone: None,
        };
        let requests = Arc::new(Kept::default());
        let sink = requests.clone();
        let mut page = Page::attach(
            cdp.clone(),
            events,
            setup,
            Interception::new(None, false),
            |main_frame_id| {
                let pages = PageLog::new(MAX_PAGE_VIEWS_MANUAL, true);
                Recorder::new(main_frame_id, pages, Arc::new(|_| {}), Some(sink))
            },
        )
        .await
        .unwrap();
        let address = Url::parse(&site.url).unwrap();
        driver::open(&mut page, &address, None).await.unwrap();

        let the_person = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(1_500)).await;
            let targets = cdp
                .send(None, "Target.getTargets", json!({}))
                .await
                .unwrap();
            for target in targets["targetInfos"].as_array().unwrap() {
                if target["type"] == "page" {
                    cdp.notify(
                        None,
                        "Target.closeTarget",
                        json!({ "targetId": target["targetId"] }),
                    );
                }
            }
        });
        let never_finished = tokio_util::sync::CancellationToken::new();
        let limit = Instant::now() + manual::SESSION_LIMIT;
        let ending = manual::record(&mut page, &never_finished, limit, Window::OnThisMachine)
            .await
            .unwrap();
        the_person.await.unwrap();
        assert_eq!(ending, Ending::BrowserGone);

        let request = CaptureRequest {
            url: site.url.clone(),
            device: DeviceProfile::Desktop,
            observe_seconds: 1,
            visitor: VisitorBehavior::Manual,
            extra_pages: 0,
            seed: 1,
            accept_consent: false,
            headless: true,
            persistent_profile: false,
            proxy: CaptureProxy::System,
            chrome_path: None,
            browser: CaptureBrowser::Local,
        };
        let capture = collect(
            page,
            &request,
            Observed::default(),
            CaptureEnvironment::default(),
            false,
            ending,
        )
        .await
        .unwrap();
        chrome.shutdown().await;

        assert_eq!(capture.pages.len(), 1);
        assert_eq!(capture.pages[0].reached_by, PageReach::Entry);
        assert_eq!(capture.final_url, site.url);
        assert!(
            capture.page_title.is_some(),
            "the title was read while the page was there"
        );
        assert!(capture.pages[0].ended_at_ms >= 1_000.0);
        assert!(!capture.events.is_empty() && !capture.scripts.is_empty());

        let records = requests.0.lock().unwrap();
        let document = records.iter().find(|record| record.seq == 0).unwrap();
        assert_eq!(
            (document.kind, document.url.as_str(), &document.end),
            (
                ResourceKind::Document,
                site.url.as_str(),
                &RequestEnd::Finished
            )
        );
        // Every request once, whether it had ended or was still open.
        let mut numbers: Vec<u32> = records.iter().map(|record| record.seq).collect();
        numbers.sort_unstable();
        assert_eq!(numbers, (0..records.len() as u32).collect::<Vec<_>>());
        assert!(records.len() > 10, "{} requests", records.len());
    }
}
