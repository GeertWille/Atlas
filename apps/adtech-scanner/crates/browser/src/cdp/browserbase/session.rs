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
/// A socket that closes this near the time limit was closed by it.
const TIME_LIMIT_TOLERANCE: Duration = Duration::from_secs(5);

/// How long a browser is waited for while the project is at its limit.
/// The service says no more than "too many" (no `retry-after`), and the
/// limit is shared with whatever else uses the project.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Waiting {
    pub pause: Duration,
    pub at_most: Duration,
}

impl Waiting {
    pub(crate) const AT_THE_LIMIT: Self = Self {
        pause: Duration::from_secs(5),
        at_most: Duration::from_secs(60),
    };

    /// How long to wait before asking again, after `waited` already; `None`
    /// when that would pass the total.
    pub(crate) fn before_asking_again(self, waited: Duration) -> Option<Duration> {
        (waited + self.pause <= self.at_most).then_some(self.pause)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum StartFailure {
    /// Cancelled while waiting for a free browser. No session exists.
    Cancelled,
    Refused(ApiFailure),
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
    /// `on_wait` hears of every wait: its number and its length. The
    /// request itself is never abandoned half-way: a session that was
    /// started without the scanner knowing its id could not be released.
    pub(crate) async fn start(
        api: &Api,
        credentials: &BrowserbaseCredentials,
        order: &SessionOrder,
        cancel: &CancellationToken,
        mut on_wait: impl FnMut(u32, Duration),
    ) -> Result<Self, StartFailure> {
        let waiting = Waiting::AT_THE_LIMIT;
        let mut waited = Duration::ZERO;
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
            let pause = match &refusal {
                ApiFailure::Limit(_) => waiting.before_asking_again(waited),
                _ => None,
            };
            let Some(pause) = pause else {
                return Err(StartFailure::Refused(refusal));
            };
            attempt += 1;
            on_wait(attempt, pause);
            tokio::select! {
                biased;
                _ = cancel.cancelled() => return Err(StartFailure::Cancelled),
                _ = tokio::time::sleep(pause) => waited += pause,
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
        let mut released = None;
        for attempt in 1..=RELEASE_ATTEMPTS {
            match self.api.release(&self.credentials, &self.session_id).await {
                Ok(state) => {
                    released = Some(state);
                    break;
                }
                Err(_) if attempt < RELEASE_ATTEMPTS => {
                    tokio::time::sleep(RELEASE_RETRY_PAUSE).await;
                }
                Err(_) => {}
            }
        }
        // It may have ended without the release: it does when its
        // connection goes away.
        let read = self.api.session(&self.credentials, &self.session_id).await;
        read.ok().or(released).filter(|state| !state.running)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_free_browser_is_waited_for_five_seconds_at_a_time_for_a_minute() {
        let waiting = Waiting::AT_THE_LIMIT;
        let mut waited = Duration::ZERO;
        let mut waits = 0;
        while let Some(pause) = waiting.before_asking_again(waited) {
            assert_eq!(pause, Duration::from_secs(5));
            waited += pause;
            waits += 1;
        }

        assert_eq!(waits, 12);
        assert_eq!(waited, Duration::from_secs(60));
        assert_eq!(waiting.before_asking_again(Duration::from_secs(56)), None);
    }
}
