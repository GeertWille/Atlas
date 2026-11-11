//! What a page can observe about the emulated device.

mod support;

use adscan_model::{Capture, DeviceProfile};
use serde_json::Value;

async fn probe(device: DeviceProfile) -> (Capture, Value) {
    let fixtures = support::Fixtures::start().await;
    let mut request = support::request(&fixtures.url("emulation.html"), device);
    request.auto_scroll = true;
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

#[tokio::test]
async fn mobile_scan_is_a_phone_to_every_check_a_wrapper_can_make() {
    let (capture, seen) = probe(DeviceProfile::Mobile).await;

    assert_eq!(seen["userAgent"], capture.user_agent.as_str());
    let user_agent = seen["userAgent"].as_str().unwrap();
    assert!(
        user_agent.contains("iPhone") && user_agent.contains("Mobile/"),
        "{user_agent}"
    );
    assert_eq!(seen["platform"], "iPhone");
    assert_eq!(seen["hintsMobile"], true);
    assert_eq!(seen["hintsPlatform"], "iOS");
    assert_eq!(seen["hintsModel"], "iPhone");
    assert_eq!(seen["maxTouchPoints"], 5);
    assert_eq!(seen["hasTouchEvents"], true);
    assert_eq!(seen["innerWidth"], 393);
    assert_eq!(seen["innerHeight"], 852);
    assert_eq!(seen["screenWidth"], 393);
    assert_eq!(seen["devicePixelRatio"], 3.0);
    assert_eq!(seen["phoneWidth"], true);
    assert_eq!(seen["desktopWidth"], false);
    assert_eq!(seen["coarsePointer"], true);
    assert_eq!(seen["webdriver"], false);

    assert_eq!(capture.device, DeviceProfile::Mobile);
    assert!(capture.viewport.mobile);
    assert_eq!(
        (capture.viewport.width, capture.viewport.height),
        (393, 852)
    );
    assert_eq!(capture.viewport.device_scale_factor, 3.0);

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
async fn desktop_scan_is_an_ordinary_desktop_chrome() {
    let (capture, seen) = probe(DeviceProfile::Desktop).await;

    assert_eq!(seen["userAgent"], capture.user_agent.as_str());
    let user_agent = seen["userAgent"].as_str().unwrap();
    assert!(
        user_agent.contains("Chrome/") && !user_agent.contains("Headless"),
        "{user_agent}"
    );
    assert!(!user_agent.contains("Mobile"));
    assert_eq!(seen["hintsMobile"], false);
    let brands: Vec<&str> = seen["hintsBrands"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert!(
        brands.contains(&"Chromium") && !brands.iter().any(|brand| brand.contains("Headless")),
        "{brands:?}"
    );
    let full_version = seen["hintsFullVersions"][1].as_str().unwrap();
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
    assert_eq!(seen["webdriver"], false);

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
