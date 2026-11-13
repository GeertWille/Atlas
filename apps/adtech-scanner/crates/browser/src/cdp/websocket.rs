//! The DevTools protocol over a WebSocket, for a browser that runs
//! somewhere else. One text frame is one protocol message.
//!
//! The HTTP upgrade is made with the app's ordinary HTTP client, so the
//! connection is verified by the platform's certificate verifier and goes
//! through the system's proxy like every other request of the app. The
//! address is a credential: it is never part of an error or a log line.
//!
//! Two tasks own the socket. The reader hands every message on without
//! ever waiting for whoever consumes them, so pings are answered and pongs
//! are seen however busy the capture is. The writer sends commands and a
//! ping at a steady interval, and gives the connection up when nothing at
//! all was heard for too long: a remote browser that went away does not
//! always close its socket.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use reqwest::header::{
    CONNECTION, HeaderMap, SEC_WEBSOCKET_ACCEPT, SEC_WEBSOCKET_KEY, SEC_WEBSOCKET_VERSION, UPGRADE,
};
use reqwest::{StatusCode, Url};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::mpsc;
use tokio::time::Instant;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::client::generate_key;
use tokio_tungstenite::tungstenite::handshake::derive_accept_key;
use tokio_tungstenite::tungstenite::protocol::{Role, WebSocketConfig};
use tokio_util::sync::CancellationToken;

use super::client::{MessageSink, MessageSource};

/// The largest message, and frame, taken from the browser. Replies are kept
/// well below it by not asking for what could be larger (`capture`'s size
/// guard); a message above it ends the connection.
pub(crate) const MAX_MESSAGE_BYTES: usize = 64 * 1024 * 1024;

/// When to ping and when to give up.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Keepalive {
    pub ping_every: Duration,
    /// Nothing heard for this long, pong or otherwise: the connection is
    /// taken for closed.
    pub silence_limit: Duration,
}

impl Keepalive {
    /// A socket with no traffic was closed after 300 s when it was
    /// measured; pings were answered and kept it open.
    pub(crate) const REMOTE_BROWSER: Self = Self {
        ping_every: Duration::from_secs(30),
        silence_limit: Duration::from_secs(90),
    };
}

/// Why no connection was made. Never carries the address.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum ConnectError {
    #[error("the address of the browser is not a WebSocket address")]
    NotAWebSocketAddress,
    /// HTTP 410: the session the address belongs to has ended.
    #[error("the browser session has already ended")]
    Gone,
    #[error("the browser refused the connection (HTTP {0})")]
    Refused(u16),
    #[error("the browser did not complete the WebSocket handshake")]
    Handshake,
    #[error("the browser could not be reached: {0}")]
    Unreachable(String),
    #[error("the browser did not answer in time")]
    Timeout,
}

/// Opens a WebSocket to `address` (`ws://` or `wss://`) through `http`,
/// which must speak HTTP/1.1 only.
pub(crate) async fn connect(
    http: &reqwest::Client,
    address: &Url,
    keepalive: Keepalive,
    patience: Duration,
) -> Result<(WebSocketSource, WebSocketSink), ConnectError> {
    let upgraded = tokio::time::timeout(patience, upgrade(http, address))
        .await
        .map_err(|_| ConnectError::Timeout)??;
    Ok(start(upgraded, Role::Client, keepalive).await)
}

async fn upgrade(http: &reqwest::Client, address: &Url) -> Result<reqwest::Upgraded, ConnectError> {
    let mut as_http = address.clone();
    let scheme = match address.scheme() {
        "ws" => "http",
        "wss" => "https",
        _ => return Err(ConnectError::NotAWebSocketAddress),
    };
    as_http
        .set_scheme(scheme)
        .map_err(|()| ConnectError::NotAWebSocketAddress)?;
    let key = generate_key();
    let response = http
        .get(as_http)
        .header(CONNECTION, "Upgrade")
        .header(UPGRADE, "websocket")
        .header(SEC_WEBSOCKET_VERSION, "13")
        .header(SEC_WEBSOCKET_KEY, &key)
        .send()
        .await
        .map_err(unreachable)?;
    match response.status() {
        StatusCode::SWITCHING_PROTOCOLS => {}
        StatusCode::GONE => return Err(ConnectError::Gone),
        status => return Err(ConnectError::Refused(status.as_u16())),
    }
    if !is_accepted(response.headers(), &key) {
        return Err(ConnectError::Handshake);
    }
    response.upgrade().await.map_err(unreachable)
}

