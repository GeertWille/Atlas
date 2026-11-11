//! Parses and normalises the batches the injected tracker sends. No I/O.

use std::ops::Range;

use adscan_model::{TrackerEvent, TrackerEventKind};
use serde::Deserialize;
use serde_json::Value;

/// A page stuck in an event loop must not exhaust memory.
const MAX_EVENTS: usize = 50_000;

/// Wire format of one tracker flush: the document's `performance.timeOrigin`
/// and events whose `t` is relative to it.
#[derive(Deserialize)]
struct Batch {
    #[serde(rename = "o")]
    time_origin: f64,
    #[serde(rename = "e")]
    events: Vec<Value>,
}

#[derive(Debug, Default)]
pub(crate) struct TrackerLog {
    time_origin: Option<f64>,
    events: Vec<TrackerEvent>,
    malformed: usize,
}

impl TrackerLog {
    /// Adds a batch and returns the index range of the events it contributed.
    ///
    /// A batch from a new document (reload after consent, client-side redirect)
    /// restarts the log: the scan describes the page the user ends up on, and
    /// every `t` stays relative to that document's navigation start.
    pub(crate) fn ingest(&mut self, payload: &str) -> Range<usize> {
        let Ok(batch) = serde_json::from_str::<Batch>(payload) else {
            self.malformed += 1;
            return self.events.len()..self.events.len();
        };
        if self.time_origin != Some(batch.time_origin) {
            self.time_origin = Some(batch.time_origin);
            self.events.clear();
        }
        let first_new = self.events.len();
        for raw in batch.events {
            if self.events.len() >= MAX_EVENTS {
                break;
            }
            match serde_json::from_value::<TrackerEvent>(raw) {
                Ok(event) if event.t.is_finite() => self.events.push(event),
                _ => self.malformed += 1,
            }
        }
        first_new..self.events.len()
    }

    pub(crate) fn events(&self) -> &[TrackerEvent] {
        &self.events
    }

    /// Unix epoch ms of the tracked document's navigation start.
    pub(crate) fn navigation_start(&self) -> Option<f64> {
        self.time_origin
    }

    /// Events that did not match the `TrackerEventKind` contract.
    pub(crate) fn malformed(&self) -> usize {
        self.malformed
    }

    pub(crate) fn page_load_ms(&self) -> Option<f64> {
        self.events.iter().find_map(|event| match &event.kind {
            TrackerEventKind::PageLifecycle { phase } if phase == "load" => Some(event.t),
            _ => None,
        })
    }

    pub(crate) fn max_scroll_depth(&self) -> f64 {
        self.events
            .iter()
            .filter_map(|event| match event.kind {
                TrackerEventKind::Scroll { depth } => Some(depth),
                _ => None,
            })
            .fold(0.0, f64::max)
            .clamp(0.0, 1.0)
    }

    pub(crate) fn into_events(self) -> Vec<TrackerEvent> {
        self.events
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn batch(origin: f64, events: &str) -> String {
        format!(r#"{{"o":{origin},"e":[{events}]}}"#)
    }

    #[test]
    fn keeps_valid_events_in_emission_order() {
        let mut log = TrackerLog::default();
        let added = log.ingest(&batch(
            1_000.0,
            r#"{"t":12.5,"type":"pageLifecycle","phase":"domContentLoaded"},
               {"t":8,"type":"auctionEnd","auctionId":"a1"}"#,
        ));
        assert_eq!(added, 0..2);
        assert_eq!(log.events()[0].t, 12.5);
        assert_eq!(
            log.events()[1].kind,
            TrackerEventKind::AuctionEnd { auction_id: "a1".into() }
        );
        assert_eq!(log.navigation_start(), Some(1_000.0));
        assert_eq!(log.malformed(), 0);
    }

    #[test]
    fn counts_events_that_break_the_contract_without_losing_their_neighbours() {
        let mut log = TrackerLog::default();
        let added = log.ingest(&batch(
            1.0,
            r#"{"t":1,"type":"bidResponse","auctionId":"a","bidder":"x"},
               {"t":2,"type":"notAThing"},
               {"type":"scroll","depth":0.5},
               {"t":3,"type":"scroll","depth":0.5}"#,
        ));
        assert_eq!(added, 0..1);
        assert_eq!(log.malformed(), 3);
        assert_eq!(log.events()[0].kind, TrackerEventKind::Scroll { depth: 0.5 });
    }

    #[test]
    fn unparsable_payload_is_one_malformed_entry() {
        let mut log = TrackerLog::default();
        assert_eq!(log.ingest("not json"), 0..0);
        assert_eq!(log.ingest(r#"{"e":[]}"#), 0..0);
        assert_eq!(log.malformed(), 2);
        assert_eq!(log.navigation_start(), None);
    }

    #[test]
    fn a_new_document_restarts_the_log() {
        let mut log = TrackerLog::default();
        log.ingest(&batch(100.0, r#"{"t":1,"type":"scroll","depth":0.9}"#));
        log.ingest(&batch(100.0, r#"{"t":2,"type":"scroll","depth":0.2}"#));
        assert_eq!(log.events().len(), 2);

        let added = log.ingest(&batch(5_000.0, r#"{"t":3,"type":"scroll","depth":0.1}"#));
        assert_eq!(added, 0..1);
        assert_eq!(log.navigation_start(), Some(5_000.0));
        assert_eq!(log.max_scroll_depth(), 0.1);
    }

    #[test]
    fn derives_load_time_and_clamped_scroll_depth() {
        let mut log = TrackerLog::default();
        log.ingest(&batch(
            1.0,
            r#"{"t":40,"type":"pageLifecycle","phase":"domContentLoaded"},
               {"t":95.5,"type":"pageLifecycle","phase":"load"},
               {"t":100,"type":"scroll","depth":0.4},
               {"t":200,"type":"scroll","depth":1.7},
               {"t":300,"type":"scroll","depth":0.0}"#,
        ));
        assert_eq!(log.page_load_ms(), Some(95.5));
        assert_eq!(log.max_scroll_depth(), 1.0);
        assert_eq!(TrackerLog::default().page_load_ms(), None);
        assert_eq!(TrackerLog::default().max_scroll_depth(), 0.0);
    }

    #[test]
    fn stops_accepting_events_at_the_cap() {
        let mut log = TrackerLog::default();
        let events = vec![r#"{"t":1,"type":"scroll","depth":0.1}"#; 1_000].join(",");
        for _ in 0..(MAX_EVENTS / 1_000 + 2) {
            log.ingest(&batch(1.0, &events));
        }
        assert_eq!(log.events().len(), MAX_EVENTS);
    }
}
