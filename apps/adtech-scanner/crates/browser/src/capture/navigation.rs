//! Turns navigation failures into messages a non-engineer can act on. No I/O.

use url::Url;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum NavigationFailure {
    #[error("\"{0}\" is not a web address. Enter a full address starting with http:// or https://")]
    InvalidAddress(String),
    #[error("{0}")]
    Network(String),
    #[error("{host} answered with an error page (HTTP {status} {reason}). {advice}")]
    HttpStatus {
        host: String,
        status: u16,
        reason: &'static str,
        advice: &'static str,
    },
    #[error(
        "{host} did not answer within {seconds} seconds. Check that the site is up and try again"
    )]
    Timeout { host: String, seconds: u64 },
    #[error(
        "The proxy {proxy} asked for a user name and password and did not accept what the scan \
         has for it. Check the proxy's user name and password in Settings"
    )]
    ProxyLogin { proxy: String },
}

pub(crate) fn parse_address(address: &str) -> Result<Url, NavigationFailure> {
    let invalid = || NavigationFailure::InvalidAddress(address.trim().to_string());
    let url = Url::parse(address.trim()).map_err(|_| invalid())?;
    let is_web = matches!(url.scheme(), "http" | "https") && url.host_str().is_some();
    if is_web { Ok(url) } else { Err(invalid()) }
}

pub(crate) fn host_of(url: &Url) -> String {
    url.host_str().unwrap_or_default().to_string()
}

/// `error_text` is Chrome's `net::ERR_*` code from `Page.navigate`; `proxy`
/// names the proxy the scan was told to use, if any.
pub(crate) fn network_failure(
    error_text: &str,
    host: &str,
    proxy: Option<&str>,
) -> NavigationFailure {
    let code = error_text.trim_start_matches("net::");
    let the_proxy = match proxy {
        Some(proxy) => format!("The proxy {proxy}"),
        None => "The system's proxy".to_string(),
    };
    let explanation = match code {
        "ERR_PROXY_CONNECTION_FAILED"
        | "ERR_SOCKS_CONNECTION_FAILED"
        | "ERR_NO_SUPPORTED_PROXIES"
        | "ERR_MANDATORY_PROXY_CONFIGURATION_FAILED" => {
            format!(
                "{the_proxy} could not be reached. Check its address and port, and that it is running"
            )
        }
        "ERR_TUNNEL_CONNECTION_FAILED" | "ERR_SOCKS_CONNECTION_HOST_UNREACHABLE" => {
            format!("{the_proxy} could not or would not connect to {host}")
        }
        "ERR_PROXY_AUTH_UNSUPPORTED" | "ERR_PROXY_AUTH_REQUESTED" | "ERR_INVALID_AUTH_CREDENTIALS" => {
            format!("{the_proxy} asked for a login the scan could not give")
        }
        "ERR_PROXY_CERTIFICATE_INVALID" => {
            format!("{the_proxy} presented a security certificate that is not valid")
        }
        "ERR_NAME_NOT_RESOLVED" | "ERR_NAME_RESOLUTION_FAILED" => {
            format!(
                "The address {host} does not exist (DNS lookup failed). Check the spelling of the domain"
            )
        }
        "ERR_INTERNET_DISCONNECTED" | "ERR_NETWORK_CHANGED" | "ERR_ADDRESS_UNREACHABLE" => {
            format!("{host} could not be reached. Check your internet connection")
        }
        "ERR_CONNECTION_REFUSED" => {
            format!(
                "{host} refused the connection. Check that the site is running and the port is correct"
            )
        }
        "ERR_CONNECTION_RESET" | "ERR_CONNECTION_CLOSED" | "ERR_EMPTY_RESPONSE" => {
            format!(
                "{host} closed the connection without answering. The site may be down or blocking automated visits"
            )
        }
        "ERR_CONNECTION_TIMED_OUT" | "ERR_TIMED_OUT" => {
            format!("{host} took too long to answer. Check that the site is up and try again")
        }
        "ERR_TOO_MANY_REDIRECTS" => {
            format!(
                "{host} redirects in a loop. Open the page in a normal browser to check it works"
            )
        }
        "ERR_HTTP_RESPONSE_CODE_FAILURE" => format!("{host} answered with an error and no page"),
        "ERR_BLOCKED_BY_CLIENT" | "ERR_BLOCKED_BY_RESPONSE" | "ERR_BLOCKED_BY_ADMINISTRATOR" => {
            format!("The request to {host} was blocked before it left the browser")
        }
        "ERR_ABORTED" => {
            format!("{host} did not return a web page (the address may be a download)")
        }
        _ if is_certificate_error(code) => {
            format!(
                "The security certificate of {host} is not valid ({code}), so the browser refused to open it"
            )
        }
        _ if code.starts_with("ERR_SSL") => {
            format!("A secure connection to {host} could not be established ({code})")
        }
        _ => format!("The browser could not open {host} ({code})"),
    };
    NavigationFailure::Network(explanation)
}

fn is_certificate_error(code: &str) -> bool {
    code.starts_with("ERR_CERT") || code == "ERR_SSL_VERSION_OR_CIPHER_MISMATCH"
}

