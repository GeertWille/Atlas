//! The page views of one visit and the rule that puts every recorded event
//! on one of them. No I/O.
//!
//! A page view starts when the main frame commits a document, or when a
//! single-page app changes its route inside one. Tracker events carry the
//! time origin of their document; that origin is bound to the document's page
//! views, and an event belongs to the page view its timestamp falls into.

use adscan_model::{CapturedPage, FollowedLink, PageReach};
use url::Url;

/// A reload that follows the scanner's own consent click this closely is the
/// consent platform restarting the page, not a new page view.
const CONSENT_RELOAD_WINDOW_MS: f64 = 3_000.0;

/// A main-frame document as the browser reported its commit.
pub(crate) struct Committed<'a> {
    pub url: &'a str,
    /// Epoch ms the document was requested; stands in for its time origin
    /// until the tracker's first batch states that.
    pub requested_at: f64,
    /// Whether the document belongs to the scanned site.
    pub on_site: bool,
    pub now: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Change {
    Opened(u32),
    /// The page view was still being opened (a redirect, a consent reload):
    /// it starts over, and what was recorded for it so far is void.
    Replaced(u32),
    /// The document is outside the scanned site; nothing is recorded on it.
    LeftSite,
    /// The visit has all the page views it may have. What happens from here
    /// on is recorded on the last one.
    OverCap,
    /// Nothing that makes a page view: a fragment change, a rewritten address.
    None,
}

/// What the tracked events say about one page view.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(crate) struct ViewFacts {
    pub load_ms: Option<f64>,
    pub max_scroll_depth: f64,
}

struct Document {
    /// `performance.timeOrigin`, once a tracker batch stated it.
    time_origin: Option<f64>,
    requested_at: f64,
    recorded: bool,
    /// Committed past the cap: its events go to the last page view.
    overflow: bool,
}

impl Document {
    fn started_at(&self) -> f64 {
        self.time_origin.unwrap_or(self.requested_at)
    }
}

struct View {
    document: usize,
    /// ms into its document at which the page view starts: 0 unless it was
    /// opened by a route change.
    offset_ms: f64,
    url: String,
    title: Option<String>,
    reached_by: PageReach,
    link: Option<FollowedLink>,
}

pub(crate) struct PageLog {
    documents: Vec<Document>,
    views: Vec<View>,
    max_views: usize,
    /// A person browses: every navigation is theirs.
    manual: bool,
    /// The link the visitor is about to follow.
    announced: Option<FollowedLink>,
    /// The current page view is still loading its first document.
    opening: bool,
    replace_until: Option<f64>,
    cap_reached: bool,
}

impl PageLog {
    pub(crate) fn new(max_views: usize, manual: bool) -> Self {
        Self {
            documents: Vec::new(),
            views: Vec::new(),
            max_views,
            manual,
            announced: None,
            opening: false,
            replace_until: None,
            cap_reached: false,
        }
    }

    /// The scripted visitor is about to click this link; the page view that
    /// follows was reached through it.
    pub(crate) fn announce(&mut self, link: FollowedLink) {
        self.announced = Some(link);
    }

    /// The announced click led nowhere.
    pub(crate) fn withdraw_announcement(&mut self) {
        self.announced = None;
    }

    /// The current page view finished opening: a document that commits from
    /// now on is a page view of its own.
    pub(crate) fn settled(&mut self) {
        self.opening = false;
    }

    /// Whether the current page view is still loading its first document.
    pub(crate) fn is_opening(&self) -> bool {
        self.opening
    }

    pub(crate) fn consent_clicked(&mut self, now: f64) {
        self.replace_until = Some(now + CONSENT_RELOAD_WINDOW_MS);
    }

