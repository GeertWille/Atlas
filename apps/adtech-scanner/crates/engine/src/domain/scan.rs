//! Value objects that guard what a scan may be started with.

use adscan_model::{CaptureRequest, ScanOptions, Settings};
use url::{Host, Url};

use super::DomainError;

const MIN_OBSERVE_SECONDS: u32 = 3;
const MAX_OBSERVE_SECONDS: u32 = 120;

/// An absolute http(s) URL. A bare domain is accepted and upgraded to https.
#[derive(Debug, Clone, PartialEq)]
pub struct ScanUrl {
    url: String,
    hostname: String,
}

impl ScanUrl {
    pub fn parse(input: &str) -> Result<Self, DomainError> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Err(DomainError::InvalidUrl);
        }
        let candidate = if trimmed.contains("://") {
            trimmed.to_string()
        } else {
            format!("https://{trimmed}")
        };
        let url = Url::parse(&candidate).map_err(|_| DomainError::InvalidUrl)?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err(DomainError::UnsupportedScheme);
        }
        let hostname = match url.host() {
            Some(Host::Domain(domain)) if domain.contains('.') || domain == "localhost" => {
                domain.to_string()
            }
            Some(Host::Ipv4(ip)) => ip.to_string(),
            Some(Host::Ipv6(ip)) => ip.to_string(),
            _ => return Err(DomainError::InvalidUrl),
        };
        Ok(Self {
            url: url.into(),
            hostname,
        })
    }

    pub fn as_str(&self) -> &str {
        &self.url
    }

    pub fn hostname(&self) -> &str {
        &self.hostname
    }
}

/// Scan options that passed validation, with the URL normalised.
#[derive(Debug, Clone, PartialEq)]
pub struct ValidatedScan {
    pub url: ScanUrl,
    pub options: ScanOptions,
}

impl ValidatedScan {
    pub fn new(mut options: ScanOptions) -> Result<Self, DomainError> {
        let url = ScanUrl::parse(&options.url)?;
        validate_observe_seconds(options.observe_seconds)?;
        options.url = url.as_str().to_string();
        Ok(Self { url, options })
    }

    pub fn capture_request(&self, settings: &Settings) -> CaptureRequest {
        CaptureRequest {
            url: self.options.url.clone(),
            device: self.options.device,
            observe_seconds: self.options.observe_seconds,
            auto_scroll: self.options.auto_scroll,
            accept_consent: self.options.accept_consent,
            headless: settings.headless,
            chrome_path: settings.chrome_path.clone(),
        }
    }
}

pub fn validate_observe_seconds(seconds: u32) -> Result<(), DomainError> {
    if (MIN_OBSERVE_SECONDS..=MAX_OBSERVE_SECONDS).contains(&seconds) {
        Ok(())
    } else {
        Err(DomainError::ObserveSecondsOutOfRange {
            min: MIN_OBSERVE_SECONDS,
            max: MAX_OBSERVE_SECONDS,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_domain_is_upgraded_to_https() {
        let url = ScanUrl::parse("  example.com/news ").unwrap();
        assert_eq!(url.as_str(), "https://example.com/news");
        assert_eq!(url.hostname(), "example.com");
    }

    #[test]
    fn loopback_and_explicit_http_are_kept() {
        assert_eq!(
            ScanUrl::parse("http://127.0.0.1:4100/").unwrap().as_str(),
            "http://127.0.0.1:4100/"
        );
        assert_eq!(
            ScanUrl::parse("localhost:3000").unwrap().hostname(),
            "localhost"
        );
    }

    #[test]
    fn rejects_other_schemes_and_garbage() {
        assert_eq!(
            ScanUrl::parse("ftp://example.com"),
            Err(DomainError::UnsupportedScheme)
        );
        assert_eq!(
            ScanUrl::parse("file:///etc/passwd"),
            Err(DomainError::UnsupportedScheme)
        );
        for bad in [
            "",
            "   ",
            "javascript:alert(1)",
            "not a url",
            "https://",
            "nodots",
        ] {
            assert_eq!(ScanUrl::parse(bad), Err(DomainError::InvalidUrl), "{bad}");
        }
    }

    #[test]
    fn observe_seconds_are_bounded() {
        assert!(validate_observe_seconds(MIN_OBSERVE_SECONDS).is_ok());
        assert!(validate_observe_seconds(MAX_OBSERVE_SECONDS).is_ok());
        assert!(validate_observe_seconds(0).is_err());
        assert!(validate_observe_seconds(MAX_OBSERVE_SECONDS + 1).is_err());
    }
}
