//! A small Chrome DevTools Protocol client. It speaks over any byte stream
//! pair; `pipe` frames the messages.
//!
//! Messages stay untyped JSON on purpose: the handful of methods the scanner
//! uses are stable, and generated protocol bindings break on events they do not
//! know when Chrome is newer than the bindings.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{mpsc, oneshot};

use super::pipe::{MessageReader, write_message};

/// Generous: a wedged renderer must not hang a scan forever, nothing more.
const CALL_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum CdpError {
    #[error("the connection to the browser closed")]
    Closed,
    #[error("{method} failed: {message}")]
    Rejected {
        method: &'static str,
        message: String,
    },
    #[error("{0} did not answer in time")]
    Timeout(&'static str),
}

#[derive(Debug)]
pub(crate) struct CdpEvent {
    pub method: String,
    pub params: Value,
    /// Flat-mode session the event belongs to; `None` for browser-level events.
    pub session_id: Option<String>,
}

type Reply = oneshot::Sender<Result<Value, CdpError>>;

struct Command {
    id: u64,
    method: &'static str,
    text: String,
    reply: Reply,
}

/// Commands sent and not yet answered, by id.
type Awaiting = HashMap<u64, (&'static str, Reply)>;

pub(crate) struct Cdp {
    commands: mpsc::UnboundedSender<Command>,
    next_id: AtomicU64,
}

impl Cdp {
    /// Starts talking to the browser at the other end of the two streams and
    /// returns the client plus the stream of protocol events. Dropping the
    /// client closes `to_browser`.
    pub(crate) fn start<R, W>(
        from_browser: R,
        to_browser: W,
    ) -> (Self, mpsc::UnboundedReceiver<CdpEvent>)
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        let (commands, command_queue) = mpsc::unbounded_channel();
        let (events, event_stream) = mpsc::unbounded_channel();
        tokio::spawn(pump(
            MessageReader::new(from_browser),
            to_browser,
            command_queue,
            events,
        ));
        (
            Self {
                commands,
                next_id: AtomicU64::new(1),
            },
            event_stream,
        )
    }

    /// Queues the command immediately and returns a future for its result, so
    /// callers can pipeline commands without awaiting each one.
    pub(crate) fn send(
        &self,
        session_id: Option<&str>,
        method: &'static str,
        params: Value,
    ) -> impl Future<Output = Result<Value, CdpError>> + Send + 'static + use<> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let mut message = json!({ "id": id, "method": method, "params": params });
        if let Some(session_id) = session_id {
            message["sessionId"] = json!(session_id);
        }
        let (reply, response) = oneshot::channel();
        // A closed connection drops `reply`, which surfaces as `Closed` below.
        let _ = self.commands.send(Command {
            id,
            method,
            text: message.to_string(),
            reply,
        });
        async move {
            match tokio::time::timeout(CALL_TIMEOUT, response).await {
                Ok(Ok(result)) => result,
                Ok(Err(_)) => Err(CdpError::Closed),
                Err(_) => Err(CdpError::Timeout(method)),
            }
        }
    }

    /// Sends a command whose result does not matter.
    pub(crate) fn notify(&self, session_id: Option<&str>, method: &'static str, params: Value) {
        drop(self.send(session_id, method, params));
    }
}

async fn pump<R, W>(
    mut from_browser: MessageReader<R>,
    mut to_browser: W,
    mut commands: mpsc::UnboundedReceiver<Command>,
    events: mpsc::UnboundedSender<CdpEvent>,
) where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut awaiting = Awaiting::new();
    loop {
        tokio::select! {
            command = commands.recv() => {
                let Some(command) = command else { break };
                awaiting.insert(command.id, (command.method, command.reply));
                if write_message(&mut to_browser, &command.text).await.is_err() {
                    break;
                }
            }
            message = from_browser.next() => {
                let Ok(Some(message)) = message else { break };
                let Ok(message) = serde_json::from_slice::<Value>(&message) else { continue };
                route(message, &mut awaiting, &events);
            }
        }
    }
}

