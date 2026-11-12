//! The public-file fetcher against a mock web server: what it reads, what it
//! refuses, and the limits it keeps. The server is on loopback, which the
//! fetcher only contacts as the scanned page's own origin.

use std::time::{Duration, Instant};

use adscan_engine::application::ports::{
    FetchError, FetchProxy, FileVersion, PublicFile, PublicFileFetcher, PublicFileRequest,
};
use adscan_engine::domain::diagnostics::ads_txt::{self, AdsTxtOutcome};
use adscan_engine::infrastructure::public_files::HttpPublicFileFetcher;
use adscan_model::{CaptureProxy, ProxyScheme};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_util::sync::CancellationToken;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const ADS_TXT: &str = "appnexus.com, 958, DIRECT\nrubiconproject.com, 1001, RESELLER\n";

fn system_proxy() -> FetchProxy {
    FetchProxy {
        route: CaptureProxy::System,
        password: None,
    }
}

fn request(server: &MockServer, file: &str) -> PublicFileRequest {
    PublicFileRequest {
        url: format!("{}{file}", server.uri()),
        max_bytes: 64 * 1024,
        timeout: Duration::from_secs(5),
        loopback_origin: Some(server.uri()),
        proxy: system_proxy(),
        known_version: None,
    }
}

async fn fetch(request: &PublicFileRequest) -> Result<PublicFile, FetchError> {
    HttpPublicFileFetcher
        .fetch(request, &CancellationToken::new())
        .await
}

async fn serve(server: &MockServer, file: &str, response: ResponseTemplate) {
    Mock::given(method("GET"))
        .and(path(file))
        .respond_with(response)
        .mount(server)
        .await;
}

fn text(body: &str) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_raw(body.as_bytes().to_vec(), "text/plain; charset=utf-8")
}

fn redirect(status: u16, location: &str) -> ResponseTemplate {
    ResponseTemplate::new(status).insert_header("location", location)
}

