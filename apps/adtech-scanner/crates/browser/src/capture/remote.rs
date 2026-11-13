//! What a capture does differently when its browser runs at Browserbase:
//! what it asks the service for, what it measures and records on arrival,
//! and how it reports the session. Nothing here changes what a page can
//! see beyond the timezone, which is set to the region's so that a visit
//! from Frankfurt does not carry a Californian clock.

use std::time::Duration;

use adscan_model::{
    ActivityKind, BrowserbaseCapture, BrowserbaseSessionFacts, CaptureActivity, CaptureProxy,
    CaptureRequest, ProxyScheme, VisitorBehavior,
};
use serde_json::{Value, json};

use crate::cdp::{Exit, SessionOrder, SessionState};

use super::clock::{ClockSample, Distance, distance, machine_epoch_ms};
use super::error::CaptureError;
use super::manual::SESSION_LIMIT;
use super::page::Page;
use super::patience::Patience;

/// From the request for a browser to the start of the navigation: the
/// wait at the project's limit (up to a minute), connecting, the probe.
const START_ALLOWANCE_SECONDS: u64 = 120;
/// Opening one page: the navigation and its load.
const PAGE_OPEN_ALLOWANCE_SECONDS: u64 = 45;
/// A scripted browse visit ends by itself after eight minutes.
const BROWSE_VISIT_SECONDS: u64 = 8 * 60;
/// Collecting what was recorded, and room for a slow machine.
const END_MARGIN_SECONDS: u64 = 120;

/// How many times the browser is asked for its time when the connection
/// is made. The sample with the shortest round trip is the one used.
const CLOCK_SAMPLES: usize = 8;

/// The time limit a session is given, in seconds: generously more than
/// the visit can take, because the session is released when the visit
/// ends and the limit only matters when the app is gone. The service
/// takes 60 to 21,600 seconds; `SessionOrder` keeps it within that.
pub(crate) fn session_timeout_seconds(request: &CaptureRequest) -> u64 {
    let visit = match request.visitor {
        VisitorBehavior::Manual => SESSION_LIMIT.as_secs(),
        VisitorBehavior::Browse => PAGE_OPEN_ALLOWANCE_SECONDS + BROWSE_VISIT_SECONDS,
        // The observation starts again once when consent is given.
        VisitorBehavior::None | VisitorBehavior::Scroll => {
            PAGE_OPEN_ALLOWANCE_SECONDS + 2 * u64::from(request.observe_seconds)
        }
    };
    START_ALLOWANCE_SECONDS + visit + END_MARGIN_SECONDS
}

/// The session to ask Browserbase for. `proxy_password` is the password of
/// the request's own proxy, when it has one.
pub(crate) fn session_order(
    request: &CaptureRequest,
    capture: &BrowserbaseCapture,
    proxy_password: Option<&str>,
) -> Result<SessionOrder, CaptureError> {
    let exit = match (&request.proxy, &capture.location) {
        (CaptureProxy::None, None) => Exit::Region,
        (CaptureProxy::None, Some(location)) => Exit::Location(location.clone()),
        (
            CaptureProxy::Custom {
                scheme,
                host,
                port,
                username,
            },
            None,
        ) => {
            let scheme = match scheme {
                ProxyScheme::Http => "http",
                ProxyScheme::Https => "https",
                ProxyScheme::Socks5 => "socks5",
            };
            Exit::External {
                server: format!("{scheme}://{host}:{port}"),
                username: username.clone(),
                password: username.as_ref().and(proxy_password).map(str::to_string),
            }
        }
        (CaptureProxy::System, _) => {
            return Err(CaptureError::Unsupported(
                "The system proxy of this machine does not apply to a browser at Browserbase",
            ));
        }
        (CaptureProxy::Custom { .. }, Some(_)) => {
            return Err(CaptureError::Unsupported(
                "A Browserbase location cannot be combined with a proxy of your own",
            ));
        }
    };
    Ok(SessionOrder {
        region: capture.region,
        timeout_seconds: session_timeout_seconds(request),
        settings: capture.settings,
        exit,
        context_id: capture.context_id.clone(),
        install_id: capture.install_id.clone(),
        scan_id: capture.scan_id.clone(),
    })
}

