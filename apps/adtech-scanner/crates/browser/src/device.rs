//! Device profiles: the device a scan emulates. No I/O.
//!
//! A desktop scan is the launched browser as it is, headless or not: its own
//! user agent, its own client hints. Only the window size is set. A mobile
//! scan emulates a phone, which is said in the scan's environment.

use adscan_model::{DeviceProfile, Viewport};
use serde_json::{Value, json};

/// Everything applied through the `Emulation` CDP domain before navigation.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Emulation {
    pub viewport: Viewport,
    /// The user agent pages see.
    pub user_agent: String,
    /// `None` when the browser presents itself as what it is.
    identity: Option<Identity>,
    pub touch: bool,
    /// Human description for progress lines.
    pub label: String,
}

/// The identity of the emulated phone.
#[derive(Debug, Clone, PartialEq)]
struct Identity {
    /// `navigator.platform` of the phone.
    platform: &'static str,
    /// `Emulation.setUserAgentOverride.userAgentMetadata` (UA client hints).
    client_hints: Value,
}

impl Emulation {
    /// Parameters of `Emulation.setUserAgentOverride`, when one is needed.
    pub(crate) fn user_agent_override(&self) -> Option<Value> {
        let identity = self.identity.as_ref()?;
        Some(json!({
            "userAgent": self.user_agent,
            "platform": identity.platform,
            "userAgentMetadata": identity.client_hints,
        }))
    }

    /// Parameters of `Emulation.setDeviceMetricsOverride`.
    pub(crate) fn device_metrics(&self) -> Value {
        let viewport = self.viewport;
        let mut params = json!({
            "width": viewport.width,
            "height": viewport.height,
            "deviceScaleFactor": viewport.device_scale_factor,
            "mobile": viewport.mobile,
            "screenWidth": viewport.width,
            "screenHeight": viewport.height,
        });
        if viewport.mobile {
            // The host screen is a landscape monitor; a phone is held upright.
            params["screenOrientation"] = json!({ "type": "portraitPrimary", "angle": 0 });
        }
        params
    }
}

const DESKTOP: Viewport = Viewport {
    width: 1440,
    height: 900,
    device_scale_factor: 1.0,
    mobile: false,
};
/// A Pixel 8. The scan runs on a Chromium engine, and Chrome on Android is
/// the phone browser it can impersonate completely: an iPhone user agent
/// would be contradicted by `navigator.userAgentData`, `window.chrome`, the
/// client-hint request headers and every Safari-only code path.
const PHONE: Viewport = Viewport {
    width: 412,
    height: 915,
    device_scale_factor: 2.625,
    mobile: true,
};
const PHONE_MODEL: &str = "Pixel 8";
const PHONE_ANDROID_VERSION: &str = "14.0.0";

/// Size of the browser window to launch for a device.
pub(crate) fn window_size(device: DeviceProfile) -> (u32, u32) {
    match device {
        DeviceProfile::Desktop => (DESKTOP.width, DESKTOP.height),
        DeviceProfile::Mobile => (PHONE.width, PHONE.height),
    }
}

/// `browser_user_agent` and `browser_product` ("Chrome/141.0.7390.55") are
/// what the launched browser reports for itself.
pub(crate) fn emulation_for(
    device: DeviceProfile,
    browser_user_agent: &str,
    browser_product: &str,
) -> Emulation {
    match device {
        DeviceProfile::Desktop => Emulation {
            viewport: DESKTOP,
            user_agent: browser_user_agent.to_string(),
            identity: None,
            touch: false,
            label: "desktop (1440x900)".into(),
        },
        DeviceProfile::Mobile => {
            // The user agent string only carries the major version ("141.0.0.0").
            let full_version = browser_product.rsplit('/').next().unwrap_or_default();
            let major = full_version.split('.').next().unwrap_or_default();
            phone(major, full_version)
        }
    }
}

fn phone(major: &str, full_version: &str) -> Emulation {
    Emulation {
        viewport: PHONE,
        // Chrome on Android sends this reduced form; the device model and OS
        // version are only in the client hints.
        user_agent: format!(
            "Mozilla/5.0 (Linux; Android 10; K) AppleWebKit/537.36 (KHTML, like Gecko) \
             Chrome/{major}.0.0.0 Mobile Safari/537.36"
        ),
        identity: Some(Identity {
            platform: "Linux armv81",
            client_hints: json!({
                "brands": brands(major, major),
                "fullVersionList": brands(major, full_version),
                "platform": "Android",
                "platformVersion": PHONE_ANDROID_VERSION,
                "architecture": "",
                "model": PHONE_MODEL,
                "mobile": true,
                "bitness": "",
            }),
        }),
        touch: true,
        label: format!(
            "{PHONE_MODEL} ({}x{} @{}x, touch)",
            PHONE.width, PHONE.height, PHONE.device_scale_factor
        ),
    }
}

