//! A scan through a proxy the user configured: where the traffic goes, how a
//! proxy that asks for a login gets it, and that the password is seen by
//! nobody else.
//!
//! The proxy is a small forward proxy in this file. Every host name it is
//! asked for is a fixture server on this machine, so nothing here needs a
//! network. The tests find "their" browser as the only one this process runs,
//! so they take turns.
#![cfg(target_os = "linux")]

mod support;

use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use adscan_browser::BrowserError;
use adscan_model::{CaptureProxy, CaptureRequest, DeviceProfile, ProxyScheme};
use base64::Engine;
use support::chrome_processes::Browser;
use support::{Fixtures, Outcome, Reported};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_util::sync::CancellationToken;

static ONE_BROWSER_AT_A_TIME: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

const USER: &str = "scanner";
const PASSWORD: &str = "c0rrect-horse-battery";

/// One request the proxy received.
#[derive(Debug, Clone, PartialEq)]
struct Seen {
    method: String,
    /// The absolute address, or `host:port` of a tunnel.
    target: String,
    /// Whether it carried the login the proxy asks for.
    logged_in: bool,
}

/// A forward proxy that sends every host name to one local server.
struct Proxy {
    port: u16,
    seen: Arc<Mutex<Vec<Seen>>>,
    shutdown: CancellationToken,
}

impl Proxy {
    /// `login` is the user name and password it insists on, if any.
    async fn start(upstream_port: u16, login: Option<(&str, &str)>) -> Self {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let shutdown = CancellationToken::new();
        let expected = login.map(|(user, password)| {
            let encoded =
                base64::engine::general_purpose::STANDARD.encode(format!("{user}:{password}"));
            format!("Basic {encoded}")
        });
        let (log, stopped) = (seen.clone(), shutdown.clone());
        tokio::spawn(async move {
            loop {
                let accepted = tokio::select! {
                    _ = stopped.cancelled() => return,
                    accepted = listener.accept() => accepted,
                };
                let Ok((client, _)) = accepted else { return };
                tokio::spawn(serve(client, upstream_port, expected.clone(), log.clone()));
            }
        });
        Self {
            port,
            seen,
            shutdown,
        }
    }

    fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }

    fn as_custom(&self, username: Option<&str>) -> CaptureProxy {
        CaptureProxy::Custom {
            scheme: ProxyScheme::Http,
            host: "127.0.0.1".into(),
            port: self.port,
            username: username.map(str::to_string),
        }
    }
}

impl Drop for Proxy {
    fn drop(&mut self) {
        self.shutdown.cancel();
    }
}

/// Serves one request of one connection: refuses it without the login, opens
/// a tunnel for CONNECT, and passes anything else on to the local server.
async fn serve(
    mut client: TcpStream,
    upstream_port: u16,
    expected_login: Option<String>,
    log: Arc<Mutex<Vec<Seen>>>,
) {
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        let Ok(byte) = client.read_u8().await else {
            return;
        };
        head.push(byte);
    }
    let head = String::from_utf8_lossy(&head).into_owned();
    let mut lines = head.split("\r\n");
    let mut request_line = lines.next().unwrap_or_default().split(' ');
    let (method, target) = (
        request_line.next().unwrap_or_default().to_string(),
        request_line.next().unwrap_or_default().to_string(),
    );
    let headers: Vec<&str> = lines.filter(|line| !line.is_empty()).collect();
    let login = headers.iter().find_map(|header| {
        let (name, value) = header.split_once(':')?;
        name.eq_ignore_ascii_case("proxy-authorization")
            .then(|| value.trim().to_string())
    });
    let logged_in = expected_login.is_none() || login == expected_login;
    log.lock().unwrap().push(Seen {
        method: method.clone(),
        target: target.clone(),
        logged_in,
    });
    if !logged_in {
        let refusal = "HTTP/1.1 407 Proxy Authentication Required\r\n\
            Proxy-Authenticate: Basic realm=\"adscan test proxy\"\r\n\
            Content-Length: 0\r\nConnection: close\r\n\r\n";
        let _ = client.write_all(refusal.as_bytes()).await;
        return;
    }
    let Ok(mut upstream) = TcpStream::connect((Ipv4Addr::LOCALHOST, upstream_port)).await else {
        return;
    };
    if method == "CONNECT" {
        let _ = client
            .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
            .await;
    } else {
        // Origin form, and one request per connection, so that the next
        // request is seen here too.
        let path = url::Url::parse(&target).map_or("/".to_string(), |url| match url.query() {
            Some(query) => format!("{}?{query}", url.path()),
            None => url.path().to_string(),
        });
        let passed_on: Vec<&str> = headers
            .iter()
            .copied()
            .filter(|header| {
                let name = header.split(':').next().unwrap_or_default();
                !name.to_ascii_lowercase().starts_with("proxy-")
                    && !name.eq_ignore_ascii_case("connection")
            })
            .collect();
        let request = format!(
            "{method} {path} HTTP/1.1\r\n{}\r\nConnection: close\r\n\r\n",
            passed_on.join("\r\n")
        );
        if upstream.write_all(request.as_bytes()).await.is_err() {
            return;
        }
    }
    let _ = tokio::io::copy_bidirectional(&mut client, &mut upstream).await;
}

