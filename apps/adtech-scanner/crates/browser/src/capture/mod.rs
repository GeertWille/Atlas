//! One capture, start to finish: launch, emulate, visit, collect.

mod bid_endpoints;
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
mod page;
mod pages;
mod progress;
mod recorder;
mod scenario;
mod scripts;
mod tabs;
mod tracker_log;
mod visitor;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use adscan_model::{
    Capture, CaptureActivity, CaptureEnvironment, CaptureProxy, CaptureRequest, CapturedPage,
    ConsentNote, DeviceProfile, ProxyScheme, VisitorBehavior,
};
use serde_json::json;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::time::Instant;
use url::Url;

use crate::cdp::{Cdp, CdpEvent, Chrome, LaunchOptions, ProfileChoice, ProxyServer};
use crate::device::{emulation_for, window_size};
use crate::{BrowserError, CaptureHooks, detect};

use environment::Observed;
use error::CaptureError;
use interception::{Interception, ProxyLogin};
use manual::Ending;
use navigation::parse_address;
use page::{Page, TabSetup};
use pages::{PageLog, ViewFacts};
use recorder::{Recorder, epoch_ms_now};

const BODY_PATIENCE: Duration = Duration::from_secs(3);
/// A browser on the kept scan profile is asked to close and given this long
/// to write its cookies before it is killed.
const CLOSE_PATIENCE: Duration = Duration::from_secs(5);
/// Page views of one visit. A person browsing gets more than a script.
const MAX_PAGE_VIEWS: usize = 8;
const MAX_PAGE_VIEWS_MANUAL: usize = 25;

/// The directory of the kept scan profile of a device, under the root the
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

pub(crate) async fn run(
    request: CaptureRequest,
    hooks: CaptureHooks,
) -> Result<Capture, BrowserError> {
    let started = Instant::now();
    let address = parse_address(&request.url).map_err(CaptureError::from)?;
    let (proxy, proxy_login) = proxy_of(&request, &hooks)?;
    let kept_profile = match (&hooks.scan_profile_dir, request.persistent_profile) {
        (Some(root), true) => Some(scan_profile_of(root, request.device)),
        (None, true) => {
            return Err(BrowserError::Other(
                "No directory was given for the scan profile".into(),
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
            ", scan profile"
        } else {
            ""
        },
        match &proxy {
            ProxyServer::System => String::new(),
            ProxyServer::Direct => ", no proxy".to_string(),
            ProxyServer::Custom { host, .. } => format!(", through the proxy {host}"),
        },
    )));

    let mut chrome = Chrome::spawn(LaunchOptions {
        executable: Path::new(&browser.path),
        headless: request.headless,
        window_size: window_size(request.device),
        profile: kept_profile
            .as_deref()
            .map_or(ProfileChoice::Throwaway, ProfileChoice::Persistent),
        proxy: &proxy,
        driven_by_a_person: request.visitor == VisitorBehavior::Manual,
    })
    .map_err(CaptureError::from)?;
    let connected = tokio::select! {
        biased;
        _ = hooks.cancel.cancelled() => Err(CaptureError::Cancelled),
        connected = crate::cdp::connect(&mut chrome) => connected.map_err(CaptureError::from),
    };
    let outcome = match connected {
        Ok((cdp, events)) => {
            let cdp = Arc::new(cdp);
            let visit = Visit {
                request: &request,
                address: &address,
                hooks: &hooks,
                proxy_login,
                started,
            };
            let outcome = tokio::select! {
                biased;
                _ = hooks.cancel.cancelled() => Err(CaptureError::Cancelled),
                outcome = visit.scan(cdp.clone(), events) => outcome,
            };
            if chrome.keeps_its_profile() {
                // Killed, the browser would lose the cookies it has not
                // written yet; closing makes it write them.
                cdp.notify(None, "Browser.close", json!({}));
                chrome.wait_for_exit(CLOSE_PATIENCE).await;
            }
            outcome
        }
        Err(error) => Err(error),
    };
    chrome.shutdown().await;
    Ok(outcome?)
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
    proxy_login: Option<ProxyLogin>,
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
        let proxy_host = self.proxy_login.as_ref().map(|proxy| proxy.host.clone());
        let mut page = attach(cdp, events, request, self.proxy_login, self.hooks).await?;
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
        let environment = CaptureEnvironment {
            headless: request.headless,
            persistent_profile: request.persistent_profile,
            webdriver: observed.webdriver,
            user_agent_overridden: page.emulation().user_agent_override().is_some(),
            viewport_overridden: page.overrides_viewport(),
            request_hosts: Vec::new(),
        };
        page.recorder.report(environment::activity(&environment));

        let (consent_accepted, ending) = match request.visitor {
            VisitorBehavior::Manual => {
                let limit = self.started + manual::SESSION_LIMIT;
                let ending = manual::record(&mut page, &self.hooks.finish, limit).await?;
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

/// Sets the browser up for the visit and attaches to its first tab.
async fn attach(
    cdp: Arc<Cdp>,
    events: UnboundedReceiver<CdpEvent>,
    request: &CaptureRequest,
    proxy_login: Option<ProxyLogin>,
    hooks: &CaptureHooks,
) -> Result<Page, CaptureError> {
    // A scanned page must not be able to put files in the user's Downloads.
    cdp.send(
        None,
        "Browser.setDownloadBehavior",
        json!({ "behavior": "deny", "eventsEnabled": true }),
    )
    .await?;
    let version = cdp.send(None, "Browser.getVersion", json!({})).await?;
    let setup = TabSetup {
        override_viewport: overrides_viewport(request),
        emulation: emulation_for(
            request.device,
            version["userAgent"].as_str().unwrap_or_default(),
            version["product"].as_str().unwrap_or_default(),
        ),
        visitor: request.visitor,
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
        Recorder::new(main_frame_id, pages, on_activity, network)
    })
    .await?;
    if !request.headless {
        // Once: the window opens behind the app otherwise. A scan never
        // takes the focus again after that.
        page.call("Page.bringToFront", json!({})).await?;
    }
    Ok(page)
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
        page.settle_bodies(BODY_PATIENCE).await?;
    }
    page.recorder.finish_requests();
    let requests = page.recorder.request_counts();
    if page.recorder.keeps_requests() {
        page.recorder.report(CaptureActivity::progress(format!(
            "Recorded {} requests, {} of them in iframes and workers with a process of their own",
            requests.total, requests.in_child_targets
        )));
    }
    let ended_at = epoch_ms_now();
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
    use adscan_model::PageReach;

    use super::*;

    /// A page cannot close the tab it was opened in, so what a person does
    /// when they close the window is done here over the DevTools connection,
    /// which only code inside this crate can reach.
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
            driven_by_a_person: true,
        })
        .unwrap();
        let (cdp, events) = crate::cdp::connect(&mut chrome).await.unwrap();
        let cdp = Arc::new(cdp);
        let emulation = emulation_for(DeviceProfile::Desktop, "Chrome/1.0", "Chrome/1.0");
        let setup = TabSetup {
            emulation,
            override_viewport: true,
            visitor: VisitorBehavior::Manual,
        };
        let mut page = Page::attach(
            cdp.clone(),
            events,
            setup,
            Interception::new(None, false),
            |main_frame_id| {
                let pages = PageLog::new(MAX_PAGE_VIEWS_MANUAL, true);
                Recorder::new(main_frame_id, pages, Arc::new(|_| {}), None)
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
        let ending = manual::record(&mut page, &never_finished, limit)
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
    }
}
