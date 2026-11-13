//! Captures in a browser that is reached the way Browserbase's are: started
//! and ended over a REST API, driven over one WebSocket. The service is
//! `support::browserbase::Cloud`, a stand-in with the measured shapes and a
//! local Chromium per session; nothing here reaches the network.

mod support;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use adscan_browser::{
    BrowserError, Browserbase, BrowserbaseEndpoint, BrowserbaseError, BrowserbaseHooks,
};
use adscan_model::{
    ActivityKind, BrowserbaseCapture, BrowserbaseCredentials, BrowserbaseLocation,
    BrowserbaseRegion, BrowserbaseSessionSettings, Capture, CaptureBrowser, CaptureProxy,
    CaptureRequest, CapturedBody, DeviceProfile, NetworkRecord, PageReach, ProxyScheme,
    RemoteSessionEvent, RequestContext, RequestEnd, ResourceKind, VisitorBehavior,
};
use serde_json::{Value, json};
use support::browserbase::{API_KEY, Cloud, OTHER_PROJECT_ID, PROJECT_ID, PageClient, SessionView};
use support::{Fixtures, Outcome, Reported, Requests};

const INSTALL: &str = "0123456789abcdef0123456789abcdef";
const SCAN: &str = "7c9e6679-7425-40de-944b-e07fc1f90ae7";
const PROXY_PASSWORD: &str = "pr0xy-pa55word-that-must-not-leak";

fn credentials() -> BrowserbaseCredentials {
    BrowserbaseCredentials {
        api_key: API_KEY.to_string(),
        project_id: PROJECT_ID.to_string(),
    }
}

/// The stand-in service, a client of it, and what captures told the engine
/// about their sessions.
struct Remote {
    cloud: Cloud,
    client: Browserbase,
    events: Arc<Mutex<Vec<RemoteSessionEvent>>>,
}

impl Remote {
    /// `None` on a machine without a Chromium to stand in for the cloud's.
    async fn start() -> Option<Self> {
        let cloud = Cloud::start().await?;
        let client = Browserbase::new(BrowserbaseEndpoint {
            api_url: cloud.base_url.clone(),
            connect_url: None,
        })
        .unwrap();
        Some(Self {
            cloud,
            client,
            events: Arc::default(),
        })
    }

    fn capture(&self) -> BrowserbaseCapture {
        BrowserbaseCapture {
            region: BrowserbaseRegion::EuCentral1,
            settings: BrowserbaseSessionSettings::STANDARD,
            location: None,
            context_id: None,
            install_id: INSTALL.to_string(),
            scan_id: SCAN.to_string(),
        }
    }

    /// A request for a visit of `url` in a browser of the stand-in.
    fn request(&self, url: &str) -> CaptureRequest {
        CaptureRequest {
            proxy: CaptureProxy::None,
            chrome_path: None,
            browser: CaptureBrowser::Browserbase(self.capture()),
            ..support::request(url, DeviceProfile::Desktop)
        }
    }

    /// Hooks that collect what the capture reports and tells the engine.
    fn reported(&self) -> Reported {
        let mut reported = Reported::new();
        let events = self.events.clone();
        reported.hooks.browserbase = Some(BrowserbaseHooks {
            client: self.client.clone(),
            credentials: credentials(),
            on_session: Arc::new(move |event| events.lock().unwrap().push(event)),
        });
        reported
    }

    fn events(&self) -> Vec<RemoteSessionEvent> {
        self.events.lock().unwrap().clone()
    }

    /// The one session of this stand-in.
    fn only_session(&self) -> SessionView {
        let sessions = self.cloud.sessions();
        assert_eq!(sessions.len(), 1, "{sessions:#?}");
        sessions.into_iter().next().unwrap()
    }