    /// The main frame committed a document.
    pub(crate) fn document_committed(&mut self, committed: Committed<'_>) -> Change {
        let document = self.documents.len();
        self.documents.push(Document {
            time_origin: None,
            requested_at: committed.requested_at,
            recorded: committed.on_site,
            overflow: false,
        });
        if !committed.on_site {
            return Change::LeftSite;
        }
        let in_consent_reload = self
            .replace_until
            .take()
            .is_some_and(|until| committed.now < until);
        let replaced = if self.opening || in_consent_reload {
            self.views.pop()
        } else {
            None
        };
        let was_replaced = replaced.is_some();
        let (reached_by, link) = match replaced {
            Some(replaced) => {
                // What the replaced document still sends is void too.
                self.documents[replaced.document].recorded = self
                    .views
                    .iter()
                    .any(|view| view.document == replaced.document);
                (replaced.reached_by, replaced.link)
            }
            None if self.views.len() >= self.max_views => {
                self.documents[document].overflow = true;
                self.cap_reached = true;
                return Change::OverCap;
            }
            None => self.reach_of_next_document(committed.url),
        };
        self.views.push(View {
            document,
            offset_ms: 0.0,
            url: committed.url.to_string(),
            title: None,
            reached_by,
            link,
        });
        self.opening = !self.manual;
        let index = self.last_index();
        if was_replaced {
            Change::Replaced(index)
        } else {
            Change::Opened(index)
        }
    }

    fn reach_of_next_document(&mut self, url: &str) -> (PageReach, Option<FollowedLink>) {
        let Some(current) = self.views.last() else {
            return (PageReach::Entry, None);
        };
        if let Some(link) = self.announced.take() {
            (PageReach::LinkClick, Some(link))
        } else if !differs_beyond_fragment(&current.url, url) {
            (PageReach::Reload, None)
        } else if self.manual {
            (PageReach::UserNavigation, None)
        } else {
            (PageReach::Redirect, None)
        }
    }

    /// The tracker of `document` reported a same-document navigation `t` ms
    /// into it. `navigation_type` is the Navigation API's: "push", "replace",
    /// "traverse" or "reload".
    pub(crate) fn route_changed(
        &mut self,
        document: usize,
        t: f64,
        url: &str,
        navigation_type: &str,
    ) -> Change {
        let Some(current) = self.view_at(document, t) else {
            return Change::None;
        };
        // An event that arrives after the visit moved on changes nothing.
        if current != self.views.len() - 1 {
            return Change::None;
        }
        let is_new_page = matches!(navigation_type, "push" | "traverse")
            && differs_beyond_fragment(&self.views[current].url, url);
        if !is_new_page {
            if navigation_type == "replace" {
                self.views[current].url = url.to_string();
            }
            return Change::None;
        }
        if self.views.len() >= self.max_views {
            self.cap_reached = true;
            return Change::OverCap;
        }
        let link = self.announced.take();
        self.views.push(View {
            document,
            offset_ms: t,
            url: url.to_string(),
            title: None,
            reached_by: PageReach::RouteChange,
            link,
        });
        Change::Opened(self.last_index())
    }

    /// The document a tracker batch with this time origin comes from. A
    /// document's origin is not known when it commits; the first batch that
    /// states an unknown one binds it to the document requested closest to it.
    pub(crate) fn document_of(&mut self, time_origin: f64) -> Option<usize> {
        let known = self
            .documents
            .iter()
            .position(|document| document.time_origin == Some(time_origin));
        if known.is_some() {
            return known;
        }
        let nearest = self
            .documents
            .iter()
            .enumerate()
            .filter(|(_, document)| document.time_origin.is_none())
            .min_by(|(_, a), (_, b)| {
                let distance = |document: &Document| (document.requested_at - time_origin).abs();
                distance(a).total_cmp(&distance(b))
            })
            .map(|(index, _)| index)?;
        self.documents[nearest].time_origin = Some(time_origin);
        Some(nearest)
    }

    /// The page view of something that happened `t` ms into `document`, and
    /// the same moment counted from the start of that page view. `None` when
    /// nothing of that document is recorded.
    pub(crate) fn place(&self, document: usize, t: f64) -> Option<(u32, f64)> {
        let source = self.documents.get(document)?;
        if !source.recorded {
            return None;
        }
        if source.overflow {
            let last = self.views.len().checked_sub(1)?;
            let since_last_started = source.started_at() - self.started_at(last);
            return Some((last as u32, t + since_last_started));
        }
        let view = self.view_at(document, t)?;
        Some((view as u32, t - self.views[view].offset_ms))
    }

    /// The page view of `document` that was showing `t` ms into it.
    fn view_at(&self, document: usize, t: f64) -> Option<usize> {
        let of_document = || {
            self.views
                .iter()
                .enumerate()
                .filter(move |(_, view)| view.document == document)
        };
        of_document()
            .filter(|(_, view)| view.offset_ms <= t)
            .last()
            .or_else(|| of_document().next())
            .map(|(index, _)| index)
    }

