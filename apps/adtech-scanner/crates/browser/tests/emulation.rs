//! What a page can observe about the device and the browser of a scan: the
//! device is emulated as asked, and the browser reports itself as it is.

mod support;

use adscan_model::{Capture, CaptureEnvironment, DeviceProfile, VisitorBehavior};
use serde_json::Value;

async fn probe(device: DeviceProfile) -> (Capture, Value) {
    let fixtures = support::Fixtures::start().await;
    let mut request = support::request(&fixtures.url("emulation.html"), device);
    request.visitor = VisitorBehavior::Scroll;
    request.observe_seconds = 3;
    let capture = support::capture_ok(request).await;
    let seen = capture
        .console
        .iter()
        .find_map(|line| line.text.strip_prefix("DEVICE "))
        .expect("the page reports what it sees");
    let seen = serde_json::from_str(seen).unwrap();
    (capture, seen)
}

fn console_has(capture: &Capture, text: &str) -> bool {
    capture.console.iter().any(|line| line.text == text)
}

/// Version of `brand` in a client-hint brand list the page read.
fn brand_version<'a>(list: &'a Value, brand: &str) -> Option<&'a str> {
    list.as_array()?
        .iter()
        .find(|entry| entry["brand"] == brand)?["version"]
        .as_str()
}

/// The Chrome major version in a user agent string.
fn chrome_major(user_agent: &str) -> &str {
    let version = user_agent.split("Chrome/").nth(1).unwrap_or_default();
    version.split('.').next().unwrap_or_default()
}

#[tokio::test]
async fn mobile_scan_is_a_phone_to_every_check_a_wrapper_can_make() {
    let (capture, seen) = probe(DeviceProfile::Mobile).await;

    assert_eq!(seen["userAgent"], capture.user_agent.as_str());
    let user_agent = seen["userAgent"].as_str().unwrap();
    assert!(
        user_agent.contains("Android") && user_agent.contains(" Mobile Safari/"),
        "{user_agent}"
    );
    assert!(
        !user_agent.contains("Headless"),
        "the emulated phone has a phone's user agent: {user_agent}"
    );
    assert_eq!(seen["platform"], "Linux armv81");

    // The client hints tell the same story as the user agent string.
    assert_eq!(seen["hintsMobile"], true);
    assert_eq!(seen["hintsPlatform"], "Android");
    assert_eq!(seen["hintsPlatformVersion"], "14.0.0");
    assert_eq!(seen["hintsModel"], "Pixel 8");
    assert_eq!(
        brand_version(&seen["hintsBrands"], "Google Chrome"),
        Some(chrome_major(user_agent))
    );
    assert_eq!(
        brand_version(&seen["hintsBrands"], "Chromium"),
        Some(chrome_major(user_agent))
    );

    assert_eq!(seen["maxTouchPoints"], 5);
    assert_eq!(seen["hasTouchEvents"], true);
    assert_eq!(seen["innerWidth"], 412);
    assert_eq!(seen["innerHeight"], 915);
    assert_eq!(seen["screenWidth"], 412);
    assert_eq!(seen["devicePixelRatio"], 2.625);
    assert_eq!(seen["phoneWidth"], true);
    assert_eq!(seen["desktopWidth"], false);
    assert_eq!(seen["coarsePointer"], true);
    assert_eq!(seen["finePointer"], false);
    assert_eq!(seen["canHover"], false);
    assert_eq!(seen["anyHover"], false);
    assert_eq!(seen["orientation"], "portrait-primary");
    assert_eq!(seen["portrait"], true);
    assert_eq!(seen["hasFocus"], true);
    assert_eq!(
        seen["webdriver"], false,
        "the scanner's window carries no automation marker either"
    );
    assert_eq!(
        capture.environment,
        CaptureEnvironment {
            browserbase: None,
            headless: true,
            persistent_profile: false,
            webdriver: Some(false),
            user_agent_overridden: true,
            viewport_overridden: true,
            request_hosts: Vec::new(),
        }
    );

    assert_eq!(capture.device, DeviceProfile::Mobile);
    assert!(capture.viewport.mobile);
    assert_eq!(
        (capture.viewport.width, capture.viewport.height),
        (412, 915)
    );
    assert_eq!(capture.viewport.device_scale_factor, 2.625);

    assert!(
        console_has(&capture, "INPUT touchstart"),
        "mobile scrolling is done by touch"
    );
    assert!(!console_has(&capture, "INPUT wheel"));
    assert!(
        capture.max_scroll_depth > 0.3,
        "depth {}",
        capture.max_scroll_depth
    );
}

#[tokio::test]
async fn desktop_scan_is_a_desktop_and_the_browser_it_really_is() {
    let (capture, seen) = probe(DeviceProfile::Desktop).await;

    assert_eq!(seen["userAgent"], capture.user_agent.as_str());
    let user_agent = seen["userAgent"].as_str().unwrap();
    assert!(
        user_agent.contains("HeadlessChrome/"),
        "a headless browser says that it is one: {user_agent}"
    );
    assert!(!user_agent.contains("Mobile"));
    assert_eq!(seen["hintsMobile"], false);
    assert_eq!(
        brand_version(&seen["hintsBrands"], "Chromium"),
        Some(chrome_major(user_agent))
    );
    let full_version = brand_version(&seen["hintsFullVersions"], "Chromium").unwrap();
    assert!(
        full_version.split('.').count() == 4 && !full_version.ends_with(".0.0.0"),
        "client hints carry the real full version, got {full_version}"
    );
    assert_eq!(seen["maxTouchPoints"], 0);
    assert_eq!(seen["hasTouchEvents"], false);
    assert_eq!(seen["innerWidth"], 1440);
    assert_eq!(seen["innerHeight"], 900);
    assert_eq!(seen["devicePixelRatio"], 1.0);
    assert_eq!(seen["phoneWidth"], false);
    assert_eq!(seen["desktopWidth"], true);
    assert_eq!(seen["coarsePointer"], false);
    assert_eq!(
        seen["finePointer"], true,
        "a desktop has a mouse, also when headless"
    );
    assert_eq!(seen["canHover"], true);
    assert_eq!(seen["anyHover"], true);
    assert_eq!(seen["orientation"], "landscape-primary");
    assert_eq!(seen["hasFocus"], true);
    assert_eq!(
        seen["webdriver"], false,
        "the scanner's window carries no automation marker either"
    );
    assert_eq!(
        capture.environment,
        CaptureEnvironment {
            browserbase: None,
            headless: true,
            persistent_profile: false,
            webdriver: Some(false),
            user_agent_overridden: false,
            viewport_overridden: true,
            request_hosts: Vec::new(),
        }
    );

    assert_eq!(capture.device, DeviceProfile::Desktop);
    assert!(!capture.viewport.mobile);
    assert!(
        console_has(&capture, "INPUT wheel"),
        "desktop scrolling is done with the wheel"
    );
    assert!(!console_has(&capture, "INPUT touchstart"));
    assert!(
        capture.max_scroll_depth > 0.3,
        "depth {}",
        capture.max_scroll_depth
    );
}
