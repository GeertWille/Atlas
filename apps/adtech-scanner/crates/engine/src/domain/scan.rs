//! Value objects that guard what a scan may be started with.

use adscan_model::{
    BrowserChoice, BrowserKind, BrowserbaseCapture, BrowserbaseRegion, BrowserbaseSessionSettings,
    CaptureBrowser, CaptureRequest, ScanOptions, Settings, VisitorBehavior,
};
use url::{Host, Url};

use super::DomainError;
use super::pricing::Budget;
use super::proxy::ResolvedProxy;
use super::reach::is_local_only;

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

    /// Whether a browser outside this machine's network can load the page.
    /// The built-in demo site, on loopback, is one that cannot.
    pub fn reachable_from_cloud(&self) -> bool {
        Url::parse(&self.url)
            .ok()
            .and_then(|url| url.host().map(|host| !is_local_only(&host)))
            .unwrap_or(false)
    }
}

/// Where a scan's browser runs, settled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolvedBrowser {
    Local,
    Browserbase { region: BrowserbaseRegion },
}

impl ResolvedBrowser {
    /// An explicit choice of Browserbase must be possible: credentials are
    /// stored and the address is one a cloud browser can reach. A choice
    /// left open follows the settings, except that an address only this
    /// machine can reach is scanned here.
    fn resolve(
        choice: Option<&BrowserChoice>,
        settings: &Settings,
        url: &ScanUrl,
    ) -> Result<Self, DomainError> {
        let cloud = &settings.browserbase;
        match choice {
            Some(BrowserChoice::Local) => Ok(Self::Local),
            Some(BrowserChoice::Browserbase { region }) => {
                if !cloud.connected {
                    return Err(DomainError::BrowserbaseNotSetUp);
                }
                if !url.reachable_from_cloud() {
                    return Err(DomainError::NotReachableFromCloud);
                }
                Ok(Self::Browserbase {
                    region: region.unwrap_or(cloud.region),
                })
            }
            None => {
                let by_default = settings.default_browser == BrowserKind::Browserbase
                    && cloud.connected
                    && url.reachable_from_cloud();
                Ok(if by_default {
                    Self::Browserbase {
                        region: cloud.region,
                    }
                } else {
                    Self::Local
                })
            }
        }
    }

    fn choice(self) -> BrowserChoice {
        match self {
            Self::Local => BrowserChoice::Local,
            Self::Browserbase { region } => BrowserChoice::Browserbase {
                region: Some(region),
            },
        }
    }
}