/// `None` when the main document's status is not an error.
pub(crate) fn http_failure(status: u16, host: &str) -> Option<NavigationFailure> {
    let (reason, advice) = match status {
        0..=399 => return None,
        401 => (
            "Unauthorized",
            "The page needs a login the scanner does not have",
        ),
        407 => (
            "Proxy Authentication Required",
            "The proxy between the scan and the site needs a user name and password",
        ),
        403 => (
            "Forbidden",
            "The site refuses this visitor; it may block automated browsers or your network",
        ),
        404 | 410 => ("Not Found", "Check the address: this page does not exist"),
        429 => (
            "Too Many Requests",
            "The site is rate limiting you; wait a moment and try again",
        ),
        400..=499 => ("Client Error", "Check the address and try again"),
        502..=504 => (
            "Bad Gateway",
            "The site is temporarily unavailable; try again later",
        ),
        _ => (
            "Server Error",
            "The site is failing on its own; try again later",
        ),
    };
    Some(NavigationFailure::HttpStatus {
        host: host.to_string(),
        status,
        reason,
        advice,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_absolute_web_addresses() {
        assert_eq!(
            host_of(&parse_address(" https://news.example/a?b=1 ").unwrap()),
            "news.example"
        );
        assert!(parse_address("http://127.0.0.1:8080/").is_ok());
        for bad in [
            "news.example",
            "ftp://news.example",
            "file:///etc/passwd",
            "javascript:alert(1)",
            "",
            "about:blank",
        ] {
            assert_eq!(
                parse_address(bad),
                Err(NavigationFailure::InvalidAddress(bad.to_string())),
                "{bad}"
            );
        }
    }

    #[test]
    fn explains_common_network_errors_in_plain_language() {
        let cases = [
            ("net::ERR_NAME_NOT_RESOLVED", "does not exist"),
            ("net::ERR_CONNECTION_REFUSED", "refused the connection"),
            ("net::ERR_INTERNET_DISCONNECTED", "internet connection"),
            ("net::ERR_CERT_AUTHORITY_INVALID", "security certificate"),
            ("net::ERR_CERT_DATE_INVALID", "security certificate"),
            ("net::ERR_SSL_PROTOCOL_ERROR", "secure connection"),
            ("net::ERR_TOO_MANY_REDIRECTS", "redirects in a loop"),
            ("net::ERR_TIMED_OUT", "too long"),
            ("net::ERR_SOMETHING_NEW", "ERR_SOMETHING_NEW"),
        ];
        for (code, expected) in cases {
            let message = network_failure(code, "news.example", None).to_string();
            assert!(message.contains(expected), "{code}: {message}");
            assert!(message.contains("news.example"), "{code}: {message}");
            assert!(!message.contains("net::"), "{code}: {message}");
        }
    }

    #[test]
    fn proxy_failures_name_the_proxy_and_not_the_site_as_the_problem() {
        let cases = [
            ("net::ERR_PROXY_CONNECTION_FAILED", "could not be reached"),
            ("net::ERR_SOCKS_CONNECTION_FAILED", "could not be reached"),
            ("net::ERR_TUNNEL_CONNECTION_FAILED", "connect to news.example"),
            ("net::ERR_PROXY_CERTIFICATE_INVALID", "certificate"),
            ("net::ERR_INVALID_AUTH_CREDENTIALS", "login"),
        ];
        for (code, expected) in cases {
            let message =
                network_failure(code, "news.example", Some("proxy.corp.example")).to_string();
            assert!(message.contains(expected), "{code}: {message}");
            assert!(
                message.starts_with("The proxy proxy.corp.example"),
                "{code}: {message}"
            );
        }
        let system = network_failure("net::ERR_PROXY_CONNECTION_FAILED", "a.test", None);
        assert!(system.to_string().starts_with("The system's proxy"));
        let login = NavigationFailure::ProxyLogin {
            proxy: "proxy.corp.example".into(),
        }
        .to_string();
        assert!(login.contains("proxy.corp.example") && login.contains("password"));
        assert!(http_failure(407, "a.test").unwrap().to_string().contains("proxy"));
    }

    #[test]
    fn only_error_statuses_fail_and_they_name_the_status() {
        for ok in [200, 204, 301, 304] {
            assert_eq!(http_failure(ok, "a.test"), None);
        }
        let not_found = http_failure(404, "a.test").unwrap().to_string();
        assert!(not_found.contains("HTTP 404 Not Found") && not_found.contains("does not exist"));
        assert!(
            http_failure(403, "a.test")
                .unwrap()
                .to_string()
                .contains("block")
        );
        assert!(
            http_failure(503, "a.test")
                .unwrap()
                .to_string()
                .contains("temporarily unavailable")
        );
        assert!(
            http_failure(500, "a.test")
                .unwrap()
                .to_string()
                .contains("HTTP 500")
        );
    }

    #[test]
    fn timeout_names_host_and_duration() {
        let message = NavigationFailure::Timeout {
            host: "a.test".into(),
            seconds: 30,
        }
        .to_string();
        assert!(message.contains("a.test") && message.contains("30 seconds"));
    }
}
