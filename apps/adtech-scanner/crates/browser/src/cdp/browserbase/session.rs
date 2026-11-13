//! One browser at Browserbase, from the request for it to the
//! confirmation that it has ended. Created without `keepAlive`, it ends by
//! itself when its connection goes away or its time limit passes; it is
//! still released explicitly every time, because browser time is billed.

use std::time::Duration;

use adscan_model::BrowserbaseCredentials;
use serde_json::json;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use super::api::{Api, BrowserAddress, SessionState};
use super::failure::ApiFailure;
use super::order::SessionOrder;
use crate::cdp::{Cdp, CdpError, CdpEvent};

/// The browser answers its first command within this time, or is taken
/// for unreachable.
const FIRST_ANSWER_PATIENCE: Duration = Duration::from_secs(30);
/// How long `Browser.close` is given to be answered. The browser ends
/// within half a second of it.
const CLOSE_PATIENCE: Duration = Duration::from_secs(3);
const RELEASE_ATTEMPTS: u32 = 3;
const RELEASE_RETRY_PAUSE: Duration = Duration::from_secs(1);
/// How often, and how far apart, the service is asked whether a session
/// has ended, when the answer to its release did not say so: the status of
/// a session may be a moment behind.
const END_LOOKS: u32 = 6;
const END_LOOK_PAUSE: Duration = Duration::from_secs(1);
/// A socket that closes this near the time limit was closed by it.
const TIME_LIMIT_TOLERANCE: Duration = Duration::from_secs(5);

/// How long a browser is waited for while the project is at its limit,
/// counted on the clock from the first request for it, and how long
/// before each new request. The service says no more than "too many" (no
/// `retry-after`), and the limit is shared with whatever else uses the
/// project.
const LIMIT_PATIENCE: Duration = Duration::from_secs(60);
const LIMIT_PAUSE: Duration = Duration::from_secs(5);
/// A wait shorter than this is not worth one more request.
const SHORTEST_LIMIT_PAUSE: Duration = Duration::from_secs(1);

/// How often, and how far apart, the service is asked for a browser it
/// may have started without the scanner hearing of it. A request that was
/// given up on may still be on its way to becoming a session.
const STRAY_LOOKS: u32 = 3;
const STRAY_LOOK_PAUSE: Duration = Duration::from_secs(2);

/// How long to wait before asking for a browser again, `asking_for` after
/// the first request; `None` when the time for asking is up. The last wait
/// is cut to end with that time.
fn pause_at_the_limit(asking_for: Duration) -> Option<Duration> {
    let left = LIMIT_PATIENCE.checked_sub(asking_for)?;
    let pause = LIMIT_PAUSE.min(left);
    (pause >= SHORTEST_LIMIT_PAUSE).then_some(pause)
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum StartFailure {
    /// Cancelled while waiting for a free browser. No session exists.
    Cancelled,
    Refused(ApiFailure),
}

/// What happens on the way to a browser, for the audit trail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StartNote {
    /// The project is at its limit: wait number `attempt`, of `pause`.
    Waiting { attempt: u32, pause: Duration },
    /// The answer to a request for a browser was lost, and the browsers
    /// the service had started for it all the same were ended.
    StraysReleased(u32),
}

pub(crate) struct RemoteBrowser {
    api: Api,
    credentials: BrowserbaseCredentials,
    session_id: String,
    address: BrowserAddress,
    created: Instant,
    time_limit: Duration,
}

