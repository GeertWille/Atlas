//! The targets below the tabs: out-of-process iframes and workers, each with
//! a session of its own. Their requests are part of the page that started
//! them. No I/O.

use std::collections::HashMap;

use serde_json::Value;

use super::assembly::Reporter;

/// A target the browser attached the scanner to, below a tab or, for a
/// shared worker, below the browser itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ChildTarget {
    pub reporter: Reporter,
    pub target_id: String,
    /// Session of the tab it belongs to. A shared worker belongs to none.
    pub tab_session: Option<String>,
}

#[derive(Debug, Default)]
pub(crate) struct ChildTargets {
    by_session: HashMap<String, ChildTarget>,
}

impl ChildTargets {
    /// Notes the target of a `Target.attachedToTarget` event and returns its
    /// session and what it is. `parent_session` is the session that reported
    /// it, `parent_is_tab` whether that is a tab's own. `None` for a target
    /// that makes no requests of a page: another tab, the browser.
    pub(crate) fn attached<'a>(
        &mut self,
        parent_session: Option<&str>,
        parent_is_tab: bool,
        attached: &'a Value,
    ) -> Option<(&'a str, Reporter)> {
        let target = &attached["targetInfo"];
        let session_id = attached["sessionId"].as_str()?;
        let reporter = match target["type"].as_str()? {
            "iframe" => Reporter::Iframe,
            "worker" | "shared_worker" => Reporter::Worker,
            "service_worker" => Reporter::ServiceWorker,
            _ => return None,
        };
        let tab_session = match parent_session {
            Some(parent) if parent_is_tab => Some(parent.to_string()),
            Some(parent) => self
                .by_session
                .get(parent)
                .and_then(|parent| parent.tab_session.clone()),
            None => None,
        };
        self.by_session.insert(
            session_id.to_string(),
            ChildTarget {
                reporter,
                target_id: target["targetId"].as_str().unwrap_or_default().to_string(),
                tab_session,
            },
        );
        Some((session_id, reporter))
    }

    pub(crate) fn detached(&mut self, session_id: &str) {
        self.by_session.remove(session_id);
    }

    /// A tab closed: what was below it is gone with it. Returns the
    /// sessions of those targets.
    pub(crate) fn tab_closed(&mut self, tab_session: &str) -> Vec<String> {
        let gone: Vec<String> = self
            .by_session
            .iter()
            .filter(|(_, child)| child.tab_session.as_deref() == Some(tab_session))
            .map(|(session_id, _)| session_id.clone())
            .collect();
        for session_id in &gone {
            self.by_session.remove(session_id);
        }
        gone
    }

    pub(crate) fn get(&self, session_id: &str) -> Option<&ChildTarget> {
        self.by_session.get(session_id)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn attached(kind: &str, session: &str) -> Value {
        json!({
            "sessionId": session,
            "targetInfo": { "type": kind, "targetId": format!("target-{session}") },
        })
    }

    #[test]
    fn a_target_belongs_to_the_tab_at_the_top_of_its_parents() {
        let mut targets = ChildTargets::default();
        assert_eq!(
            targets.attached(Some("tab"), true, &attached("iframe", "frame")),
            Some(("frame", Reporter::Iframe))
        );
        assert_eq!(
            targets.attached(Some("frame"), false, &attached("worker", "worker")),
            Some(("worker", Reporter::Worker))
        );
        assert_eq!(
            targets.attached(Some("tab"), true, &attached("service_worker", "sw")),
            Some(("sw", Reporter::ServiceWorker))
        );

        let worker = targets.get("worker").unwrap();
        assert_eq!(worker.tab_session.as_deref(), Some("tab"));
        assert_eq!(worker.target_id, "target-worker");
        assert_eq!(
            targets.get("sw").unwrap().tab_session.as_deref(),
            Some("tab")
        );
    }

    #[test]
    fn a_shared_worker_belongs_to_no_tab_and_other_targets_are_not_children() {
        let mut targets = ChildTargets::default();
        assert_eq!(
            targets.attached(None, false, &attached("shared_worker", "shared")),
            Some(("shared", Reporter::Worker))
        );
        assert_eq!(targets.get("shared").unwrap().tab_session, None);

        assert_eq!(
            targets.attached(None, false, &attached("page", "tab-2")),
            None
        );
        assert_eq!(
            targets.attached(None, false, &attached("browser", "b")),
            None
        );
        assert!(targets.get("tab-2").is_none());
    }

    #[test]
    fn a_detached_target_and_those_of_a_closed_tab_are_forgotten() {
        let mut targets = ChildTargets::default();
        targets.attached(Some("tab"), true, &attached("iframe", "frame"));
        targets.attached(Some("frame"), false, &attached("worker", "worker"));
        targets.attached(Some("other-tab"), true, &attached("iframe", "other"));
        let mut gone = targets.tab_closed("tab");
        gone.sort();
        assert_eq!(gone, ["frame", "worker"]);
        assert!(targets.get("frame").is_none() && targets.get("worker").is_none());
        assert!(targets.get("other").is_some());

        targets.attached(Some("tab"), true, &attached("iframe", "frame"));
        targets.detached("frame");
        assert!(targets.get("frame").is_none());
        // What attaches below a session that is gone has no tab.
        targets.attached(Some("frame"), false, &attached("worker", "late"));
        assert_eq!(targets.get("late").unwrap().tab_session, None);
    }
}