    /// Waits until the stand-in has a session that matches.
    async fn until_a_session(
        &self,
        what: &str,
        matches: impl Fn(&SessionView) -> bool,
    ) -> SessionView {
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            if let Some(session) = self.cloud.sessions().into_iter().find(&matches) {
                return session;
            }
            assert!(
                Instant::now() < deadline,
                "no session {what}: {:#?}",
                self.cloud.sessions()
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    /// Waits until the engine was told that a person can take over, for
    /// the time number `after` + 1, and returns the session's id.
    async fn live_view_ready(&self, after: usize) -> String {
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            let ready: Vec<String> = self
                .events()
                .into_iter()
                .filter_map(|event| match event {
                    RemoteSessionEvent::LiveViewReady { session_id } => Some(session_id),
                    _ => None,
                })
                .collect();
            if let Some(session_id) = ready.get(after) {
                return session_id.clone();
            }
            assert!(Instant::now() < deadline, "{:?}", self.events());
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    /// Nothing the capture reported, returned or failed with holds a
    /// credential: not the API key, not the address or key of a browser,
    /// not a live-view address, not a proxy password.
    fn assert_nothing_leaked(&self, outcome: &Outcome) {
        let mut said = Vec::new();
        for activity in &outcome.activities {
            said.push(("an activity line", activity.message.clone()));
            said.push(("an activity's data", format!("{:?}", activity.data)));
        }
        match &outcome.result {
            Ok(capture) => {
                said.push((
                    "the capture's environment",
                    format!("{:?}", capture.environment),
                ));
                said.push(("the capture's console", format!("{:?}", capture.console)));
            }
            Err(error) => {
                said.push(("the error", error.to_string()));
                said.push(("the error's debug form", format!("{error:?}")));
            }
        }
        said.push(("a session event", format!("{:?}", self.events())));
        for (what, text) in said {
            self.cloud.assert_no_secret_in(&text, what);
            assert!(
                !text.contains(PROXY_PASSWORD),
                "{what} holds the proxy password: {text}"
            );
            assert!(
                !text.contains(PROJECT_ID),
                "{what} holds the project id: {text}"
            );
        }
    }
}

/// The capture of an outcome that must have succeeded.
#[track_caller]
fn captured(outcome: &Outcome) -> &Capture {
    outcome.result.as_ref().unwrap_or_else(|error| {
        panic!("capture failed: {error}\nprogress: {:#?}", outcome.progress)
    })
}

fn remote_events(outcome: &Outcome) -> Vec<(String, Value)> {
    outcome
        .activities_of(ActivityKind::RemoteBrowser)
        .iter()
        .map(|activity| {
            let detail = activity.data.clone().unwrap_or_default();
            (
                detail["event"].as_str().unwrap_or_default().to_string(),
                detail,
            )
        })
        .collect()
}

fn names(events: &[(String, Value)]) -> Vec<&str> {
    events.iter().map(|(name, _)| name.as_str()).collect()
}

fn path_of(record: &NetworkRecord) -> String {
    let url = url::Url::parse(&record.url).unwrap();
    format!(
        "{}:{}{}",
        url.host_str().unwrap_or_default(),
        url.port().unwrap_or(0),
        url.path()
    )
}

fn kind_name(event: &adscan_model::TrackerEvent) -> String {
    serde_json::to_value(&event.kind).unwrap()["type"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

/// What a capture of the demo site must show whichever way its browser is
/// reached: what was requested and from where, the scripts as they were
/// served, and what the tracker saw of the ad stack.
struct Essentials {
    requests: BTreeSet<(String, String, String)>,
    script_bodies: BTreeMap<String, Vec<u8>>,
    scripts: BTreeSet<String>,
    tracker_kinds: BTreeSet<String>,
    auctions: usize,
    ad_units: BTreeSet<String>,
    bidders_with_a_bid: BTreeSet<String>,
    slots_rendered: BTreeSet<String>,
    title: Option<String>,
    final_url: String,
    webdriver: Option<bool>,
    frames_in_child_targets: usize,
}

fn essentials(capture: &Capture, records: &[NetworkRecord]) -> Essentials {
    use adscan_model::TrackerEventKind as Kind;
    let finished = |record: &&NetworkRecord| record.end == RequestEnd::Finished;
    let mut essentials = Essentials {
        requests: records
            .iter()
            .filter(finished)
            // Fired when the visit ends, or not, as the moment has it.
            .filter(|record| !matches!(record.kind, ResourceKind::Ping | ResourceKind::Image))
            .map(|record| {
                (
                    path_of(record),
                    format!("{:?}", record.kind),
                    format!("{:?}", record.context),
                )
            })
            .collect(),
        script_bodies: records
            .iter()
            .filter(finished)
            .filter(|record| record.kind == ResourceKind::Script)
            .filter_map(|record| match &record.body {
                CapturedBody::Read { bytes, .. } => Some((path_of(record), bytes.clone())),
                _ => None,
            })
            .collect(),
        scripts: capture
            .scripts
            .iter()
            .filter(|script| !script.inline)
            .map(|script| script.url.clone())
            .collect(),
        tracker_kinds: capture.events.iter().map(kind_name).collect(),
        auctions: 0,
        ad_units: BTreeSet::new(),
        bidders_with_a_bid: BTreeSet::new(),
        slots_rendered: BTreeSet::new(),
        title: capture.page_title.clone(),
        final_url: capture.final_url.clone(),
        webdriver: capture.environment.webdriver,
        frames_in_child_targets: records
            .iter()
            .filter(|record| record.context == RequestContext::Iframe && record.target_id.is_some())
            .count(),
    };
    for event in &capture.events {
        match &event.kind {
            Kind::AuctionInit { ad_units, .. } => {
                essentials.auctions += 1;
                essentials
                    .ad_units
                    .extend(ad_units.iter().map(|unit| unit.code.clone()));
            }
            Kind::BidResponse { bidder, .. } => {
                essentials.bidders_with_a_bid.insert(bidder.clone());
            }
            Kind::SlotRenderEnded { element_id, .. } => {
                essentials.slots_rendered.insert(element_id.clone());
            }
            _ => {}
        }
    }
    essentials
}

/// The same visit of the demo site, once in Chrome on its pipe and once in
/// a browser behind the stand-in service: what the scan is about is the
/// same, and the session is asked for, set up and ended as decided.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_visit_over_the_websocket_records_what_the_same_visit_over_the_pipe_does() {
    let Some(remote) = Remote::start().await else {
        return;
    };
    let site = adscan_browser::start_demo_site().await.unwrap();
    let visit = |mut request: CaptureRequest| {
        request.visitor = VisitorBehavior::Scroll;
        request.observe_seconds = 9;
        request
    };

    let over_the_pipe = Requests::keeping_text();
    let mut local = Reported::new();
    local.hooks.network = Some(over_the_pipe.clone());
    let local = local
        .run(visit(support::request(&site.url, DeviceProfile::Desktop)))
        .await;
    let over_the_socket = Requests::keeping_text();
    let mut cloud = remote.reported();
    cloud.hooks.network = Some(over_the_socket.clone());
    let cloud = cloud.run(visit(remote.request(&site.url))).await;

    let pipe = essentials(captured(&local), &over_the_pipe.records());
    let socket = essentials(captured(&cloud), &over_the_socket.records());
    assert_eq!(socket.requests, pipe.requests);
    assert_eq!(
        socket.script_bodies.keys().collect::<Vec<_>>(),
        pipe.script_bodies.keys().collect::<Vec<_>>()
    );
    for (script, body) in &pipe.script_bodies {
        assert!(
            &socket.script_bodies[script] == body,
            "{script} was read differently"
        );
    }
    assert!(
        pipe.script_bodies.len() >= 4,
        "{:?}",
        pipe.script_bodies.keys()
    );
    assert_eq!(socket.scripts, pipe.scripts);
    assert_eq!(socket.tracker_kinds, pipe.tracker_kinds);
    assert_eq!(socket.auctions, pipe.auctions);
    assert_eq!(socket.ad_units, pipe.ad_units);
    assert_eq!(socket.bidders_with_a_bid, pipe.bidders_with_a_bid);
    assert_eq!(socket.slots_rendered, pipe.slots_rendered);
    assert_eq!(
        (&socket.title, &socket.final_url),
        (&pipe.title, &pipe.final_url)
    );
    assert_eq!(socket.webdriver, pipe.webdriver);
    assert!(pipe.auctions >= 1 && !pipe.bidders_with_a_bid.is_empty());
    assert_eq!(
        socket.frames_in_child_targets, 0,
        "the stand-in has no frame in a process of its own"
    );

    // What was asked of the service, field by field.
    let session = remote.only_session();
    assert_eq!(
        session.body,
        json!({
            "projectId": PROJECT_ID,
            "region": "eu-central-1",
            // Start 120 + open 45 + twice the 9 s observation + margin 120.
            "timeout": 303,
            "keepAlive": false,
            "browserSettings": {
                "solveCaptchas": true,
                "recordSession": true,
                "logSession": true,
                "blockAds": false,
            },
            "userMetadata": { "app": "adscan", "install": INSTALL, "scan": SCAN },
        })
    );
    assert!(
        !session.body.to_string().contains("127.0.0.1"),
        "the scanned address was sent"
    );

    // How the browser was set up: one client, the viewport and the
    // region's timezone given, nothing brought to the front, and no proxy
    // login armed.
    assert_eq!(session.connections, 1);
    assert!(session.sent("Page.bringToFront").is_empty());
    assert!(session.sent("Target.createTarget").is_empty());
    assert!(session.sent("Target.createBrowserContext").is_empty());
    assert!(session.sent("Fetch.continueWithAuth").is_empty());
    assert!(session.sent("Fetch.disable").is_empty());
    assert_eq!(session.sent("Emulation.setDeviceMetricsOverride").len(), 1);
    let timezones: Vec<&Value> = session
        .sent("Emulation.setTimezoneOverride")
        .iter()
        .map(|command| &command.params)
        .collect();
    assert_eq!(timezones, [&json!({ "timezoneId": "Europe/Berlin" })]);
    for paused in session.sent("Fetch.enable") {
        assert_ne!(paused.params["handleAuthRequests"], true, "{paused:?}");
    }
    assert_eq!(session.sent("Browser.close").len(), 1);
    assert_eq!(session.release_requests, 1);
    assert!(!session.running);

    // What the engine and the audit trail were told.
    let events = remote_events(&cloud);
    assert_eq!(
        names(&events),
        [
            "sessionCreated",
            "connected",
            "timezone",
            "serviceScripts",
            "released"
        ]
    );
    assert_eq!(
        events[0].1,
        json!({
            "event": "sessionCreated",
            "sessionId": session.id,
            "region": "eu-central-1",
            "proxy": "none",
            "context": false,
            "timeoutSeconds": 303,
        })
    );
    assert!(
        events[1].1["browserVersion"]
            .as_str()
            .unwrap()
            .contains("Chrome/")
    );
    let round_trip = events[1].1["roundTripMs"].as_f64().unwrap();
    let clock_offset = events[1].1["clockOffsetMs"].as_f64().unwrap();
    assert!((0.0..500.0).contains(&round_trip), "{round_trip}");
    assert!(
        clock_offset.abs() < 500.0,
        "the same machine's clock: {clock_offset}"
    );
    assert_eq!(events[2].1["applied"], "Europe/Berlin");
    assert!(events[2].1["original"].is_string());
    assert!(events[4].1["sessionSeconds"].as_f64().unwrap() >= 9.0);
    assert!(matches!(
        remote.events().as_slice(),
        [
            RemoteSessionEvent::Created { session_id: created },
            RemoteSessionEvent::Released { session_id: released, session_seconds: Some(seconds), .. },
        ] if *created == session.id && *released == session.id && *seconds >= 9.0
    ));

    // What the service itself ran in the page is told, and is not taken
    // for the page's: not its scripts, not its heartbeat in the console.
    let of_the_service = &events[3].1;
    assert_eq!(of_the_service["count"], 1, "{of_the_service}");
    assert!(of_the_service["characters"].as_u64().unwrap() > 200);
    let inline = |capture: &Capture| {
        capture
            .scripts
            .iter()
            .filter(|script| script.inline)
            .count()
    };
    assert_eq!(inline(captured(&cloud)), inline(captured(&local)));
    for script in &captured(&cloud).scripts {
        assert!(!script.body.contains("browserbase"), "{}", script.url);
    }
    let heartbeats = |capture: &Capture| {
        capture
            .console
            .iter()
            .filter(|line| line.text.to_lowercase().contains("browserbase"))
            .count()
    };
    assert_eq!(heartbeats(captured(&cloud)), 0);
    assert!(!captured(&cloud).console.is_empty());

    // What the capture says of its browser.
    let environment = &captured(&cloud).environment;
    assert!(environment.headless && environment.viewport_overridden);
    assert!(!environment.persistent_profile);
    let facts = environment.browserbase.as_ref().unwrap();
    assert_eq!(facts.session_id, session.id);
    assert_eq!(facts.applied_timezone.as_deref(), Some("Europe/Berlin"));
    assert_eq!(facts.round_trip_ms, Some(round_trip));
    assert_eq!(facts.clock_offset_ms, Some(clock_offset));
    assert!(facts.language.is_some() && facts.browser_version.is_some());
    assert_eq!(captured(&local).environment.browserbase, None);
    let reported = cloud.activities_of(ActivityKind::Environment);
    let probe = &reported[0].data.as_ref().unwrap()["browserbase"];
    assert_eq!(probe["webdriverMainWorld"], probe["webdriverIsolatedWorld"]);
    assert_eq!(probe["userAgentMatchesBrowser"], true);
    assert!(probe["timezone"].is_string() && probe["languages"].is_array());
    assert!(
        !reported[0].message.contains("visible"),
        "{}",
        reported[0].message
    );
    assert_eq!(captured(&cloud).viewport.width, 1440);

    remote.assert_nothing_leaked(&cloud);
}

/// The session is released however the capture ends, and the engine hears
/// of that end every time.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_browser_is_released_when_the_capture_is_cancelled_fails_or_is_dropped() {
    let Some(remote) = Remote::start().await else {
        return;
    };
    let fixtures = Fixtures::start().await;
    let released = |session: &SessionView| {
        assert!(!session.running, "{session:#?}");
        assert!(session.release_requests >= 1, "{session:#?}");
        assert!(
            remote.events().iter().any(|event| matches!(
                event,
                RemoteSessionEvent::Released { session_id, .. } if *session_id == session.id
            )),
            "{:?}",
            remote.events()
        );
    };

    // Cancelled in the middle of the visit.
    let reported = remote.reported();
    let cancel = reported.hooks.cancel.clone();
    let mut request = remote.request(&fixtures.url("network-kinds.html"));
    request.observe_seconds = 60;
    let running = tokio::spawn(reported.run(request));
    let visited = remote
        .until_a_session("that is visiting", |session| {
            !session.sent("Page.navigate").is_empty()
        })
        .await;
    cancel.cancel();
    let cancelled = running.await.unwrap();
    assert!(
        matches!(cancelled.result, Err(BrowserError::Cancelled)),
        "{:?}",
        cancelled.progress
    );
    released(&remote.cloud.session(&visited.id));
    remote.assert_nothing_leaked(&cancelled);

    // Failed: nothing answers at the address.
    let nowhere = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        format!(
            "http://127.0.0.1:{}/",
            listener.local_addr().unwrap().port()
        )
    };
    let failed = remote.reported().run(remote.request(&nowhere)).await;
    assert!(
        matches!(failed.result, Err(BrowserError::Navigation(_))),
        "{:?}",
        failed.result
    );
    let sessions = remote.cloud.sessions();
    assert_eq!(sessions.len(), 2);
    released(&sessions[1]);
    assert_eq!(names(&remote_events(&failed)).last(), Some(&"released"));
    remote.assert_nothing_leaked(&failed);

    // Dropped: whoever asked for the capture stopped waiting for it.
    let mut request = remote.request(&fixtures.url("network-kinds.html"));
    request.observe_seconds = 60;
    let running = tokio::spawn(remote.reported().run(request));
    remote
        .until_a_session("that is visiting", |session| {
            session.running && !session.sent("Page.navigate").is_empty()
        })
        .await;
    running.abort();
    let dropped = remote
        .until_a_session("that was released after the drop", |session| {
            !session.running
                && session.release_requests >= 1
                && session.id != visited.id
                && session.id != sessions[1].id
        })
        .await;
    // The capture's own task tells the engine once the service confirmed.
    let deadline = Instant::now() + Duration::from_secs(10);
    while !remote.events().iter().any(|event| {
        matches!(
            event,
            RemoteSessionEvent::Released { session_id, .. } if *session_id == dropped.id
        )
    }) {
        assert!(Instant::now() < deadline, "{:?}", remote.events());
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(remote.cloud.sessions().len(), 3);
}

/// A release that fails is tried again, and a session whose end cannot be
/// confirmed is not reported as ended.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_release_that_fails_is_tried_again() {
    let Some(remote) = Remote::start().await else {
        return;
    };
    let fixtures = Fixtures::start().await;
    remote.cloud.fail_releases(2);
    let mut request = remote.request(&fixtures.url("emulation.html"));
    request.observe_seconds = 1;

    let outcome = remote.reported().run(request).await;

    captured(&outcome);
    let session = remote.only_session();
    assert_eq!(session.release_requests, 3);
    assert_eq!(names(&remote_events(&outcome)).last(), Some(&"released"));
    assert!(matches!(
        remote.events().last(),
        Some(RemoteSessionEvent::Released { .. })
    ));
}

/// The release answered that the session has ended, and the reading right
/// after it still says "running": the status at the service is a moment
/// behind. The capture reports the end all the same, with what the session
/// used, so the engine closes its record and books the browser time.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_release_is_confirmed_although_the_session_reads_as_running_for_a_moment_longer() {
    let Some(remote) = Remote::start().await else {
        return;
    };
    let fixtures = Fixtures::start().await;
    remote.cloud.status_lags(1);
    let mut request = remote.request(&fixtures.url("emulation.html"));
    request.observe_seconds = 1;

    let outcome = remote.reported().run(request).await;

    captured(&outcome);
    let session = remote.only_session();
    assert_eq!(session.release_requests, 1);
    let events = remote_events(&outcome);
    assert_eq!(names(&events).last(), Some(&"released"), "{events:?}");
    assert!(
        !names(&events).contains(&"releaseUnconfirmed"),
        "{events:?}"
    );
    match remote.events().last() {
        Some(RemoteSessionEvent::Released {
            session_id,
            session_seconds,
            ..
        }) => {
            assert_eq!(*session_id, session.id);
            assert!(
                session_seconds.is_some(),
                "the release said how long it ran"
            );
        }
        other => panic!("{other:?}"),
    }

    // The other way round: no release gets through at all, and the status
    // of the browser, which ended when it was closed, lags as well. The
    // capture reads the session a few times before it gives up on it.
    remote.cloud.status_lags(2);
    remote.cloud.fail_releases(3);
    let mut request = remote.request(&fixtures.url("emulation.html"));
    request.observe_seconds = 1;
    let unreleased = remote.reported().run(request).await;
    captured(&unreleased);
    assert_eq!(names(&remote_events(&unreleased)).last(), Some(&"released"));
    assert_eq!(remote.cloud.sessions()[1].release_requests, 3);
}

/// The service started a browser and its answer did not say which: no id
/// reached the scanner, so nothing on its record could ever end it. It is
/// found by the tags it was started with (this app, this installation,
/// this scan) and ended. The browsers of another installation and of
/// another scan of this one are not touched, whatever they share with it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_browser_started_behind_an_answer_that_names_no_session_is_found_and_ended() {
    let Some(remote) = Remote::start().await else {
        return;
    };
    let start = |metadata: Value| {
        let body = json!({
            "projectId": PROJECT_ID,
            "region": "us-west-2",
            "timeout": 60,
            "userMetadata": metadata,
        });
        let url = format!("{}/v1/sessions", remote.cloud.base_url);
        async move {
            let created: Value = reqwest::Client::new()
                .post(url)
                .header("X-BB-API-Key", API_KEY)
                .json(&body)
                .send()
                .await
                .unwrap()
                .json()
                .await
                .unwrap();
            created["id"].as_str().unwrap().to_string()
        }
    };
    let another_install = start(json!({
        "app": "adscan", "install": "ffffffffffffffffffffffffffffffff", "scan": SCAN,
    }))
    .await;
    let another_scan = start(json!({
        "app": "adscan", "install": INSTALL, "scan": "11111111-2222-4333-8444-555555555555",
    }))
    .await;
    remote.cloud.lose_create_answers(1);