fn refusal(result: Result<PublicFile, FetchError>) -> String {
    match result {
        Err(FetchError::Refused(reason)) => reason,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[tokio::test]
async fn a_file_is_read_with_its_status_and_content_type() {
    let server = MockServer::start().await;
    serve(&server, "/ads.txt", text(ADS_TXT)).await;

    let file = fetch(&request(&server, "/ads.txt")).await.unwrap();
    assert_eq!(
        file,
        PublicFile {
            status: 200,
            content_type: Some("text/plain; charset=utf-8".into()),
            body: ADS_TXT.as_bytes().to_vec(),
            final_url: format!("{}/ads.txt", server.uri()),
            version: FileVersion::default(),
        }
    );

    // The request is anonymous and says what it accepts.
    let received = &server.received_requests().await.unwrap()[0];
    let header = |name: &str| {
        received
            .headers
            .get(name)
            .map(|value| value.to_str().unwrap().to_string())
    };
    assert!(header("user-agent").unwrap().starts_with("adtech-scanner/"));
    assert!(header("accept").unwrap().starts_with("text/plain"));
    assert_eq!(header("cookie"), None);
    assert_eq!(header("authorization"), None);
    assert_eq!(header("referer"), None);
}

#[tokio::test]
async fn a_known_version_is_asked_about_and_an_unchanged_file_is_not_sent_again() {
    use wiremock::matchers::header;
    let server = MockServer::start().await;
    // The file as it is now, to whoever does not have it yet.
    Mock::given(method("GET"))
        .and(path("/sellers.json"))
        .and(header("if-none-match", "\"v7\""))
        .respond_with(ResponseTemplate::new(304).insert_header("etag", "\"v7\""))
        .mount(&server)
        .await;
    serve(
        &server,
        "/sellers.json",
        text("{}")
            .insert_header("etag", "\"v7\"")
            .insert_header("last-modified", "Wed, 11 Nov 2026 09:00:00 GMT"),
    )
    .await;

    let first = fetch(&request(&server, "/sellers.json")).await.unwrap();
    let version = FileVersion {
        etag: Some("\"v7\"".into()),
        last_modified: Some("Wed, 11 Nov 2026 09:00:00 GMT".into()),
    };
    assert_eq!((first.status, &first.version), (200, &version));
    assert_eq!(first.final_url, format!("{}/sellers.json", server.uri()));
    let sent = &server.received_requests().await.unwrap()[0];
    assert!(sent.headers.get("if-none-match").is_none());
    assert!(sent.headers.get("if-modified-since").is_none());

    // "Not modified" is an answer, not a redirect without a target.
    let again = PublicFileRequest {
        known_version: Some(version),
        ..request(&server, "/sellers.json")
    };
    let unchanged = fetch(&again).await.unwrap();
    assert_eq!((unchanged.status, unchanged.body.len()), (304, 0));
    let asked = &server.received_requests().await.unwrap()[1];
    assert_eq!(
        asked.headers.get("if-modified-since").unwrap(),
        "Wed, 11 Nov 2026 09:00:00 GMT"
    );

    // A version the server no longer has gets the file.
    let outdated = PublicFileRequest {
        known_version: Some(FileVersion {
            etag: Some("\"v6\"".into()),
            last_modified: None,
        }),
        ..request(&server, "/sellers.json")
    };
    let changed = fetch(&outdated).await.unwrap();
    assert_eq!((changed.status, changed.body.as_slice()), (200, &b"{}"[..]));
}

#[tokio::test]
async fn a_compressed_answer_is_read_as_the_text_it_carries() {
    // `ADS_TXT`, gzip-compressed.
    const GZIPPED: &[u8] = &[
        0x1f, 0x8b, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0xff, 0x4b, 0x2c, 0x28, 0xc8, 0x4b,
        0xad, 0x28, 0x2d, 0xd6, 0x4b, 0xce, 0xcf, 0xd5, 0x51, 0xb0, 0x34, 0xb5, 0xd0, 0x51, 0x70,
        0xf1, 0x0c, 0x72, 0x75, 0x0e, 0xe1, 0x2a, 0x2a, 0x4d, 0xca, 0x4c, 0xce, 0xcf, 0x2b, 0x28,
        0xca, 0xcf, 0x4a, 0x4d, 0x2e, 0x81, 0xc8, 0x1b, 0x1a, 0x18, 0x18, 0xea, 0x28, 0x04, 0xb9,
        0x06, 0xbb, 0xfa, 0xf8, 0xb8, 0x06, 0x71, 0x01, 0x00, 0x3a, 0xe8, 0x07, 0x75, 0x3d, 0x00,
        0x00, 0x00,
    ];
    let server = MockServer::start().await;
    serve(
        &server,
        "/ads.txt",
        ResponseTemplate::new(200)
            .insert_header("content-encoding", "gzip")
            .set_body_raw(GZIPPED.to_vec(), "text/plain"),
    )
    .await;

    let mut limited = request(&server, "/ads.txt");
    limited.max_bytes = ADS_TXT.len();
    let file = fetch(&limited).await.unwrap();
    assert_eq!(String::from_utf8(file.body).unwrap(), ADS_TXT);

    // The limit is on what the file unpacks to.
    limited.max_bytes = ADS_TXT.len() - 1;
    assert_eq!(
        fetch(&limited).await,
        Err(FetchError::TooLarge(ADS_TXT.len() - 1))
    );
}

#[tokio::test]
async fn error_statuses_come_back_without_a_body() {
    let server = MockServer::start().await;
    serve(
        &server,
        "/ads.txt",
        ResponseTemplate::new(404).set_body_raw("<html>Not found</html>", "text/html"),
    )
    .await;
    serve(&server, "/sellers.json", ResponseTemplate::new(503)).await;

    let missing = fetch(&request(&server, "/ads.txt")).await.unwrap();
    assert_eq!(
        (missing.status, missing.body.len()),
        (404, 0),
        "an error page is not downloaded"
    );
    assert_eq!(
        ads_txt::read_response(
            missing.status,
            missing.content_type.as_deref(),
            &missing.body
        ),
        AdsTxtOutcome::Missing
    );
    assert_eq!(
        fetch(&request(&server, "/sellers.json"))
            .await
            .unwrap()
            .status,
        503
    );
}

#[tokio::test]
async fn a_web_page_served_as_the_file_is_fetched_but_not_read_as_ads_txt() {
    let server = MockServer::start().await;
    serve(
        &server,
        "/ads.txt",
        ResponseTemplate::new(200).set_body_raw(
            "<!doctype html><title>Are you human?</title>",
            "text/html; charset=utf-8",
        ),
    )
    .await;
    serve(
        &server,
        "/typeless/ads.txt",
        ResponseTemplate::new(200).set_body_raw(ADS_TXT, "application/octet-stream"),
    )
    .await;

    let page = fetch(&request(&server, "/ads.txt")).await.unwrap();
    assert_eq!(
        ads_txt::read_response(page.status, page.content_type.as_deref(), &page.body),
        AdsTxtOutcome::Unreadable(
            "the site answered with a web page instead of a text file".into()
        )
    );
    let typeless = fetch(&request(&server, "/typeless/ads.txt")).await.unwrap();
    assert!(matches!(
        ads_txt::read_response(typeless.status, typeless.content_type.as_deref(), &typeless.body),
        AdsTxtOutcome::Found(file) if file.records.len() == 2
    ));
}

#[tokio::test]
async fn redirects_within_the_site_are_followed_without_carrying_cookies() {
    let server = MockServer::start().await;
    serve(
        &server,
        "/ads.txt",
        redirect(301, "/files/ads.txt").insert_header("set-cookie", "session=abc; Path=/"),
    )
    .await;
    // A relative and an absolute target.
    serve(
        &server,
        "/files/ads.txt",
        redirect(302, &format!("{}/final/ads.txt", server.uri())),
    )
    .await;
    serve(&server, "/final/ads.txt", text(ADS_TXT)).await;

    let file = fetch(&request(&server, "/ads.txt")).await.unwrap();
    assert_eq!((file.status, file.body), (200, ADS_TXT.as_bytes().to_vec()));

    let received = server.received_requests().await.unwrap();
    assert_eq!(
        received
            .iter()
            .map(|request| request.url.path().to_string())
            .collect::<Vec<_>>(),
        ["/ads.txt", "/files/ads.txt", "/final/ads.txt"]
    );
    assert!(
        received
            .iter()
            .all(|request| request.headers.get("cookie").is_none()),
        "a cookie set on the way is not sent back"
    );
}

#[tokio::test]
async fn a_redirect_loop_ends_after_a_few_hops() {
    let server = MockServer::start().await;
    serve(&server, "/ads.txt", redirect(302, "/ads.txt")).await;

    assert_eq!(
        refusal(fetch(&request(&server, "/ads.txt")).await),
        "the address was not contacted: too many redirects"
    );
    assert_eq!(server.received_requests().await.unwrap().len(), 6);
}

#[tokio::test]
async fn redirects_to_private_plain_http_or_unusable_targets_are_refused() {
    let other_service = MockServer::start().await;
    serve(&other_service, "/admin", text("secret")).await;
    let server = MockServer::start().await;
    let cases = [
        // Another service on this machine: reachable, and not the scanned site.
        (
            format!("{}/admin", other_service.uri()),
            "the address was not contacted: only https addresses are read",
        ),
        (
            "https://192.168.1.1/ads.txt".to_string(),
            "the address was not contacted: it points to this machine or a private network",
        ),
        (
            "https://169.254.169.254/latest/meta-data/".to_string(),
            "the address was not contacted: it points to this machine or a private network",
        ),
        (
            "https://[::1]/ads.txt".to_string(),
            "the address was not contacted: it points to this machine or a private network",
        ),
        (
            "https://localhost/ads.txt".to_string(),
            "the address was not contacted: it points to this machine or a private network",
        ),
        (
            "http://example.com/ads.txt".to_string(),
            "the address was not contacted: only https addresses are read",
        ),
        (
            "file:///etc/passwd".to_string(),
            "the address was not contacted: only https addresses are read",
        ),
        (
            "https://user:pw@example.com/ads.txt".to_string(),
            "the address was not contacted: it carries credentials",
        ),
        (
            "https://example.com:8443/ads.txt".to_string(),
            "the address was not contacted: only the standard https port is read",
        ),
        (
            "https://[::ffff:127.0.0.1]/ads.txt".to_string(),
            "the address was not contacted: it points to this machine or a private network",
        ),
        (
            "https://localhost./ads.txt".to_string(),
            "the address was not contacted: it points to this machine or a private network",
        ),
    ];
    for (index, (target, expected)) in cases.iter().enumerate() {
        let file = format!("/case-{index}/ads.txt");
        serve(&server, &file, redirect(302, target)).await;
        assert_eq!(
            refusal(fetch(&request(&server, &file)).await),
            *expected,
            "{target}"
        );
    }
    serve(&server, "/nowhere/ads.txt", ResponseTemplate::new(302)).await;
    assert_eq!(
        refusal(fetch(&request(&server, "/nowhere/ads.txt")).await),
        "the address was not contacted: the redirect has no usable target"
    );
    assert_eq!(
        other_service.received_requests().await.unwrap().len(),
        0,
        "the other service was never contacted"
    );
}

#[tokio::test]
async fn loopback_is_only_read_as_the_scanned_origin() {
    let server = MockServer::start().await;
    serve(&server, "/ads.txt", text(ADS_TXT)).await;

    let mut not_the_scanned_site = request(&server, "/ads.txt");
    not_the_scanned_site.loopback_origin = None;
    assert_eq!(
        refusal(fetch(&not_the_scanned_site).await),
        "the address was not contacted: only https addresses are read"
    );
    let mut another_port = request(&server, "/ads.txt");
    another_port.loopback_origin = Some("http://127.0.0.1:1".into());
    assert_eq!(
        refusal(fetch(&another_port).await),
        "the address was not contacted: only https addresses are read"
    );
    assert_eq!(server.received_requests().await.unwrap().len(), 0);

    let mut not_an_address = request(&server, "/ads.txt");
    not_an_address.url = "ads.txt".into();
    assert_eq!(
        refusal(fetch(&not_an_address).await),
        "the address was not contacted: it is not a valid address"
    );
}

#[tokio::test]
async fn a_scanned_page_on_localhost_is_read_at_this_machine_by_name() {
    let server = MockServer::start().await;
    serve(&server, "/ads.txt", text(ADS_TXT)).await;
    let origin = format!("http://localhost:{}", server.address().port());

    let by_name = PublicFileRequest {
        url: format!("{origin}/ads.txt"),
        loopback_origin: Some(origin),
        ..request(&server, "/ads.txt")
    };
    assert_eq!(fetch(&by_name).await.unwrap().body, ADS_TXT.as_bytes());
}

#[tokio::test]
async fn an_oversized_file_is_an_error_not_a_truncated_file() {
    let server = MockServer::start().await;
    let line = "appnexus.com, 958, DIRECT\n";
    serve(&server, "/ads.txt", text(&line.repeat(4_000))).await;
    serve(&server, "/exact/ads.txt", text(&"x".repeat(1_024))).await;

    let mut oversized = request(&server, "/ads.txt");
    oversized.max_bytes = 1_024;
    assert_eq!(fetch(&oversized).await, Err(FetchError::TooLarge(1_024)));

    let mut exact = request(&server, "/exact/ads.txt");
    exact.max_bytes = 1_024;
    assert_eq!(fetch(&exact).await.unwrap().body.len(), 1_024);
}

#[tokio::test]
async fn the_size_limit_counts_decompressed_bytes() {
    // 100,000 line feeds, gzip-compressed to 133 bytes.
    const BOMB: &[u8] = &[
        0x1f, 0x8b, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0xff, 0xed, 0xc1, 0x21, 0x01, 0x00,
        0x00, 0x00, 0xc3, 0x20, 0xbf, 0xfe, 0x81, 0x5f, 0xe3, 0x02, 0x28, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0xe0, 0xd5, 0x00, 0x7c, 0x0a, 0x6d, 0xd2, 0xa0, 0x86, 0x01, 0x00,
    ];
    let server = MockServer::start().await;
    serve(
        &server,
        "/ads.txt",
        ResponseTemplate::new(200)
            .insert_header("content-encoding", "gzip")
            .set_body_raw(BOMB.to_vec(), "text/plain"),
    )
    .await;

    let mut limited = request(&server, "/ads.txt");
    limited.max_bytes = 64 * 1024;
    assert!(BOMB.len() < limited.max_bytes);
    assert_eq!(fetch(&limited).await, Err(FetchError::TooLarge(64 * 1024)));

    let mut roomy = request(&server, "/ads.txt");
    roomy.max_bytes = 100_000;
    assert_eq!(fetch(&roomy).await.unwrap().body.len(), 100_000);
}

#[tokio::test]
async fn a_slow_server_runs_into_the_time_limit() {
    let server = MockServer::start().await;
    serve(
        &server,
        "/ads.txt",
        text(ADS_TXT).set_delay(Duration::from_secs(30)),
    )
    .await;

    let mut slow = request(&server, "/ads.txt");
    slow.timeout = Duration::from_millis(300);
    let started = Instant::now();
    assert_eq!(fetch(&slow).await, Err(FetchError::TimedOut));
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[tokio::test]
async fn a_cancelled_scan_stops_the_request() {
    let server = MockServer::start().await;
    serve(
        &server,
        "/ads.txt",
        text(ADS_TXT).set_delay(Duration::from_secs(30)),
    )
    .await;

    let cancel = CancellationToken::new();
    let stop = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        stop.cancel();
    });
    let started = Instant::now();
    let outcome = HttpPublicFileFetcher
        .fetch(&request(&server, "/ads.txt"), &cancel)
        .await;
    assert_eq!(outcome, Err(FetchError::Cancelled));
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[tokio::test]
async fn an_unreachable_server_is_a_network_error_in_plain_words() {
    // A port nothing listens on any more.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    let gone = PublicFileRequest {
        url: format!("{origin}/ads.txt"),
        max_bytes: 1_024,
        timeout: Duration::from_secs(5),
        loopback_origin: Some(origin),
        proxy: system_proxy(),
        known_version: None,
    };
    assert_eq!(
        fetch(&gone).await,
        Err(FetchError::Network("could not connect".into()))
    );
}

/// A proxy that records the first request it is sent and refuses it.
async fn recording_proxy() -> (u16, tokio::task::JoinHandle<String>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let received = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        let mut buffer = [0u8; 1024];
        while !request.windows(4).any(|window| window == b"\r\n\r\n") {
            let read = socket.read(&mut buffer).await.unwrap();
            if read == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..read]);
        }
        let _ = socket
            .write_all(b"HTTP/1.1 502 Bad Gateway\r\ncontent-length: 0\r\n\r\n")
            .await;
        String::from_utf8_lossy(&request).into_owned()
    });
    (port, received)
}