/// The timezone the capture gives the browser, if any. Without a proxy of
/// Browserbase's the browser reports America/Los_Angeles in every region;
/// with one, the service has set the timezone of the exit already.
pub(crate) fn timezone_to_apply(capture: &BrowserbaseCapture) -> Option<&'static str> {
    capture
        .location
        .is_none()
        .then(|| capture.region.timezone())
}

/// What a blank page of the browser reports, read in the page's own world
/// and in the scanner's isolated one. Recorded, never acted on.
const PROBE: &str = "(() => ({ \
    webdriver: navigator.webdriver, \
    userAgent: navigator.userAgent, \
    brands: ((navigator.userAgentData || {}).brands || []).map((b) => b.brand + ' ' + b.version), \
    timezone: Intl.DateTimeFormat().resolvedOptions().timeZone, \
    language: navigator.language, \
    languages: Array.from(navigator.languages || []) }))()";

/// What the browser reported before the capture changed anything.
#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct Seen {
    /// `navigator.webdriver` in the page's own world and in the scanner's.
    pub webdriver_main_world: Option<bool>,
    pub webdriver_isolated_world: Option<bool>,
    /// `navigator.userAgent`, and what the browser says of itself.
    pub user_agent: Option<String>,
    pub browser_user_agent: Option<String>,
    pub brands: Vec<String>,
    pub timezone: Option<String>,
    pub language: Option<String>,
    pub languages: Vec<String>,
    /// Whether the browser runs headless, from its command line or its
    /// product name. `None` when neither told.
    pub headless: Option<bool>,
}

impl Seen {
    /// The user agent the page reads names a headless browser.
    pub(crate) fn user_agent_shows_headless(&self) -> bool {
        self.user_agent
            .as_deref()
            .is_some_and(|user_agent| user_agent.contains("Headless"))
    }

    /// The part of the environment report only a remote browser has.
    pub(crate) fn report(&self) -> Value {
        json!({
            "webdriverMainWorld": self.webdriver_main_world,
            "webdriverIsolatedWorld": self.webdriver_isolated_world,
            "userAgent": self.user_agent,
            "browserUserAgent": self.browser_user_agent,
            "userAgentMatchesBrowser": match (&self.user_agent, &self.browser_user_agent) {
                (Some(page), Some(browser)) => json!(page == browser),
                _ => Value::Null,
            },
            "brands": self.brands,
            "timezone": self.timezone,
            "language": self.language,
            "languages": self.languages,
            "headlessBrowser": self.headless,
            "headlessShownInUserAgent": self.user_agent_shows_headless(),
        })
    }
}

fn texts(list: &Value) -> Vec<String> {
    list.as_array()
        .into_iter()
        .flatten()
        .filter_map(|text| text.as_str().map(str::to_string))
        .collect()
}

/// Whether the browser runs headless, as far as it tells: `command_line`
/// is the answer to `Browser.getBrowserCommandLine`, when it gave one.
pub(crate) fn headless_of(command_line: Option<&Value>, product: &str) -> Option<bool> {
    if product.starts_with("Headless") {
        return Some(true);
    }
    let arguments = command_line?["arguments"].as_array()?;
    Some(arguments.iter().any(|argument| {
        argument
            .as_str()
            .is_some_and(|argument| argument == "--headless" || argument.starts_with("--headless="))
    }))
}

/// What was learned on arrival in the remote browser.
pub(crate) struct Arrival {
    pub seen: Seen,
    pub distance: Option<Distance>,
    pub applied_timezone: Option<&'static str>,
}

impl Arrival {
    pub(crate) fn facts(
        &self,
        session_id: &str,
        browser_version: Option<&str>,
    ) -> BrowserbaseSessionFacts {
        BrowserbaseSessionFacts {
            session_id: session_id.to_string(),
            browser_version: browser_version.map(str::to_string),
            original_timezone: self.seen.timezone.clone(),
            applied_timezone: self.applied_timezone.map(str::to_string),
            language: self.seen.language.clone(),
            round_trip_ms: self.distance.map(|distance| distance.round_trip_ms),
            clock_offset_ms: self.distance.map(|distance| distance.clock_offset_ms),
        }
    }
}

