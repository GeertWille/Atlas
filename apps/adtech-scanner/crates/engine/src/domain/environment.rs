//! What the scanned pages could tell about the visit: the automation
//! indicators the browser reported and the vendors on the page that are in
//! the business of telling automated visits apart.

use adscan_model::{
    AutomationDetector, BrowserUse, BrowserbaseProxyKind, BrowserbaseSessionSettings,
    BrowserbaseVisit, Capture, CaptureProxy, DetectorKind, DeviceProfile, RemoteUsage,
    ScanEnvironment,
};

use super::scan::{ResolvedBrowser, ValidatedScan};

use DetectorKind::{BotProtection, InvalidTrafficMeasurement};

/// Registrable domains of detection vendors. Each ownership was checked
/// against the DuckDuckGo Tracker Radar entity list (2026-11); vendors whose
/// hosts could not be confirmed there are left out rather than guessed.
const DETECTORS: &[(&str, &str, DetectorKind)] = &[
    (
        "doubleverify.com",
        "DoubleVerify",
        InvalidTrafficMeasurement,
    ),
    (
        "adsafeprotected.com",
        "Integral Ad Science",
        InvalidTrafficMeasurement,
    ),
    ("adrta.com", "Pixalate", InvalidTrafficMeasurement),
    ("pixalate.com", "Pixalate", InvalidTrafficMeasurement),
    ("cheqzone.com", "CHEQ", InvalidTrafficMeasurement),
    ("perimeterx.net", "HUMAN", BotProtection),
    ("px-cdn.net", "HUMAN", BotProtection),
    ("datadome.co", "DataDome", BotProtection),
    ("captcha-delivery.com", "DataDome", BotProtection),
    ("hcaptcha.com", "hCaptcha", BotProtection),
    ("arkoselabs.com", "Arkose Labs", BotProtection),
    ("funcaptcha.com", "Arkose Labs", BotProtection),
    ("recaptcha.net", "reCAPTCHA", BotProtection),
];

/// The visit as it was, for the report. The visitor, the seed, the proxy and
/// where the browser ran are what the scan asked for; everything else is
/// what the browser, and for a Browserbase browser the service, reported.
/// `usage` is what Browserbase said the session used, once it has ended.
pub fn detect(
    capture: &Capture,
    scan: &ValidatedScan,
    usage: Option<&RemoteUsage>,
) -> ScanEnvironment {
    let observed = &capture.environment;
    ScanEnvironment {
        visitor: scan.visitor,
        seed: scan.seed,
        headless: observed.headless,
        persistent_profile: observed.persistent_profile,
        webdriver: observed.webdriver,
        user_agent_overridden: observed.user_agent_overridden,
        viewport_overridden: observed.viewport_overridden,
        emulated_mobile: capture.device == DeviceProfile::Mobile,
        detectors: detectors(&observed.request_hosts),
        proxy: scan.proxy.usage(),
        browser: browser_use(capture, scan, usage),
    }
}

fn browser_use(capture: &Capture, scan: &ValidatedScan, usage: Option<&RemoteUsage>) -> BrowserUse {
    let ResolvedBrowser::Browserbase { region } = scan.browser else {
        return BrowserUse::Local;
    };
    let facts = capture.environment.browserbase.clone().unwrap_or_default();
    let (proxy_kind, proxy_country) = match (&scan.proxy.location, &scan.proxy.capture) {
        (Some(location), _) => (
            BrowserbaseProxyKind::Browserbase,
            Some(location.country.clone()),
        ),
        (None, CaptureProxy::Custom { .. }) => (BrowserbaseProxyKind::External, None),
        (None, CaptureProxy::None | CaptureProxy::System) => (BrowserbaseProxyKind::None, None),
    };
    BrowserUse::Browserbase(Box::new(BrowserbaseVisit {
        region,
        session_id: facts.session_id,
        browser_version: facts.browser_version,
        user_agent: Some(capture.user_agent.clone()).filter(|agent| !agent.is_empty()),
        proxy_kind,
        proxy_country,
        settings: BrowserbaseSessionSettings::STANDARD,
        original_timezone: facts.original_timezone,
        applied_timezone: facts.applied_timezone,
        language: facts.language,
        round_trip_ms: facts.round_trip_ms,
        session_seconds: usage.and_then(|usage| usage.session_seconds),
        proxy_bytes: usage.and_then(|usage| usage.proxy_bytes),
    }))
}