impl RemoteBrowser {
    /// Asks for a browser, waiting while the project is at its limit.
    /// `note` hears of every wait and of browsers that had to be ended.
    /// The request itself is never abandoned half-way: a session that was
    /// started without the scanner knowing its id could not be released.
    /// When a request fails in a way that may have started one all the
    /// same, the sessions of this installation and this scan are looked
    /// for and ended before the failure is returned.
    pub(crate) async fn start(
        api: &Api,
        credentials: &BrowserbaseCredentials,
        order: &SessionOrder,
        cancel: &CancellationToken,
        mut note: impl FnMut(StartNote),
    ) -> Result<Self, StartFailure> {
        let first_asked = Instant::now();
        let mut attempt = 0;
        loop {
            if cancel.is_cancelled() {
                return Err(StartFailure::Cancelled);
            }
            let refusal = match api.create_session(credentials, order).await {
                Ok((state, address)) => {
                    return Ok(Self {
                        api: api.clone(),
                        credentials: credentials.clone(),
                        session_id: state.id,
                        address,
                        created: Instant::now(),
                        time_limit: Duration::from_secs(order.timeout()),
                    });
                }
                Err(refusal) => refusal,
            };
            if refusal.session_may_exist {
                let released = release_strays(api, credentials, order).await;
                if released > 0 {
                    note(StartNote::StraysReleased(released));
                }
            }
            let pause = match &refusal.failure {
                ApiFailure::Limit(_) => pause_at_the_limit(first_asked.elapsed()),
                _ => None,
            };
            let Some(pause) = pause else {
                return Err(StartFailure::Refused(refusal.failure));
            };
            attempt += 1;
            note(StartNote::Waiting { attempt, pause });
            tokio::select! {
                biased;
                _ = cancel.cancelled() => return Err(StartFailure::Cancelled),
                _ = tokio::time::sleep(pause) => {}
            }
        }
    }

    /// The service's id of the session. An identifier, not a credential.
    pub(crate) fn session_id(&self) -> &str {
        &self.session_id
    }

    /// Opens the one DevTools connection the session takes, and waits
    /// until the browser answers.
    pub(crate) async fn connect(&self) -> Result<(Cdp, UnboundedReceiver<CdpEvent>), ApiFailure> {
        let (from_browser, to_browser) = self.api.connect(&self.address).await?;
        let (cdp, events) = Cdp::start(from_browser, to_browser);
        let ready = cdp.send(None, "Browser.getVersion", json!({}));
        let unanswered = |why: &str| {
            ApiFailure::Failed(format!(
                "The scanner could not connect to the Browserbase browser: {why}"
            ))
        };
        match tokio::time::timeout(FIRST_ANSWER_PATIENCE, ready).await {
            Ok(Ok(_)) => Ok((cdp, events)),
            Ok(Err(CdpError::Closed)) => Err(unanswered("the connection closed at once")),
            Ok(Err(error)) => Err(unanswered(&error.to_string())),
            Err(_) => Err(unanswered("the browser did not answer in time")),
        }
    }

    /// Whether the session is as old as the time limit it was given: its
    /// connection closing now is the service ending it.
    pub(crate) fn has_reached_its_time_limit(&self) -> bool {
        self.created.elapsed() + TIME_LIMIT_TOLERANCE >= self.time_limit
    }

    /// Ends the browser and asks the service whether it has. Returns the
    /// session as it ended, or `None` when its end could not be confirmed.
    pub(crate) async fn end(self, cdp: Option<&Cdp>) -> Option<SessionState> {
        if let Some(cdp) = cdp {
            let closed = cdp.send(None, "Browser.close", json!({}));
            let _ = tokio::time::timeout(CLOSE_PATIENCE, closed).await;
        }
        let released = release(&self.api, &self.credentials, &self.session_id).await;
        confirmed_end(&self.api, &self.credentials, &self.session_id, released).await
    }
}

/// Asks the service to end a session, a few times over when it does not
/// answer. Returns the session as the service then described it.
async fn release(
    api: &Api,
    credentials: &BrowserbaseCredentials,
    session_id: &str,
) -> Option<SessionState> {
    for attempt in 1..=RELEASE_ATTEMPTS {
        match api.release(credentials, session_id).await {
            Ok(state) => return Some(state),
            Err(_) if attempt < RELEASE_ATTEMPTS => {
                tokio::time::sleep(RELEASE_RETRY_PAUSE).await;
            }
            Err(_) => {}
        }
    }
    None
}

