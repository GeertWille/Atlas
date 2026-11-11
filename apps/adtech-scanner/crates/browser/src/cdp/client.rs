//! A small Chrome DevTools Protocol client over the browser WebSocket.
//!
//! Messages stay untyped JSON on purpose: the handful of methods the scanner
//! uses are stable, and generated protocol bindings break on events they do not
//! know when Chrome is newer than the bindings.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;

/// Generous: a wedged renderer must not hang a scan forever, nothing more.
const CALL_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum CdpError {
    #[error("could not connect to the browser: {0}")]
    Connect(String),
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
    /// Connects and returns the client plus the stream of protocol events.
    /// Dropping the client closes the connection.
    pub(crate) async fn connect(
        ws_url: &str,
    ) -> Result<(Self, mpsc::UnboundedReceiver<CdpEvent>), CdpError> {
        // Script bodies arrive as single messages; the default 64 MiB cap is too tight.
        let config = WebSocketConfig::default()
            .max_message_size(None)
            .max_frame_size(None);
        let (socket, _) = tokio_tungstenite::connect_async_with_config(ws_url, Some(config), false)
            .await
            .map_err(|error| CdpError::Connect(error.to_string()))?;
        let (commands, command_queue) = mpsc::unbounded_channel();
        let (events, event_stream) = mpsc::unbounded_channel();
        tokio::spawn(pump(socket, command_queue, events));
        Ok((
            Self {
                commands,
                next_id: AtomicU64::new(1),
            },
            event_stream,
        ))
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

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn pump(
    socket: Socket,
    mut commands: mpsc::UnboundedReceiver<Command>,
    events: mpsc::UnboundedSender<CdpEvent>,
) {
    let (mut sink, mut stream) = socket.split();
    let mut awaiting = Awaiting::new();
    loop {
        tokio::select! {
            command = commands.recv() => {
                let Some(command) = command else { break };
                awaiting.insert(command.id, (command.method, command.reply));
                if sink.send(Message::text(command.text)).await.is_err() {
                    break;
                }
            }
            frame = stream.next() => match frame {
                Some(Ok(Message::Text(text))) => {
                    let Ok(message) = serde_json::from_str::<Value>(text.as_str()) else { continue };
                    route(message, &mut awaiting, &events);
                }
                Some(Ok(_)) => {}
                Some(Err(_)) | None => break,
            },
        }
    }
    let _ = sink.close().await;
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
