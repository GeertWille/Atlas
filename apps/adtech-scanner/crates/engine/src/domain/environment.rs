//! What the scanned pages could tell about the visit: the automation
//! indicators the browser reported and the vendors on the page that are in
//! the business of telling automated visits apart.

use adscan_model::{
    AutomationDetector, Capture, DetectorKind, DeviceProfile, ProxyUse, ScanEnvironment,
    VisitorBehavior,
};

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

/// The visit as it was, for the report. `visitor`, `seed` and `proxy` are
/// what the scan asked for; everything else is what the browser reported.
pub fn detect(
    capture: &Capture,
    visitor: VisitorBehavior,
    seed: u32,
    proxy: ProxyUse,
) -> ScanEnvironment {
    let observed = &capture.environment;
    ScanEnvironment {
        visitor,
        seed,
        headless: observed.headless,
        persistent_profile: observed.persistent_profile,
        webdriver: observed.webdriver,
        user_agent_overridden: observed.user_agent_overridden,
        viewport_overridden: observed.viewport_overridden,
        emulated_mobile: capture.device == DeviceProfile::Mobile,
        detectors: detectors(&observed.request_hosts),
        proxy,
    }
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
    use adscan_model::{CaptureEnvironment, ProxyKind};

    use super::*;

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
        };
        let proxy = ProxyUse {
            kind: ProxyKind::Profile,
            name: Some("Belgium".into()),
            host: Some("be.proxy.example".into()),
        };
        let environment = detect(&capture, VisitorBehavior::Browse, 42, proxy.clone());
        assert_eq!(environment.visitor, VisitorBehavior::Browse);
        assert_eq!(environment.seed, 42);
        assert!(environment.headless && environment.emulated_mobile);
        assert_eq!(environment.webdriver, Some(true));
        assert_eq!(environment.detectors.len(), 1);
        assert_eq!(environment.detectors[0].vendor, "Integral Ad Science");
        assert_eq!(environment.proxy, proxy);
    }
}
