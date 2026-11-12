//! Reads public text files (ads.txt, sellers.json) from the open web.
//!
//! The addresses come from the scanned page and from files it publishes, so
//! every hop is checked before it is contacted: https on its standard port
//! only, public addresses only, no credentials, no cookies, redirects followed
//! by hand within the scope the specifications allow, and hard limits on size
//! (after decompression) and time. The one exception is the scanned page's own
//! origin when that page is served from this machine, as the built-in demo
//! site is.
//!
//! A request leaves the way its scan's browser did: directly, through the
//! system's proxy, or through the proxy profile the scan chose. Behind any
//! proxy the proxy resolves the host name, so two of the protections above
//! are weaker there, and nothing here can restore them:
//!
//! - the address check covers IP addresses and whatever this machine's own
//!   resolver answers, not the address the proxy ends up connecting to;
//! - the connection cannot be pinned to the checked addresses.
//!
//! What a proxy may reach is the proxy's to decide. Everything else holds
//! behind a proxy too: https only, no credentials or cookies, redirects by
//! hand, size and time limits. The proxy's own password is sent to the proxy
//! and to nothing else.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;

use async_trait::async_trait;
use reqwest::header::{ACCEPT, CONTENT_TYPE, LOCATION, USER_AGENT};
use reqwest::redirect::Policy;
use tokio_util::sync::CancellationToken;
use url::{Host, Url};

use super::http::describe_transport_error;
use adscan_model::{CaptureProxy, ProxyScheme};

use crate::application::ports::{
    FetchError, FetchProxy, PublicFile, PublicFileFetcher, PublicFileRequest,
};
use crate::domain::diagnostics::site::{RedirectScope, is_loopback};

const CLIENT_NAME: &str = concat!("adtech-scanner/", env!("CARGO_PKG_VERSION"));
const CONNECT_TIMEOUT: Duration = Duration::from_secs(8);
const HTTPS_PORT: u16 = 443;

#[derive(Debug, Default, Clone, Copy)]
pub struct HttpPublicFileFetcher;

#[async_trait]
impl PublicFileFetcher for HttpPublicFileFetcher {
    async fn fetch(
        &self,
        request: &PublicFileRequest,
        cancel: &CancellationToken,
    ) -> Result<PublicFile, FetchError> {
        tokio::select! {
            () = cancel.cancelled() => Err(FetchError::Cancelled),
            outcome = tokio::time::timeout(request.timeout, exchange(request)) => {
                outcome.unwrap_or(Err(FetchError::TimedOut))
            }
        }
    }
}

async fn exchange(request: &PublicFileRequest) -> Result<PublicFile, FetchError> {
    let mut url = Url::parse(&request.url).map_err(|_| refused("it is not a valid address"))?;
    let mut scope = RedirectScope::starting_at(url.host_str().unwrap_or_default());
    loop {
        let route = admit(&url, request.loopback_origin.as_deref()).await?;
        let mut response = client(&url, &route, &request.proxy)?
            .get(url.clone())
            .header(USER_AGENT, CLIENT_NAME)
            .header(ACCEPT, "text/plain, application/json;q=0.9, */*;q=0.1")
            .send()
            .await
            .map_err(network)?;
        let status = response.status();
        if status.is_redirection() {
            let next = response
                .headers()
                .get(LOCATION)
                .and_then(|location| location.to_str().ok())
                .and_then(|location| url.join(location).ok())
                .ok_or_else(|| refused("the redirect has no usable target"))?;
            scope
                .follow(next.host_str().unwrap_or_default())
                .map_err(refused)?;
            url = next;
            continue;
        }
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        let mut body = Vec::new();
        if status.is_success() {
            let too_large = || FetchError::TooLarge(request.max_bytes);
            if response
                .content_length()
                .is_some_and(|length| length > request.max_bytes as u64)
            {
                return Err(too_large());
            }
            while let Some(chunk) = response.chunk().await.map_err(network)? {
                if body.len() + chunk.len() > request.max_bytes {
                    return Err(too_large());
                }
                body.extend_from_slice(&chunk);
            }
        }
        return Ok(PublicFile {
            status: status.as_u16(),
            content_type,
            body,
        });
    }
}

/// How an admitted address is to be reached.
#[derive(Debug, PartialEq, Eq)]
enum Route {
    /// The scanned page's own origin on this machine: directly, never
    /// through a proxy.
    ScannedOrigin,
    /// A public IP address.
    PublicAddress,
    /// A public host name with the addresses that were checked. A direct
    /// connection goes to exactly those, so a second DNS answer cannot point
    /// it elsewhere; with none, only the system's proxy can reach the host.
    PublicName(Vec<SocketAddr>),
}