impl Page {
    /// Asks the browser for its time a few times over.
    async fn sample_clock(&mut self) -> Result<Vec<ClockSample>, CaptureError> {
        let mut samples = Vec::with_capacity(CLOCK_SAMPLES);
        for _ in 0..CLOCK_SAMPLES {
            let asked_ms = machine_epoch_ms();
            let answer = self
                .call(
                    "Runtime.evaluate",
                    json!({ "expression": "Date.now()", "returnByValue": true }),
                )
                .await?;
            let answered_ms = machine_epoch_ms();
            if let Some(browser_ms) = answer["result"]["value"].as_f64() {
                samples.push(ClockSample {
                    asked_ms,
                    answered_ms,
                    browser_ms,
                });
            }
        }
        Ok(samples)
    }

    /// Everything that happens between connecting to a remote browser and
    /// the first navigation, on the blank page the browser starts with:
    /// measures how far away the browser is and sets the capture's clock
    /// and patience by it, records what a page can see, and gives the
    /// browser the timezone of its region. `version` is the answer to
    /// `Browser.getVersion`.
    pub(crate) async fn arrive_at_remote_browser(
        &mut self,
        capture: &BrowserbaseCapture,
        version: &Value,
    ) -> Result<Arrival, CaptureError> {
        let measured = distance(&self.sample_clock().await?);
        if let Some(measured) = measured {
            let round_trip = Duration::from_secs_f64(measured.round_trip_ms.max(0.0) / 1_000.0);
            self.cdp.allow_for_distance(round_trip);
            self.patience = Patience::across(round_trip);
            self.recorder.clock = measured.clock();
        }

        let in_main_world = self
            .call(
                "Runtime.evaluate",
                json!({ "expression": PROBE, "returnByValue": true }),
            )
            .await?;
        let main = &in_main_world["result"]["value"];
        let isolated = self.evaluate(PROBE).await?.unwrap_or_default();
        let command_line = self
            .cdp
            .send(None, "Browser.getBrowserCommandLine", json!({}));
        let command_line = self.record_until_answered(command_line).await.ok();
        let text = |value: &Value| value.as_str().map(str::to_string);
        let seen = Seen {
            webdriver_main_world: main["webdriver"].as_bool(),
            webdriver_isolated_world: isolated["webdriver"].as_bool(),
            user_agent: text(&main["userAgent"]),
            browser_user_agent: text(&version["userAgent"]),
            brands: texts(&main["brands"]),
            timezone: text(&main["timezone"]),
            language: text(&main["language"]),
            languages: texts(&main["languages"]),
            headless: headless_of(
                command_line.as_ref(),
                version["product"].as_str().unwrap_or_default(),
            ),
        };

        let applied_timezone = timezone_to_apply(capture);
        if let Some(timezone) = applied_timezone {
            self.set_timezone(timezone).await?;
        }
        Ok(Arrival {
            seen,
            distance: measured,
            applied_timezone,
        })
    }
}

// ---------------------------------------------------------- activity lines

fn activity(message: String, detail: Value) -> CaptureActivity {
    CaptureActivity {
        kind: ActivityKind::RemoteBrowser,
        message,
        page: None,
        data: Some(detail),
    }
}

pub(crate) fn session_created(session_id: &str, order: &SessionOrder) -> CaptureActivity {
    let region = order.region.as_str();
    let timeout = order.timeout();
    activity(
        format!(
            "Browserbase started a browser in {region} (session {session_id}, time limit {timeout} s)"
        ),
        json!({
            "event": "sessionCreated",
            "sessionId": session_id,
            "region": region,
            "proxy": order.exit.kind(),
            "context": order.context_id.is_some(),
            "timeoutSeconds": timeout,
        }),
    )
}

pub(crate) fn rate_limited(attempt: u32, wait: Duration) -> CaptureActivity {
    let seconds = wait.as_secs();
    activity(
        format!("Browserbase is at its limit of browsers; trying again in {seconds} s"),
        json!({ "event": "rateLimited", "waitSeconds": seconds, "attempt": attempt }),
    )
}