/// The session as it ended, once either the answer to its release or a
/// later reading says that it has; `None` when neither does.
///
/// A reading that says so is the newer word and is what is returned. One
/// that still says "running" does not undo a release that answered
/// "ended": the status can be a moment behind. Without such an answer the
/// session is read a few times more, because it also ends without the
/// release, when its connection goes away.
async fn confirmed_end(
    api: &Api,
    credentials: &BrowserbaseCredentials,
    session_id: &str,
    released: Option<SessionState>,
) -> Option<SessionState> {
    let released_as_ended = released.filter(|state| !state.running);
    for look in 0..END_LOOKS {
        if look > 0 {
            tokio::time::sleep(END_LOOK_PAUSE).await;
        }
        match api.session(credentials, session_id).await {
            Ok(read) if !read.running => return Some(read),
            _ if released_as_ended.is_some() => break,
            _ => {}
        }
    }
    released_as_ended
}

/// Ends the browsers the service started for `order` without the scanner
/// learning their ids, and returns how many it ended. Only a running
/// session that carries this app's tag, this installation's id and this
/// scan's id is touched: the project may be shared with other
/// applications, and this installation may be running another scan.
async fn release_strays(
    api: &Api,
    credentials: &BrowserbaseCredentials,
    order: &SessionOrder,
) -> u32 {
    for look in 0..STRAY_LOOKS {
        if look > 0 {
            tokio::time::sleep(STRAY_LOOK_PAUSE).await;
        }
        let Ok(running) = api.running_sessions(credentials, &order.install_id).await else {
            continue;
        };
        let strays: Vec<SessionState> = running
            .into_iter()
            .filter(|session| {
                session.running && session.is_of_scan(&order.install_id, &order.scan_id)
            })
            .collect();
        if strays.is_empty() {
            continue;
        }
        let mut ended = 0;
        for stray in &strays {
            if release(api, credentials, &stray.id).await.is_some() {
                ended += 1;
            }
        }
        return ended;
    }
    0
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use adscan_model::{BrowserbaseRegion, BrowserbaseSessionSettings};
    use serde_json::{Value, json};

    use super::super::order::Exit;
    use super::super::stand_in::{Answer, StandIn};
    use super::*;

    const KEY: &str = "bb_live_K3y";
    const PROJECT: &str = "3f8a1c52-6e0b-4b7d-9a43-5d2c8e9f1a07";
    const INSTALL: &str = "0123456789abcdef0123456789abcdef";
    const SCAN: &str = "7c9e6679-7425-40de-944b-e07fc1f90ae7";
    const SESSION: &str = "5e551001-0000-4000-8000-000000000001";
    const SESSION_PATH: &str = "/v1/sessions/5e551001-0000-4000-8000-000000000001";

    fn credentials() -> BrowserbaseCredentials {
        BrowserbaseCredentials {
            api_key: KEY.into(),
            project_id: PROJECT.into(),
        }
    }

    fn order() -> SessionOrder {
        SessionOrder {
            region: BrowserbaseRegion::EuCentral1,
            timeout_seconds: 240,
            settings: BrowserbaseSessionSettings::STANDARD,
            exit: Exit::Region,
            context_id: None,
            install_id: INSTALL.into(),
            scan_id: SCAN.into(),
        }
    }

    fn session(id: &str, status: &str, metadata: Value) -> Value {
        let mut session = json!({
            "id": id,
            "status": status,
            "startedAt": "2026-11-13T08:26:07.000Z",
            "proxyBytes": 0,
            "userMetadata": metadata,
        });
        if status != "RUNNING" {
            session["endedAt"] = json!("2026-11-13T08:26:48.000Z");
        }
        session
    }

    fn ours(status: &str) -> Value {
        session(
            SESSION,
            status,
            json!({ "app": "adscan", "install": INSTALL, "scan": SCAN }),
        )
    }

    /// Asks the stand-in for a browser and collects what is noted on the way.
    async fn start(api: &Api) -> (Result<RemoteBrowser, StartFailure>, Vec<StartNote>) {
        let notes = Mutex::new(Vec::new());
        let started = RemoteBrowser::start(
            api,
            &credentials(),
            &order(),
            &CancellationToken::new(),
            |note| notes.lock().unwrap().push(note),
        )
        .await;
        (started, notes.into_inner().unwrap())
    }

    fn releases(service: &StandIn) -> Vec<String> {
        service
            .asked()
            .into_iter()
            .filter(|asked| asked.method == "POST" && asked.path != "/v1/sessions")
            .map(|asked| asked.path.trim_start_matches("/v1/sessions/").to_string())
            .collect()
    }

    #[test]
    fn a_free_browser_is_waited_for_five_seconds_at_a_time_for_a_minute_on_the_clock() {
        // With requests that take no time: twelve waits of five seconds.
        let mut asking_for = Duration::ZERO;
        let mut waits = 0;
        while let Some(pause) = pause_at_the_limit(asking_for) {
            assert_eq!(pause, Duration::from_secs(5));
            asking_for += pause;
            waits += 1;
        }
        assert_eq!(waits, 12);
        assert_eq!(asking_for, Duration::from_secs(60));

        // The time the requests themselves took counts: a wait never ends
        // after the minute, and one too short to be worth it is not begun.
        assert_eq!(
            pause_at_the_limit(Duration::from_secs(56)),
            Some(Duration::from_secs(4))
        );
        assert_eq!(
            pause_at_the_limit(Duration::from_millis(58_500)),
            Some(Duration::from_millis(1_500))
        );
        assert_eq!(pause_at_the_limit(Duration::from_millis(59_001)), None);
        assert_eq!(pause_at_the_limit(Duration::from_secs(60)), None);
        assert_eq!(pause_at_the_limit(Duration::from_secs(450)), None);
    }

    /// The request for a browser is given up on, but the service started
    /// one. Nobody has its id: it is found by what it was tagged with, and
    /// nothing with other tags is touched, whoever started it.
    #[tokio::test]
    async fn a_browser_started_behind_a_lost_answer_is_found_by_its_tags_and_ended() {
        let other_scan = "11111111-2222-4333-8444-555555555555";
        let other_install = "ffffffffffffffffffffffffffffffff";
        let tagged = |app: &str, install: &str, scan: &str| json!({ "app": app, "install": install, "scan": scan });
        let running = json!([
            session(
                "stray-of-this-scan",
                "RUNNING",
                tagged("adscan", INSTALL, SCAN)
            ),
            session(
                "another-scan-of-this-install",
                "RUNNING",
                tagged("adscan", INSTALL, other_scan)
            ),
            session(
                "this-scan-id-of-another-install",
                "RUNNING",
                tagged("adscan", other_install, SCAN)
            ),
            session(
                "another-application",
                "RUNNING",
                tagged("other", INSTALL, SCAN)
            ),
            session(
                "the-production-application",
                "RUNNING",
                json!({ "scanId": SCAN, "organizationId": "3" })
            ),
            session("untagged", "RUNNING", json!({})),
            session(
                "this-scan-but-ended",
                "COMPLETED",
                tagged("adscan", INSTALL, SCAN)
            ),
        ]);
        let service = StandIn::start(move |asked, _| {
            if asked.is("POST", "/v1/sessions") {
                // Started, and answered too late to be heard.
                Answer::Late(Duration::from_millis(600), 201, ours("RUNNING"))
            } else if asked.is("GET", "/v1/sessions") {
                Answer::Json(200, running.clone())
            } else {
                Answer::Json(200, session("released", "COMPLETED", json!({})))
            }
        })
        .await;
        let api = Api::new(&service.url, None)
            .unwrap()
            .giving_a_new_browser(Duration::from_millis(150));

        let (started, notes) = start(&api).await;

        assert_eq!(
            started.err(),
            Some(StartFailure::Refused(ApiFailure::Failed(
                "Browserbase could not be reached: it did not answer in time".into()
            )))
        );
        assert_eq!(releases(&service), ["stray-of-this-scan"]);
        assert_eq!(notes, [StartNote::StraysReleased(1)]);
        let asked = service.asked();
        let listed = asked
            .iter()
            .find(|asked| asked.is("GET", "/v1/sessions"))
            .unwrap();
        assert_eq!(
            listed.query,
            format!("status=RUNNING&q=user_metadata['install']:'{INSTALL}'")
        );
        let release = asked
            .iter()
            .find(|asked| asked.method == "POST" && asked.path != "/v1/sessions")
            .unwrap();
        assert_eq!(
            release.body,
            json!({ "projectId": PROJECT, "status": "REQUEST_RELEASE" })
        );
        assert!(asked.iter().all(|asked| asked.key.as_deref() == Some(KEY)));
    }

    /// An answer that names no session, and a failure of the service or a
    /// gateway in front of it, say nothing about whether a browser runs.
    #[tokio::test]
    async fn an_answer_that_does_not_settle_whether_a_browser_runs_is_followed_by_a_look() {
        for (status, answer) in [
            (201, json!({})),
            (201, json!("ok")),
            (504, json!({ "message": "Gateway Timeout" })),
        ] {
            let service = StandIn::start(move |asked, _| {
                if asked.is("POST", "/v1/sessions") {
                    Answer::Json(status, answer.clone())
                } else if asked.is("GET", "/v1/sessions") {
                    Answer::Json(200, json!([ours("RUNNING")]))
                } else {
                    Answer::Json(200, ours("COMPLETED"))
                }
            })
            .await;
            let api = Api::new(&service.url, None).unwrap();

            let (started, notes) = start(&api).await;

            assert!(
                matches!(started, Err(StartFailure::Refused(ApiFailure::Failed(_)))),
                "{status}"
            );
            assert_eq!(releases(&service), [SESSION], "{status}");
            assert_eq!(notes, [StartNote::StraysReleased(1)], "{status}");
        }
    }

    /// A refusal is the service's own word that it started nothing, and a
    /// request that never left started nothing either: nothing is listed
    /// and nothing is released.
    #[tokio::test]
    async fn a_refusal_or_a_request_that_never_left_is_not_followed_by_a_look() {
        for status in [400, 401, 402, 403] {
            let service = StandIn::start(move |_, _| {
                Answer::Json(status, json!({ "statusCode": status, "message": "No" }))
            })
            .await;
            let api = Api::new(&service.url, None).unwrap();

            let (started, notes) = start(&api).await;

            assert!(matches!(started, Err(StartFailure::Refused(_))), "{status}");
            assert_eq!(service.asked().len(), 1, "{status}: {:?}", service.asked());
            assert!(notes.is_empty(), "{status}");
        }

        // At the limit: waited for, not looked behind. A cancel ends the wait.
        let service = StandIn::start(|_, _| {
            Answer::Json(
                429,
                json!({ "statusCode": 429, "message": "Too Many Requests" }),
            )
        })
        .await;
        let api = Api::new(&service.url, None).unwrap();
        let cancel = CancellationToken::new();
        let started = RemoteBrowser::start(&api, &credentials(), &order(), &cancel, |note| {
            assert!(matches!(note, StartNote::Waiting { attempt: 1, .. }));
            cancel.cancel();
        })
        .await;
        assert_eq!(started.err(), Some(StartFailure::Cancelled));
        assert_eq!(service.asked().len(), 1);

        // Nobody listens there: the request was never made.
        let nobody = {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            listener.local_addr().unwrap().port()
        };
        let api = Api::new(&format!("http://127.0.0.1:{nobody}"), None).unwrap();
        let unsent = api
            .create_session(&credentials(), &order())
            .await
            .err()
            .unwrap();
        assert!(!unsent.session_may_exist, "{unsent:?}");
    }

    /// The browser may come up after the request for it was given up on:
    /// it is looked for more than once, and then no longer.
    #[tokio::test]
    async fn a_browser_that_shows_up_late_is_still_found_and_one_that_never_does_is_let_be() {
        let late = StandIn::start(|asked, before| {
            if asked.is("POST", "/v1/sessions") {
                Answer::Json(502, json!({}))
            } else if asked.is("GET", "/v1/sessions") && before == 0 {
                Answer::Json(200, json!([]))
            } else if asked.is("GET", "/v1/sessions") {
                Answer::Json(200, json!([ours("RUNNING")]))
            } else {
                Answer::Json(200, ours("COMPLETED"))
            }
        })
        .await;
        let never = StandIn::start(|asked, _| {
            if asked.is("POST", "/v1/sessions") {
                Answer::Json(502, json!({}))
            } else {
                Answer::Json(200, json!([]))
            }
        })
        .await;
        let (late_api, never_api) = (
            Api::new(&late.url, None).unwrap(),
            Api::new(&never.url, None).unwrap(),
        );

        let ((_, found), (_, nothing)) = tokio::join!(start(&late_api), start(&never_api));

        assert_eq!(found, [StartNote::StraysReleased(1)]);
        assert_eq!(late.count("GET", "/v1/sessions"), 2);
        assert_eq!(releases(&late), [SESSION]);
        assert!(nothing.is_empty());
        assert_eq!(never.count("GET", "/v1/sessions"), STRAY_LOOKS as usize);
        assert!(releases(&never).is_empty());
    }

    /// A stand-in that hands out one session, answers its release with
    /// `released` and every reading of it with what `read` says for the
    /// number of readings before.
    async fn ending(
        released: impl Fn(usize) -> Answer + Send + Sync + 'static,
        read: impl Fn(usize) -> Answer + Send + Sync + 'static,
    ) -> (StandIn, RemoteBrowser) {
        let service = StandIn::start(move |asked, before| {
            if asked.is("POST", "/v1/sessions") {
                Answer::Json(201, ours("RUNNING"))
            } else if asked.method == "POST" {
                released(before)
            } else {
                read(before)
            }
        })
        .await;
        let api = Api::new(&service.url, None).unwrap();
        let browser = start(&api).await.0.unwrap();
        (service, browser)
    }

    /// The release answered that the session has ended. A reading right
    /// after it that still says "running" is the status lagging, not the
    /// browser running.
    #[tokio::test]
    async fn a_release_that_answered_ended_stands_when_a_reading_lags_behind_it() {
        let (service, browser) = ending(
            |_| Answer::Json(200, ours("COMPLETED")),
            |_| Answer::Json(200, ours("RUNNING")),
        )
        .await;

        let ended = browser.end(None).await.expect("the release said so");

        assert!(!ended.running);
        assert_eq!(ended.seconds(), Some(41.0));
        assert_eq!(service.count("POST", SESSION_PATH), 1);
        assert_eq!(service.count("GET", SESSION_PATH), 1);
    }

    /// The other way round: the answer to the release still says
    /// "running", and the session reads as ended a moment later.
    #[tokio::test]
    async fn a_session_that_reads_as_ended_a_moment_after_its_release_has_ended() {
        let (service, browser) = ending(
            |_| Answer::Json(200, ours("RUNNING")),
            |before| Answer::Json(200, ours(if before < 2 { "RUNNING" } else { "COMPLETED" })),
        )
        .await;

        let ended = browser.end(None).await.expect("the third reading said so");

        assert!(!ended.running);
        assert_eq!(service.count("GET", SESSION_PATH), 3);
    }

    /// No release got through, but the session ended all the same, as it
    /// does when its connection goes away.
    #[tokio::test]
    async fn a_session_that_ended_without_its_release_is_read_as_ended() {
        let (service, browser) = ending(
            |_| Answer::Json(500, json!({ "message": "Internal Server Error" })),
            |_| Answer::Json(200, ours("COMPLETED")),
        )
        .await;

        assert!(browser.end(None).await.is_some());
        assert_eq!(
            service.count("POST", SESSION_PATH),
            RELEASE_ATTEMPTS as usize
        );
        assert_eq!(service.count("GET", SESSION_PATH), 1);
    }

    /// Neither answer says so within a few seconds: the end is not
    /// confirmed, and whoever keeps the record keeps the session on it.
    #[tokio::test]
    async fn a_session_that_keeps_reading_as_running_is_not_reported_as_ended() {
        let (service, browser) = ending(
            |_| Answer::Json(200, ours("RUNNING")),
            |before| match before % 2 {
                0 => Answer::Json(200, ours("RUNNING")),
                _ => Answer::Json(503, json!({})),
            },
        )
        .await;
        let began = Instant::now();

        assert_eq!(browser.end(None).await, None);

        assert_eq!(service.count("GET", SESSION_PATH), END_LOOKS as usize);
        assert!(began.elapsed() >= END_LOOK_PAUSE * (END_LOOKS - 1));
    }
}