    let outcome = remote
        .reported()
        .run(remote.request("http://pub.adscan-fixture.example/"))
        .await;

    match &outcome.result {
        Err(BrowserError::Remote(message)) => assert_eq!(
            message,
            "Browserbase answered in a way the scanner does not understand."
        ),
        other => panic!("{other:?}"),
    }
    let sessions = remote.cloud.sessions();
    assert_eq!(sessions.len(), 3, "{sessions:#?}");
    let stray = &sessions[2];
    assert_eq!(stray.body["userMetadata"]["scan"], SCAN);
    assert_eq!(stray.body["userMetadata"]["install"], INSTALL);
    assert!(!stray.running, "the stray browser still runs: {stray:#?}");
    assert_eq!(stray.release_requests, 1);
    assert_eq!(stray.connections, 0, "nothing ever connected to it");
    for untouched in [&another_install, &another_scan] {
        let session = remote.cloud.session(untouched);
        assert!(session.running, "{session:#?}");
        assert_eq!(session.release_requests, 0, "{session:#?}");
    }
    let events = remote_events(&outcome);
    assert_eq!(names(&events), ["strayReleased"]);
    assert_eq!(events[0].1, json!({ "event": "strayReleased", "count": 1 }));
    assert!(
        remote.events().is_empty(),
        "no session the engine could put on record: {:?}",
        remote.events()
    );
    remote.assert_nothing_leaked(&outcome);
}

