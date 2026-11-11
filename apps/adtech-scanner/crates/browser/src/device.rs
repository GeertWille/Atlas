//! Device profiles: what a scan pretends to be. No I/O.

use adscan_model::{DeviceProfile, Viewport};
use serde_json::{Value, json};

/// Everything applied through the `Emulation` CDP domain before navigation.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Emulation {
    pub viewport: Viewport,
    pub user_agent: String,
    /// `navigator.platform` override; `None` keeps the host value.
    pub platform: Option<&'static str>,
    /// `Emulation.setUserAgentOverride.userAgentMetadata` (UA client hints).
    pub client_hints: Value,
    pub touch: bool,
    /// Human description for progress lines.
    pub label: String,
}

impl Emulation {
    /// Parameters of `Emulation.setUserAgentOverride`.
    pub(crate) fn user_agent_override(&self) -> Value {
        let mut params = json!({ "userAgent": self.user_agent, "userAgentMetadata": self.client_hints });
        if let Some(platform) = self.platform {
            params["platform"] = json!(platform);
        }
        params
    }
}

const DESKTOP: Viewport = Viewport { width: 1440, height: 900, device_scale_factor: 1.0, mobile: false };
const IPHONE: Viewport = Viewport { width: 393, height: 852, device_scale_factor: 3.0, mobile: true };

const IPHONE_USER_AGENT: &str = "Mozilla/5.0 (iPhone; CPU iPhone OS 18_6 like Mac OS X) \
     AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.0 Mobile/15E148 Safari/604.1";

/// Size of the browser window to launch for a device.
pub(crate) fn window_size(device: DeviceProfile) -> (u32, u32) {
    match device {
        DeviceProfile::Desktop => (DESKTOP.width, DESKTOP.height),
        DeviceProfile::Mobile => (IPHONE.width, IPHONE.height),
    }
}

/// `browser_user_agent` and `browser_product` ("Chrome/141.0.7390.55") are
/// what the launched browser reports for itself.
pub(crate) fn emulation_for(device: DeviceProfile, browser_user_agent: &str, browser_product: &str) -> Emulation {
    match device {
        DeviceProfile::Desktop => desktop(browser_user_agent, browser_product),
        DeviceProfile::Mobile => iphone(),
    }
}

fn desktop(browser_user_agent: &str, browser_product: &str) -> Emulation {
    // Headless builds announce themselves; ad stacks treat that as a bot.
    let user_agent = browser_user_agent.replace("HeadlessChrome/", "Chrome/");
    // The user agent string only carries the major version ("141.0.0.0").
    let full_version = browser_product.rsplit('/').next().unwrap_or_default().to_string();
    let major = full_version.split('.').next().unwrap_or("0").to_string();
    let (platform, architecture) = host_platform(&user_agent);
    let brands = |version: &str| {
        json!([
            { "brand": "Not?A_Brand", "version": if version == major { "99" } else { "99.0.0.0" } },
            { "brand": "Chromium", "version": version },
            { "brand": "Google Chrome", "version": version },
        ])
    };
    Emulation {
        viewport: DESKTOP,
        client_hints: json!({
            "brands": brands(&major),
            "fullVersionList": brands(&full_version),
            "platform": platform,
            "platformVersion": "",
            "architecture": architecture,
            "model": "",
            "mobile": false,
            "bitness": "64",
        }),
        user_agent,
        platform: None,
        touch: false,
        label: "desktop (1440x900)".into(),
    }
}

fn iphone() -> Emulation {
    Emulation {
        viewport: IPHONE,
        user_agent: IPHONE_USER_AGENT.into(),
        platform: Some("iPhone"),
        client_hints: json!({
            "brands": [],
            "fullVersionList": [],
            "platform": "iOS",
            "platformVersion": "18.6.0",
            "architecture": "",
            "model": "iPhone",
            "mobile": true,
        }),
        touch: true,
        label: "iPhone (393x852 @3x, touch)".into(),
    }
}

fn host_platform(user_agent: &str) -> (&'static str, &'static str) {
    if user_agent.contains("Macintosh") {
        ("macOS", "arm")
    } else if user_agent.contains("Windows") {
        ("Windows", "x86")
    } else {
        ("Linux", "x86")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEADLESS_LINUX: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 \
        (KHTML, like Gecko) HeadlessChrome/153.0.0.0 Safari/537.36";

    #[test]
    fn desktop_hides_headless_and_matches_client_hints_to_the_browser() {
        let emulation = emulation_for(DeviceProfile::Desktop, HEADLESS_LINUX, "HeadlessChrome/153.0.8010.12");
        assert!(!emulation.user_agent.contains("Headless"));
        assert!(emulation.user_agent.contains(" Chrome/153.0.0.0"));
        assert_eq!((emulation.viewport.width, emulation.viewport.height), (1440, 900));
        assert!(!emulation.viewport.mobile && !emulation.touch);
        assert_eq!(emulation.client_hints["mobile"], false);
        assert_eq!(emulation.client_hints["platform"], "Linux");
        assert_eq!(emulation.client_hints["brands"][2]["version"], "153");
        assert_eq!(emulation.client_hints["fullVersionList"][1]["version"], "153.0.8010.12");
    }

    #[test]
    fn desktop_platform_follows_the_host_user_agent() {
        let mac = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 Chrome/141.0.0.0 Safari/537.36";
        assert_eq!(emulation_for(DeviceProfile::Desktop, mac, "Chrome/141.0.7390.55").client_hints["platform"], "macOS");
        let windows = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/141.0.0.0 Safari/537.36";
        assert_eq!(emulation_for(DeviceProfile::Desktop, windows, "Chrome/141.0.7390.55").client_hints["platform"], "Windows");
    }

    #[test]
    fn mobile_is_a_touch_iphone_regardless_of_the_host_browser() {
        let emulation = emulation_for(DeviceProfile::Mobile, HEADLESS_LINUX, "HeadlessChrome/153.0.8010.12");
        assert!(emulation.user_agent.contains("iPhone") && emulation.user_agent.contains("Mobile/"));
        assert!(emulation.viewport.mobile && emulation.touch);
        assert!(emulation.viewport.width < 768);
        assert_eq!(emulation.viewport.device_scale_factor, 3.0);
        assert_eq!(emulation.platform, Some("iPhone"));
        assert_eq!(emulation.client_hints["mobile"], true);
    }
}
