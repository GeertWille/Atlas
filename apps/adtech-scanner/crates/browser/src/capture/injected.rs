//! The scripts evaluated inside the scanned page and the names they share with Rust.

use serde_json::{Value, json};

use super::visitor::Point;
use super::visitor::site::Site;

/// Runs before any page script in the top frame and reports ad-stack
/// activity. A function expression: see [`tracker_script`].
pub(crate) const TRACKER_SOURCE: &str = include_str!("../../injected/tracker.js");

const CONSENT_SOURCE: &str = include_str!("../../injected/consent.js");

const PAGE_FACTS_SOURCE: &str = include_str!("../../injected/links.js");
const HIT_TEST_SOURCE: &str = include_str!("../../injected/hit.js");

/// CDP binding the tracker sends its batches through.
pub(crate) const TRACKER_BINDING: &str = "__adscan_emit";

/// DOM event that makes the tracker send what it has buffered.
pub(crate) const TRACKER_FLUSH_EVENT: &str = "__adscan_flush";

/// The tracker as installed in a tab. With a `site` it stays silent on every
/// other site; without one it runs wherever the tab goes, which is right only
/// until the visit's first page is reached.
pub(crate) fn tracker_script(site: Option<&Site>) -> String {
    let site = site.map_or(Value::Null, |site| {
        let (domain, port) = site.domain_and_port();
        json!({ "domain": domain, "port": port })
    });
    format!("{TRACKER_SOURCE}({site});")
}

/// Whether a script the debugger reports is the scanner's own tracker.
pub(crate) fn is_tracker_script(source: &str) -> bool {
    source.starts_with(TRACKER_SOURCE)
}

/// Expression that evaluates to the page's scroll state and reading targets,
/// and to its links when `links` is set. `slot_ids` are the ad containers the
/// tracker has reported.
pub(crate) fn page_facts_expression(slot_ids: &[String], links: bool) -> String {
    let request = json!({ "slotIds": slot_ids, "links": links });
    format!("{PAGE_FACTS_SOURCE}({request})")
}

/// What `injected/consent.js` is asked to do in a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConsentAsk {
    /// Say what the frame shows of a consent dialog: its accept-all control
    /// and where it is, or the dialog alone.
    Look,
    /// Click the accept-all control from script.
    Click,
    /// Resolve to whether the accept-all control is clicked within this
    /// many ms.
    WatchForClick(u32),
}

pub(crate) fn consent_expression(ask: ConsentAsk) -> String {
    let request = match ask {
        ConsentAsk::Look => json!({}),
        ConsentAsk::Click => json!({ "click": true }),
        ConsentAsk::WatchForClick(ms) => json!({ "watchMs": ms }),
    };
    format!("{CONSENT_SOURCE}({request})")
}

/// Expression that evaluates to the link a click at `point` would follow.
pub(crate) fn hit_test_expression(point: Point) -> String {
    let point = json!({ "x": point.x, "y": point.y });
    format!("{HIT_TEST_SOURCE}({point})")
}

#[cfg(test)]
mod tests {
    use url::Url;

    use super::*;

    #[test]
    fn tracker_uses_the_names_rust_listens_for() {
        assert!(TRACKER_SOURCE.contains(&format!("'{TRACKER_BINDING}'")));
        assert!(TRACKER_SOURCE.contains(&format!("'{TRACKER_FLUSH_EVENT}'")));
    }

    #[test]
    fn the_tracker_is_called_with_the_site_it_may_run_on() {
        let anywhere = tracker_script(None);
        assert!(
            anywhere.ends_with("})\n(null);"),
            "{}",
            &anywhere[anywhere.len() - 20..]
        );
        assert!(is_tracker_script(&anywhere));

        let site = Site::of(&Url::parse("https://www.paper.co.uk/a").unwrap()).unwrap();
        assert!(
            tracker_script(Some(&site)).ends_with(r#"({"domain":"paper.co.uk","port":null});"#)
        );
        let local = Site::of(&Url::parse("http://127.0.0.1:4100/").unwrap()).unwrap();
        assert!(tracker_script(Some(&local)).ends_with(r#"({"domain":"127.0.0.1","port":4100});"#));
        assert!(!is_tracker_script("(function () { pageCode(); })();"));
    }

    #[test]
    fn page_scripts_are_called_with_json_arguments() {
        let facts = page_facts_expression(&["ad-\"top".to_string()], true);
        assert!(
            facts.ends_with(r#"({"links":true,"slotIds":["ad-\"top"]})"#),
            "{}",
            &facts[facts.len() - 60..]
        );
        let hit = hit_test_expression(Point { x: 10.5, y: 20.0 });
        assert!(hit.ends_with(r#"({"x":10.5,"y":20.0})"#));
    }
}