fn route(mut message: Value, awaiting: &mut Awaiting, events: &mpsc::UnboundedSender<CdpEvent>) {
    if let Some(id) = message["id"].as_u64() {
        let Some((method, reply)) = awaiting.remove(&id) else {
            return;
        };
        let result = match message.get("error") {
            Some(error) => Err(CdpError::Rejected {
                method,
                message: error["message"]
                    .as_str()
                    .unwrap_or("unknown error")
                    .to_string(),
            }),
            None => Ok(message["result"].take()),
        };
        let _ = reply.send(result);
        return;
    }
    if let Some(method) = message["method"].as_str() {
        let _ = events.send(CdpEvent {
            method: method.to_string(),
            session_id: message["sessionId"].as_str().map(str::to_string),
            params: message["params"].take(),
        });
    }
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncWriteExt, DuplexStream};

    use super::*;

    /// The browser's side of the connection.
    struct Browser {
        commands: MessageReader<DuplexStream>,
        replies: DuplexStream,
    }

    impl Browser {
        async fn received(&mut self) -> Value {
            let command = self.commands.next().await.unwrap().unwrap();
            serde_json::from_slice(&command).unwrap()
        }

        async fn say(&mut self, message: Value) {
            write_message(&mut self.replies, &message.to_string())
                .await
                .unwrap();
        }
    }

    fn connected() -> (Cdp, mpsc::UnboundedReceiver<CdpEvent>, Browser) {
        let (to_browser, commands) = tokio::io::duplex(1024);
        let (replies, from_browser) = tokio::io::duplex(1024);
        let (cdp, events) = Cdp::start(from_browser, to_browser);
        let browser = Browser {
            commands: MessageReader::new(commands),
            replies,
        };
        (cdp, events, browser)
    }

    #[tokio::test]
    async fn replies_reach_their_commands_in_whatever_order_they_arrive() {
        let (cdp, _events, mut browser) = connected();
        let version = cdp.send(None, "Browser.getVersion", json!({}));
        let frames = cdp.send(Some("tab"), "Page.getFrameTree", json!({ "depth": 1 }));

        assert_eq!(
            browser.received().await,
            json!({ "id": 1, "method": "Browser.getVersion", "params": {} })
        );
        assert_eq!(
            browser.received().await,
            json!({
                "id": 2,
                "method": "Page.getFrameTree",
                "params": { "depth": 1 },
                "sessionId": "tab",
            })
        );
        browser
            .say(json!({ "id": 2, "result": { "frameTree": {} }, "sessionId": "tab" }))
            .await;
        browser
            .say(json!({ "id": 1, "result": { "product": "Chrome/141" } }))
            .await;

        assert_eq!(frames.await, Ok(json!({ "frameTree": {} })));
        assert_eq!(version.await, Ok(json!({ "product": "Chrome/141" })));
    }

    #[tokio::test]
    async fn a_protocol_error_names_the_method_that_failed() {
        let (cdp, _events, mut browser) = connected();
        let reply = cdp.send(None, "Target.attachToTarget", json!({}));
        browser.received().await;
        browser
            .say(json!({ "id": 1, "error": { "code": -32602, "message": "No target" } }))
            .await;

        assert_eq!(
            reply.await,
            Err(CdpError::Rejected {
                method: "Target.attachToTarget",
                message: "No target".to_string(),
            })
        );
    }

    #[tokio::test]
    async fn events_carry_their_session_and_garbage_is_skipped() {
        let (_cdp, mut events, mut browser) = connected();
        browser.replies.write_all(b"not json\0").await.unwrap();
        browser
            .say(json!({ "method": "Target.targetCreated", "params": { "n": 1 } }))
            .await;
        browser
            .say(json!({ "method": "Page.loadEventFired", "params": {}, "sessionId": "tab" }))
            .await;

        let browser_level = events.recv().await.unwrap();
        assert_eq!(browser_level.method, "Target.targetCreated");
        assert_eq!(browser_level.params, json!({ "n": 1 }));
        assert_eq!(browser_level.session_id, None);
        let from_tab = events.recv().await.unwrap();
        assert_eq!(from_tab.method, "Page.loadEventFired");
        assert_eq!(from_tab.session_id.as_deref(), Some("tab"));
    }

    #[tokio::test]
    async fn a_browser_that_goes_away_fails_pending_and_later_commands() {
        let (cdp, mut events, mut browser) = connected();
        let pending = cdp.send(None, "Browser.getVersion", json!({}));
        browser.received().await;

        drop(browser);

        assert_eq!(pending.await, Err(CdpError::Closed));
        assert_eq!(
            cdp.send(None, "Browser.getVersion", json!({})).await,
            Err(CdpError::Closed)
        );
        assert!(events.recv().await.is_none());
    }

    /// Chrome exits when its end of the pipe reports end of file; this is
    /// what makes it go away with the app.
    #[tokio::test]
    async fn dropping_the_client_closes_the_stream_to_the_browser() {
        let (cdp, _events, mut browser) = connected();

        drop(cdp);

        assert_eq!(browser.commands.next().await.unwrap(), None);
    }
}