/// Decides whether `url` may be contacted, and how.
async fn admit(url: &Url, loopback_origin: Option<&str>) -> Result<Route, FetchError> {
    if !url.username().is_empty() || url.password().is_some() {
        return Err(refused("it carries credentials"));
    }
    let is_scanned_origin = matches!(url.scheme(), "http" | "https")
        && url.host().is_some_and(|host| is_loopback(&host))
        && loopback_origin.is_some_and(|origin| url.origin().ascii_serialization() == origin);
    if is_scanned_origin {
        return Ok(Route::ScannedOrigin);
    }
    if url.scheme() != "https" {
        return Err(refused("only https addresses are read"));
    }
    // An address on this machine or a private network is refused as that,
    // whatever its port.
    let host = url.host().ok_or_else(|| refused("it has no host"))?;
    let domain = match host {
        // `localhost` in any spelling, whether or not this machine resolves
        // it: a proxy would resolve it to itself.
        Host::Domain(_) if is_loopback(&host) => return Err(not_public()),
        Host::Ipv4(ip) => {
            require_public(IpAddr::V4(ip))?;
            None
        }
        Host::Ipv6(ip) => {
            require_public(IpAddr::V6(ip))?;
            None
        }
        Host::Domain(domain) => Some(domain),
    };
    // `port()` is `None` for the scheme's own port.
    if url.port().is_some() {
        return Err(refused("only the standard https port is read"));
    }
    let Some(domain) = domain else {
        return Ok(Route::PublicAddress);
    };
    let resolved: Vec<SocketAddr> = tokio::net::lookup_host((domain, HTTPS_PORT))
        .await
        .map(Iterator::collect)
        .unwrap_or_default();
    for address in &resolved {
        require_public(address.ip())?;
    }
    Ok(Route::PublicName(resolved))
}

fn client(url: &Url, route: &Route, proxy: &FetchProxy) -> Result<reqwest::Client, FetchError> {
    let builder = reqwest::Client::builder()
        .redirect(Policy::none())
        .connect_timeout(CONNECT_TIMEOUT);
    let host = url.host_str().unwrap_or_default();
    // The scanned origin on this machine is exempt below: `no_proxy` there
    // is set last and wins.
    let builder = match &proxy.route {
        CaptureProxy::System => builder,
        CaptureProxy::None => builder.no_proxy(),
        CaptureProxy::Custom {
            scheme,
            host,
            port,
            username,
        } => builder.proxy(custom_proxy(
            *scheme,
            host,
            *port,
            username.as_deref(),
            proxy.password.as_deref(),
        )?),
    };
    let builder = match route {
        // `localhost` is this machine whatever a resolver says.
        Route::ScannedOrigin => builder.no_proxy().resolve_to_addrs(
            host,
            &[
                SocketAddr::from((Ipv4Addr::LOCALHOST, 0)),
                SocketAddr::from((Ipv6Addr::LOCALHOST, 0)),
            ],
        ),
        Route::PublicAddress => builder,
        // Pinned even when nothing was resolved: the client then has no
        // address of its own to connect to, instead of asking DNS again.
        Route::PublicName(pinned) => builder.resolve_to_addrs(host, pinned),
    };
    builder.build().map_err(network)
}

fn custom_proxy(
    scheme: ProxyScheme,
    host: &str,
    port: u16,
    username: Option<&str>,
    password: Option<&str>,
) -> Result<reqwest::Proxy, FetchError> {
    let scheme = match scheme {
        ProxyScheme::Http => "http",
        ProxyScheme::Https => "https",
        // `socks5h`: the proxy resolves the name, as an http proxy does, so
        // the file is looked up from where the scan's pages were.
        ProxyScheme::Socks5 => "socks5h",
    };
    let host = match host.parse::<Ipv6Addr>() {
        Ok(address) => format!("[{address}]"),
        Err(_) => host.to_string(),
    };
    let proxy = reqwest::Proxy::all(format!("{scheme}://{host}:{port}"))
        .map_err(|_| FetchError::Network("the proxy address is not valid".into()))?;
    Ok(match username {
        Some(username) => proxy.basic_auth(username, password.unwrap_or_default()),
        None => proxy,
    })
}

fn require_public(ip: IpAddr) -> Result<(), FetchError> {
    if is_public(ip) {
        Ok(())
    } else {
        Err(not_public())
    }
}

fn not_public() -> FetchError {
    refused("it points to this machine or a private network")
}

fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => is_public_v4(ip),
        IpAddr::V6(ip) => match embedded_v4(ip) {
            Some(embedded) => is_public_v4(embedded),
            None => is_public_v6(ip),
        },
    }
}

