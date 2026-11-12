//! What the scanned pages can tell about the browser that visits them. The
//! values are read from the browser and reported as they are: a visit the
//! scanner drives is automated and shows as such, and a rule of the analysis
//! takes that into account. Nothing here, or anywhere, changes them.

use adscan_model::{ActivityKind, CaptureActivity, CaptureEnvironment, Viewport};
use serde_json::json;

use super::error::CaptureError;
use super::page::Page;

/// What the page's own scripts read from the browser.
#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct Observed {
    /// `navigator.webdriver`.
    pub webdriver: Option<bool>,
    pub user_agent: Option<String>,
    pub viewport: Option<Viewport>,
}

impl Page {
    /// Reads what the loaded page can see. Evaluated in the scanner's own
    /// world of the page, where the values are the browser's, whatever the
    /// page's scripts did to their copies.
    pub(crate) async fn observe_environment(&mut self) -> Result<Observed, CaptureError> {
        let seen = self
            .evaluate(
                "[navigator.webdriver, navigator.userAgent, innerWidth, innerHeight, devicePixelRatio]",
            )
            .await?
            .unwrap_or_default();
        let whole = |index: usize| seen[index].as_f64().map(|value| value.round() as u32);
        let mobile = self.emulation().viewport.mobile;
        Ok(Observed {
            webdriver: seen[0].as_bool(),
            user_agent: seen[1].as_str().map(str::to_string),
            viewport: match (whole(2), whole(3), seen[4].as_f64()) {
                (Some(width), Some(height), Some(device_scale_factor)) => Some(Viewport {
                    width,
                    height,
                    device_scale_factor,
                    mobile,
                }),
                _ => None,
            },
        })
    }
}

/// The audit-trail entry that states how recognisable the visit is.
pub(crate) fn activity(environment: &CaptureEnvironment) -> CaptureActivity {
    let mut signs = Vec::new();
    match environment.webdriver {
        Some(true) => signs.push("navigator.webdriver is true"),
        Some(false) => {}
        None => signs.push("navigator.webdriver could not be read"),
    }
    if environment.headless {
        signs.push("headless");
    }
    let message = if environment.webdriver == Some(true) || environment.headless {
        format!(
            "This visit is identifiable as automated: {}",
            signs.join(", ")
        )
    } else if environment.webdriver == Some(false) {
        "This visit is not flagged as automated: navigator.webdriver is false and the browser is visible"
            .to_string()
    } else {
        "Whether this visit is flagged as automated is unknown: navigator.webdriver could not be read"
            .to_string()
    };
    CaptureActivity {
        kind: ActivityKind::Environment,
        message,
        page: Some(0),
        data: Some(json!({
            "webdriver": environment.webdriver,
            "headless": environment.headless,
            "persistentProfile": environment.persistent_profile,
            "userAgentOverridden": environment.user_agent_overridden,
            "viewportOverridden": environment.viewport_overridden,
        })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn environment(webdriver: Option<bool>, headless: bool) -> CaptureEnvironment {
        CaptureEnvironment {
            headless,
            webdriver,
            ..CaptureEnvironment::default()
        }
    }

    #[test]
    fn an_automated_visit_is_called_that_with_the_signs_that_show_it() {
        let headless = activity(&environment(Some(true), true));
        assert_eq!(
            headless.message,
            "This visit is identifiable as automated: navigator.webdriver is true, headless"
        );
        assert_eq!(headless.kind, ActivityKind::Environment);
        assert_eq!(
            headless.data,
            Some(json!({
                "webdriver": true,
                "headless": true,
                "persistentProfile": false,
                "userAgentOverridden": false,
                "viewportOverridden": false,
            }))
        );
        assert_eq!(
            activity(&environment(Some(true), false)).message,
            "This visit is identifiable as automated: navigator.webdriver is true"
        );
        assert_eq!(
            activity(&environment(None, true)).message,
            "This visit is identifiable as automated: navigator.webdriver could not be read, headless"
        );
    }

    #[test]
    fn a_visit_without_the_signs_is_not_called_automated_and_an_unread_value_is_unknown() {
        assert!(
            activity(&environment(Some(false), false))
                .message
                .starts_with("This visit is not flagged as automated")
        );
        let unknown = activity(&environment(None, false));
        assert!(unknown.message.contains("unknown"), "{}", unknown.message);
        assert_eq!(unknown.data.unwrap()["webdriver"], serde_json::Value::Null);
    }
}