/// The ids a Browserbase browser is started with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserbaseSessionIds {
    pub install_id: String,
    pub scan_id: String,
    /// The context of the kept browser profile, for a scan that keeps one.
    pub context_id: Option<String>,
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
    pub browser: ResolvedBrowser,
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
        let browser = ResolvedBrowser::resolve(options.browser.as_ref(), settings, &url)?;
        let on_browserbase = matches!(browser, ResolvedBrowser::Browserbase { .. });
        // A person cannot drive a browser without a window, and their consent
        // choice is theirs to make. A browse visit is shown unless asked not
        // to: headless Chrome is easier to recognise as automated. A
        // Browserbase browser is always headless, whatever was asked; a
        // person drives it through Browserbase's live view.
        let headless = match visitor {
            _ if on_browserbase => true,
            VisitorBehavior::Manual => false,
            VisitorBehavior::Browse => options.headless.unwrap_or(false),
            VisitorBehavior::None | VisitorBehavior::Scroll => {
                options.headless.unwrap_or(settings.headless)
            }
        };
        let accept_consent = options.accept_consent && visitor != VisitorBehavior::Manual;
        // A profile kept at Browserbase is cookies stored with a third
        // party: only with its own switch on, whatever the scan or the
        // local setting say.
        let persistent_profile = if on_browserbase {
            settings.browserbase.persistent_profile && options.persistent_profile.unwrap_or(true)
        } else {
            options
                .persistent_profile
                .unwrap_or(settings.persistent_profile)
        };
        let budget_usd =
            Budget::new(options.budget_usd.unwrap_or(settings.budget_usd))?.limit_usd();
        let proxy = if on_browserbase {
            ResolvedProxy::resolve_for_browserbase(
                options
                    .proxy
                    .as_ref()
                    .unwrap_or(&settings.browserbase.default_proxy),
                &settings.proxies,
            )?
        } else {
            ResolvedProxy::resolve(
                options.proxy.as_ref().unwrap_or(&settings.default_proxy),
                &settings.proxies,
            )?
        };
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
        options.browser = Some(browser.choice());
        Ok(Self {
            url,
            options,
            visitor,
            seed,
            headless,
            persistent_profile,
            budget_usd,
            proxy,
            browser,
            extra_pages,
        })
    }

    /// `session` names the Browserbase session to start; a local scan needs
    /// none and ignores it.
    pub fn capture_request(
        &self,
        settings: &Settings,
        session: Option<BrowserbaseSessionIds>,
    ) -> CaptureRequest {
        let browser = match (self.browser, session) {
            (ResolvedBrowser::Browserbase { region }, Some(session)) => {
                CaptureBrowser::Browserbase(BrowserbaseCapture {
                    region,
                    settings: BrowserbaseSessionSettings::STANDARD,
                    location: self.proxy.location.clone(),
                    // Only a scan that keeps a profile gets the context.
                    context_id: session.context_id.filter(|_| self.persistent_profile),
                    install_id: session.install_id,
                    scan_id: session.scan_id,
                })
            }
            _ => CaptureBrowser::Local,
        };
        CaptureRequest {
            browser,
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
    use adscan_model::{BrowserbaseLocation, CaptureProxy, ProxyChoice, ProxyProfile, ProxyScheme};

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
                browser: Some(BrowserChoice::Local),
                ..ScanOptions::for_url("")
            }
        );
        let request = scan.capture_request(&settings, None);
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
        assert_eq!(scan.capture_request(&settings, None).extra_pages, 2);
        assert!(!scan.headless, "browse is headed unless asked otherwise");
        assert!(scan.options.auto_scroll);

        let headless = ScanOptions {
            headless: Some(true),
            extra_pages: Some(5),
            ..browse.clone()
        };
        let scan = resolved(headless, &settings);
        assert!(scan.headless);
        assert_eq!(scan.capture_request(&settings, None).extra_pages, 5);

        let scroll = ScanOptions {
            extra_pages: Some(3),
            ..ScanOptions::for_url("pub.example")
        };
        let scan = resolved(scroll, &settings);
        assert_eq!(scan.options.extra_pages, Some(0));
        assert_eq!(scan.capture_request(&settings, None).extra_pages, 0);

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
        let request = scan.capture_request(&settings, None);
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
            scan.capture_request(&settings, None).proxy,
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

    // -- Browserbase: browser x proxy x URL ----------------------------------

    fn profile(id: &str, name: &str, host: &str) -> ProxyProfile {
        ProxyProfile {
            id: id.into(),
            name: name.into(),
            scheme: ProxyScheme::Http,
            host: host.into(),
            port: 3128,
            username: Some("scanner".into()),
            has_password: true,
        }
    }

    fn named(id: &str) -> ProxyChoice {
        ProxyChoice::Profile {
            id: id.into(),
            name: None,
            host: None,
        }
    }

    fn location(country: &str) -> ProxyChoice {
        ProxyChoice::BrowserbaseLocation {
            country: country.into(),
            state: None,
            city: None,
        }
    }

    /// Credentials stored, one proxy the internet can reach and one it
    /// cannot.
    fn connected() -> Settings {
        let mut settings = Settings {
            proxies: vec![
                profile("public", "Belgium", "be.proxy.example"),
                profile("office", "Office", "10.0.0.7"),
            ],
            ..Settings::default()
        };
        settings.browserbase.connected = true;
        settings
    }

    fn on_browserbase(url: &str) -> ScanOptions {
        ScanOptions {
            browser: Some(BrowserChoice::Browserbase { region: None }),
            ..ScanOptions::for_url(url)
        }
    }

    const FRANKFURT: ResolvedBrowser = ResolvedBrowser::Browserbase {
        region: BrowserbaseRegion::EuCentral1,
    };

    #[test]
    fn a_scan_runs_locally_unless_browserbase_is_chosen_or_the_default() {
        let settings = connected();
        let local = resolved(ScanOptions::for_url("pub.example"), &settings);
        assert_eq!(local.browser, ResolvedBrowser::Local);
        assert_eq!(local.options.browser, Some(BrowserChoice::Local));

        let mut by_default = connected();
        by_default.default_browser = BrowserKind::Browserbase;
        by_default.browserbase.region = BrowserbaseRegion::EuCentral1;
        let cloud = resolved(ScanOptions::for_url("pub.example"), &by_default);
        assert_eq!(cloud.browser, FRANKFURT);
        assert_eq!(
            cloud.options.browser,
            Some(BrowserChoice::Browserbase {
                region: Some(BrowserbaseRegion::EuCentral1)
            }),
            "stored with the region it ran in"
        );

        // An explicit choice wins over the default, either way.
        let explicit_local = ScanOptions {
            browser: Some(BrowserChoice::Local),
            ..ScanOptions::for_url("pub.example")
        };
        assert_eq!(
            resolved(explicit_local, &by_default).browser,
            ResolvedBrowser::Local
        );
        let explicit_region = ScanOptions {
            browser: Some(BrowserChoice::Browserbase {
                region: Some(BrowserbaseRegion::ApSoutheast1),
            }),
            ..ScanOptions::for_url("pub.example")
        };
        assert_eq!(
            resolved(explicit_region, &settings).browser,
            ResolvedBrowser::Browserbase {
                region: BrowserbaseRegion::ApSoutheast1
            }
        );
    }

    #[test]
    fn browserbase_needs_stored_credentials() {
        let not_connected = Settings::default();
        assert_eq!(
            ValidatedScan::new(on_browserbase("pub.example"), &not_connected, 7),
            Err(DomainError::BrowserbaseNotSetUp)
        );
        // A default that lost its credentials falls back instead of failing.
        let stale_default = Settings {
            default_browser: BrowserKind::Browserbase,
            ..Settings::default()
        };
        assert_eq!(
            resolved(ScanOptions::for_url("pub.example"), &stale_default).browser,
            ResolvedBrowser::Local
        );
    }

    #[test]
    fn addresses_only_this_machine_or_its_network_can_reach_never_run_on_browserbase() {
        let mut by_default = connected();
        by_default.default_browser = BrowserKind::Browserbase;
        for url in [
            // The built-in demo site is served like this.
            "http://127.0.0.1:49152/",
            "http://localhost:3000/",
            "https://app.localhost/",
            "http://192.168.1.20/",
            "http://10.0.0.5:8080/news",
            "https://printer.local/",
            "https://build.internal/",
            "http://[::1]:4100/",
        ] {
            assert_eq!(
                ValidatedScan::new(on_browserbase(url), &by_default, 7),
                Err(DomainError::NotReachableFromCloud),
                "{url}"
            );
            // Left open, the scan runs here instead of failing.
            assert_eq!(
                resolved(ScanOptions::for_url(url), &by_default).browser,
                ResolvedBrowser::Local,
                "{url}"
            );
        }
        assert_eq!(
            resolved(on_browserbase("https://www.pub.example/news"), &by_default).browser,
            ResolvedBrowser::Browserbase {
                region: BrowserbaseRegion::UsWest2
            }
        );
    }

    #[test]
    fn the_exit_of_a_browserbase_scan_is_its_region_a_named_proxy_or_a_location() {
        let settings = connected();
        let with = |proxy: ProxyChoice| ScanOptions {
            proxy: Some(proxy),
            ..on_browserbase("pub.example")
        };

        let region = resolved(with(ProxyChoice::None), &settings);
        assert_eq!(region.proxy.capture, CaptureProxy::None);
        assert_eq!(region.proxy.location, None);
        assert!(!region.proxy.system_proxy_dropped);

        // This machine's proxy means nothing to a cloud browser.
        let system = resolved(with(ProxyChoice::System), &settings);
        assert_eq!(system.options.proxy, Some(ProxyChoice::None));
        assert_eq!(system.proxy.capture, CaptureProxy::None);
        assert!(system.proxy.system_proxy_dropped);

        let own = resolved(with(named("public")), &settings);
        assert_eq!(
            own.proxy.capture,
            CaptureProxy::Custom {
                scheme: ProxyScheme::Http,
                host: "be.proxy.example".into(),
                port: 3128,
                username: Some("scanner".into()),
            }
        );
        assert_eq!(own.proxy.profile_id.as_deref(), Some("public"));

        let residential = resolved(
            with(ProxyChoice::BrowserbaseLocation {
                country: "us".into(),
                state: Some(" ny ".into()),
                city: Some("New York".into()),
            }),
            &settings,
        );
        assert_eq!(residential.proxy.capture, CaptureProxy::None);
        assert_eq!(
            residential.proxy.location,
            Some(BrowserbaseLocation {
                country: "US".into(),
                state: Some("NY".into()),
                city: Some("NEW_YORK".into()),
            })
        );
        assert_eq!(
            residential.options.proxy,
            Some(ProxyChoice::BrowserbaseLocation {
                country: "US".into(),
                state: Some("NY".into()),
                city: Some("NEW_YORK".into()),
            })
        );

        assert_eq!(
            ValidatedScan::new(with(named("office")), &settings, 7),
            Err(DomainError::ProxyNotReachableFromCloud("Office".into()))
        );
        assert_eq!(
            ValidatedScan::new(with(named("gone")), &settings, 7),
            Err(DomainError::UnknownProxy)
        );
        assert_eq!(
            ValidatedScan::new(with(location("Belgium")), &settings, 7),
            Err(DomainError::LocationCountryInvalid)
        );
    }

    #[test]
    fn a_browserbase_scan_left_open_follows_the_browserbase_default_not_the_local_one() {
        let mut settings = connected();
        // The local default is the office proxy, which a cloud browser
        // could never use.
        settings.default_proxy = named("office");
        settings.browserbase.default_proxy = location("BE");
        let scan = resolved(on_browserbase("pub.example"), &settings);
        assert_eq!(
            scan.proxy.location.map(|location| location.country),
            Some("BE".to_string())
        );
        // And the local scan keeps the local default.
        assert_eq!(
            resolved(ScanOptions::for_url("pub.example"), &settings)
                .proxy
                .profile_id
                .as_deref(),
            Some("office")
        );
    }

    #[test]
    fn a_browserbase_location_is_refused_for_a_local_browser() {
        let local = ScanOptions {
            proxy: Some(location("BE")),
            ..ScanOptions::for_url("pub.example")
        };
        assert_eq!(
            ValidatedScan::new(local, &connected(), 7),
            Err(DomainError::LocationNeedsBrowserbase)
        );
    }

    #[test]
    fn ads_txt_of_a_browserbase_scan_goes_through_the_named_proxy_or_from_this_machine() {
        let settings = connected();
        let with = |proxy: ProxyChoice| {
            resolved(
                ScanOptions {
                    proxy: Some(proxy),
                    ..on_browserbase("pub.example")
                },
                &settings,
            )
            .proxy
        };
        let own = with(named("public"));
        assert_eq!(own.fetch, own.capture);
        assert_eq!(with(ProxyChoice::None).fetch, CaptureProxy::System);
        assert_eq!(with(location("BE")).fetch, CaptureProxy::System);
        // A local scan reads it the way its browser reaches the site.
        let local = resolved(
            ScanOptions {
                proxy: Some(ProxyChoice::None),
                ..ScanOptions::for_url("pub.example")
            },
            &settings,
        );
        assert_eq!(local.proxy.fetch, CaptureProxy::None);
    }

    #[test]
    fn a_browserbase_browser_is_headless_whatever_was_asked_and_manual_is_allowed() {
        let settings = Settings {
            headless: false,
            ..connected()
        };
        let shown = ScanOptions {
            headless: Some(false),
            ..on_browserbase("pub.example")
        };
        let scan = resolved(shown, &settings);
        assert!(scan.headless);
        assert_eq!(scan.options.headless, Some(true));

        let manual = ScanOptions {
            visitor: Some(VisitorBehavior::Manual),
            accept_consent: true,
            ..on_browserbase("pub.example")
        };
        let scan = resolved(manual, &settings);
        assert_eq!(scan.visitor, VisitorBehavior::Manual);
        assert_eq!(
            scan.browser,
            ResolvedBrowser::Browserbase {
                region: BrowserbaseRegion::UsWest2
            }
        );
        assert!(scan.headless, "a person drives it through the live view");
        assert!(!scan.options.accept_consent);
    }

    #[test]
    fn a_profile_is_kept_at_browserbase_only_with_its_own_switch() {
        let asked = |persistent_profile: Option<bool>| ScanOptions {
            persistent_profile,
            ..on_browserbase("pub.example")
        };
        // The local setting plays no part.
        let local_on = Settings {
            persistent_profile: true,
            ..connected()
        };
        assert!(!resolved(asked(None), &local_on).persistent_profile);
        let stored = resolved(asked(Some(true)), &local_on);
        assert!(!stored.persistent_profile, "not without the switch");
        assert_eq!(stored.options.persistent_profile, Some(false));

        let mut switched_on = connected();
        switched_on.browserbase.persistent_profile = true;
        assert!(resolved(asked(None), &switched_on).persistent_profile);
        assert!(resolved(asked(Some(true)), &switched_on).persistent_profile);
        assert!(!resolved(asked(Some(false)), &switched_on).persistent_profile);
        // And the switch says nothing about local scans.
        assert!(!resolved(ScanOptions::for_url("pub.example"), &switched_on).persistent_profile);
    }

    #[test]
    fn the_capture_request_of_a_browserbase_scan_names_the_session_to_start() {
        let mut settings = connected();
        settings.browserbase.persistent_profile = true;
        let ids = |context_id: Option<&str>| BrowserbaseSessionIds {
            install_id: "0123456789abcdef0123456789abcdef".into(),
            scan_id: "scan-1".into(),
            context_id: context_id.map(str::to_string),
        };
        let scan = resolved(
            ScanOptions {
                proxy: Some(location("BE")),
                browser: Some(BrowserChoice::Browserbase {
                    region: Some(BrowserbaseRegion::EuCentral1),
                }),
                ..ScanOptions::for_url("pub.example")
            },
            &settings,
        );
        let request = scan.capture_request(&settings, Some(ids(Some("context-1"))));
        assert_eq!(
            request.browser,
            CaptureBrowser::Browserbase(BrowserbaseCapture {
                region: BrowserbaseRegion::EuCentral1,
                settings: BrowserbaseSessionSettings {
                    solve_captchas: true,
                    record_session: true,
                    log_session: true,
                    block_ads: false,
                    keep_alive: false,
                },
                location: Some(BrowserbaseLocation {
                    country: "BE".into(),
                    state: None,
                    city: None,
                }),
                context_id: Some("context-1".into()),
                install_id: "0123456789abcdef0123456789abcdef".into(),
                scan_id: "scan-1".into(),
            })
        );
        assert_eq!(request.proxy, CaptureProxy::None);
        assert!(request.persistent_profile && request.headless);

        // A scan that keeps no profile never carries a context.
        let throwaway = resolved(
            ScanOptions {
                persistent_profile: Some(false),
                ..on_browserbase("pub.example")
            },
            &settings,
        );
        let request = throwaway.capture_request(&settings, Some(ids(Some("context-1"))));
        let CaptureBrowser::Browserbase(cloud) = request.browser else {
            panic!("the scan runs on Browserbase");
        };
        assert_eq!(cloud.context_id, None);
        assert!(!request.persistent_profile);

        // A local scan ignores session ids altogether.
        let local = resolved(ScanOptions::for_url("pub.example"), &settings);
        assert_eq!(
            local.capture_request(&settings, Some(ids(None))).browser,
            CaptureBrowser::Local
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
