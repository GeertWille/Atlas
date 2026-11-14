//! Parses the batches the injected tracker sends and puts every event on its
//! page view. No I/O.

use std::ops::Range;

use adscan_model::{TrackerEvent, TrackerEventKind};
use serde::Deserialize;
use serde_json::Value;

use super::pages::{Change, PageLog, ViewFacts};

/// A page stuck in an event loop must not exhaust memory.
const MAX_EVENTS: usize = 50_000;
const MAX_EVENTS_PER_PAGE: usize = 20_000;

/// Wire format of one tracker flush: the document's `performance.timeOrigin`
/// and events whose `t` is relative to it.
#[derive(Deserialize)]
struct Batch {
    #[serde(rename = "o")]
    time_origin: f64,
    #[serde(rename = "e")]
    events: Vec<Value>,
}

/// What one batch added.
pub(crate) struct Ingested {
    /// Index range of the events it contributed.
    pub events: Range<usize>,
    /// What its route changes did to the page views.
    pub changes: Vec<Change>,
}

#[derive(Debug, Default)]
pub(crate) struct TrackerLog {
    /// In arrival order; `t` already counts from the start of `page`.
    events: Vec<TrackerEvent>,
    per_page: Vec<usize>,
    malformed: usize,
}

impl TrackerLog {
    /// Adds a batch. Events of a document that is not recorded (another
    /// site, a page view that was replaced) are left out.
    pub(crate) fn ingest(&mut self, payload: &str, pages: &mut PageLog) -> Ingested {
        let first_new = self.events.len();
        let mut changes = Vec::new();
        let Ok(batch) = serde_json::from_str::<Batch>(payload) else {
            self.malformed += 1;
            return Ingested {
                events: first_new..first_new,
                changes,
            };
        };
        let document = pages.document_of(batch.time_origin);
        for raw in batch.events {
            let mut event = match serde_json::from_value::<TrackerEvent>(raw) {
                Ok(event) if event.t.is_finite() => event,
                _ => {
                    self.malformed += 1;
                    continue;
                }
            };
            let Some(document) = document else { continue };
            if let TrackerEventKind::RouteChange {
                url,
                navigation_type,
            } = &event.kind
            {
                changes.push(pages.route_changed(document, event.t, url, navigation_type));
            }
            let Some((page, t)) = pages.place(document, event.t) else {
                continue;
            };
            let index = page as usize;
            if self.per_page.len() <= index {
                self.per_page.resize(index + 1, 0);
            }
            if self.events.len() >= MAX_EVENTS || self.per_page[index] >= MAX_EVENTS_PER_PAGE {
                continue;
            }
            self.per_page[index] += 1;
            if let TrackerEventKind::BidsRequested {
                released_at: Some(released_at),
                ..
            } = &mut event.kind
            {
                // On the clock of the page view the call was made on.
                *released_at = (*released_at + t - event.t).max(t);
            }
            event.page = page;
            event.t = t;
            self.events.push(event);
        }
        changes.retain(|change| *change != Change::None);
        Ingested {
            events: first_new..self.events.len(),
            changes,
        }
    }

    /// Forgets what was recorded for a page view that starts over.
    pub(crate) fn drop_page(&mut self, page: u32) {
        self.events.retain(|event| event.page != page);
        if let Some(count) = self.per_page.get_mut(page as usize) {
            *count = 0;
        }
    }

    pub(crate) fn events(&self) -> &[TrackerEvent] {
        &self.events
    }

    /// Events that did not match the `TrackerEventKind` contract.
    pub(crate) fn malformed(&self) -> usize {
        self.malformed
    }

    /// Load time and scroll depth of one page view, as its events tell them.
    pub(crate) fn facts(&self, page: u32) -> ViewFacts {
        let of_page = || self.events.iter().filter(move |event| event.page == page);
        ViewFacts {
            load_ms: of_page().find_map(|event| match &event.kind {
                TrackerEventKind::PageLifecycle { phase } if phase == "load" => Some(event.t),
                _ => None,
            }),
            max_scroll_depth: of_page()
                .filter_map(|event| match event.kind {
                    TrackerEventKind::Scroll { depth } => Some(depth),
                    _ => None,
                })
                .fold(0.0, f64::max)
                .clamp(0.0, 1.0),
        }
    }

    /// The events in the order they happened: by page view, then by time.
    /// Events of one moment keep the order the tracker emitted them in.
    pub(crate) fn into_events(mut self) -> Vec<TrackerEvent> {
        self.events
            .sort_by(|a, b| a.page.cmp(&b.page).then(a.t.total_cmp(&b.t)));
        self.events
    }
}

#[cfg(test)]
mod tests {
    use super::super::pages::Committed;
    use super::*;

    const ORIGIN: f64 = 1_700_000_000_000.0;