pub(crate) fn connected(facts: &BrowserbaseSessionFacts) -> CaptureActivity {
    let browser = facts.browser_version.as_deref().unwrap_or("the browser");
    let message = match facts.round_trip_ms {
        Some(round_trip) => format!("Connected to {browser}, round trip {round_trip:.0} ms"),
        None => format!("Connected to {browser}"),
    };
    activity(
        message,
        json!({
            "event": "connected",
            "browserVersion": facts.browser_version,
            "roundTripMs": facts.round_trip_ms,
            "clockOffsetMs": facts.clock_offset_ms,
        }),
    )
}

pub(crate) fn timezone(facts: &BrowserbaseSessionFacts) -> CaptureActivity {
    let original = facts.original_timezone.as_deref().unwrap_or("none");
    let message = match &facts.applied_timezone {
        Some(applied) => format!(
            "Timezone set to {applied} to match the region (the browser reported {original})"
        ),
        None => format!(
            "The browser reports the timezone {original}, set by Browserbase for its proxy; \
             left as it is"
        ),
    };
    activity(
        message,
        json!({
            "event": "timezone",
            "original": facts.original_timezone,
            "applied": facts.applied_timezone,
        }),
    )
}

pub(crate) fn live_view() -> CaptureActivity {
    activity(
        "The browser is ready for you in Browserbase's live view".into(),
        json!({ "event": "liveView" }),
    )
}

pub(crate) fn bodies_too_large(count: u32, largest_bytes: u64) -> CaptureActivity {
    let message = match count {
        1 => "1 response body was too large for the connection to the remote browser and was \
              not read"
            .to_string(),
        count => format!(
            "{count} response bodies were too large for the connection to the remote browser \
             and were not read"
        ),
    };
    activity(
        message,
        json!({ "event": "bodiesTooLarge", "count": count, "largestBytes": largest_bytes }),
    )
}

/// The service's own scripts in the pages' world: recorded, because they
/// are part of what ran there, and kept out of the scan's scripts.
pub(crate) fn service_scripts(count: u32, chars: u64) -> CaptureActivity {
    activity(
        format!(
            "Browserbase ran {count} scripts of its own in the page ({chars} characters); they \
             are not among the scan's scripts"
        ),
        json!({ "event": "serviceScripts", "count": count, "characters": chars }),
    )
}

pub(crate) fn connection_lost() -> CaptureActivity {
    activity(
        "The connection to the Browserbase browser was lost".into(),
        json!({ "event": "connectionLost" }),
    )
}

pub(crate) fn released(ended: &SessionState) -> CaptureActivity {
    let message = match ended.seconds() {
        Some(seconds) => format!("Browserbase ended the browser after {seconds:.0} s"),
        None => "Browserbase ended the browser".to_string(),
    };
    activity(
        message,
        json!({
            "event": "released",
            "sessionSeconds": ended.seconds(),
            "proxyBytes": ended.proxy_bytes,
        }),
    )
}

pub(crate) fn release_unconfirmed() -> CaptureActivity {
    activity(
        "Browserbase did not confirm that the browser has ended; it ends by itself at its time \
         limit at the latest"
            .into(),
        json!({ "event": "releaseUnconfirmed" }),
    )
}

#[cfg(test)]
mod tests {
    use adscan_model::{
        BrowserbaseLocation, BrowserbaseRegion, BrowserbaseSessionSettings, CaptureBrowser,
        DeviceProfile,
    };

    use super::*;

    fn capture() -> BrowserbaseCapture {
        BrowserbaseCapture {
            region: BrowserbaseRegion::EuCentral1,
            settings: BrowserbaseSessionSettings::STANDARD,
            location: None,
            context_id: None,
            install_id: "0123456789abcdef0123456789abcdef".into(),
            scan_id: "7c9e6679-7425-40de-944b-e07fc1f90ae7".into(),
        }
    }

    fn request(
        visitor: VisitorBehavior,
        observe_seconds: u32,
        proxy: CaptureProxy,
    ) -> CaptureRequest {
        CaptureRequest {
            url: "https://news.example/".into(),
            device: DeviceProfile::Desktop,
            observe_seconds,
            visitor,
            extra_pages: 2,
            seed: 1,
            accept_consent: true,
            headless: true,
            persistent_profile: false,
            proxy,
            chrome_path: None,
            browser: CaptureBrowser::Browserbase(capture()),
        }
    }