fn through(proxy: CaptureProxy, url: &str) -> CaptureRequest {
    let mut request = support::request(url, DeviceProfile::Desktop);
    request.proxy = proxy;
    request.observe_seconds = 2;
    request
}

fn requests_for<'a>(seen: &'a [Seen], host: &str) -> Vec<&'a Seen> {
    seen.iter()
        .filter(|request| request.target.contains(host))
        .collect()
}

/// Everything a capture told the app, as one text to search.
fn everything_reported(outcome: &Outcome) -> String {
    let activities = serde_json::to_string(&outcome.activities).unwrap();
    let result = match &outcome.result {
        Ok(capture) => serde_json::to_string(capture).unwrap(),
        Err(error) => format!("{error} {error:?}"),
    };
    format!("{activities}\n{result}")
}

#[tokio::test]
async fn a_scan_goes_through_the_proxy_it_was_given() {
    let _turn = ONE_BROWSER_AT_A_TIME.lock().await;
    let fixtures = Fixtures::start().await;
    let proxy = Proxy::start(fixtures.port, None).await;
    let request = through(
        proxy.as_custom(None),
        "http://site.adscan.test/proxied.html",
    );
    let outcome = support::run(request).await;
    let capture = outcome.result.as_ref().unwrap_or_else(|error| {
        panic!("capture failed: {error}\nprogress: {:#?}", outcome.progress)
    });

    assert_eq!(capture.final_url, "http://site.adscan.test/proxied.html");
    assert_eq!(capture.page_title.as_deref(), Some("A page behind a proxy"));
    let said = |text: &str| capture.console.iter().any(|line| line.text == text);
    assert!(said("PROXIED page at site.adscan.test"));
    assert!(
        said("BINDING visible=false"),
        "the script came through it too"
    );

    let seen = proxy.seen();
    let page = requests_for(&seen, "site.adscan.test/proxied.html");
    assert_eq!(page.len(), 1, "{seen:#?}");
    assert_eq!(page[0].method, "GET");
    assert_eq!(requests_for(&seen, "cdn.adscan-cdn.test").len(), 1);
    let tunnel = requests_for(&seen, "secure.adscan-tls.test:443");
    assert!(
        tunnel.iter().all(|request| request.method == "CONNECT") && !tunnel.is_empty(),
        "{seen:#?}"
    );
    assert_eq!(
        fixtures.requested(),
        ["/proxied.html", "/browse-shared.js"],
        "the site was reached through the proxy and no other way"
    );
    assert_eq!(
        capture.environment.request_hosts,
        ["cdn.adscan-cdn.test", "secure.adscan-tls.test"]
    );
    assert!(
        outcome.progress[0].ends_with("through the proxy 127.0.0.1)"),
        "{}",
        outcome.progress[0]
    );
}