/// The wait at the limit is a minute on the clock, whatever the service
/// takes to say "full": refusals that take four seconds each to arrive
/// leave room for seven waits, not twelve, and the capture gives up when
/// the minute is over, not after two.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_wait_at_the_limit_is_a_minute_on_the_clock_however_slowly_the_service_refuses() {
    let Some(remote) = Remote::start().await else {
        return;
    };
    remote.cloud.stay_full();
    remote.cloud.refuse_slowly(Duration::from_secs(4));
    let started = Instant::now();

    let outcome = remote
        .reported()
        .run(remote.request("http://pub.adscan-fixture.example/"))
        .await;

    let waited = started.elapsed();
    assert!(
        matches!(outcome.result, Err(BrowserError::RemoteLimit(_))),
        "{:?}",
        outcome.result
    );
    // The last request starts within the minute and is heard out.
    assert!(
        waited >= Duration::from_secs(60) && waited < Duration::from_secs(70),
        "{waited:?}"
    );
    let events = remote_events(&outcome);
    assert!(events.iter().all(|(name, _)| name == "rateLimited"));
    assert!(
        (6..=8).contains(&events.len()),
        "{} waits: {:?}",
        events.len(),
        events
    );
    let pauses: u64 = events
        .iter()
        .map(|(_, detail)| detail["waitSeconds"].as_u64().unwrap())
        .sum();
    assert!(pauses <= 60, "{pauses} s of pauses");
    assert!(remote.cloud.sessions().is_empty());
}

/// The project is at its limit, which another application shares: the
/// capture waits five seconds at a time, says so, and goes on when a
/// browser is free.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_project_at_its_limit_is_waited_for() {
    let Some(remote) = Remote::start().await else {
        return;
    };
    let fixtures = Fixtures::start().await;
    remote.cloud.others_hold(3);
    let mut request = remote.request(&fixtures.url("emulation.html"));
    request.observe_seconds = 1;
    let started = Instant::now();

    let outcome = remote.reported().run(request).await;

    captured(&outcome);
    assert!(
        started.elapsed() >= Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );
    let events = remote_events(&outcome);
    assert_eq!(
        events[0].1,
        json!({ "event": "rateLimited", "waitSeconds": 5, "attempt": 1 })
    );
    assert_eq!(
        names(&events),
        [
            "rateLimited",
            "sessionCreated",
            "connected",
            "timezone",
            "serviceScripts",
            "released"
        ]
    );
    let asked_for_a_browser = remote
        .cloud
        .asked()
        .iter()
        .filter(|asked| asked.method == "POST" && asked.path_and_query == "/v1/sessions")
        .count();
    assert_eq!(asked_for_a_browser, 2);
    remote.assert_nothing_leaked(&outcome);
}

/// A project that stays full is given up on after a minute, with the
/// service's numbers; a cancel ends the wait at once and leaves no session.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_project_that_stays_full_is_given_up_on_and_a_cancel_ends_the_wait() {
    let Some(remote) = Remote::start().await else {
        return;
    };
    remote.cloud.stay_full();

    let reported = remote.reported();
    let cancel = reported.hooks.cancel.clone();
    let waiting = tokio::spawn(reported.run(remote.request("http://pub.adscan-fixture.example/")));
    tokio::time::sleep(Duration::from_secs(2)).await;
    let cancelled_at = Instant::now();
    cancel.cancel();
    let cancelled = waiting.await.unwrap();
    assert!(matches!(cancelled.result, Err(BrowserError::Cancelled)));
    assert!(cancelled_at.elapsed() < Duration::from_secs(2));

    let started = Instant::now();
    let outcome = remote
        .reported()
        .run(remote.request("http://pub.adscan-fixture.example/"))
        .await;

    let waited = started.elapsed();
    assert!(
        waited >= Duration::from_secs(60) && waited < Duration::from_secs(75),
        "{waited:?}"
    );
    match &outcome.result {
        Err(BrowserError::RemoteLimit(message)) => assert_eq!(
            message,
            "Browserbase is at its limit of browsers running at once (3 of 3, shared by \
             everything that uses this project). Try again when one has finished."
        ),
        other => panic!("{other:?}"),
    }
    let events = remote_events(&outcome);
    assert_eq!(events.len(), 12, "{:?}", names(&events));
    assert!(events.iter().all(|(name, _)| name == "rateLimited"));
    assert_eq!(events[11].1["attempt"], 12);
    assert!(remote.cloud.sessions().is_empty());
    assert!(
        remote.events().is_empty(),
        "no session, nothing to tell the engine"
    );
    remote.assert_nothing_leaked(&outcome);
}