    fn batch(origin: f64, events: &str) -> String {
        format!(r#"{{"o":{origin},"e":[{events}]}}"#)
    }

    fn commit(pages: &mut PageLog, url: &str, requested_at: f64) -> Change {
        pages.document_committed(Committed {
            url,
            requested_at,
            on_site: true,
            restored: false,
            now: requested_at,
        })
    }

    /// A visit whose entry page has loaded.
    fn visit() -> PageLog {
        let mut pages = PageLog::new(8, false);
        commit(&mut pages, "https://news.example/a", ORIGIN);
        pages.settled();
        pages
    }

    #[test]
    fn keeps_valid_events_with_their_own_timestamps() {
        let (mut log, mut pages) = (TrackerLog::default(), visit());
        let added = log.ingest(
            &batch(
                ORIGIN,
                r#"{"t":12.5,"type":"pageLifecycle","phase":"domContentLoaded"},
                   {"t":8,"type":"auctionEnd","auctionId":"a1"}"#,
            ),
            &mut pages,
        );
        assert_eq!(added.events, 0..2);
        assert!(added.changes.is_empty());
        assert_eq!((log.events()[0].t, log.events()[0].page), (12.5, 0));
        assert_eq!(
            log.events()[1].kind,
            TrackerEventKind::AuctionEnd {
                auction_id: "a1".into()
            }
        );
        assert_eq!(pages.navigation_start(), Some(ORIGIN));
        assert_eq!(log.malformed(), 0);
    }

    #[test]
    fn counts_events_that_break_the_contract_without_losing_their_neighbours() {
        let (mut log, mut pages) = (TrackerLog::default(), visit());
        let added = log.ingest(
            &batch(
                ORIGIN,
                r#"{"t":1,"type":"bidResponse","auctionId":"a","bidder":"x"},
                   {"t":2,"type":"notAThing"},
                   {"type":"scroll","depth":0.5},
                   {"t":3,"type":"scroll","depth":0.5}"#,
            ),
            &mut pages,
        );
        assert_eq!(added.events, 0..1);
        assert_eq!(log.malformed(), 3);
        assert_eq!(
            log.events()[0].kind,
            TrackerEventKind::Scroll { depth: 0.5 }
        );
    }

    #[test]
    fn unparsable_payload_is_one_malformed_entry() {
        let (mut log, mut pages) = (TrackerLog::default(), visit());
        assert_eq!(log.ingest("not json", &mut pages).events, 0..0);
        assert_eq!(log.ingest(r#"{"e":[]}"#, &mut pages).events, 0..0);
        assert_eq!(log.malformed(), 2);
    }

    #[test]
    fn a_document_that_replaces_the_opening_page_restarts_its_events() {
        let (mut log, mut pages) = (TrackerLog::default(), PageLog::new(8, false));
        commit(&mut pages, "https://news.example/a", ORIGIN);
        log.ingest(
            &batch(ORIGIN, r#"{"t":1,"type":"scroll","depth":0.9}"#),
            &mut pages,
        );

        assert_eq!(
            commit(&mut pages, "https://news.example/a", ORIGIN + 5_000.0),
            Change::Replaced(0)
        );
        log.drop_page(0);
        // The replaced document still flushes what it had buffered.
        log.ingest(
            &batch(ORIGIN, r#"{"t":2,"type":"scroll","depth":0.8}"#),
            &mut pages,
        );
        let added = log.ingest(
            &batch(ORIGIN + 5_000.0, r#"{"t":3,"type":"scroll","depth":0.1}"#),
            &mut pages,
        );

        assert_eq!(added.events, 0..1);
        assert_eq!(pages.navigation_start(), Some(ORIGIN + 5_000.0));
        assert_eq!(log.facts(0).max_scroll_depth, 0.1);
    }

    #[test]
    fn a_later_document_is_a_page_view_of_its_own_with_its_own_clock() {
        let (mut log, mut pages) = (TrackerLog::default(), visit());
        log.ingest(
            &batch(ORIGIN, r#"{"t":900,"type":"scroll","depth":0.9}"#),
            &mut pages,
        );
        assert_eq!(
            commit(&mut pages, "https://news.example/b", ORIGIN + 20_000.0),
            Change::Opened(1)
        );
        log.ingest(
            &batch(
                ORIGIN + 20_000.0,
                r#"{"t":35.5,"type":"pageLifecycle","phase":"load"},
                   {"t":40,"type":"scroll","depth":0.2}"#,
            ),
            &mut pages,
        );
        // Sent by the first page as it was left; arrives last.
        log.ingest(
            &batch(ORIGIN, r#"{"t":19990,"type":"scroll","depth":0.95}"#),
            &mut pages,
        );

        assert_eq!(log.facts(0).max_scroll_depth, 0.95);
        assert_eq!(
            log.facts(1),
            ViewFacts {
                load_ms: Some(35.5),
                max_scroll_depth: 0.2
            }
        );
        let order: Vec<(u32, f64)> = log
            .into_events()
            .iter()
            .map(|event| (event.page, event.t))
            .collect();
        assert_eq!(order, [(0, 900.0), (0, 19_990.0), (1, 35.5), (1, 40.0)]);
    }

    #[test]
    fn a_route_change_starts_a_page_view_and_later_events_count_from_it() {
        let (mut log, mut pages) = (TrackerLog::default(), visit());
        let added = log.ingest(
            &batch(
                ORIGIN,
                r#"{"t":7000,"type":"scroll","depth":0.5},
                   {"t":8000,"type":"routeChange","url":"https://news.example/kelp","navigationType":"push"},
                   {"t":8120.5,"type":"auctionEnd","auctionId":"a2"},
                   {"t":7950,"type":"slotDefined","elementId":"ad-top","adUnitPath":null,"sizes":[],"inViewport":true},
                   {"t":8200,"type":"routeChange","url":"https://news.example/kelp#photos","navigationType":"push"}"#,
            ),
            &mut pages,
        );

        assert_eq!(added.changes, [Change::Opened(1)]);
        let placed: Vec<(u32, f64)> = log
            .events()
            .iter()
            .map(|event| (event.page, event.t))
            .collect();
        assert_eq!(
            placed,
            [(0, 7_000.0), (1, 0.0), (1, 120.5), (0, 7_950.0), (1, 200.0)]
        );
        assert_eq!(pages.len(), 2);
    }

    #[test]
    fn a_bid_request_is_placed_by_its_call_and_its_release_counts_on_that_page_view() {
        let (mut log, mut pages) = (TrackerLog::default(), visit());
        log.ingest(
            &batch(
                ORIGIN,
                r#"{"t":8000,"type":"routeChange","url":"https://news.example/kelp","navigationType":"push"},
                   {"t":7900,"type":"bidsRequested","adUnitCodes":[],"adUnitCount":2,"timeout":null,"releasedAt":8300},
                   {"t":8100,"type":"bidsRequested","adUnitCodes":[],"adUnitCount":2,"timeout":null,"releasedAt":9600.5},
                   {"t":8200,"type":"bidsRequested","adUnitCodes":[],"adUnitCount":2,"timeout":null,"releasedAt":null,"stillHeld":true}"#,
            ),
            &mut pages,
        );

        let placed: Vec<(u32, f64, Option<f64>, bool)> = log
            .events()
            .iter()
            .filter_map(|event| match event.kind {
                TrackerEventKind::BidsRequested {
                    released_at,
                    still_held,
                    ..
                } => Some((event.page, event.t, released_at, still_held)),
                _ => None,
            })
            .collect();
        assert_eq!(
            placed,
            [
                // Made before the route change, let through after it.
                (0, 7_900.0, Some(8_300.0), false),
                (1, 100.0, Some(1_600.5), false),
                (1, 200.0, None, true),
            ]
        );
    }

    #[test]
    fn events_of_another_site_or_an_unknown_document_are_left_out() {
        let (mut log, mut pages) = (TrackerLog::default(), visit());
        log.ingest(
            &batch(ORIGIN, r#"{"t":1,"type":"scroll","depth":0.1}"#),
            &mut pages,
        );
        pages.document_committed(Committed {
            url: "https://partner.example/",
            requested_at: ORIGIN + 9_000.0,
            on_site: false,
            restored: false,
            now: ORIGIN + 9_000.0,
        });

        let elsewhere = batch(ORIGIN + 9_000.0, r#"{"t":5,"type":"scroll","depth":0.7}"#);
        assert_eq!(log.ingest(&elsewhere, &mut pages).events, 1..1);
        let nowhere = batch(ORIGIN + 50_000.0, r#"{"t":5,"type":"scroll","depth":0.7}"#);
        assert_eq!(log.ingest(&nowhere, &mut pages).events, 1..1);
        assert_eq!(log.malformed(), 0);
    }

    #[test]
    fn derives_load_time_and_clamped_scroll_depth() {
        let (mut log, mut pages) = (TrackerLog::default(), visit());
        log.ingest(
            &batch(
                ORIGIN,
                r#"{"t":40,"type":"pageLifecycle","phase":"domContentLoaded"},
                   {"t":95.5,"type":"pageLifecycle","phase":"load"},
                   {"t":100,"type":"scroll","depth":0.4},
                   {"t":200,"type":"scroll","depth":1.7},
                   {"t":300,"type":"scroll","depth":0.0}"#,
            ),
            &mut pages,
        );
        assert_eq!(
            log.facts(0),
            ViewFacts {
                load_ms: Some(95.5),
                max_scroll_depth: 1.0
            }
        );
        assert_eq!(TrackerLog::default().facts(0), ViewFacts::default());
    }

    #[test]
    fn stops_accepting_events_at_the_caps() {
        let (mut log, mut pages) = (TrackerLog::default(), PageLog::new(8, false));
        let events = vec![r#"{"t":1,"type":"scroll","depth":0.1}"#; 1_000].join(",");
        for page in 0..4 {
            let origin = ORIGIN + f64::from(page) * 10_000.0;
            commit(&mut pages, &format!("https://news.example/{page}"), origin);
            pages.settled();
            for _ in 0..(MAX_EVENTS_PER_PAGE / 1_000 + 2) {
                log.ingest(&batch(origin, &events), &mut pages);
            }
            let expected = (MAX_EVENTS_PER_PAGE * (page as usize + 1)).min(MAX_EVENTS);
            assert_eq!(log.events().len(), expected, "after page {page}");
        }
        assert_eq!(log.events().len(), MAX_EVENTS);
    }
}