fn through_proxy(port: u16, username: Option<&str>, password: Option<&str>) -> FetchProxy {
    FetchProxy {
        route: CaptureProxy::Custom {
            scheme: ProxyScheme::Http,
            host: "127.0.0.1".into(),
            port,
            username: username.map(str::to_string),
        },
        password: password.map(str::to_string),
    }
}

#[tokio::test]
async fn a_public_file_is_requested_through_the_scans_proxy_with_its_credentials() {
    let (port, received) = recording_proxy().await;
    // A public address, so nothing depends on this machine's resolver.
    let request = PublicFileRequest {
        url: "https://93.184.216.34/ads.txt".into(),
        max_bytes: 1_024,
        timeout: Duration::from_secs(5),
        loopback_origin: None,
        proxy: through_proxy(port, Some("scanner"), Some("s3cret")),
        known_version: None,
    };
    // The proxy refuses the tunnel; what matters is what it was asked.
    assert!(matches!(fetch(&request).await, Err(FetchError::Network(_))));
    let asked = received.await.unwrap();
    assert!(
        asked.starts_with("CONNECT 93.184.216.34:443 "),
        "the proxy was asked: {asked}"
    );
    // base64("scanner:s3cret")
    assert!(
        asked
            .to_ascii_lowercase()
            .contains("proxy-authorization: basic c2nhbm5lcjpzm2nyzxq="),
        "the proxy was asked: {asked}"
    );
}