    fn own_proxy(scheme: ProxyScheme, username: Option<&str>) -> CaptureProxy {
        CaptureProxy::Custom {
            scheme,
            host: "proxy.example.net".into(),
            port: 3128,
            username: username.map(str::to_string),
        }
    }

    #[test]
    fn the_time_limit_covers_the_longest_the_visit_can_take() {
        let limit = |visitor, observe| {
            session_timeout_seconds(&request(visitor, observe, CaptureProxy::None))
        };
        // Start 120 + open 45 + twice the observation + margin 120.
        assert_eq!(limit(VisitorBehavior::None, 20), 325);
        assert_eq!(limit(VisitorBehavior::Scroll, 20), 325);
        assert_eq!(limit(VisitorBehavior::Scroll, 0), 285);
        assert_eq!(limit(VisitorBehavior::Scroll, 120), 525);
        // Start + open + the eight minutes of a browse visit + margin.
        assert_eq!(limit(VisitorBehavior::Browse, 20), 765);
        // Start + the fifteen minutes of a manual visit + margin.
        assert_eq!(limit(VisitorBehavior::Manual, 20), 1_140);
        // Whatever is asked for stays inside what the service takes.
        let longest = request(VisitorBehavior::Scroll, u32::MAX, CaptureProxy::None);
        let order = session_order(&longest, &capture(), None).ok().unwrap();
        assert_eq!(order.timeout(), 21_600);
    }

    #[test]
    fn the_exit_follows_the_requests_proxy_and_the_chosen_location() {
        let exit = |proxy: CaptureProxy, location: Option<BrowserbaseLocation>, password| {
            let capture = BrowserbaseCapture {
                location,
                ..capture()
            };
            session_order(
                &request(VisitorBehavior::None, 5, proxy),
                &capture,
                password,
            )
            .map(|order| order.exit)
        };
        let belgium = BrowserbaseLocation {
            country: "BE".into(),
            state: None,
            city: None,
        };

        assert!(exit(CaptureProxy::None, None, None).ok() == Some(Exit::Region));
        assert!(
            exit(CaptureProxy::None, Some(belgium.clone()), None).ok()
                == Some(Exit::Location(belgium.clone()))
        );
        assert!(
            exit(
                own_proxy(ProxyScheme::Http, Some("scanner")),
                None,
                Some("pa55")
            )
            .ok()
                == Some(Exit::External {
                    server: "http://proxy.example.net:3128".into(),
                    username: Some("scanner".into()),
                    password: Some("pa55".into()),
                })
        );
        // A password without a user name is no login.
        assert!(
            exit(own_proxy(ProxyScheme::Https, None), None, Some("pa55")).ok()
                == Some(Exit::External {
                    server: "https://proxy.example.net:3128".into(),
                    username: None,
                    password: None,
                })
        );
        assert!(
            exit(own_proxy(ProxyScheme::Socks5, None), None, None).ok()
                == Some(Exit::External {
                    server: "socks5://proxy.example.net:3128".into(),
                    username: None,
                    password: None,
                })
        );
        assert!(exit(CaptureProxy::System, None, None).is_err());
        assert!(exit(own_proxy(ProxyScheme::Http, None), Some(belgium), None).is_err());
    }

    #[test]
    fn the_order_carries_the_region_settings_context_and_ids_of_the_capture() {
        let capture = BrowserbaseCapture {
            context_id: Some("context-1".into()),
            ..capture()
        };
        let order = session_order(
            &request(VisitorBehavior::Scroll, 20, CaptureProxy::None),
            &capture,
            None,
        )
        .ok()
        .unwrap();

        assert_eq!(order.region, BrowserbaseRegion::EuCentral1);
        assert_eq!(order.settings, BrowserbaseSessionSettings::STANDARD);
        assert_eq!(order.context_id.as_deref(), Some("context-1"));
        assert_eq!(order.install_id, "0123456789abcdef0123456789abcdef");
        assert_eq!(order.scan_id, "7c9e6679-7425-40de-944b-e07fc1f90ae7");
        assert_eq!(order.timeout_seconds, 325);
        // The scanned address is no part of it.
        let body = order.body("project").to_string();
        assert!(!body.contains("news.example"), "{body}");
    }