/// One entry per vendor, named by the first of its hosts that was requested.
fn detectors(request_hosts: &[String]) -> Vec<AutomationDetector> {
    let mut found: Vec<AutomationDetector> = Vec::new();
    for host in request_hosts {
        let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
        let vendor = DETECTORS.iter().find(|(domain, _, _)| {
            host == *domain
                || host
                    .strip_suffix(domain)
                    .is_some_and(|prefix| prefix.ends_with('.'))
        });
        if let Some((_, vendor, kind)) = vendor
            && !found.iter().any(|detector| detector.vendor == *vendor)
        {
            found.push(AutomationDetector {
                vendor: (*vendor).to_string(),
                kind: *kind,
                host,
            });
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use adscan_model::{
        BrowserChoice, BrowserbaseRegion, BrowserbaseSessionFacts, CaptureEnvironment, ProxyChoice,
        ProxyKind, ProxyProfile, ProxyScheme, ProxyUse, ScanOptions, Settings, VisitorBehavior,
    };

    use super::*;

    fn belgium() -> ProxyProfile {
        ProxyProfile {
            id: "p1".into(),
            name: "Belgium".into(),
            scheme: ProxyScheme::Http,
            host: "be.proxy.example".into(),
            port: 3128,
            username: None,
            has_password: false,
        }
    }

    fn connected() -> Settings {
        let mut settings = Settings {
            proxies: vec![belgium()],
            ..Settings::default()
        };
        settings.browserbase.connected = true;
        settings
    }

    fn scan(options: ScanOptions) -> ValidatedScan {
        ValidatedScan::new(options, &connected(), 42).unwrap()
    }

    fn on_browserbase(proxy: ProxyChoice) -> ValidatedScan {
        scan(ScanOptions {
            browser: Some(BrowserChoice::Browserbase {
                region: Some(BrowserbaseRegion::EuCentral1),
            }),
            proxy: Some(proxy),
            ..ScanOptions::for_url("pub.example")
        })
    }

    fn hosts(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| name.to_string()).collect()
    }

    #[test]
    fn vendors_are_found_by_registrable_domain_once_each() {
        let found = detectors(&hosts(&[
            "cdn.example.net",
            "PUB.DoubleVerify.com",
            "tps.doubleverify.com",
            "client.perimeterx.net",
            "adrta.com.",
        ]));
        let summary: Vec<(&str, DetectorKind, &str)> = found
            .iter()
            .map(|detector| {
                (
                    detector.vendor.as_str(),
                    detector.kind,
                    detector.host.as_str(),
                )
            })
            .collect();
        assert_eq!(
            summary,
            [
                (
                    "DoubleVerify",
                    InvalidTrafficMeasurement,
                    "pub.doubleverify.com"
                ),
                ("HUMAN", BotProtection, "client.perimeterx.net"),
                ("Pixalate", InvalidTrafficMeasurement, "adrta.com"),
            ]
        );
    }

    #[test]
    fn a_host_that_only_ends_like_a_vendor_domain_is_not_that_vendor() {
        assert!(detectors(&hosts(&["notdoubleverify.com", "datadome.co.evil.example"])).is_empty());
    }

    #[test]
    fn the_environment_reports_what_the_browser_reported() {
        let mut capture = Capture::single_page("https://pub.example/", Vec::new());
        capture.device = DeviceProfile::Mobile;
        capture.environment = CaptureEnvironment {
            headless: true,
            persistent_profile: false,
            webdriver: Some(true),
            user_agent_overridden: true,
            viewport_overridden: true,
            request_hosts: hosts(&["static.adsafeprotected.com"]),
            browserbase: None,
        };
        let browse = scan(ScanOptions {
            visitor: Some(VisitorBehavior::Browse),
            proxy: Some(ProxyChoice::Profile {
                id: "p1".into(),
                name: None,
                host: None,
            }),
            ..ScanOptions::for_url("pub.example")
        });
        let environment = detect(&capture, &browse, None);
        assert_eq!(environment.visitor, VisitorBehavior::Browse);
        assert_eq!(environment.seed, 42);
        assert!(environment.headless && environment.emulated_mobile);
        assert_eq!(environment.webdriver, Some(true));
        assert_eq!(environment.detectors.len(), 1);
        assert_eq!(environment.detectors[0].vendor, "Integral Ad Science");
        assert_eq!(
            environment.proxy,
            ProxyUse {
                kind: ProxyKind::Profile,
                name: Some("Belgium".into()),
                host: Some("be.proxy.example".into()),
            }
        );
        assert_eq!(environment.browser, BrowserUse::Local);
    }

    #[test]
    fn a_browserbase_visit_records_what_was_asked_and_what_the_browser_and_service_reported() {
        let mut capture = Capture::single_page("https://pub.example/", Vec::new());
        capture.user_agent = "Mozilla/5.0 (X11; Linux x86_64) Chrome/157.0.0.0".into();
        capture.environment = CaptureEnvironment {
            headless: true,
            webdriver: Some(false),
            viewport_overridden: true,
            browserbase: Some(BrowserbaseSessionFacts {
                session_id: "session-1".into(),
                browser_version: Some("Chrome/157.0.8103.79".into()),
                original_timezone: Some("America/Los_Angeles".into()),
                applied_timezone: Some("Europe/Berlin".into()),
                language: Some("en-US".into()),
                round_trip_ms: Some(118.6),
                clock_offset_ms: Some(-53.0),
            }),
            ..CaptureEnvironment::default()
        };
        let usage = RemoteUsage {
            region: BrowserbaseRegion::EuCentral1,
            session_seconds: Some(41.0),
            proxy_bytes: Some(0.0),
        };

        let environment = detect(&capture, &on_browserbase(ProxyChoice::None), Some(&usage));

        assert_eq!(environment.webdriver, Some(false), "as the page read it");
        assert!(
            environment.headless,
            "the browser runs headless all the same"
        );
        assert_eq!(
            environment.browser,
            BrowserUse::Browserbase(Box::new(BrowserbaseVisit {
                region: BrowserbaseRegion::EuCentral1,
                session_id: "session-1".into(),
                browser_version: Some("Chrome/157.0.8103.79".into()),
                user_agent: Some("Mozilla/5.0 (X11; Linux x86_64) Chrome/157.0.0.0".into()),
                proxy_kind: BrowserbaseProxyKind::None,
                proxy_country: None,
                settings: BrowserbaseSessionSettings {
                    solve_captchas: true,
                    record_session: true,
                    log_session: true,
                    block_ads: false,
                    keep_alive: false,
                },
                original_timezone: Some("America/Los_Angeles".into()),
                applied_timezone: Some("Europe/Berlin".into()),
                language: Some("en-US".into()),
                round_trip_ms: Some(118.6),
                session_seconds: Some(41.0),
                proxy_bytes: Some(0.0),
            }))
        );
    }

    #[test]
    fn the_exit_of_a_browserbase_visit_is_named_by_its_kind() {
        let capture = Capture::single_page("https://pub.example/", Vec::new());
        let kind_of =
            |proxy: ProxyChoice| match detect(&capture, &on_browserbase(proxy), None).browser {
                BrowserUse::Browserbase(visit) => (visit.proxy_kind, visit.proxy_country),
                BrowserUse::Local => panic!("the scan ran on Browserbase"),
            };
        assert_eq!(
            kind_of(ProxyChoice::Profile {
                id: "p1".into(),
                name: None,
                host: None,
            }),
            (BrowserbaseProxyKind::External, None)
        );
        assert_eq!(
            kind_of(ProxyChoice::BrowserbaseLocation {
                country: "be".into(),
                state: None,
                city: None,
            }),
            (BrowserbaseProxyKind::Browserbase, Some("BE".into()))
        );
        // The system's proxy means nothing to a cloud browser.
        assert_eq!(
            kind_of(ProxyChoice::System),
            (BrowserbaseProxyKind::None, None)
        );
    }
}
