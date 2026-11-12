//! Value objects that guard what a scan may be started with.

use adscan_model::{CaptureRequest, ScanOptions, Settings, VisitorBehavior};
use url::{Host, Url};

use super::DomainError;
use super::pricing::Budget;
use super::proxy::ResolvedProxy;

const MIN_OBSERVE_SECONDS: u32 = 3;
const MAX_OBSERVE_SECONDS: u32 = 120;
const MAX_EXTRA_PAGES: u32 = 5;

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

/// Scan options that passed validation, with the URL normalised and every
/// choice the options left open settled against the settings.
#[derive(Debug, Clone, PartialEq)]
pub struct ValidatedScan {
    pub url: ScanUrl,
    /// What is stored with the scan: every optional field is filled in.
    pub options: ScanOptions,
    pub visitor: VisitorBehavior,
    pub seed: u32,
    pub headless: bool,
    pub persistent_profile: bool,
    /// Cap on LLM spend in USD, within the bounds of [`Budget`].
    pub budget_usd: f64,
    pub proxy: ResolvedProxy,
    extra_pages: u32,
}

impl ValidatedScan {
    /// `fallback_seed` is used when the options name none, so that a scan
    /// repeated from its stored options plans the same visit.
    pub fn new(
        mut options: ScanOptions,
        settings: &Settings,
        fallback_seed: u32,
    ) -> Result<Self, DomainError> {
        let url = ScanUrl::parse(&options.url)?;
        validate_observe_seconds(options.observe_seconds)?;
        let visitor = options.visitor.unwrap_or(if options.auto_scroll {
            VisitorBehavior::Scroll
        } else {
            VisitorBehavior::None
        });
        let requested_pages = options.extra_pages.unwrap_or(settings.default_extra_pages);
        validate_extra_pages(requested_pages)?;
        // Only a browse visit follows links.
        let extra_pages = if visitor == VisitorBehavior::Browse {
            requested_pages
        } else {
            0
        };
        // A person cannot drive a browser without a window, and their consent
        // choice is theirs to make. A browse visit is shown unless asked not
        // to: headless Chrome is easier to recognise as automated.
        let headless = match visitor {
            VisitorBehavior::Manual => false,
            VisitorBehavior::Browse => options.headless.unwrap_or(false),
            VisitorBehavior::None | VisitorBehavior::Scroll => {
                options.headless.unwrap_or(settings.headless)
            }
        };
        let accept_consent = options.accept_consent && visitor != VisitorBehavior::Manual;
        let persistent_profile = options
            .persistent_profile
            .unwrap_or(settings.persistent_profile);
        let budget_usd =
            Budget::new(options.budget_usd.unwrap_or(settings.budget_usd))?.limit_usd();
        let proxy = ResolvedProxy::resolve(
            options.proxy.as_ref().unwrap_or(&settings.default_proxy),
            &settings.proxies,
        )?;
        let seed = options.seed.unwrap_or(fallback_seed);

        options.url = url.as_str().to_string();
        options.auto_scroll = visitor != VisitorBehavior::None;
        options.accept_consent = accept_consent;
        options.visitor = Some(visitor);
        options.extra_pages = Some(extra_pages);
        options.seed = Some(seed);
        options.headless = Some(headless);
        options.persistent_profile = Some(persistent_profile);
        options.proxy = Some(proxy.choice.clone());
        options.budget_usd = Some(budget_usd);
        Ok(Self {
            url,
            options,
            visitor,
            seed,
            headless,
            persistent_profile,
            budget_usd,
            proxy,
            extra_pages,
        })
    }

    pub fn capture_request(&self, settings: &Settings) -> CaptureRequest {
        CaptureRequest {
            url: self.options.url.clone(),
            device: self.options.device,
            observe_seconds: self.options.observe_seconds,
            visitor: self.visitor,
            extra_pages: self.extra_pages,
            seed: self.seed,
            accept_consent: self.options.accept_consent,
            headless: self.headless,
            persistent_profile: self.persistent_profile,
            proxy: self.proxy.capture.clone(),
            chrome_path: settings.chrome_path.clone(),
        }
    }
}