/// The server proved it understood the request as a WebSocket handshake.
fn is_accepted(headers: &HeaderMap, key: &str) -> bool {
    let upgrades_to_websocket = headers
        .get(UPGRADE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.eq_ignore_ascii_case("websocket"));
    let accepted_key = headers
        .get(SEC_WEBSOCKET_ACCEPT)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value == derive_accept_key(key.as_bytes()));
    upgrades_to_websocket && accepted_key
}

/// What went wrong, in the HTTP client's words without the address.
fn unreachable(error: reqwest::Error) -> ConnectError {
    let error = error.without_url();
    let mut reason = error.to_string();
    let mut source = std::error::Error::source(&error);
    while let Some(cause) = source {
        reason = cause.to_string();
        source = cause.source();
    }
    ConnectError::Unreachable(reason)
}

/// Messages from the browser, in the order they arrived.
pub(crate) struct WebSocketSource(mpsc::UnboundedReceiver<Vec<u8>>);

impl MessageSource for WebSocketSource {
    async fn next(&mut self) -> std::io::Result<Option<Vec<u8>>> {
        Ok(self.0.recv().await)
    }
}

/// Messages to the browser. Dropping it closes the socket.
pub(crate) struct WebSocketSink(mpsc::UnboundedSender<String>);

impl MessageSink for WebSocketSink {
    async fn send(&mut self, message: &str) -> std::io::Result<()> {
        self.0
            .send(message.to_string())
            .map_err(|_| std::io::ErrorKind::BrokenPipe.into())
    }
}

/// Speaks WebSocket over a stream whose handshake is done.
async fn start<S>(stream: S, role: Role, keepalive: Keepalive) -> (WebSocketSource, WebSocketSink)
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let limits = WebSocketConfig::default()
        .max_message_size(Some(MAX_MESSAGE_BYTES))
        .max_frame_size(Some(MAX_MESSAGE_BYTES));
    let socket = WebSocketStream::from_raw_socket(stream, role, Some(limits)).await;
    let (to_browser, from_browser) = socket.split();
    let (incoming, source) = mpsc::unbounded_channel();
    let (sink, outgoing) = mpsc::unbounded_channel();
    let closed = CancellationToken::new();
    let heard = Arc::new(LastHeard::now());
    tokio::spawn(read(from_browser, incoming, heard.clone(), closed.clone()));
    tokio::spawn(write(to_browser, outgoing, keepalive, heard, closed));
    (WebSocketSource(source), WebSocketSink(sink))
}

/// When the browser was last heard from, as time since the socket opened.
struct LastHeard {
    opened: Instant,
    after_ms: AtomicU64,
}

impl LastHeard {
    fn now() -> Self {
        Self {
            opened: Instant::now(),
            after_ms: AtomicU64::new(0),
        }
    }

    fn note(&self) {
        let after = self.opened.elapsed().as_millis() as u64;
        self.after_ms.store(after, Ordering::Relaxed);
    }

    fn silent_for(&self) -> Duration {
        let heard = Duration::from_millis(self.after_ms.load(Ordering::Relaxed));
        self.opened.elapsed().saturating_sub(heard)
    }
}

async fn read<S>(
    mut from_browser: SplitStream<WebSocketStream<S>>,
    incoming: mpsc::UnboundedSender<Vec<u8>>,
    heard: Arc<LastHeard>,
    closed: CancellationToken,
) where
    S: AsyncRead + AsyncWrite + Unpin,
{
    // Dropping `incoming` at the end is what tells the client the browser
    // is gone, whichever task noticed.
    let _close_writer = closed.clone().drop_guard();
    loop {
        let frame = tokio::select! {
            biased;
            _ = closed.cancelled() => return,
            frame = from_browser.next() => frame,
        };
        heard.note();
        let delivered = match frame {
            Some(Ok(Message::Text(text))) => incoming.send(text.as_bytes().to_vec()),
            Some(Ok(Message::Binary(bytes))) => incoming.send(bytes.to_vec()),
            // Answered by the socket itself; a pong only proves life.
            Some(Ok(Message::Ping(_) | Message::Pong(_) | Message::Frame(_))) => Ok(()),
            // Closed, cut off, or a message over the limit.
            Some(Ok(Message::Close(_)) | Err(_)) | None => return,
        };
        if delivered.is_err() {
            return;
        }
    }
}