#[tokio::test]
async fn a_proxy_that_asks_for_a_login_gets_it_and_nobody_else_sees_the_password() {
    let _turn = ONE_BROWSER_AT_A_TIME.lock().await;
    let fixtures = Fixtures::start().await;
    let proxy = Proxy::start(fixtures.port, Some((USER, PASSWORD))).await;
    let mut request = through(
        proxy.as_custom(Some(USER)),
        "http://site.adscan.test/proxied.html",
    );
    request.observe_seconds = 4;
    let mut reported = Reported::new();
    reported.hooks.proxy_password = Some(PASSWORD.to_string());
    let running = tokio::spawn(reported.run(request));

    // While the scan runs: no process of the browser was started with it.
    let browser = Browser::launched_by(std::process::id()).await;
    tokio::time::sleep(Duration::from_millis(2_500)).await;
    let command_lines = browser.command_lines();
    assert!(command_lines.len() >= 4, "{command_lines:#?}");
    for command_line in &command_lines {
        assert!(!command_line.contains(PASSWORD), "{command_line}");
        assert!(!command_line.contains(USER), "{command_line}");
    }
    assert!(
        command_lines
            .iter()
            .any(|line| line.contains(&format!("--proxy-server=http://127.0.0.1:{}", proxy.port)))
    );

    let outcome = running.await.unwrap();
    let capture = outcome.result.as_ref().unwrap_or_else(|error| {
        panic!("capture failed: {error}\nprogress: {:#?}", outcome.progress)
    });
    assert!(
        capture
            .console
            .iter()
            .any(|line| line.text == "PROXIED page at site.adscan.test")
    );
    assert!(!everything_reported(&outcome).contains(PASSWORD));

    // Challenged once, on the page itself; everything after carries the login
    // without being asked, tunnels included.
    let seen = proxy.seen();
    let page = requests_for(&seen, "site.adscan.test/proxied.html");
    let logins: Vec<bool> = page.iter().map(|request| request.logged_in).collect();
    assert_eq!(logins, [false, true], "{seen:#?}");
    let afterwards: Vec<&Seen> = seen
        .iter()
        .filter(|request| {
            request.target.contains("adscan-cdn.test") || request.target.contains("adscan-tls.test")
        })
        .collect();
    assert!(afterwards.len() >= 2, "{seen:#?}");
    assert!(
        afterwards.iter().all(|request| request.logged_in),
        "{seen:#?}"
    );
    assert_eq!(fixtures.requested(), ["/proxied.html", "/browse-shared.js"]);
}

/// The proxy's login is for the proxy. A site reached through it that asks
/// for a login of its own gets no answer from the scan.
#[tokio::test]
async fn a_site_that_asks_for_a_login_is_not_given_the_proxys() {
    let _turn = ONE_BROWSER_AT_A_TIME.lock().await;
    let fixtures = Fixtures::start().await;
    let proxy = Proxy::start(fixtures.port, Some((USER, PASSWORD))).await;
    let request = through(
        proxy.as_custom(Some(USER)),
        "http://site.adscan.test/members-only",
    );
    let mut reported = Reported::new();
    reported.hooks.proxy_password = Some(PASSWORD.to_string());
    let outcome = reported.run(request).await;

    let Err(BrowserError::Navigation(message)) = &outcome.result else {
        panic!("{:?}", outcome.result);
    };
    assert!(
        message.contains("401") && message.contains("needs a login the scanner does not have"),
        "the site, not the proxy, is named as the one that wants a login: {message}"
    );
    assert!(!everything_reported(&outcome).contains(PASSWORD));
    // The proxy let the scan in, and the site was asked without any login.
    let seen = proxy.seen();
    let page = requests_for(&seen, "site.adscan.test/members-only");
    assert!(page.iter().any(|request| request.logged_in), "{seen:#?}");
    let asked = fixtures.requested();
    assert!(!asked.is_empty(), "the site was reached");
    assert!(
        asked.iter().all(|asked| asked == "/members-only"),
        "the site was sent a login: {asked:#?}"
    );
}