    /// The page view that was showing at `epoch_ms`, and that moment counted
    /// from its start. For what the network reports rather than the tracker.
    pub(crate) fn place_at(&self, epoch_ms: f64) -> (u32, f64) {
        let index = (0..self.views.len())
            .rev()
            .find(|index| self.started_at(*index) <= epoch_ms)
            .unwrap_or(0);
        let started_at = if self.views.is_empty() {
            epoch_ms
        } else {
            self.started_at(index)
        };
        (index as u32, epoch_ms - started_at)
    }

    /// Epoch ms at which a page view started.
    fn started_at(&self, view: usize) -> f64 {
        let view = &self.views[view];
        self.documents[view.document].started_at() + view.offset_ms
    }

    /// Epoch ms every time of the capture is in the end measured from: the
    /// start of the entry page.
    pub(crate) fn navigation_start(&self) -> Option<f64> {
        (!self.views.is_empty()).then(|| self.started_at(0))
    }

    fn last_index(&self) -> u32 {
        (self.views.len() - 1) as u32
    }

    /// Index of the page view the visit is on.
    pub(crate) fn current(&self) -> Option<u32> {
        (!self.views.is_empty()).then(|| self.last_index())
    }

    pub(crate) fn len(&self) -> usize {
        self.views.len()
    }

    pub(crate) fn url(&self, index: u32) -> Option<&str> {
        self.views.get(index as usize).map(|view| view.url.as_str())
    }

    pub(crate) fn reached_by(&self, index: u32) -> Option<PageReach> {
        self.views.get(index as usize).map(|view| view.reached_by)
    }

    pub(crate) fn link(&self, index: u32) -> Option<&FollowedLink> {
        self.views.get(index as usize)?.link.as_ref()
    }

    /// ms from the start of the visit to the start of a page view.
    pub(crate) fn started_at_ms(&self, index: u32) -> f64 {
        if index == 0 || index as usize >= self.views.len() {
            return 0.0;
        }
        self.started_at(index as usize) - self.started_at(0)
    }

    pub(crate) fn has_title(&self, index: u32) -> bool {
        self.views
            .get(index as usize)
            .is_some_and(|view| view.title.is_some())
    }

    pub(crate) fn set_title(&mut self, index: u32, title: String) {
        if let Some(view) = self.views.get_mut(index as usize) {
            view.title = Some(title);
        }
    }

    /// True once, the first time a page view was refused for the cap.
    pub(crate) fn take_cap_notice(&mut self) -> bool {
        std::mem::take(&mut self.cap_reached)
    }

    /// The page views as the capture reports them. `ended_at` is the epoch ms
    /// at which recording stopped.
    pub(crate) fn into_captured(
        self,
        ended_at: f64,
        facts: impl Fn(u32) -> ViewFacts,
    ) -> Vec<CapturedPage> {
        let Some(navigation_start) = self.navigation_start() else {
            return Vec::new();
        };
        let starts: Vec<f64> = (0..self.views.len())
            .map(|index| self.started_at_ms(index as u32))
            .collect();
        let ended_at_ms = (ended_at - navigation_start).max(*starts.last().unwrap_or(&0.0));
        self.views
            .into_iter()
            .enumerate()
            .map(|(index, view)| {
                let facts = facts(index as u32);
                CapturedPage {
                    url: view.url,
                    title: view.title,
                    started_at_ms: starts[index],
                    ended_at_ms: starts.get(index + 1).copied().unwrap_or(ended_at_ms),
                    // A route change loads no document.
                    load_ms: facts
                        .load_ms
                        .filter(|_| view.reached_by != PageReach::RouteChange),
                    reached_by: view.reached_by,
                    link: view.link,
                    max_scroll_depth: facts.max_scroll_depth,
                }
            })
            .collect()
    }
}