    #[test]
    fn the_timezone_follows_the_region_unless_browserbase_set_one_for_its_proxy() {
        let in_region = |region| BrowserbaseCapture {
            region,
            ..capture()
        };
        assert_eq!(
            timezone_to_apply(&in_region(BrowserbaseRegion::EuCentral1)),
            Some("Europe/Berlin")
        );
        assert_eq!(
            timezone_to_apply(&in_region(BrowserbaseRegion::UsEast1)),
            Some("America/New_York")
        );
        assert_eq!(
            timezone_to_apply(&in_region(BrowserbaseRegion::UsWest2)),
            Some("America/Los_Angeles")
        );
        assert_eq!(
            timezone_to_apply(&in_region(BrowserbaseRegion::ApSoutheast1)),
            Some("Asia/Singapore")
        );
        let with_location = BrowserbaseCapture {
            location: Some(BrowserbaseLocation {
                country: "BE".into(),
                state: None,
                city: None,
            }),
            ..capture()
        };
        assert_eq!(timezone_to_apply(&with_location), None);
    }

    #[test]
    fn headless_is_read_from_the_command_line_or_the_product_and_not_guessed() {
        let measured = json!({ "arguments": [
            "/usr/bin/chromium-browser",
            "--disable-blink-features=AutomationControlled",
            "--start-maximized",
            "--headless=new",
        ] });
        assert_eq!(
            headless_of(Some(&measured), "Chrome/157.0.8103.79"),
            Some(true)
        );
        let old_style = json!({ "arguments": ["chrome", "--headless"] });
        assert_eq!(
            headless_of(Some(&old_style), "Chrome/120.0.0.0"),
            Some(true)
        );
        let visible = json!({ "arguments": ["chrome", "--headless-not", "--window-size=1,1"] });
        assert_eq!(
            headless_of(Some(&visible), "Chrome/157.0.8103.79"),
            Some(false)
        );
        assert_eq!(headless_of(None, "HeadlessChrome/153.0.0.0"), Some(true));
        assert_eq!(headless_of(None, "Chrome/157.0.8103.79"), None);
        assert_eq!(headless_of(Some(&json!({})), "Chrome/157.0.8103.79"), None);
    }

    #[test]
    fn what_was_seen_is_reported_as_it_was() {
        let user_agent = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) \
                          Chrome/157.0.0.0 Safari/537.36";
        let seen = Seen {
            webdriver_main_world: Some(false),
            webdriver_isolated_world: Some(false),
            user_agent: Some(user_agent.into()),
            browser_user_agent: Some(user_agent.into()),
            brands: vec!["Not-A.Brand 99".into(), "Google Chrome 157".into()],
            timezone: Some("America/Los_Angeles".into()),
            language: Some("en-US".into()),
            languages: vec!["en-US".into(), "en".into()],
            headless: Some(true),
        };