/// The client-hint brand list Chrome `major` sends, with `version` as the
/// version of the real brands. Chrome adds a meaningless "GREASE" brand and
/// derives its spelling, its version and the order of the list from the major
/// version; servers that check a visitor's hints against its user agent
/// expect exactly that combination.
fn brands(major: &str, version: &str) -> Value {
    const GREASE_CHARS: [&str; 11] = [" ", "(", ":", "-", ".", "/", ")", ";", "=", "?", "_"];
    const GREASE_VERSIONS: [&str; 3] = ["8", "99", "24"];
    const ORDERS: [[usize; 3]; 6] = [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ];
    let seed: usize = major.parse().unwrap_or(0);
    let grease_brand = format!(
        "Not{}A{}Brand",
        GREASE_CHARS[seed % GREASE_CHARS.len()],
        GREASE_CHARS[(seed + 1) % GREASE_CHARS.len()]
    );
    let grease_major = GREASE_VERSIONS[seed % GREASE_VERSIONS.len()];
    let grease_version = if version == major {
        grease_major.to_string()
    } else {
        format!("{grease_major}.0.0.0")
    };
    let entries = [
        json!({ "brand": grease_brand, "version": grease_version }),
        json!({ "brand": "Chromium", "version": version }),
        json!({ "brand": "Google Chrome", "version": version }),
    ];
    let mut list = [Value::Null, Value::Null, Value::Null];
    for (entry, position) in entries.into_iter().zip(ORDERS[seed % ORDERS.len()]) {
        list[position] = entry;
    }
    Value::from(list.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEADLESS_LINUX: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 \
        (KHTML, like Gecko) HeadlessChrome/153.0.0.0 Safari/537.36";

    fn brand_header(list: &Value) -> String {
        list.as_array()
            .unwrap()
            .iter()
            .map(|entry| {
                format!(
                    "\"{}\";v=\"{}\"",
                    entry["brand"].as_str().unwrap(),
                    entry["version"].as_str().unwrap()
                )
            })
            .collect::<Vec<_>>()
            .join(", ")
    }

    #[test]
    fn brand_lists_match_what_real_chrome_releases_send() {
        // `sec-ch-ua` headers recorded from Chrome 124, 125 and 126.
        assert_eq!(
            brand_header(&brands("124", "124")),
            r#""Chromium";v="124", "Google Chrome";v="124", "Not-A.Brand";v="99""#
        );
        assert_eq!(
            brand_header(&brands("125", "125")),
            r#""Google Chrome";v="125", "Chromium";v="125", "Not.A/Brand";v="24""#
        );
        assert_eq!(
            brand_header(&brands("126", "126")),
            r#""Not/A)Brand";v="8", "Chromium";v="126", "Google Chrome";v="126""#
        );
        assert_eq!(
            brand_header(&brands("126", "126.0.6478.127")),
            r#""Not/A)Brand";v="8.0.0.0", "Chromium";v="126.0.6478.127", "Google Chrome";v="126.0.6478.127""#
        );
    }

    #[test]
    fn a_desktop_scan_presents_the_browser_as_it_is_headless_or_not() {
        let headless = emulation_for(
            DeviceProfile::Desktop,
            HEADLESS_LINUX,
            "HeadlessChrome/153.0.8010.12",
        );
        assert_eq!(headless.user_agent, HEADLESS_LINUX);
        assert_eq!(headless.user_agent_override(), None);
        assert_eq!(
            (headless.viewport.width, headless.viewport.height),
            (1440, 900)
        );
        assert!(!headless.viewport.mobile && !headless.touch);
        assert_eq!(
            headless.device_metrics().get("screenOrientation"),
            None,
            "a desktop keeps the host's landscape screen"
        );

        let mac = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 \
            (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36";
        let visible = emulation_for(DeviceProfile::Desktop, mac, "Chrome/141.0.7390.55");
        assert_eq!(visible.user_agent, mac);
        assert_eq!(visible.user_agent_override(), None);
    }

    #[test]
    fn mobile_is_chrome_on_an_android_phone_of_the_host_browsers_version() {
        let emulation = emulation_for(
            DeviceProfile::Mobile,
            HEADLESS_LINUX,
            "HeadlessChrome/153.0.8010.12",
        );
        assert_eq!(
            emulation.user_agent,
            "Mozilla/5.0 (Linux; Android 10; K) AppleWebKit/537.36 (KHTML, like Gecko) \
             Chrome/153.0.0.0 Mobile Safari/537.36"
        );
        assert!(emulation.viewport.mobile && emulation.touch);
        assert!(emulation.viewport.width < 768);
        assert_eq!(
            window_size(DeviceProfile::Mobile),
            (emulation.viewport.width, emulation.viewport.height)
        );

        let identity = emulation.user_agent_override().unwrap();
        assert_eq!(identity["platform"], "Linux armv81");
        let hints = &identity["userAgentMetadata"];
        assert_eq!(hints["mobile"], true);
        assert_eq!(hints["platform"], "Android");
        assert_eq!(hints["model"], "Pixel 8");
        assert_eq!(hints["fullVersionList"], brands("153", "153.0.8010.12"));
        assert_eq!(
            emulation.device_metrics()["screenOrientation"]["type"],
            "portraitPrimary"
        );
    }
}