/// Whether two addresses are different pages: a fragment alone is a
/// position on the same page.
fn differs_beyond_fragment(current: &str, next: &str) -> bool {
    let without_fragment = |address: &str| match Url::parse(address) {
        Ok(mut url) => {
            url.set_fragment(None);
            String::from(url)
        }
        Err(_) => address.split('#').next().unwrap_or_default().to_string(),
    };
    without_fragment(current) != without_fragment(next)
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: f64 = 1_700_000_000_000.0;

    fn committed(url: &str, requested_at: f64) -> Committed<'_> {
        Committed {
            url,
            requested_at,
            on_site: true,
            now: requested_at + 50.0,
        }
    }

    fn link(text: &str) -> FollowedLink {
        FollowedLink {
            text: text.into(),
            href: "https://news.example/b".into(),
            reason: "in the main content".into(),
        }
    }

    /// A scripted visit that has loaded its entry page.
    fn on_entry_page() -> PageLog {
        let mut log = PageLog::new(8, false);
        assert_eq!(
            log.document_committed(committed("https://news.example/a", T0)),
            Change::Opened(0)
        );
        assert_eq!(log.document_of(T0 + 3.0), Some(0));
        log.settled();
        log
    }

    fn pages(log: PageLog, ended_at: f64) -> Vec<CapturedPage> {
        log.into_captured(ended_at, |_| ViewFacts::default())
    }

    #[test]
    fn the_first_batch_binds_its_time_origin_and_refines_the_start() {
        let mut log = PageLog::new(8, false);
        log.document_committed(committed("https://news.example/a", T0));
        assert_eq!(log.navigation_start(), Some(T0), "provisional");

        assert_eq!(log.document_of(T0 + 3.5), Some(0));
        assert_eq!(log.document_of(T0 + 3.5), Some(0), "known from now on");

        assert_eq!(log.navigation_start(), Some(T0 + 3.5));
        assert_eq!(log.place(0, 120.0), Some((0, 120.0)));
        assert_eq!(log.document_of(T0 + 9_000.0), None, "no document is waiting");
    }

    #[test]
    fn an_unknown_origin_binds_to_the_document_requested_closest_to_it() {
        let mut log = on_entry_page();
        log.announce(link("B"));
        log.document_committed(committed("https://news.example/b", T0 + 20_000.0));
        log.settled();
        log.document_committed(committed("https://news.example/c", T0 + 40_000.0));

        // The last page's batch arrives before the one of the page before it.
        assert_eq!(log.document_of(T0 + 40_004.0), Some(2));
        assert_eq!(log.document_of(T0 + 20_002.0), Some(1));
        assert_eq!(log.place(1, 10.0), Some((1, 10.0)));
        assert_eq!(log.place(2, 10.0), Some((2, 10.0)));
    }

    #[test]
    fn a_document_that_commits_while_the_page_is_opening_replaces_it() {
        let mut log = PageLog::new(8, false);
        log.document_committed(committed("http://news.example/", T0));
        log.document_of(T0 + 1.0);

        // Redirected by script before the page finished loading.
        assert_eq!(
            log.document_committed(committed("https://news.example/home", T0 + 400.0)),
            Change::Replaced(0)
        );
        log.document_of(T0 + 401.0);
        log.settled();

        assert_eq!(log.len(), 1);
        assert_eq!(log.url(0), Some("https://news.example/home"));
        assert_eq!(log.reached_by(0), Some(PageReach::Entry));
        assert_eq!(log.navigation_start(), Some(T0 + 401.0));
        assert_eq!(log.place(0, 5.0), None, "the replaced document is void");
        assert_eq!(log.place(1, 5.0), Some((0, 5.0)));
    }

    #[test]
    fn a_reload_right_after_the_consent_click_replaces_and_a_later_one_does_not() {
        let mut log = on_entry_page();
        log.consent_clicked(T0 + 2_000.0);
        let reload = Committed {
            now: T0 + 3_500.0,
            ..committed("https://news.example/a", T0 + 3_400.0)
        };
        assert_eq!(log.document_committed(reload), Change::Replaced(0));
        log.settled();

        let much_later = committed("https://news.example/a", T0 + 30_000.0);
        assert_eq!(log.document_committed(much_later), Change::Opened(1));
        assert_eq!(log.reached_by(1), Some(PageReach::Reload));

        let mut slow = on_entry_page();
        slow.consent_clicked(T0 + 2_000.0);
        let late = Committed {
            now: T0 + 5_100.0,
            ..committed("https://news.example/a", T0 + 5_000.0)
        };
        assert_eq!(slow.document_committed(late), Change::Opened(1));
    }

    #[test]
    fn how_a_page_view_was_reached_is_recorded() {
        let mut log = on_entry_page();
        assert_eq!(log.reached_by(0), Some(PageReach::Entry));

        log.announce(link("B"));
        log.document_committed(committed("https://news.example/b", T0 + 10_000.0));
        log.settled();
        assert_eq!(log.reached_by(1), Some(PageReach::LinkClick));
        assert_eq!(log.link(1), Some(&link("B")));

        log.document_committed(committed("https://news.example/c", T0 + 20_000.0));
        log.settled();
        assert_eq!(log.reached_by(2), Some(PageReach::Redirect));
        assert_eq!(log.link(2), None);

        log.document_committed(committed("https://news.example/c#top", T0 + 30_000.0));
        assert_eq!(log.reached_by(3), Some(PageReach::Reload));

        let mut manual = PageLog::new(25, true);
        manual.document_committed(committed("https://news.example/a", T0));
        manual.document_committed(committed("https://news.example/b", T0 + 900.0));
        manual.document_committed(committed("https://news.example/b", T0 + 5_000.0));
        assert_eq!(manual.reached_by(0), Some(PageReach::Entry));
        assert_eq!(manual.reached_by(1), Some(PageReach::UserNavigation));
        assert_eq!(manual.reached_by(2), Some(PageReach::Reload));
    }

    #[test]
    fn a_link_that_was_withdrawn_is_not_credited_with_the_next_page() {
        let mut log = on_entry_page();
        log.announce(link("dead"));
        log.withdraw_announcement();
        log.document_committed(committed("https://news.example/b", T0 + 9_000.0));
        assert_eq!(log.reached_by(1), Some(PageReach::Redirect));
    }

    #[test]
    fn a_route_change_opens_a_page_view_inside_the_document() {
        let mut log = on_entry_page();
        log.announce(link("Kelp"));
        let opened = log.route_changed(0, 8_000.0, "https://news.example/kelp", "push");
        assert_eq!(opened, Change::Opened(1));
        assert_eq!(log.reached_by(1), Some(PageReach::RouteChange));
        assert_eq!(log.link(1), Some(&link("Kelp")));
        assert_eq!(log.started_at_ms(1), 8_000.0);

        // Timestamps decide, not arrival: a slot defined before the route
        // changed is reported late and still belongs to the first page view.
        assert_eq!(log.place(0, 7_990.0), Some((0, 7_990.0)));
        assert_eq!(log.place(0, 8_000.0), Some((1, 0.0)));
        assert_eq!(log.place(0, 8_250.5), Some((1, 250.5)));

        let back = log.route_changed(0, 12_000.0, "https://news.example/a", "traverse");
        assert_eq!(back, Change::Opened(2));
        assert_eq!(log.place(0, 12_100.0), Some((2, 100.0)));
    }

    #[test]
    fn fragments_replaced_addresses_and_stale_events_open_nothing() {
        let mut log = on_entry_page();
        let unchanged = [
            ("https://news.example/a#comments", "push"),
            ("https://news.example/a", "push"),
            ("https://news.example/a?utm=x", "reload"),
        ];
        for (url, navigation_type) in unchanged {
            assert_eq!(
                log.route_changed(0, 1_000.0, url, navigation_type),
                Change::None,
                "{url}"
            );
            assert_eq!(log.url(0), Some("https://news.example/a"));
        }
        assert_eq!(
            log.route_changed(0, 1_500.0, "https://news.example/a?ref=canonical", "replace"),
            Change::None
        );
        assert_eq!(log.url(0), Some("https://news.example/a?ref=canonical"));
        assert_eq!(log.len(), 1);

        log.document_committed(committed("https://news.example/b", T0 + 5_000.0));
        let stale = log.route_changed(0, 4_900.0, "https://news.example/x", "push");
        assert_eq!(stale, Change::None, "the visit is on another document");
        assert_eq!(log.route_changed(7, 1.0, "https://news.example/x", "push"), Change::None);
    }

    #[test]
    fn a_document_outside_the_site_is_not_a_page_view_and_records_nothing() {
        let mut log = on_entry_page();
        let elsewhere = Committed {
            on_site: false,
            ..committed("https://partner.example/", T0 + 6_000.0)
        };
        assert_eq!(log.document_committed(elsewhere), Change::LeftSite);
        assert_eq!(log.len(), 1);
        assert_eq!(log.document_of(T0 + 6_010.0), Some(1));
        assert_eq!(log.place(1, 30.0), None);

        log.document_committed(committed("https://news.example/back", T0 + 9_000.0));
        assert_eq!(log.reached_by(1), Some(PageReach::Redirect));
    }

    #[test]
    fn past_the_cap_everything_stays_on_the_last_page_view() {
        let mut log = PageLog::new(2, false);
        log.document_committed(committed("https://news.example/a", T0));
        log.document_of(T0);
        log.settled();
        log.route_changed(0, 4_000.0, "https://news.example/b", "push");
        assert!(!log.take_cap_notice());

        assert_eq!(
            log.route_changed(0, 6_000.0, "https://news.example/c", "push"),
            Change::OverCap
        );
        assert!(log.take_cap_notice());
        assert!(!log.take_cap_notice(), "said once");
        assert_eq!(log.place(0, 6_500.0), Some((1, 2_500.0)));

        assert_eq!(
            log.document_committed(committed("https://news.example/d", T0 + 10_000.0)),
            Change::OverCap
        );
        assert_eq!(log.document_of(T0 + 10_000.0), Some(1));
        assert_eq!(log.place(1, 250.0), Some((1, 6_250.0)));
        assert_eq!(log.len(), 2);
    }

    #[test]
    fn what_the_network_reports_is_placed_by_wall_clock() {
        let mut log = on_entry_page();
        log.route_changed(0, 8_000.0, "https://news.example/kelp", "push");
        log.document_committed(committed("https://news.example/b", T0 + 20_000.0));
        log.document_of(T0 + 20_001.0);

        assert_eq!(log.place_at(T0 + 3.0 + 500.0), (0, 500.0));
        assert_eq!(log.place_at(T0 + 3.0 + 9_000.0), (1, 1_000.0));
        assert_eq!(log.place_at(T0 + 20_001.0 + 70.0), (2, 70.0));
        assert_eq!(log.place_at(T0 - 100.0), (0, -103.0), "before the visit");
        assert_eq!(PageLog::new(8, false).place_at(T0), (0, 0.0));
    }

    #[test]
    fn page_views_are_reported_with_their_span_and_facts() {
        let mut log = on_entry_page();
        log.set_title(0, "Tide town".into());
        log.announce(link("Kelp"));
        log.route_changed(0, 8_000.0, "https://news.example/kelp", "push");
        log.document_committed(committed("https://news.example/b", T0 + 20_000.0));
        log.document_of(T0 + 20_003.0);
        assert!(log.has_title(0) && !log.has_title(1));
        assert_eq!(log.current(), Some(2));

        let pages = log.into_captured(T0 + 31_003.0, |index| ViewFacts {
            load_ms: Some(400.0 + f64::from(index)),
            max_scroll_depth: 0.25 * f64::from(index),
        });

        assert_eq!(pages.len(), 3);
        assert_eq!(pages[0].title.as_deref(), Some("Tide town"));
        assert_eq!((pages[0].started_at_ms, pages[0].ended_at_ms), (0.0, 8_000.0));
        assert_eq!((pages[1].started_at_ms, pages[1].ended_at_ms), (8_000.0, 20_000.0));
        assert_eq!((pages[2].started_at_ms, pages[2].ended_at_ms), (20_000.0, 31_000.0));
        assert_eq!(pages[0].load_ms, Some(400.0));
        assert_eq!(pages[1].load_ms, None, "a route change loads no document");
        assert_eq!(pages[2].load_ms, Some(402.0));
        assert_eq!(pages[2].max_scroll_depth, 0.5);
        assert_eq!(pages[1].link, Some(link("Kelp")));
        assert_eq!(pages[1].reached_by, PageReach::RouteChange);
    }

    #[test]
    fn a_visit_that_never_committed_a_document_has_no_page_views() {
        assert!(pages(PageLog::new(8, false), T0).is_empty());
        assert_eq!(PageLog::new(8, false).current(), None);
    }
}
