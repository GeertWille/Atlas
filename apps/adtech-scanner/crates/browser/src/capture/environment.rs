//! What the scanned pages can tell about the browser that visits them. The
//! values are read from the browser and reported as they are: nothing here
//! changes what the page sees, and a rule of the analysis takes the report
//! into account. The browser is launched without Chrome's automation marker,
//! whoever drives it, so these values normally say the visit is not flagged;
//! a headless run still shows as one.

use adscan_model::{ActivityKind, CaptureActivity, CaptureEnvironment, Viewport};
use serde_json::json;

use super::error::CaptureError;
use super::page::Page;
use super::remote::Seen;

/// What the page's own scripts read from the browser.
#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct Observed {
    /// `navigator.webdriver`.
    pub webdriver: Option<bool>,
    pub user_agent: Option<String>,
    pub viewport: Option<Viewport>,
    /// The brands of the user agent's client hints, each with its major
    /// version. Empty where the page has none (not a secure context).
    pub brands: Vec<String>,
}

impl Page {
    /// Reads what the loaded page can see. Evaluated in the scanner's own
    /// world of the page, where the values are the browser's, whatever the
    /// page's scripts did to their copies.
    pub(crate) async fn observe_environment(&mut self) -> Result<Observed, CaptureError> {
        let seen = self
            .evaluate(
                "[navigator.webdriver, navigator.userAgent, innerWidth, innerHeight, devicePixelRatio, \
                 ((navigator.userAgentData || {}).brands || []).map((b) => b.brand + ' ' + b.version)]",
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
            brands: seen[5]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|brand| brand.as_str().map(str::to_string))
                .collect(),
        })
    }
}

/// The audit-trail entry that states how recognisable the visit is.
/// `at_browserbase` is what a browser at Browserbase reported on arrival.
pub(crate) fn activity(
    environment: &CaptureEnvironment,
    at_browserbase: Option<&Seen>,
) -> CaptureActivity {
    let message = match at_browserbase {
        Some(seen) => remote_message(environment, seen),
        None => local_message(environment),
    };
    let mut data = json!({
        "webdriver": environment.webdriver,
        "headless": environment.headless,
        "persistentProfile": environment.persistent_profile,
        "userAgentOverridden": environment.user_agent_overridden,
        "viewportOverridden": environment.viewport_overridden,
    });
    if let Some(seen) = at_browserbase {
        data["browserbase"] = seen.report();
    }
    CaptureActivity {
        kind: ActivityKind::Environment,
        message,
        page: Some(0),
        data: Some(data),
    }
}

fn local_message(environment: &CaptureEnvironment) -> String {
    let mut signs = Vec::new();
    match environment.webdriver {
        Some(true) => signs.push("navigator.webdriver is true"),
        Some(false) => {}
        None => signs.push("navigator.webdriver could not be read"),
    }
    if environment.headless {
        signs.push("headless");
    }
    if environment.webdriver == Some(true) || environment.headless {
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
    }
}

/// A browser at Browserbase runs headless without its user agent saying
/// so, and is started with the automation flag off: what the page can
/// tell is what it reads, and that is what is stated.
fn remote_message(environment: &CaptureEnvironment, seen: &Seen) -> String {
    let headless = if seen.user_agent_shows_headless() {
        "it runs headless, and its user agent says so"
    } else {
        "it runs headless, which its user agent does not show"
    };
    match environment.webdriver {
        Some(true) => format!(
            "This visit is identifiable as automated: navigator.webdriver is true; {headless}"
        ),
        Some(false) if seen.user_agent_shows_headless() => format!(
            "This visit is identifiable as automated: navigator.webdriver is false (Browserbase \
             starts its browser that way), but {headless}"
        ),
        Some(false) => format!(
            "This visit is not flagged as automated: navigator.webdriver is false (Browserbase \
             starts its browser that way); {headless}"
        ),
        None => format!(
            "Whether this visit is flagged as automated is unknown: navigator.webdriver could \
             not be read; {headless}"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn activity_of(environment: CaptureEnvironment) -> CaptureActivity {
        activity(&environment, None)
    }

    fn environment(webdriver: Option<bool>, headless: bool) -> CaptureEnvironment {
        CaptureEnvironment {
            headless,
            webdriver,
            ..CaptureEnvironment::default()
        }
    }

    #[test]
    fn an_automated_visit_is_called_that_with_the_signs_that_show_it() {
        let headless = activity_of(environment(Some(true), true));
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
            activity_of(environment(Some(true), false)).message,
            "This visit is identifiable as automated: navigator.webdriver is true"
        );
        assert_eq!(
            activity_of(environment(None, true)).message,
            "This visit is identifiable as automated: navigator.webdriver could not be read, headless"
        );
    }

    #[test]
    fn a_visit_without_the_signs_is_not_called_automated_and_an_unread_value_is_unknown() {
        assert!(
            activity_of(environment(Some(false), false))
                .message
                .starts_with("This visit is not flagged as automated")
        );
        let unknown = activity_of(environment(None, false));
        assert!(unknown.message.contains("unknown"), "{}", unknown.message);
        assert_eq!(unknown.data.unwrap()["webdriver"], serde_json::Value::Null);
    }
    /// A browser at Browserbase is headless without saying so and has the
    /// automation flag off: the line states what the page reads, and never
    /// that the browser is visible.
    #[test]
    fn a_visit_at_browserbase_is_described_by_what_its_pages_can_read() {
        let chrome = "Mozilla/5.0 (X11; Linux x86_64) Chrome/157.0.0.0 Safari/537.36";
        let seen = |user_agent: &str| Seen {
            webdriver_main_world: Some(false),
            webdriver_isolated_world: Some(false),
            user_agent: Some(user_agent.into()),
            headless: Some(true),
            ..Seen::default()
        };
        let at_browserbase = |webdriver, user_agent: &str| {
            activity(&environment(webdriver, true), Some(&seen(user_agent)))
        };

        let unflagged = at_browserbase(Some(false), chrome);
        assert_eq!(
            unflagged.message,
            "This visit is not flagged as automated: navigator.webdriver is false (Browserbase \
             starts its browser that way); it runs headless, which its user agent does not show"
        );
        let data = unflagged.data.unwrap();
        assert_eq!(data["headless"], true);
        assert_eq!(data["browserbase"]["webdriverMainWorld"], false);
        assert_eq!(data["browserbase"]["headlessBrowser"], true);
        assert_eq!(data["browserbase"]["headlessShownInUserAgent"], false);

        assert_eq!(
            at_browserbase(Some(true), chrome).message,
            "This visit is identifiable as automated: navigator.webdriver is true; it runs \
             headless, which its user agent does not show"
        );
        assert_eq!(
            at_browserbase(Some(false), "Mozilla/5.0 HeadlessChrome/153.0.0.0").message,
            "This visit is identifiable as automated: navigator.webdriver is false (Browserbase \
             starts its browser that way), but it runs headless, and its user agent says so"
        );
        assert!(at_browserbase(None, chrome).message.contains("unknown"));
        for message in [
            at_browserbase(Some(false), chrome).message,
            at_browserbase(None, chrome).message,
        ] {
            assert!(!message.contains("visible"), "{message}");
        }
    }
}