/// The kept browser profile is a context at the service: what one visit
/// leaves in the browser, the next visit in the same context finds.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_kept_profile_is_a_context_that_the_next_visit_finds_again() {
    let Some(remote) = Remote::start().await else {
        return;
    };
    let fixtures = Fixtures::start().await;
    let context = remote.client.create_context(&credentials()).await.unwrap();
    assert_eq!(remote.cloud.context_ids(), [context.as_str()]);
    let visit = |context_id: Option<String>| {
        let mut request = remote.request(&fixtures.url("cookie.html"));
        request.observe_seconds = 1;
        request.persistent_profile = context_id.is_some();
        request.browser = CaptureBrowser::Browserbase(BrowserbaseCapture {
            context_id,
            ..remote.capture()
        });
        remote.reported().run(request)
    };
    let cookie_before = |outcome: &Outcome| {
        captured(outcome)
            .console
            .iter()
            .find_map(|line| line.text.strip_prefix("COOKIE before=").map(str::to_string))
            .expect("the page logs its cookie")
    };

    let first = visit(Some(context.clone())).await;
    let second = visit(Some(context.clone())).await;
    let without = visit(None).await;

    assert_eq!(cookie_before(&first), "");
    assert_eq!(cookie_before(&second), "returning=yes");
    assert_eq!(cookie_before(&without), "");
    let sessions = remote.cloud.sessions();
    for session in &sessions[..2] {
        assert_eq!(
            session.body["browserSettings"]["context"],
            json!({ "id": context, "persist": true })
        );
    }
    assert!(sessions[2].body["browserSettings"].get("context").is_none());
    assert!(captured(&second).environment.persistent_profile);
    assert!(!captured(&without).environment.persistent_profile);
    assert_eq!(remote_events(&second)[0].1["context"], true);

    // A context that was deleted is said to be gone, in the user's words.
    remote
        .client
        .delete_context(&credentials(), &context)
        .await
        .unwrap();
    remote
        .client
        .delete_context(&credentials(), &context)
        .await
        .unwrap();
    let gone = visit(Some(context)).await;
    match &gone.result {
        Err(BrowserError::Remote(message)) => assert_eq!(
            message,
            "The browser profile kept at Browserbase no longer exists. Clear it in Settings \
             and scan again."
        ),
        other => panic!("{other:?}"),
    }
    assert_eq!(
        remote.cloud.sessions().len(),
        3,
        "no session was started for it"
    );
    for outcome in [&first, &second, &without, &gone] {
        remote.assert_nothing_leaked(outcome);
    }
}

/// The connection to the browser goes away in the middle of a visit: the
/// capture ends with the reason, keeps nothing, and still makes sure the
/// session is over.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_connection_that_is_lost_ends_the_capture_and_keeps_nothing() {
    let Some(remote) = Remote::start().await else {
        return;
    };
    let fixtures = Fixtures::start().await;
    for visitor in [VisitorBehavior::Scroll, VisitorBehavior::Manual] {
        let mut request = remote.request(&fixtures.url("network-kinds.html"));
        request.visitor = visitor;
        request.observe_seconds = 60;
        let before = remote.cloud.sessions().len();
        let running = tokio::spawn(remote.reported().run(request));
        let visiting = remote
            .until_a_session("that is being observed", |session| {
                session.running
                    && session.sent("Runtime.evaluate").iter().any(|command| {
                        command.params["expression"].as_str().is_some_and(|script| {
                            script.contains("navigator.webdriver, navigator.userAgent, innerWidth")
                        })
                    })
            })
            .await;
        tokio::time::sleep(Duration::from_millis(500)).await;
        remote.cloud.cut_connection(&visiting.id);

        let outcome = tokio::time::timeout(Duration::from_secs(30), running)
            .await
            .expect("the capture ends when its connection is gone")
            .unwrap();

        match &outcome.result {
            Err(BrowserError::Remote(message)) => assert_eq!(
                message,
                "The connection to the Browserbase browser was lost. The browser has ended. \
                 Nothing was kept.",
                "{visitor:?}"
            ),
            other => panic!("{visitor:?}: {other:?}"),
        }
        let events = remote_events(&outcome);
        let last_two = &names(&events)[events.len() - 2..];
        assert_eq!(last_two, ["connectionLost", "released"], "{visitor:?}");
        let session = remote.cloud.session(&visiting.id);
        assert!(!session.running && session.release_requests >= 1);
        assert_eq!(remote.cloud.sessions().len(), before + 1);
        remote.assert_nothing_leaked(&outcome);
    }
}

fn body_state(record: &NetworkRecord) -> String {
    match &record.body {
        CapturedBody::Read { bytes, total_bytes } => {
            format!("read {} of {total_bytes}", bytes.len())
        }
        other => format!("{other:?}"),
    }
}

/// A body whose answer could be more than one message of the connection
/// carries is never asked for. One that grows large while it loads is
/// sent along as it arrives and recorded from that copy; one that was
/// there before a copy could be asked for is recorded as too large.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_body_too_large_for_one_message_is_never_asked_for() {
    const MIB: usize = 1024 * 1024;
    const LARGE: usize = 10 * MIB;
    const ORDINARY: usize = 3 * MIB;
    let Some(remote) = Remote::start().await else {
        return;
    };
    let fixtures = Fixtures::start().await;
    // Ten megabytes over three seconds, as from the network; the same at
    // once, as from a cache; and a body of ordinary size.
    let slowly = format!("size={LARGE}&drip=200&by=slowly");
    let at_once = format!("size={LARGE}&by=at-once");
    let ordinary = format!("size={ORDINARY}&by=ordinary");
    let page = fixtures.url(&format!(
        "remote-fetches.html?fetch={}",
        [&slowly, &at_once, &ordinary]
            .map(|query| format!("/payload?{query}")
                .replace('?', "%3F")
                .replace('&', "%26"))
            .join(",")
    ));
    let requests = Requests::admitting(|_| adscan_model::BodyAdmission::Read {
        max_bytes: 64 * MIB,
    });
    let mut reported = remote.reported();
    reported.hooks.network = Some(requests.clone());
    let mut request = remote.request(&page);
    request.observe_seconds = 8;

    let outcome = reported.run(request).await;

    captured(&outcome);
    let records = requests.records();
    let the = |query: &str| {
        records
            .iter()
            .find(|record| record.url.ends_with(&format!("/payload?{query}")))
            .unwrap_or_else(|| panic!("no request for {query}: {:#?}", outcome.progress))
    };
    // Sent along while it loaded: the bytes that were served.
    match &the(&slowly).body {
        CapturedBody::Read { bytes, total_bytes } => {
            assert_eq!(*total_bytes, LARGE as u64);
            assert!(
                *bytes == support::payload(&slowly),
                "not the bytes that were served"
            );
        }
        other => panic!("{other:?}"),
    }
    // There before anyone could ask: the copy if it was whole, else the
    // honest state. Never a guess.
    match &the(&at_once).body {
        CapturedBody::Read { bytes, .. } => assert!(*bytes == support::payload(&at_once)),
        CapturedBody::TooLargeForRemote { total_bytes } => assert_eq!(*total_bytes, LARGE as u64),
        other => panic!("{other:?}"),
    }
    assert_eq!(
        body_state(the(&ordinary)),
        format!("read {ORDINARY} of {ORDINARY}")
    );

    // No answer of the browser was anywhere near what the connection
    // cannot carry: the largest is the ordinary body, read the usual way.
    let session = remote.only_session();
    let read: Vec<usize> = session
        .sent("Network.getResponseBody")
        .iter()
        .filter_map(|command| command.reply_bytes)
        .collect();
    assert!(read.iter().any(|bytes| *bytes >= ORDINARY), "{read:?}");
    assert!(read.iter().all(|bytes| *bytes < 4 * MIB), "{read:?}");
    let largest_answer = session
        .commands
        .iter()
        .filter_map(|command| command.reply_bytes)
        .max()
        .unwrap();
    assert!(largest_answer < 16 * MIB, "{largest_answer}");
    assert!(!session.sent("Network.streamResourceContent").is_empty());

    let too_large: Vec<Value> = remote_events(&outcome)
        .into_iter()
        .filter(|(name, _)| name == "bodiesTooLarge")
        .map(|(_, detail)| detail)
        .collect();
    match &the(&at_once).body {
        CapturedBody::TooLargeForRemote { .. } => assert_eq!(
            too_large,
            [json!({ "event": "bodiesTooLarge", "count": 1, "largestBytes": LARGE })]
        ),
        _ => assert!(too_large.is_empty(), "{too_large:?}"),
    }
}