#[tokio::test]
async fn the_scanned_origin_on_this_machine_is_never_read_through_the_proxy() {
    let server = MockServer::start().await;
    serve(&server, "/ads.txt", text(ADS_TXT)).await;
    // Nothing listens at the proxy: a request sent there could not succeed.
    let unused = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let proxy_port = unused.local_addr().unwrap().port();
    drop(unused);

    let request = PublicFileRequest {
        proxy: through_proxy(proxy_port, None, None),
        ..request(&server, "/ads.txt")
    };
    assert_eq!(fetch(&request).await.unwrap().body, ADS_TXT.as_bytes());
}

#[tokio::test]
async fn the_address_checks_hold_whatever_the_proxy() {
    let (port, _received) = recording_proxy().await;
    for proxy in [
        through_proxy(port, None, None),
        FetchProxy {
            route: CaptureProxy::None,
            password: None,
        },
    ] {
        for (url, reason) in [
            (
                "http://example.com/ads.txt",
                "only https addresses are read",
            ),
            (
                "https://10.0.0.8/ads.txt",
                "it points to this machine or a private network",
            ),
            (
                "https://user:secret@example.com/ads.txt",
                "it carries credentials",
            ),
        ] {
            let request = PublicFileRequest {
                url: url.into(),
                max_bytes: 1_024,
                timeout: Duration::from_secs(5),
                loopback_origin: None,
                proxy: proxy.clone(),
                known_version: None,
            };
            assert_eq!(
                refusal(fetch(&request).await),
                format!("the address was not contacted: {reason}"),
                "{url}"
            );
        }
    }
}

#[test]
fn a_proxy_password_never_shows_in_debug_output() {
    let printed = format!("{:?}", through_proxy(8080, Some("scanner"), Some("s3cret")));
    assert!(!printed.contains("s3cret"), "{printed}");
    assert!(printed.contains("has_password: true"), "{printed}");
}
