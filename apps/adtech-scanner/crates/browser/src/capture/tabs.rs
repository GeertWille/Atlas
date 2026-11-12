//! The tabs of the browser being scanned: which one is recorded, and what
//! happens to a tab the page or the visitor opens.
//!
//! Every new tab starts paused, so that the tracker is installed before its
//! first script runs. The policy below decides whether it is then closed,
//! watched, or made the tab the scan records.

use adscan_model::VisitorBehavior;

pub(crate) struct Tab {
    pub target_id: String,
    pub session_id: String,
    pub main_frame_id: String,
    /// Sessions of cross-site iframes, where consent banners often live.
    pub frame_sessions: Vec<String>,
    /// Address of the tab's current document.
    pub url: String,
}

/// What to do with a tab that just opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NewTab {
    /// A popup nobody asked for.
    Close,
    /// Let it load; its first document decides.
    Watch,
}

/// `expected`: the scripted visitor just clicked a link, which may open one.
pub(crate) fn on_new_tab(visitor: VisitorBehavior, expected: bool) -> NewTab {
    match visitor {
        VisitorBehavior::Manual => NewTab::Watch,
        VisitorBehavior::Browse if expected => NewTab::Watch,
        VisitorBehavior::Browse | VisitorBehavior::Scroll | VisitorBehavior::None => NewTab::Close,
    }
}

/// The open tabs, oldest first, and the one that is recorded.
pub(crate) struct Tabs {
    open: Vec<Tab>,
    recorded: String,
}

impl Tabs {
    pub(crate) fn starting_with(tab: Tab) -> Self {
        Self {
            recorded: tab.session_id.clone(),
            open: vec![tab],
        }
    }

    /// The tab being recorded. `None` once it was closed and no other tab
    /// took its place.
    pub(crate) fn recorded(&self) -> Option<&Tab> {
        self.open.iter().find(|tab| tab.session_id == self.recorded)
    }

    pub(crate) fn is_recorded(&self, session_id: &str) -> bool {
        self.recorded == session_id && self.recorded().is_some()
    }

    pub(crate) fn contains(&self, session_id: &str) -> bool {
        self.open.iter().any(|tab| tab.session_id == session_id)
    }

    pub(crate) fn by_session(&mut self, session_id: &str) -> Option<&mut Tab> {
        self.open
            .iter_mut()
            .find(|tab| tab.session_id == session_id)
    }

    pub(crate) fn add(&mut self, tab: Tab) {
        self.open.push(tab);
    }

    /// Makes an open tab the recorded one and returns it.
    pub(crate) fn record(&mut self, session_id: &str) -> Option<&Tab> {
        let tab = self.open.iter().find(|tab| tab.session_id == session_id)?;
        self.recorded = tab.session_id.clone();
        Some(tab)
    }

    /// Forgets a tab that closed. When it was the recorded one, the tab
    /// opened last takes its place and is returned.
    pub(crate) fn closed(&mut self, session_id: &str) -> Option<&Tab> {
        let was_recorded = self.is_recorded(session_id);
        self.open.retain(|tab| tab.session_id != session_id);
        if !was_recorded {
            return None;
        }
        let successor = self.open.last()?;
        self.recorded = successor.session_id.clone();
        Some(successor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tab(name: &str) -> Tab {
        Tab {
            target_id: format!("target-{name}"),
            session_id: name.to_string(),
            main_frame_id: format!("frame-{name}"),
            frame_sessions: Vec::new(),
            url: String::new(),
        }
    }

    fn recorded(tabs: &Tabs) -> Option<&str> {
        tabs.recorded().map(|tab| tab.session_id.as_str())
    }

    #[test]
    fn only_a_person_or_a_click_of_the_visitor_may_open_a_tab() {
        use VisitorBehavior::{Browse, Manual, None, Scroll};
        assert_eq!(on_new_tab(None, false), NewTab::Close);
        assert_eq!(on_new_tab(Scroll, false), NewTab::Close);
        assert_eq!(on_new_tab(Scroll, true), NewTab::Close);
        assert_eq!(on_new_tab(Browse, false), NewTab::Close);
        assert_eq!(on_new_tab(Browse, true), NewTab::Watch);
        assert_eq!(on_new_tab(Manual, false), NewTab::Watch);
    }

    #[test]
    fn recording_moves_to_a_tab_when_told_to() {
        let mut tabs = Tabs::starting_with(tab("first"));
        tabs.add(tab("second"));
        assert_eq!(recorded(&tabs), Some("first"));
        assert!(tabs.is_recorded("first") && !tabs.is_recorded("second"));

        assert_eq!(tabs.record("second").unwrap().main_frame_id, "frame-second");
        assert_eq!(recorded(&tabs), Some("second"));
        assert!(tabs.record("unknown").is_none());
        assert_eq!(recorded(&tabs), Some("second"));
    }

    #[test]
    fn closing_the_recorded_tab_falls_back_to_the_tab_opened_last() {
        let mut tabs = Tabs::starting_with(tab("first"));
        tabs.add(tab("second"));
        tabs.add(tab("third"));
        tabs.record("second");

        assert!(
            tabs.closed("first").is_none(),
            "another tab closing changes nothing"
        );
        assert_eq!(recorded(&tabs), Some("second"));

        assert_eq!(tabs.closed("second").unwrap().session_id, "third");
        assert_eq!(recorded(&tabs), Some("third"));

        assert!(tabs.closed("third").is_none());
        assert!(!tabs.contains("third"));
        assert_eq!(recorded(&tabs), None);
        assert!(!tabs.is_recorded("third"));
    }
}