/// Browserbase has no iframe in a process of its own: every frame of
/// every site reports on the page's session. Frames that are gone the
/// moment they have their answer still have their bodies recorded, and
/// each request is filed under the frame that made it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn frames_that_live_in_the_pages_process_are_recorded_with_their_bodies() {
    const ROUNDS: usize = 3;
    let Some(remote) = Remote::start().await else {
        return;
    };
    let fixtures = Fixtures::start().await;
    let asks = "/payload?size=50000&wait=200";
    let page = fixtures.url(&format!(
        "network-short-lived.html?rounds={ROUNDS}&asks={}",
        asks.replace('?', "%3F").replace('&', "%26")
    ));
    let requests = Requests::keeping_text();
    let mut reported = remote.reported();
    reported.hooks.network = Some(requests.clone());
    let mut request = remote.request(&page);
    request.observe_seconds = 5;

    let outcome = reported.run(request).await;

    captured(&outcome);
    let records = requests.records();
    assert!(
        records
            .iter()
            .any(|record| record.url.ends_with("/status/200?all-gone")),
        "the page did not get through its frames: {:#?}",
        outcome.progress
    );
    for by in ["other-site-frame", "same-site-frame", "worker"] {
        for round in 0..ROUNDS {
            let query = format!("size=50000&wait=200&by={by}&round={round}");
            let fetched = records
                .iter()
                .find(|record| record.url.ends_with(&format!("/payload?{query}")))
                .unwrap_or_else(|| panic!("no request for {query}"));
            // A worker has a session of its own on Browserbase too.
            let expected = match by {
                "worker" => (RequestContext::Worker, true),
                _ => (RequestContext::Iframe, false),
            };
            assert_eq!(
                (fetched.context, fetched.target_id.is_some()),
                expected,
                "{query}"
            );
            match &fetched.body {
                CapturedBody::Read { bytes, .. } => {
                    assert!(
                        *bytes == support::payload(&query),
                        "{query}: not what was served"
                    );
                }
                other => panic!("{query}: {other:?}"),
            }
        }
    }
    // The frames' own documents, of the other site too, are the page
    // session's to report.
    let frame_documents: Vec<&NetworkRecord> = records
        .iter()
        .filter(|record| record.url.contains("/network-short-lived-frame.html"))
        .collect();
    assert_eq!(frame_documents.len(), 2 * ROUNDS);
    for document in frame_documents {
        assert_eq!(
            (
                document.kind,
                document.context,
                document.target_id.is_some()
            ),
            (ResourceKind::Document, RequestContext::Iframe, false),
            "{}",
            document.url
        );
        assert!(
            matches!(document.body, CapturedBody::Read { .. }),
            "{}",
            body_state(document)
        );
    }
}

/// A consent dialog in a frame of another site, which on Browserbase is
/// in the page's own process, is found and accepted.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_consent_dialog_in_a_frame_of_the_pages_process_is_accepted() {
    let Some(remote) = Remote::start().await else {
        return;
    };
    let fixtures = Fixtures::start().await;
    let mut request = remote.request(&fixtures.url("consent-iframe.html"));
    request.accept_consent = true;
    request.observe_seconds = 3;

    let outcome = remote.reported().run(request).await;

    let capture = captured(&outcome);
    assert!(capture.consent_accepted, "{:#?}", outcome.progress);
    assert!(
        capture
            .console
            .iter()
            .any(|line| line.text == "CONSENT accepted"),
        "{:?}",
        capture.console
    );
}

/// A person browses in Browserbase's live view, which is a second client
/// on the page next to the capture's one. The capture records where they
/// go, tells the engine when the live view can be opened, keeps the
/// service's own console lines out, and finishes when asked.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_manual_visit_records_what_a_second_client_does_until_it_is_finished() {
    let Some(remote) = Remote::start().await else {
        return;
    };
    let fixtures = Fixtures::start().await;
    let reported = remote.reported();
    let finish = reported.hooks.finish.clone();
    let mut request = remote.request(&fixtures.url("emulation.html"));
    request.visitor = VisitorBehavior::Manual;
    request.accept_consent = false;
    let running = tokio::spawn(reported.run(request));

    // The engine is told when a person can take over, by id only.
    let session_id = remote.live_view_ready(0).await;
    // What the engine opens in the user's browser. A credential: it comes
    // from this call and from nowhere else.
    let live_view = remote
        .client
        .live_view_url(&credentials(), &session_id)
        .await
        .unwrap();
    assert!(live_view.contains("devtools-fullscreen") && live_view.contains("token="));

    // The person, through the live view: the service's own lines appear
    // in the console, then they go to a second page and click there.
    let person = PageClient::attach(&remote.cloud.page_socket(&session_id).await).await;
    person.log_like_the_service().await;
    person.evaluate("console.log('PERSON was here')").await;
    person
        .call(
            "Page.navigate",
            json!({ "url": fixtures.url("manual.html?step=2") }),
        )
        .await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    for kind in ["mousePressed", "mouseReleased"] {
        person
            .call(
                "Input.dispatchMouseEvent",
                json!({ "type": kind, "x": 200, "y": 200, "button": "left", "clickCount": 1 }),
            )
            .await;
    }
    person.log_like_the_service().await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(
        !running.is_finished(),
        "a manual visit waits to be finished"
    );
    let finished_at = Instant::now();
    finish.cancel();
    let outcome = running.await.unwrap();

    let capture = captured(&outcome);
    assert!(
        finished_at.elapsed() < Duration::from_secs(20),
        "{:?}",
        finished_at.elapsed()
    );
    assert_eq!(capture.pages.len(), 2, "{:#?}", outcome.progress);
    assert_eq!(capture.pages[1].reached_by, PageReach::UserNavigation);
    assert!(capture.pages[1].url.ends_with("/manual.html?step=2"));
    let console: Vec<&str> = capture
        .console
        .iter()
        .map(|line| line.text.as_str())
        .collect();
    assert!(console.contains(&"PERSON was here"), "{console:?}");
    assert!(
        console.contains(&"INPUT mousedown"),
        "the person's click reached the page: {console:?}"
    );
    assert!(
        !console
            .iter()
            .any(|line| line.to_lowercase().contains("browserbase")),
        "the service's own lines were recorded: {console:?}"
    );
    let events = remote_events(&outcome);
    assert_eq!(
        names(&events),
        [
            "sessionCreated",
            "connected",
            "timezone",
            "liveView",
            "serviceScripts",
            "released"
        ]
    );
    let session = remote.only_session();
    assert_eq!(
        session.connections, 1,
        "the capture opened one client and no second"
    );
    assert!(
        session.sent("Input.dispatchMouseEvent").is_empty(),
        "the scanner did nothing on the page"
    );
    assert!(
        outcome
            .progress
            .iter()
            .any(|line| line.contains("Browserbase's live view")),
        "{:#?}",
        outcome.progress
    );
    // Start 120 + the 15 minutes of a manual visit + margin 120.
    assert_eq!(session.body["timeout"], 1_140);
    remote.assert_nothing_leaked(&outcome);
    assert!(
        !outcome
            .progress
            .iter()
            .any(|line| line.contains(&live_view)),
        "the live view's address was logged"
    );

    // Once the browser has ended there is no live view to open.
    assert_eq!(
        remote
            .client
            .live_view_url(&credentials(), &session_id)
            .await,
        Err(BrowserbaseError::Failed(
            "The Browserbase browser has ended.".into()
        ))
    );
}

/// The person closes the last tab in the live view: the visit is over and
/// what was recorded is kept. A cancel keeps nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_manual_visit_ends_when_its_last_tab_is_closed_and_a_cancel_keeps_nothing() {
    let Some(remote) = Remote::start().await else {
        return;
    };
    let fixtures = Fixtures::start().await;
    let manual = || {
        let mut request = remote.request(&fixtures.url("emulation.html"));
        request.visitor = VisitorBehavior::Manual;
        request.accept_consent = false;
        request
    };
    let running = tokio::spawn(remote.reported().run(manual()));
    let session_id = remote.live_view_ready(0).await;
    let person = PageClient::attach(&remote.cloud.page_socket(&session_id).await).await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    person.send("Page.close", json!({}));
    let closed = tokio::time::timeout(Duration::from_secs(30), running)
        .await
        .expect("closing the last tab ends the visit")
        .unwrap();
    let capture = captured(&closed);
    assert_eq!(capture.pages.len(), 1);
    assert!(capture.final_url.ends_with("/emulation.html"));
    assert!(!remote.cloud.session(&session_id).running);

    let reported = remote.reported();
    let cancel = reported.hooks.cancel.clone();
    let running = tokio::spawn(reported.run(manual()));
    let session_id = remote.live_view_ready(1).await;
    cancel.cancel();
    let cancelled = running.await.unwrap();
    assert!(matches!(cancelled.result, Err(BrowserError::Cancelled)));
    let session = remote.cloud.session(&session_id);
    assert!(!session.running && session.release_requests == 1);
}