#[tokio::test]
async fn a_login_the_proxy_does_not_accept_fails_the_scan_with_a_plain_message() {
    let _turn = ONE_BROWSER_AT_A_TIME.lock().await;
    let fixtures = Fixtures::start().await;
    let proxy = Proxy::start(fixtures.port, Some((USER, PASSWORD))).await;
    let wrong = "not-the-password";
    let cases: [(Option<&str>, Option<&str>); 3] =
        [(Some(USER), Some(wrong)), (Some(USER), None), (None, None)];
    for (username, password) in cases {
        let request = through(
            proxy.as_custom(username),
            "http://site.adscan.test/proxied.html",
        );
        let mut reported = Reported::new();
        reported.hooks.proxy_password = password.map(str::to_string);
        let outcome = reported.run(request).await;

        let Err(BrowserError::Navigation(message)) = &outcome.result else {
            panic!("{username:?}/{password:?}: {:?}", outcome.result);
        };
        assert!(
            message.starts_with("The proxy 127.0.0.1 asked for a user name and password"),
            "{message}"
        );
        let reported = everything_reported(&outcome);
        assert!(!reported.contains(wrong) && !reported.contains(PASSWORD));
    }
    assert!(
        proxy.seen().iter().all(|request| !request.logged_in),
        "{:#?}",
        proxy.seen()
    );
    assert_eq!(
        fixtures.requested(),
        Vec::<String>::new(),
        "nothing got past the proxy"
    );
}

#[tokio::test]
async fn a_site_on_this_machine_is_reached_directly_whatever_the_proxy() {
    let _turn = ONE_BROWSER_AT_A_TIME.lock().await;
    let site = adscan_browser::start_demo_site().await.unwrap();
    let fixtures = Fixtures::start().await;
    let proxy = Proxy::start(fixtures.port, Some((USER, PASSWORD))).await;
    let outcome = support::run(through(proxy.as_custom(None), &site.url)).await;
    let capture = outcome.result.unwrap_or_else(|error| {
        panic!("capture failed: {error}\nprogress: {:#?}", outcome.progress)
    });

    assert_eq!(capture.final_url, site.url);
    assert!(capture.scripts.len() >= 5, "the demo page loaded in full");
    let demo_host = site.url.trim_start_matches("http://").trim_end_matches('/');
    let seen = proxy.seen();
    assert!(
        requests_for(&seen, demo_host).is_empty() && requests_for(&seen, "127.0.0.1").is_empty(),
        "{seen:#?}"
    );
    assert!(fixtures.requested().is_empty());

    // The same without any proxy at all, the system's included.
    let direct = support::run(through(CaptureProxy::None, &site.url)).await;
    assert!(direct.result.is_ok(), "{:?}", direct.result.err());
    assert!(
        direct.progress[0].ends_with("(headless, no proxy)"),
        "{}",
        direct.progress[0]
    );
}

#[tokio::test]
async fn a_proxy_that_is_not_there_is_named_as_the_problem() {
    let _turn = ONE_BROWSER_AT_A_TIME.lock().await;
    // A port that was free a moment ago and has nobody listening.
    let closed = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let port = closed.local_addr().unwrap().port();
    drop(closed);
    let proxy = CaptureProxy::Custom {
        scheme: ProxyScheme::Http,
        host: "127.0.0.1".into(),
        port,
        username: None,
    };
    let outcome = support::run(through(proxy, "http://site.adscan.test/proxied.html")).await;

    let Err(BrowserError::Navigation(message)) = &outcome.result else {
        panic!("{:?}", outcome.result);
    };
    assert_eq!(
        message,
        "The proxy 127.0.0.1 could not be reached. Check its address and port, and that it is running"
    );
}

#[tokio::test]
async fn a_socks_proxy_with_a_login_is_refused_before_a_browser_starts() {
    let proxy = CaptureProxy::Custom {
        scheme: ProxyScheme::Socks5,
        host: "127.0.0.1".into(),
        port: 1080,
        username: Some(USER.into()),
    };
    let mut reported = Reported::new();
    reported.hooks.proxy_password = Some(PASSWORD.to_string());
    let started = std::time::Instant::now();
    let outcome = reported
        .run(through(proxy, "http://site.adscan.test/proxied.html"))
        .await;

    let Err(BrowserError::Other(message)) = &outcome.result else {
        panic!("{:?}", outcome.result);
    };
    assert!(
        message.contains("SOCKS5") && message.contains("user name"),
        "{message}"
    );
    assert!(!everything_reported(&outcome).contains(PASSWORD));
    assert!(outcome.activities.is_empty(), "nothing was launched");
    assert!(started.elapsed() < Duration::from_secs(2));
}
