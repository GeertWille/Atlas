//! A programmable HTTP server in the place of Browserbase's API, for the
//! tests of this module: each test says what every request is answered
//! with, and reads back what was asked.

use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, Method, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use serde_json::Value;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

/// A request the stand-in received.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Asked {
    pub method: String,
    pub path: String,
    pub query: String,
    /// The API key it came with.
    pub key: Option<String>,
    pub body: Value,
}

impl Asked {
    pub(crate) fn is(&self, method: &str, path: &str) -> bool {
        self.method == method && self.path == path
    }
}

pub(crate) enum Answer {
    Json(u16, Value),
    /// The same, after a pause.
    Late(Duration, u16, Value),
    /// A 302 to that address.
    Redirect(String),
}

type Script = dyn Fn(&Asked, usize) -> Answer + Send + Sync;

struct Service {
    script: Box<Script>,
    asked: Mutex<Vec<Asked>>,
}

/// Stops when dropped.
pub(crate) struct StandIn {
    pub url: String,
    pub port: u16,
    service: Arc<Service>,
    shutdown: CancellationToken,
}

impl StandIn {
    /// `script` answers every request; its second argument says how many
    /// requests of the same method and path came before this one.
    pub(crate) async fn start(
        script: impl Fn(&Asked, usize) -> Answer + Send + Sync + 'static,
    ) -> Self {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let service = Arc::new(Service {
            script: Box::new(script),
            asked: Mutex::default(),
        });
        let shutdown = CancellationToken::new();
        let stopped = shutdown.clone().cancelled_owned();
        let routes = Router::new().fallback(answer).with_state(service.clone());
        tokio::spawn(async move {
            let _ = axum::serve(listener, routes)
                .with_graceful_shutdown(stopped)
                .await;
        });
        Self {
            url: format!("http://127.0.0.1:{port}"),
            port,
            service,
            shutdown,
        }
    }

    /// Every request so far, in the order they arrived.
    pub(crate) fn asked(&self) -> Vec<Asked> {
        self.service.asked.lock().unwrap().clone()
    }

    /// How many requests of that method and path arrived.
    pub(crate) fn count(&self, method: &str, path: &str) -> usize {
        self.asked()
            .iter()
            .filter(|asked| asked.is(method, path))
            .count()
    }
}

impl Drop for StandIn {
    fn drop(&mut self) {
        self.shutdown.cancel();
    }
}

async fn answer(
    State(service): State<Arc<Service>>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let asked = Asked {
        method: method.to_string(),
        path: uri.path().to_string(),
        query: url::form_urlencoded::parse(uri.query().unwrap_or_default().as_bytes())
            .map(|(name, value)| format!("{name}={value}"))
            .collect::<Vec<_>>()
            .join("&"),
        key: headers
            .get("x-bb-api-key")
            .and_then(|value| value.to_str().ok())
            .map(str::to_string),
        body: serde_json::from_slice(&body).unwrap_or_default(),
    };
    let answer = {
        let mut all = service.asked.lock().unwrap();
        let before = all
            .iter()
            .filter(|earlier| earlier.is(&asked.method, &asked.path))
            .count();
        all.push(asked.clone());
        (service.script)(&asked, before)
    };
    let json = |status: u16, body: Value| {
        (
            StatusCode::from_u16(status).unwrap(),
            [(header::CONTENT_TYPE, "application/json")],
            body.to_string(),
        )
            .into_response()
    };
    match answer {
        Answer::Json(status, body) => json(status, body),
        Answer::Late(pause, status, body) => {
            tokio::time::sleep(pause).await;
            json(status, body)
        }
        Answer::Redirect(to) => (StatusCode::FOUND, [(header::LOCATION, to)]).into_response(),
    }
}