/// The IPv4 address an IPv6 address stands for, where it is a wrapper around
/// one: IPv4-mapped and IPv4-compatible addresses, NAT64 and 6to4.
fn embedded_v4(ip: Ipv6Addr) -> Option<Ipv4Addr> {
    let segments = ip.segments();
    let last_32_bits = || {
        let [.., a, b, c, d] = ip.octets();
        Ipv4Addr::new(a, b, c, d)
    };
    match segments {
        [0, 0, 0, 0, 0, 0xffff, ..] => Some(last_32_bits()),
        [0, 0, 0, 0, 0, 0, high, low] if high != 0 || low > 1 => Some(last_32_bits()),
        [0x64, 0xff9b, 0, 0, 0, 0, ..] => Some(last_32_bits()),
        [0x2002, high, low, ..] => {
            let [a, b] = high.to_be_bytes();
            let [c, d] = low.to_be_bytes();
            Some(Ipv4Addr::new(a, b, c, d))
        }
        _ => None,
    }
}

fn is_public_v4(ip: Ipv4Addr) -> bool {
    let [a, b, ..] = ip.octets();
    let shared_address_space = a == 100 && (64..128).contains(&b);
    let protocol_assignments = a == 192 && b == 0 && ip.octets()[2] == 0;
    let benchmarking = a == 198 && (18..20).contains(&b);
    let reserved = a >= 240;
    !(ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_unspecified()
        || ip.is_broadcast()
        || ip.is_multicast()
        || a == 0
        || shared_address_space
        || protocol_assignments
        || benchmarking
        || reserved)
}

fn is_public_v6(ip: Ipv6Addr) -> bool {
    let first = ip.segments()[0];
    let unique_local = first & 0xfe00 == 0xfc00;
    let link_local = first & 0xffc0 == 0xfe80;
    let site_local = first & 0xffc0 == 0xfec0;
    // Teredo (which hides an IPv4 address) and the documentation range.
    let special_purpose = first == 0x2001 && matches!(ip.segments()[1], 0 | 0xdb8);
    !(ip.is_loopback()
        || ip.is_unspecified()
        || ip.is_multicast()
        || unique_local
        || link_local
        || site_local
        || special_purpose)
}

fn refused(reason: &str) -> FetchError {
    FetchError::Refused(format!("the address was not contacted: {reason}"))
}