/// Where the visit leaves from: the user's own proxy is the service's to
/// sign in to, a Browserbase location is the service's to set a timezone
/// for, and neither puts a password anywhere but in the request for the
/// session.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_users_proxy_and_a_browserbase_location_are_the_services_to_apply() {
    let Some(remote) = Remote::start().await else {
        return;
    };
    let fixtures = Fixtures::start().await;
    let page = fixtures.url("emulation.html");

    // The user's own proxy: an external proxy of the session, with its
    // login. The scanner answers no proxy challenge itself.
    let mut reported = remote.reported();
    reported.hooks.proxy_password = Some(PROXY_PASSWORD.to_string());
    let mut request = remote.request(&page);
    request.observe_seconds = 1;
    request.proxy = CaptureProxy::Custom {
        scheme: ProxyScheme::Http,
        host: "proxy.example.net".into(),
        port: 3128,
        username: Some("scanner".into()),
    };
    let through_own_proxy = reported.run(request).await;
    captured(&through_own_proxy);
    let session = &remote.cloud.sessions()[0];
    assert_eq!(
        session.body["proxies"],
        json!([{
            "type": "external",
            "server": "http://proxy.example.net:3128",
            "username": "scanner",
            "password": PROXY_PASSWORD,
        }])
    );
    for paused in session.sent("Fetch.enable") {
        assert_ne!(paused.params["handleAuthRequests"], true);
    }
    assert!(session.sent("Fetch.continueWithAuth").is_empty());
    for command in &session.commands {
        assert!(
            !command.params.to_string().contains(PROXY_PASSWORD),
            "{command:?}"
        );
    }
    // Decision 7: the region's timezone goes with the user's proxy too.
    assert_eq!(session.sent("Emulation.setTimezoneOverride").len(), 1);
    assert_eq!(remote_events(&through_own_proxy)[0].1["proxy"], "external");
    remote.assert_nothing_leaked(&through_own_proxy);

    // A Browserbase location: the service has set the timezone of the exit,
    // and the scanner leaves it alone.
    remote.cloud.report_proxy_bytes(131_844, 1);
    let mut request = remote.request(&page);
    request.observe_seconds = 1;
    request.browser = CaptureBrowser::Browserbase(BrowserbaseCapture {
        location: Some(BrowserbaseLocation {
            country: "BE".into(),
            state: None,
            city: None,
        }),
        ..remote.capture()
    });
    let through_a_location = remote.reported().run(request).await;
    let capture = captured(&through_a_location);
    let session = &remote.cloud.sessions()[1];
    assert_eq!(
        session.body["proxies"],
        json!([{ "type": "browserbase", "geolocation": { "country": "BE" } }])
    );
    assert!(session.sent("Emulation.setTimezoneOverride").is_empty());
    let facts = capture.environment.browserbase.as_ref().unwrap();
    assert_eq!(facts.applied_timezone, None);
    assert!(facts.original_timezone.is_some());
    let events = remote_events(&through_a_location);
    assert_eq!(events[0].1["proxy"], "browserbase");
    assert_eq!(events[2].1["applied"], Value::Null);
    // The count of proxy traffic lags at the service: what the capture
    // reads at the end may still be zero, and a later read has it.
    assert!(matches!(
        remote.events().last(),
        Some(RemoteSessionEvent::Released { proxy_bytes: Some(bytes), .. })
            if *bytes == 0.0 || *bytes == 131_844.0
    ));
    let later = remote
        .client
        .session_usage(&credentials(), &session.id)
        .await
        .unwrap();
    assert!(later.ended);
    assert_eq!(later.proxy_bytes, Some(131_844.0));
    assert!(later.session_seconds.unwrap() >= 1.0);

    // What the engine never sends, refused without a session.
    let mut request = remote.request(&page);
    request.proxy = CaptureProxy::System;
    let through_the_system_proxy = remote.reported().run(request).await;
    assert!(matches!(
        through_the_system_proxy.result,
        Err(BrowserError::Other(_))
    ));
    assert_eq!(remote.cloud.sessions().len(), 2);
}

/// A capture that is asked to run at Browserbase without the means to.
#[tokio::test]
async fn a_capture_without_credentials_says_that_browserbase_is_not_set_up() {
    let mut request = support::request("https://news.example/", DeviceProfile::Desktop);
    request.browser = CaptureBrowser::Browserbase(BrowserbaseCapture {
        region: BrowserbaseRegion::UsWest2,
        settings: BrowserbaseSessionSettings::STANDARD,
        location: None,
        context_id: None,
        install_id: INSTALL.into(),
        scan_id: SCAN.into(),
    });
    request.proxy = CaptureProxy::None;

    let outcome = support::run(request).await;

    match outcome.result {
        Err(BrowserError::RemoteAuth(message)) => {
            assert_eq!(
                message,
                "Browserbase is not set up. Add the API key in Settings."
            );
        }
        other => panic!("{other:?}"),
    }
}