/// A seed for the scripted visit of a scan that was given none: the same
/// for the same scan id, different between scans.
pub fn seed_from_id(scan_id: &str) -> u32 {
    // FNV-1a.
    scan_id.bytes().fold(0x811c_9dc5_u32, |hash, byte| {
        (hash ^ u32::from(byte)).wrapping_mul(0x0100_0193)
    })
}

pub fn validate_extra_pages(pages: u32) -> Result<(), DomainError> {
    if pages <= MAX_EXTRA_PAGES {
        Ok(())
    } else {
        Err(DomainError::ExtraPagesOutOfRange {
            max: MAX_EXTRA_PAGES,
        })
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
    use adscan_model::{CaptureProxy, ProxyChoice, ProxyProfile, ProxyScheme};

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

    fn resolved(options: ScanOptions, settings: &Settings) -> ValidatedScan {
        ValidatedScan::new(options, settings, 7).unwrap()
    }

    #[test]
    fn options_from_before_visitors_resolve_from_auto_scroll() {
        let settings = Settings::default();
        let mut options = ScanOptions::for_url("pub.example");
        let scroll = resolved(options.clone(), &settings);
        assert_eq!(scroll.visitor, VisitorBehavior::Scroll);
        assert_eq!(scroll.options.visitor, Some(VisitorBehavior::Scroll));
        assert_eq!(scroll.options.url, "https://pub.example/");

        options.auto_scroll = false;
        let none = resolved(options, &settings);
        assert_eq!(none.visitor, VisitorBehavior::None);
        assert!(!none.options.auto_scroll);
    }

    #[test]
    fn every_open_choice_is_settled_and_stored() {
        let settings = Settings {
            headless: true,
            persistent_profile: true,
            budget_usd: 0.25,
            ..Settings::default()
        };
        let scan = resolved(ScanOptions::for_url("pub.example"), &settings);
        assert_eq!(
            scan.options,
            ScanOptions {
                url: "https://pub.example/".into(),
                visitor: Some(VisitorBehavior::Scroll),
                extra_pages: Some(0),
                seed: Some(7),
                headless: Some(true),
                persistent_profile: Some(true),
                proxy: Some(ProxyChoice::System),
                budget_usd: Some(0.25),
                ..ScanOptions::for_url("")
            }
        );
        let request = scan.capture_request(&settings);
        assert_eq!(request.seed, 7);
        assert_eq!(request.proxy, CaptureProxy::System);
        assert!(request.persistent_profile && request.headless);
    }

    #[test]
    fn a_seed_and_explicit_choices_are_kept() {
        let options = ScanOptions {
            seed: Some(99),
            headless: Some(false),
            persistent_profile: Some(false),
            budget_usd: Some(1.5),
            proxy: Some(ProxyChoice::None),
            ..ScanOptions::for_url("pub.example")
        };
        let settings = Settings {
            persistent_profile: true,
            ..Settings::default()
        };
        let scan = resolved(options, &settings);
        assert_eq!(scan.seed, 99);
        assert!(!scan.headless && !scan.persistent_profile);
        assert_eq!(scan.budget_usd, 1.5);
        assert_eq!(scan.proxy.capture, CaptureProxy::None);
    }

    #[test]
    fn only_a_browse_visit_follows_links_and_it_is_shown_by_default() {
        let settings = Settings::default();
        let browse = ScanOptions {
            visitor: Some(VisitorBehavior::Browse),
            ..ScanOptions::for_url("pub.example")
        };
        let scan = resolved(browse.clone(), &settings);
        assert_eq!(scan.capture_request(&settings).extra_pages, 2);
        assert!(!scan.headless, "browse is headed unless asked otherwise");
        assert!(scan.options.auto_scroll);

        let headless = ScanOptions {
            headless: Some(true),
            extra_pages: Some(5),
            ..browse.clone()
        };
        let scan = resolved(headless, &settings);
        assert!(scan.headless);
        assert_eq!(scan.capture_request(&settings).extra_pages, 5);

        let scroll = ScanOptions {
            extra_pages: Some(3),
            ..ScanOptions::for_url("pub.example")
        };
        let scan = resolved(scroll, &settings);
        assert_eq!(scan.options.extra_pages, Some(0));
        assert_eq!(scan.capture_request(&settings).extra_pages, 0);

        let too_many = ScanOptions {
            extra_pages: Some(6),
            ..browse
        };
        assert_eq!(
            ValidatedScan::new(too_many, &settings, 7),
            Err(DomainError::ExtraPagesOutOfRange { max: 5 })
        );
    }

    #[test]
    fn a_manual_visit_is_headed_and_leaves_consent_to_the_person() {
        let settings = Settings::default();
        let manual = ScanOptions {
            visitor: Some(VisitorBehavior::Manual),
            headless: Some(true),
            accept_consent: true,
            extra_pages: Some(4),
            ..ScanOptions::for_url("pub.example")
        };
        let scan = resolved(manual, &settings);
        let request = scan.capture_request(&settings);
        assert!(!request.headless);
        assert!(!request.accept_consent);
        assert_eq!(request.extra_pages, 0);
        assert_eq!(scan.options.headless, Some(false));
        assert!(!scan.options.accept_consent);
    }

    #[test]
    fn the_proxy_comes_from_the_options_or_the_settings_and_must_exist() {
        let profile = ProxyProfile {
            id: "p1".into(),
            name: "Belgium".into(),
            scheme: ProxyScheme::Http,
            host: "be.proxy.example".into(),
            port: 3128,
            username: Some("scanner".into()),
            has_password: true,
        };
        let choice = ProxyChoice::Profile {
            id: "p1".into(),
            name: None,
            host: None,
        };
        let settings = Settings {
            proxies: vec![profile],
            default_proxy: choice.clone(),
            ..Settings::default()
        };
        let scan = resolved(ScanOptions::for_url("pub.example"), &settings);
        assert_eq!(
            scan.options.proxy,
            Some(ProxyChoice::Profile {
                id: "p1".into(),
                name: Some("Belgium".into()),
                host: Some("be.proxy.example".into()),
            })
        );
        assert_eq!(
            scan.capture_request(&settings).proxy,
            CaptureProxy::Custom {
                scheme: ProxyScheme::Http,
                host: "be.proxy.example".into(),
                port: 3128,
                username: Some("scanner".into()),
            }
        );

        let direct = ScanOptions {
            proxy: Some(ProxyChoice::None),
            ..ScanOptions::for_url("pub.example")
        };
        assert_eq!(
            resolved(direct, &settings).proxy.capture,
            CaptureProxy::None
        );

        let gone = ScanOptions {
            proxy: Some(choice),
            ..ScanOptions::for_url("pub.example")
        };
        assert_eq!(
            ValidatedScan::new(gone, &Settings::default(), 7),
            Err(DomainError::UnknownProxy)
        );
    }

    #[test]
    fn the_budget_of_a_scan_is_bounded_like_the_settings_one() {
        let over = ScanOptions {
            budget_usd: Some(1000.0),
            ..ScanOptions::for_url("pub.example")
        };
        assert!(matches!(
            ValidatedScan::new(over, &Settings::default(), 7),
            Err(DomainError::BudgetOutOfRange { .. })
        ));
    }

    #[test]
    fn the_seed_of_a_scan_id_is_stable_and_differs_between_ids() {
        assert_eq!(seed_from_id("scan-a"), seed_from_id("scan-a"));
        assert_ne!(seed_from_id("scan-a"), seed_from_id("scan-b"));
    }

    #[test]
    fn observe_seconds_are_bounded() {
        assert!(validate_observe_seconds(MIN_OBSERVE_SECONDS).is_ok());
        assert!(validate_observe_seconds(MAX_OBSERVE_SECONDS).is_ok());
        assert!(validate_observe_seconds(0).is_err());
        assert!(validate_observe_seconds(MAX_OBSERVE_SECONDS + 1).is_err());
    }
}
