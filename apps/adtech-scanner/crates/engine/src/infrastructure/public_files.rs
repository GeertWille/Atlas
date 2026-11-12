//! Reads public text files (ads.txt, sellers.json) from the open web.
//!
//! The addresses come from the scanned page and from files it publishes, so
//! every hop is checked before it is contacted: https only, public addresses
//! only, no credentials, no cookies, redirects followed by hand within the
//! scope the specifications allow, and hard limits on size and time. The one
//! exception is the scanned page's own origin when that page is served from
//! this machine, as the built-in demo site is.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;

use async_trait::async_trait;
use reqwest::header::{ACCEPT, CONTENT_TYPE, LOCATION, USER_AGENT};
use reqwest::redirect::Policy;
use tokio_util::sync::CancellationToken;
use url::{Host, Url};

use super::http::describe_transport_error;
use crate::application::ports::{FetchError, PublicFile, PublicFileFetcher, PublicFileRequest};
use crate::domain::diagnostics::site::RedirectScope;

const CLIENT_NAME: &str = concat!("adtech-scanner/", env!("CARGO_PKG_VERSION"));
const CONNECT_TIMEOUT: Duration = Duration::from_secs(8);

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
        let pinned = admit(&url, request.loopback_origin.as_deref()).await?;
        let mut response = client(&url, &pinned)?
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

/// Decides whether `url` may be contacted. For a host name it returns the
/// addresses that were checked, so the connection goes to exactly those and a
/// second DNS answer cannot point it elsewhere.
async fn admit(url: &Url, loopback_origin: Option<&str>) -> Result<Vec<SocketAddr>, FetchError> {
    if !url.username().is_empty() || url.password().is_some() {
        return Err(refused("it carries credentials"));
    }
    let is_scanned_origin =
        loopback_origin.is_some_and(|origin| url.origin().ascii_serialization() == origin);
    if is_scanned_origin && matches!(url.scheme(), "http" | "https") {
        return Ok(Vec::new());
    }
    if url.scheme() != "https" {
        return Err(refused("only https addresses are read"));
    }
    match url.host() {
        Some(Host::Ipv4(ip)) => require_public(IpAddr::V4(ip)).map(|()| Vec::new()),
        Some(Host::Ipv6(ip)) => require_public(IpAddr::V6(ip)).map(|()| Vec::new()),
        Some(Host::Domain(domain)) => {
            let port = url.port_or_known_default().unwrap_or(443);
            // Behind a proxy that does the resolving, a name this machine
            // cannot resolve is left to the proxy.
            let resolved: Vec<SocketAddr> = tokio::net::lookup_host((domain, port))
                .await
                .map(Iterator::collect)
                .unwrap_or_default();
            for address in &resolved {
                require_public(address.ip())?;
            }
            Ok(resolved)
        }
        None => Err(refused("it has no host")),
    }
}

fn client(url: &Url, pinned: &[SocketAddr]) -> Result<reqwest::Client, FetchError> {
    let mut builder = reqwest::Client::builder()
        .redirect(Policy::none())
        .connect_timeout(CONNECT_TIMEOUT);
    if let (Some(host), false) = (url.host_str(), pinned.is_empty()) {
        builder = builder.resolve_to_addrs(host, pinned);
    }
    builder.build().map_err(network)
}

fn require_public(ip: IpAddr) -> Result<(), FetchError> {
    if is_public(ip) {
        Ok(())
    } else {
        Err(refused("it points to this machine or a private network"))
    }
}

fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => is_public_v4(ip),
        IpAddr::V6(ip) => match ip.to_ipv4_mapped() {
            Some(mapped) => is_public_v4(mapped),
            None => is_public_v6(ip),
        },
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
    !(ip.is_loopback() || ip.is_unspecified() || ip.is_multicast() || unique_local || link_local)
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
                .map(|_| ())
                .map_err(|error| error.to_string())
        };
        assert_eq!(
            admitted("http://127.0.0.1:4455/ads.txt", scanned).await,
            Ok(())
        );
        assert_eq!(
            admitted("https://93.184.216.34/ads.txt", None).await,
            Ok(())
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
    }
}