async fn write<S>(
    mut to_browser: SplitSink<WebSocketStream<S>, Message>,
    mut outgoing: mpsc::UnboundedReceiver<String>,
    keepalive: Keepalive,
    heard: Arc<LastHeard>,
    closed: CancellationToken,
) where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let _close_reader = closed.clone().drop_guard();
    let mut ping =
        tokio::time::interval_at(Instant::now() + keepalive.ping_every, keepalive.ping_every);
    ping.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        let frame = tokio::select! {
            biased;
            _ = closed.cancelled() => return,
            message = outgoing.recv() => match message {
                Some(message) => Message::text(message),
                None => break,
            },
            _ = ping.tick() => {
                if heard.silent_for() >= keepalive.silence_limit {
                    return;
                }
                Message::Ping(Default::default())
            }
        };
        let sent = tokio::select! {
            biased;
            _ = closed.cancelled() => return,
            sent = to_browser.send(frame) => sent,
        };
        if sent.is_err() {
            return;
        }
    }
    // The client is done: say goodbye, without waiting on a peer that is
    // not listening.
    const GOODBYE_PATIENCE: Duration = Duration::from_secs(2);
    let _ = tokio::time::timeout(GOODBYE_PATIENCE, to_browser.close()).await;
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};
    use tokio::net::{TcpListener, TcpStream};
    use tokio_tungstenite::tungstenite::protocol::CloseFrame;

    use super::*;

    const PATIENT: Keepalive = Keepalive {
        ping_every: Duration::from_secs(3_600),
        silence_limit: Duration::from_secs(3_600),
    };

    /// The browser's end of a socket whose handshake is done, as a plain
    /// tungstenite server.
    async fn paired(
        keepalive: Keepalive,
    ) -> (
        WebSocketSource,
        WebSocketSink,
        WebSocketStream<DuplexStream>,
    ) {
        let (near, far) = tokio::io::duplex(64 * 1024);
        let (source, sink) = start(near, Role::Client, keepalive).await;
        let limits = WebSocketConfig::default()
            .max_message_size(None)
            .max_frame_size(None);
        let browser = WebSocketStream::from_raw_socket(far, Role::Server, Some(limits)).await;
        (source, sink, browser)
    }

    #[tokio::test]
    async fn one_text_frame_is_one_message_in_each_direction() {
        let (mut source, mut sink, mut browser) = paired(PATIENT).await;

        sink.send(r#"{"id":1,"method":"Browser.getVersion"}"#)
            .await
            .unwrap();
        sink.send("").await.unwrap();
        assert_eq!(
            browser.next().await.unwrap().unwrap(),
            Message::text(r#"{"id":1,"method":"Browser.getVersion"}"#)
        );
        assert_eq!(browser.next().await.unwrap().unwrap(), Message::text(""));

        browser
            .send(Message::text(r#"{"id":1,"result":{}}"#))
            .await
            .unwrap();
        browser
            .send(Message::text("NUL is just a byte here: \u{0}"))
            .await
            .unwrap();
        assert_eq!(
            source.next().await.unwrap().unwrap(),
            br#"{"id":1,"result":{}}"#
        );
        assert_eq!(
            source.next().await.unwrap().unwrap(),
            "NUL is just a byte here: \u{0}".as_bytes()
        );
    }

    #[tokio::test]
    async fn a_message_of_many_megabytes_arrives_whole_and_one_over_the_limit_ends_the_connection()
    {
        let (mut source, _sink, mut browser) = paired(PATIENT).await;
        let large = "x".repeat(40 * 1024 * 1024);
        let too_large = "y".repeat(MAX_MESSAGE_BYTES + 1);
        let sent = large.clone();
        let the_browser = tokio::spawn(async move {
            browser.send(Message::text(sent)).await.unwrap();
            browser.send(Message::text("next")).await.unwrap();
            // The client hangs up while this is on its way.
            let _ = browser.send(Message::text(too_large)).await;
        });

        assert!(source.next().await.unwrap().unwrap() == large.as_bytes());
        assert_eq!(source.next().await.unwrap().unwrap(), b"next");
        assert_eq!(source.next().await.unwrap(), None);
        the_browser.await.unwrap();
    }

    #[tokio::test]
    async fn the_browser_is_pinged_at_the_interval_and_its_pings_are_answered() {
        let keepalive = Keepalive {
            ping_every: Duration::from_millis(40),
            silence_limit: Duration::from_secs(60),
        };
        let (_source, _sink, mut browser) = paired(keepalive).await;
        let started = Instant::now();

        browser
            .send(Message::Ping("are you there".into()))
            .await
            .unwrap();
        let mut pings = 0;
        let mut answered = false;
        while pings < 3 {
            match browser.next().await.unwrap().unwrap() {
                Message::Ping(_) => pings += 1,
                Message::Pong(payload) => answered = payload.as_ref() == b"are you there",
                other => panic!("unexpected {other:?}"),
            }
        }

        assert!(answered, "the browser's own ping was answered");
        assert!(started.elapsed() >= Duration::from_millis(120));
    }

    #[tokio::test]
    async fn a_browser_that_answers_pings_is_kept_however_long_nothing_else_is_said() {
        let keepalive = Keepalive {
            ping_every: Duration::from_millis(20),
            silence_limit: Duration::from_millis(100),
        };
        let (mut source, _sink, mut browser) = paired(keepalive).await;
        // Reading is what makes the stand-in answer pings.
        let the_browser = tokio::spawn(async move {
            let quiet_until = Instant::now() + Duration::from_millis(400);
            while let Ok(frame) = tokio::time::timeout_at(quiet_until, browser.next()).await {
                frame.unwrap().unwrap();
            }
            browser.send(Message::text("still here")).await.unwrap();
            browser
        });

        assert_eq!(source.next().await.unwrap().unwrap(), b"still here");
        drop(the_browser.await.unwrap());
    }

    #[tokio::test]
    async fn a_browser_that_falls_silent_is_taken_for_closed() {
        let keepalive = Keepalive {
            ping_every: Duration::from_millis(20),
            silence_limit: Duration::from_millis(100),
        };
        // Nobody reads the far end: pings go unanswered.
        let (mut source, mut sink, _browser_that_never_reads) = paired(keepalive).await;
        let started = Instant::now();

        assert_eq!(source.next().await.unwrap(), None);

        let silent_for = started.elapsed();
        assert!(
            silent_for >= Duration::from_millis(100) && silent_for < Duration::from_secs(5),
            "{silent_for:?}"
        );
        // The writer notices on its own time; then nothing can be sent.
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(sink.send("{}").await.is_err());
    }

    #[tokio::test]
    async fn messages_are_read_while_nobody_takes_them_so_a_ping_behind_them_is_answered() {
        let (mut source, _sink, mut browser) = paired(PATIENT).await;
        for number in 0..2_000 {
            browser
                .send(Message::text(format!("{number}:{}", "e".repeat(1_024))))
                .await
                .unwrap();
        }
        browser.send(Message::Ping("behind".into())).await.unwrap();

        // Nothing was taken from `source` yet, and the pong is there.
        let pong = tokio::time::timeout(Duration::from_secs(5), browser.next())
            .await
            .expect("the ping was answered while 2 MB waited to be consumed")
            .unwrap()
            .unwrap();
        assert_eq!(pong, Message::Pong("behind".into()));
        assert!(source.next().await.unwrap().unwrap().starts_with(b"0:"));
    }

    #[tokio::test]
    async fn dropping_the_sink_closes_the_socket_and_a_close_from_the_browser_ends_the_source() {
        let (_source, sink, mut browser) = paired(PATIENT).await;
        drop(sink);
        assert!(matches!(
            browser.next().await.unwrap().unwrap(),
            Message::Close(_)
        ));

        let (mut source, _sink, mut browser) = paired(PATIENT).await;
        browser.close(None::<CloseFrame>).await.unwrap();
        assert_eq!(source.next().await.unwrap(), None);

        let (mut source, _sink, browser) = paired(PATIENT).await;
        drop(browser);
        assert_eq!(source.next().await.unwrap(), None);
    }

    fn http_client() -> reqwest::ClientBuilder {
        reqwest::Client::builder().http1_only()
    }

    /// A WebSocket server that answers every text message with its upper
    /// case. Returns its port.
    async fn shouting_server<S, F>(wrap: impl Fn(TcpStream) -> F + Send + 'static) -> u16
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
        F: Future<Output = std::io::Result<S>> + Send,
    {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((connection, _)) = listener.accept().await {
                let Ok(stream) = wrap(connection).await else {
                    continue;
                };
                tokio::spawn(async move {
                    let Ok(mut socket) = tokio_tungstenite::accept_async(stream).await else {
                        return;
                    };
                    while let Some(Ok(Message::Text(text))) = socket.next().await {
                        let shouted = text.as_str().to_uppercase();
                        if socket.send(Message::text(shouted)).await.is_err() {
                            return;
                        }
                    }
                });
            }
        });
        port
    }

    #[tokio::test]
    async fn the_upgrade_is_made_by_the_http_client_and_the_socket_carries_messages() {
        let port = shouting_server(|connection| async move { Ok(connection) }).await;
        let address = Url::parse(&format!("ws://127.0.0.1:{port}/devtools?token=s3cret")).unwrap();
        let http = http_client().no_proxy().build().unwrap();

        let (mut source, mut sink) = connect(&http, &address, PATIENT, Duration::from_secs(10))
            .await
            .unwrap();
        sink.send("over the wire").await.unwrap();

        assert_eq!(source.next().await.unwrap().unwrap(), b"OVER THE WIRE");
    }

    /// A proxy that tunnels `CONNECT` and counts the tunnels it opened.
    async fn connect_proxy() -> (u16, Arc<AtomicU64>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let tunnels = Arc::new(AtomicU64::new(0));
        let opened = tunnels.clone();
        tokio::spawn(async move {
            while let Ok((mut client, _)) = listener.accept().await {
                let opened = opened.clone();
                tokio::spawn(async move {
                    let mut head = Vec::new();
                    while !head.ends_with(b"\r\n\r\n") {
                        head.push(client.read_u8().await.unwrap());
                    }
                    let head = String::from_utf8(head).unwrap();
                    let target = head
                        .strip_prefix("CONNECT ")
                        .and_then(|rest| rest.split(' ').next())
                        .expect("a CONNECT request");
                    let mut server = TcpStream::connect(target).await.unwrap();
                    opened.fetch_add(1, Ordering::Relaxed);
                    client
                        .write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
                        .await
                        .unwrap();
                    let _ = tokio::io::copy_bidirectional(&mut client, &mut server).await;
                });
            }
        });
        (port, tunnels)
    }

    /// A certificate for `localhost` and its key, made for these tests only.
    const TEST_CERTIFICATE: &str = include_str!("../../tests/fixtures/tls/localhost.crt");
    const TEST_KEY: &str = include_str!("../../tests/fixtures/tls/localhost.key");

    fn tls_acceptor() -> tokio_rustls::TlsAcceptor {
        use rustls_pki_types::pem::PemObject;
        use rustls_pki_types::{CertificateDer, PrivateKeyDer};

        let certificate = CertificateDer::from_pem_slice(TEST_CERTIFICATE.as_bytes()).unwrap();
        let key = PrivateKeyDer::from_pem_slice(TEST_KEY.as_bytes()).unwrap();
        let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
        let config = rustls::ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(vec![certificate], key)
            .unwrap();
        tokio_rustls::TlsAcceptor::from(Arc::new(config))
    }

    #[tokio::test]
    async fn a_secure_socket_is_verified_and_goes_through_the_http_proxy() {
        let acceptor = tls_acceptor();
        let port = shouting_server(move |connection| {
            let acceptor = acceptor.clone();
            async move { acceptor.accept(connection).await }
        })
        .await;
        let (proxy_port, tunnels) = connect_proxy().await;
        let address = Url::parse(&format!("wss://localhost:{port}/devtools")).unwrap();
        let through_proxy =
            reqwest::Proxy::https(format!("http://127.0.0.1:{proxy_port}")).unwrap();

        // Without the test certificate among the trusted ones, the
        // connection is refused: verification is on.
        let distrusting = http_client().proxy(through_proxy.clone()).build().unwrap();
        let refused = connect(&distrusting, &address, PATIENT, Duration::from_secs(10)).await;
        assert!(
            matches!(refused, Err(ConnectError::Unreachable(_))),
            "an unknown certificate was accepted"
        );

        let trusted = reqwest::Certificate::from_pem(TEST_CERTIFICATE.as_bytes()).unwrap();
        let http = http_client()
            .proxy(through_proxy)
            .tls_certs_merge([trusted])
            .build()
            .unwrap();
        let (mut source, mut sink) = connect(&http, &address, PATIENT, Duration::from_secs(10))
            .await
            .unwrap();
        sink.send("through the tunnel").await.unwrap();

        assert_eq!(source.next().await.unwrap().unwrap(), b"THROUGH THE TUNNEL");
        assert_eq!(tunnels.load(Ordering::Relaxed), 2);
    }

    /// An HTTP server that answers every request with `status` and no body.
    async fn refusing_server(status: &'static str) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((mut client, _)) = listener.accept().await {
                let mut head = Vec::new();
                while !head.ends_with(b"\r\n\r\n") {
                    head.push(client.read_u8().await.unwrap());
                }
                let answer = format!("HTTP/1.1 {status}\r\ncontent-length: 0\r\n\r\n");
                client.write_all(answer.as_bytes()).await.unwrap();
            }
        });
        port
    }

    #[tokio::test]
    async fn a_refused_upgrade_says_why_without_the_address() {
        let http = http_client().no_proxy().build().unwrap();
        let patience = Duration::from_secs(10);
        let secret = "signingKey=s3cret-signing-key";
        let at = |port: u16| Url::parse(&format!("ws://127.0.0.1:{port}/?{secret}")).unwrap();

        let ended = refusing_server("410 Gone").await;
        let forbidden = refusing_server("403 Forbidden").await;
        let not_a_socket = refusing_server("200 OK").await;
        let nobody = {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            listener.local_addr().unwrap().port()
        };
        let failures = [
            connect(&http, &at(ended), PATIENT, patience).await.err(),
            connect(&http, &at(forbidden), PATIENT, patience)
                .await
                .err(),
            connect(&http, &at(not_a_socket), PATIENT, patience)
                .await
                .err(),
            connect(&http, &at(nobody), PATIENT, patience).await.err(),
            connect(
                &http,
                &Url::parse(&format!("https://127.0.0.1/?{secret}")).unwrap(),
                PATIENT,
                patience,
            )
            .await
            .err(),
        ];

        assert_eq!(failures[0], Some(ConnectError::Gone));
        assert_eq!(failures[1], Some(ConnectError::Refused(403)));
        assert_eq!(failures[2], Some(ConnectError::Refused(200)));
        assert!(matches!(failures[3], Some(ConnectError::Unreachable(_))));
        assert_eq!(failures[4], Some(ConnectError::NotAWebSocketAddress));
        for failure in failures {
            let said = format!("{0} {0:?}", failure.unwrap());
            assert!(
                !said.contains("s3cret") && !said.contains("127.0.0.1"),
                "{said}"
            );
        }
    }

    #[tokio::test]
    async fn a_server_that_never_answers_the_upgrade_is_given_up_on() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let http = http_client().no_proxy().build().unwrap();
        let address = Url::parse(&format!("ws://127.0.0.1:{port}/")).unwrap();

        let outcome = connect(&http, &address, PATIENT, Duration::from_millis(200)).await;

        assert_eq!(outcome.err(), Some(ConnectError::Timeout));
        drop(listener);
    }
}