fn network(error: reqwest::Error) -> FetchError {
    FetchError::Network(describe_transport_error(error))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_public_addresses_pass() {
        let cases = [
            ("93.184.216.34", true),
            ("8.8.8.8", true),
            ("2606:4700:4700::1111", true),
            ("127.0.0.1", false),
            ("127.8.9.10", false),
            ("10.1.2.3", false),
            ("172.16.0.1", false),
            ("172.31.255.255", false),
            ("192.168.1.1", false),
            ("169.254.169.254", false),
            ("100.64.0.1", false),
            ("192.0.0.8", false),
            ("198.18.0.1", false),
            ("0.0.0.0", false),
            ("0.1.2.3", false),
            ("224.0.0.1", false),
            ("240.0.0.1", false),
            ("255.255.255.255", false),
            ("::1", false),
            ("::", false),
            ("fc00::1", false),
            ("fd12:3456::1", false),
            ("fe80::1", false),
            ("ff02::1", false),
            ("::ffff:127.0.0.1", false),
            ("::ffff:10.0.0.1", false),
            ("::ffff:8.8.8.8", true),
            ("::ffff:169.254.169.254", false),
            ("::127.0.0.1", false),
            ("::10.0.0.1", false),
            ("64:ff9b::7f00:1", false),
            ("64:ff9b::a9fe:a9fe", false),
            ("64:ff9b::808:808", true),
            ("2002:7f00:1::1", false),
            ("2002:c0a8:101::1", false),
            ("2002:808:808::1", true),
            ("fec0::1", false),
            ("2001:db8::1", false),
            ("2001:0:4136:e378:8000:63bf:3fff:fdd2", false),
            ("100.127.255.255", false),
            ("100.128.0.1", true),
        ];
        for (address, expected) in cases {
            assert_eq!(is_public(address.parse().unwrap()), expected, "{address}");
        }
    }

    #[tokio::test]
    async fn addresses_are_admitted_by_scheme_host_and_the_scanned_origin() {
        let scanned = Some("http://127.0.0.1:4455");
        let admitted = |url: &'static str, origin: Option<&'static str>| async move {
            admit(&Url::parse(url).unwrap(), origin)
                .await
                .map_err(|error| error.to_string())
        };
        assert_eq!(
            admitted("http://127.0.0.1:4455/ads.txt", scanned).await,
            Ok(Route::ScannedOrigin)
        );
        assert_eq!(
            admitted("https://93.184.216.34/ads.txt", None).await,
            Ok(Route::PublicAddress)
        );

        let private = Err(
            "the address was not contacted: it points to this machine or a private network"
                .to_string(),
        );
        let https_only =
            Err("the address was not contacted: only https addresses are read".to_string());
        // Another port of this machine is another service.
        assert_eq!(
            admitted("https://127.0.0.1:4456/ads.txt", scanned).await,
            private
        );
        assert_eq!(
            admitted("http://127.0.0.1:4456/ads.txt", scanned).await,
            https_only
        );
        assert_eq!(
            admitted("http://127.0.0.1:4455/ads.txt", None).await,
            https_only
        );
        assert_eq!(admitted("https://localhost/ads.txt", None).await, private);
        assert_eq!(
            admitted("https://192.168.1.1/admin", scanned).await,
            private
        );
        assert_eq!(admitted("https://[::1]/ads.txt", None).await, private);
        assert_eq!(
            admitted("http://example.com/ads.txt", None).await,
            https_only
        );
        assert_eq!(
            admitted("ftp://example.com/ads.txt", None).await,
            https_only
        );
        assert_eq!(admitted("file:///etc/passwd", None).await, https_only);
        assert_eq!(
            admitted("https://user:secret@example.com/ads.txt", None).await,
            Err("the address was not contacted: it carries credentials".to_string())
        );
        assert_eq!(
            admitted("https://user@93.184.216.34/ads.txt", None).await,
            Err("the address was not contacted: it carries credentials".to_string())
        );
        let standard_port =
            Err("the address was not contacted: only the standard https port is read".to_string());
        for other_port in [
            "https://93.184.216.34:8443/ads.txt",
            "https://93.184.216.34:22/",
            "https://example.com:6379/sellers.json",
        ] {
            assert_eq!(admitted(other_port, scanned).await, standard_port);
        }
        assert_eq!(
            admitted("https://93.184.216.34:443/ads.txt", None).await,
            Ok(Route::PublicAddress)
        );
        // Every spelling of a private address is the same address.
        for spelling in [
            "https://2130706433/ads.txt",
            "https://0x7f.1/ads.txt",
            "https://017700000001/ads.txt",
            "https://[::ffff:7f00:1]/ads.txt",
            "https://[::ffff:169.254.169.254]/latest/meta-data/",
            "https://169.254.169.254/latest/meta-data/",
            "https://localhost./ads.txt",
            "https://LOCALHOST/ads.txt",
            "https://app.localhost/ads.txt",
        ] {
            assert_eq!(admitted(spelling, None).await, private, "{spelling}");
        }
    }

    #[tokio::test]
    async fn a_caller_cannot_name_a_public_origin_as_the_scanned_one() {
        let admitted = admit(
            &Url::parse("http://93.184.216.34/ads.txt").unwrap(),
            Some("http://93.184.216.34"),
        )
        .await;
        assert_eq!(
            admitted.map_err(|error| error.to_string()),
            Err("the address was not contacted: only https addresses are read".to_string())
        );
    }

    /// Answers one HTTP request on loopback with "pinned" and returns its port.
    async fn one_shot_server() -> u16 {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 1024];
            let _ = stream.read(&mut request).await;
            let _ = stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\ncontent-length: 6\r\nconnection: close\r\n\r\npinned",
                )
                .await;
        });
        port
    }

    fn system_proxy() -> FetchProxy {
        FetchProxy {
            route: CaptureProxy::System,
            password: None,
        }
    }

    #[tokio::test]
    async fn a_host_name_is_connected_at_the_checked_addresses_only() {
        // The name does not exist, so only the pinned address can answer for it.
        let port = one_shot_server().await;
        let url = Url::parse(&format!("http://files.pinned.invalid:{port}/ads.txt")).unwrap();
        let pinned = Route::PublicName(vec![SocketAddr::from(([127, 0, 0, 1], port))]);
        let body = client(&url, &pinned, &system_proxy())
            .unwrap()
            .get(url.clone())
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert_eq!(body, "pinned");
    }

    #[tokio::test]
    async fn a_name_that_did_not_resolve_is_not_looked_up_a_second_time() {
        // `localhost` resolves on every machine. With no checked address the
        // client must have nowhere to connect to rather than resolve it itself.
        let port = one_shot_server().await;
        let url = Url::parse(&format!("http://localhost:{port}/ads.txt")).unwrap();
        let outcome = client(&url, &Route::PublicName(Vec::new()), &system_proxy())
            .unwrap()
            .get(url.clone())
            .send()
            .await;
        assert!(outcome.is_err(), "{outcome:?}");
    }
}