        assert_eq!(
            seen.report(),
            json!({
                "webdriverMainWorld": false,
                "webdriverIsolatedWorld": false,
                "userAgent": user_agent,
                "browserUserAgent": user_agent,
                "userAgentMatchesBrowser": true,
                "brands": ["Not-A.Brand 99", "Google Chrome 157"],
                "timezone": "America/Los_Angeles",
                "language": "en-US",
                "languages": ["en-US", "en"],
                "headlessBrowser": true,
                "headlessShownInUserAgent": false,
            })
        );
        assert!(!seen.user_agent_shows_headless());
        let local = Seen {
            user_agent: Some("Mozilla/5.0 HeadlessChrome/153.0.0.0".into()),
            ..Seen::default()
        };
        assert!(local.user_agent_shows_headless());
        assert_eq!(local.report()["userAgentMatchesBrowser"], Value::Null);
    }

    #[test]
    fn the_sessions_lines_say_what_happened_without_a_credential() {
        let order = session_order(
            &request(
                VisitorBehavior::Scroll,
                20,
                own_proxy(ProxyScheme::Http, Some("scanner")),
            ),
            &capture(),
            Some("pa55w0rd"),
        )
        .ok()
        .unwrap();
        let created = session_created("5e551001-0000-4000-8000-000000000001", &order);
        assert_eq!(
            created.message,
            "Browserbase started a browser in eu-central-1 (session \
             5e551001-0000-4000-8000-000000000001, time limit 325 s)"
        );
        assert_eq!(
            created.data,
            Some(json!({
                "event": "sessionCreated",
                "sessionId": "5e551001-0000-4000-8000-000000000001",
                "region": "eu-central-1",
                "proxy": "external",
                "context": false,
                "timeoutSeconds": 325,
            }))
        );
        assert_eq!(created.kind, ActivityKind::RemoteBrowser);
        assert_eq!(created.page, None);
        let said = format!("{created:?}");
        assert!(
            !said.contains("pa55w0rd") && !said.contains("proxy.example.net"),
            "{said}"
        );

        assert_eq!(
            rate_limited(2, Duration::from_secs(5)).data,
            Some(json!({ "event": "rateLimited", "waitSeconds": 5, "attempt": 2 }))
        );
        assert_eq!(
            bodies_too_large(1, 9_000_000).message,
            "1 response body was too large for the connection to the remote browser and was not read"
        );
        assert_eq!(
            bodies_too_large(2, 40_000_000).data,
            Some(json!({ "event": "bodiesTooLarge", "count": 2, "largestBytes": 40_000_000_u64 }))
        );
        assert_eq!(live_view().data, Some(json!({ "event": "liveView" })));
        assert_eq!(
            service_scripts(5, 271_667).data,
            Some(json!({ "event": "serviceScripts", "count": 5, "characters": 271_667 }))
        );
        assert_eq!(
            connection_lost().data,
            Some(json!({ "event": "connectionLost" }))
        );
    }

    #[test]
    fn the_connection_and_timezone_lines_carry_what_was_measured() {
        let arrival = Arrival {
            seen: Seen {
                timezone: Some("America/Los_Angeles".into()),
                language: Some("en-US".into()),
                ..Seen::default()
            },
            distance: Some(Distance {
                round_trip_ms: 118.4,
                clock_offset_ms: -53.0,
            }),
            applied_timezone: Some("Europe/Berlin"),
        };
        let facts = arrival.facts("session-1", Some("Chrome/157.0.8103.79"));

        assert_eq!(
            facts,
            BrowserbaseSessionFacts {
                session_id: "session-1".into(),
                browser_version: Some("Chrome/157.0.8103.79".into()),
                original_timezone: Some("America/Los_Angeles".into()),
                applied_timezone: Some("Europe/Berlin".into()),
                language: Some("en-US".into()),
                round_trip_ms: Some(118.4),
                clock_offset_ms: Some(-53.0),
            }
        );
        assert_eq!(
            connected(&facts).message,
            "Connected to Chrome/157.0.8103.79, round trip 118 ms"
        );
        assert_eq!(
            connected(&facts).data,
            Some(json!({
                "event": "connected",
                "browserVersion": "Chrome/157.0.8103.79",
                "roundTripMs": 118.4,
                "clockOffsetMs": -53.0,
            }))
        );
        assert_eq!(
            timezone(&facts).message,
            "Timezone set to Europe/Berlin to match the region (the browser reported \
             America/Los_Angeles)"
        );
        let left_alone = BrowserbaseSessionFacts {
            original_timezone: Some("Europe/Brussels".into()),
            applied_timezone: None,
            ..facts
        };
        assert_eq!(
            timezone(&left_alone).data,
            Some(json!({ "event": "timezone", "original": "Europe/Brussels", "applied": null }))
        );
    }

    #[test]
    fn the_release_line_states_how_long_the_browser_ran() {
        let ended = SessionState {
            id: "session-1".into(),
            running: false,
            started_at_ms: Some(1_000.0),
            ended_at_ms: Some(42_400.0),
            proxy_bytes: Some(131_844.0),
            app: None,
            install: None,
        };
        let line = released(&ended);
        assert_eq!(line.message, "Browserbase ended the browser after 41 s");
        assert_eq!(
            line.data,
            Some(json!({ "event": "released", "sessionSeconds": 41.4, "proxyBytes": 131_844.0 }))
        );
    }
}