/// What the app asks of the service outside a capture.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_account_calls_read_the_project_and_touch_only_this_installations_sessions() {
    let Some(remote) = Remote::start().await else {
        return;
    };
    let client = &remote.client;
    let own = credentials();

    // The key and the project.
    let projects = client.projects(API_KEY).await.unwrap();
    assert_eq!(projects.len(), 1);
    assert_eq!(
        (
            projects[0].id.as_str(),
            projects[0].name.as_str(),
            projects[0].concurrency
        ),
        (PROJECT_ID, "Local", 3)
    );
    assert_eq!(client.project(&own).await.unwrap(), projects[0]);
    let usage = client.project_usage(&own).await.unwrap();
    assert_eq!(
        (usage.browser_minutes, usage.proxy_bytes),
        (483.0, 1_998_349_077.0)
    );
    assert_eq!(
        client.projects("a-key-that-is-wrong").await,
        Err(BrowserbaseError::Auth(
            "Browserbase did not accept the API key. Check it in Settings.".into()
        ))
    );
    assert_eq!(
        client.projects("a key\nwith a line break").await,
        Err(BrowserbaseError::Auth(
            "Browserbase did not accept the API key. Check it in Settings.".into()
        ))
    );
    let other_project = BrowserbaseCredentials {
        project_id: OTHER_PROJECT_ID.into(),
        ..credentials()
    };
    for refused in [
        client.project(&other_project).await.map(drop),
        client.project_usage(&other_project).await.map(drop),
    ] {
        assert_eq!(
            refused,
            Err(BrowserbaseError::Auth(
                "This Browserbase project was not found for this key.".into()
            ))
        );
    }
    let not_an_id = BrowserbaseCredentials {
        project_id: "../sessions".into(),
        ..credentials()
    };
    assert!(matches!(
        client.project(&not_an_id).await,
        Err(BrowserbaseError::Failed(_))
    ));

    // Three browsers run in the project: this installation's, one of
    // another installation of this app, and one of another application.
    let start = |metadata: Value| {
        let body = json!({
            "projectId": PROJECT_ID,
            "region": "us-west-2",
            "timeout": 60,
            "userMetadata": metadata,
        });
        let url = format!("{}/v1/sessions", remote.cloud.base_url);
        async move {
            let created: Value = reqwest::Client::new()
                .post(url)
                .header("X-BB-API-Key", API_KEY)
                .json(&body)
                .send()
                .await
                .unwrap()
                .json()
                .await
                .unwrap();
            created["id"].as_str().unwrap().to_string()
        }
    };
    let ours = start(json!({ "app": "adscan", "install": INSTALL, "scan": "probe" })).await;
    let another_install =
        start(json!({ "app": "adscan", "install": "ffffffffffffffffffffffffffffffff" })).await;
    let another_application =
        start(json!({ "scanId": "s", "auditId": "a", "organizationId": INSTALL })).await;

    assert_eq!(client.running_session_count(&own).await.unwrap(), 3);
    assert_eq!(
        client.running_sessions_of(&own, INSTALL).await.unwrap(),
        [ours.as_str()]
    );
    assert!(
        client
            .running_sessions_of(&own, "0000")
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        client
            .running_sessions_of(&own, "x' OR '1")
            .await
            .unwrap()
            .is_empty()
    );
    let asked = remote.cloud.asked();
    let listing = asked
        .iter()
        .find(|asked| asked.path_and_query.contains(INSTALL))
        .unwrap();
    assert_eq!(
        listing.path_and_query,
        format!("/v1/sessions?status=RUNNING&q=user_metadata%5B%27install%27%5D%3A%27{INSTALL}%27")
    );
    assert!(
        asked.iter().all(|asked| asked.key.is_some()),
        "every call carries the key header"
    );

    // Asked for by its id, a session says whether it runs and whose it is.
    use adscan_browser::BrowserbaseSessionStanding as Standing;
    assert_eq!(
        client.session_standing(&own, &ours, INSTALL).await,
        Ok(Standing::Running)
    );
    for not_ours in [&another_install, &another_application] {
        assert_eq!(
            client.session_standing(&own, not_ours, INSTALL).await,
            Ok(Standing::NotThisInstallations)
        );
    }
    assert_eq!(
        client
            .session_standing(&own, &ours, "ffffffffffffffffffffffffffffffff")
            .await,
        Ok(Standing::NotThisInstallations)
    );
    assert_eq!(
        client
            .session_standing(&own, "5e55ffff-0000-4000-8000-00000000ffff", INSTALL)
            .await,
        Ok(Standing::Unknown)
    );
    assert!(matches!(
        client.session_standing(&own, "../projects", INSTALL).await,
        Err(BrowserbaseError::Failed(_))
    ));

    // Releasing is idempotent, and only what was named is released.
    let running = client.session_usage(&own, &ours).await.unwrap();
    assert!(!running.ended && running.session_seconds.is_none());
    client.release_session(&own, &ours).await.unwrap();
    client.release_session(&own, &ours).await.unwrap();
    let ended = client.session_usage(&own, &ours).await.unwrap();
    assert!(ended.ended && ended.session_seconds.is_some());
    assert_eq!(
        client.session_standing(&own, &ours, INSTALL).await,
        Ok(Standing::Ended)
    );
    assert!(remote.cloud.session(&another_install).running);
    assert!(remote.cloud.session(&another_application).running);
    assert_eq!(client.running_session_count(&own).await.unwrap(), 2);
    assert!(
        client
            .running_sessions_of(&own, INSTALL)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        client
            .release_session(&own, "5e55ffff-0000-4000-8000-00000000ffff")
            .await,
        Err(BrowserbaseError::Failed(
            "Browserbase refused the request: Not Found".into()
        ))
    );

    // Contexts.
    let context = client.create_context(&own).await.unwrap();
    assert_eq!(remote.cloud.context_ids(), [context.as_str()]);
    client.delete_context(&own, &context).await.unwrap();
    client.delete_context(&own, &context).await.unwrap();
    assert!(remote.cloud.context_ids().is_empty());
    assert_eq!(
        client.create_context(&other_project).await,
        Err(BrowserbaseError::Auth(
            "This Browserbase project was not found for this key.".into()
        ))
    );
}

/// The probe starts one browser, reads what its blank page reports and
/// ends it, also when the reading fails.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_probe_reads_a_blank_page_of_one_short_session_and_ends_it() {
    let Some(remote) = Remote::start().await else {
        return;
    };

    let probe = remote
        .client
        .probe(&credentials(), BrowserbaseRegion::ApSoutheast1, INSTALL)
        .await
        .unwrap();

    assert_eq!(probe.region, BrowserbaseRegion::ApSoutheast1);
    assert!(
        probe
            .browser_version
            .as_deref()
            .unwrap()
            .contains("Chrome/")
    );
    assert!(
        probe
            .user_agent
            .as_deref()
            .unwrap()
            .starts_with("Mozilla/5.0")
    );
    assert_eq!(
        probe.webdriver,
        Some(false),
        "the stand-in is started like Browserbase's browser"
    );
    assert!(probe.timezone.is_some() && probe.language.is_some());
    assert!((0.0..500.0).contains(&probe.round_trip_ms.unwrap()));
    assert!(probe.session_seconds.is_some());
    let session = remote.only_session();
    assert_eq!(
        session.body,
        json!({
            "projectId": PROJECT_ID,
            "region": "ap-southeast-1",
            "timeout": 60,
            "keepAlive": false,
            "browserSettings": {
                "solveCaptchas": true,
                "recordSession": true,
                "logSession": true,
                "blockAds": false,
            },
            "userMetadata": { "app": "adscan", "install": INSTALL, "scan": "probe" },
        })
    );
    assert!(!session.running);
    assert_eq!(session.release_requests, 1);
    assert!(
        session.sent("Page.navigate").is_empty(),
        "the probe goes nowhere"
    );
    assert!(session.sent("Target.createTarget").is_empty());

    // A key that is refused starts nothing.
    let wrong = BrowserbaseCredentials {
        api_key: "wrong".into(),
        ..credentials()
    };
    assert!(matches!(
        remote
            .client
            .probe(&wrong, BrowserbaseRegion::UsWest2, INSTALL)
            .await,
        Err(BrowserbaseError::Auth(_))
    ));
    assert_eq!(remote.cloud.sessions().len(), 1);
}

/// An address the service hands out that is not one of its own is not
/// connected to, and the session it came with is still ended.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_connect_address_can_be_replaced_for_tests_and_a_bad_one_is_refused() {
    let Some(remote) = Remote::start().await else {
        return;
    };
    assert_eq!(
        Browserbase::new(BrowserbaseEndpoint {
            api_url: "http://api.browserbase.com".into(),
            connect_url: None,
        })
        .err(),
        Some(BrowserbaseError::Failed(
            "Browserbase is misconfigured: its API address must be https.".into()
        ))
    );
    // Nothing listens at the replacement: the capture fails to connect,
    // says so without the address, and releases the session.
    let nowhere = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        format!(
            "ws://127.0.0.1:{}/devtools/browser/s3cret-override",
            listener.local_addr().unwrap().port()
        )
    };
    let client = Browserbase::new(BrowserbaseEndpoint {
        api_url: remote.cloud.base_url.clone(),
        connect_url: Some(nowhere),
    })
    .unwrap();
    let mut reported = remote.reported();
    reported.hooks.browserbase.as_mut().unwrap().client = client;

    let outcome = reported
        .run(remote.request("http://pub.adscan-fixture.example/"))
        .await;

    match &outcome.result {
        Err(BrowserError::Remote(message)) => {
            assert!(
                message.starts_with("The scanner could not connect to the Browserbase browser: "),
                "{message}"
            );
            assert!(
                !message.contains("s3cret") && !message.contains("127.0.0.1"),
                "{message}"
            );
        }
        other => panic!("{other:?}"),
    }
    let session = remote.only_session();
    assert_eq!(session.connections, 0);
    assert!(!session.running && session.release_requests == 1);
    assert_eq!(
        names(&remote_events(&outcome)),
        ["sessionCreated", "released"]
    );
    remote.assert_nothing_leaked(&outcome);
}
