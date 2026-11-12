//! A request as the network view shows it, put together from its index row
//! and, while the pack is there, its document.

use std::collections::HashMap;

use adscan_model::{
    InitiatorRef, NetworkRow, RequestOutcome, RowPhases, ScriptRecord, ServedFrom, StackFrame,
    TimingDetail, request_id,
};

use super::classify;
use crate::domain::archive::RequestDocument;
use crate::domain::archive::document::{StoredInitiator, StoredTiming};
use crate::domain::archive::index::{RequestEndKind, RequestIndexRow};
use crate::domain::scripts::stored_url;

/// The scan's scripts by the two things a request can know of one: its URL
/// and its content.
pub struct ScriptIndex<'a> {
    by_url: HashMap<&'a str, &'a str>,
    by_sha256: HashMap<&'a str, &'a str>,
}

impl<'a> ScriptIndex<'a> {
    pub fn new(scripts: &'a [ScriptRecord]) -> Self {
        let mut by_url = HashMap::new();
        let mut by_sha256 = HashMap::new();
        for script in scripts {
            // A URL loaded twice names its first load; so does the viewer.
            by_url
                .entry(script.url.as_str())
                .or_insert(script.id.as_str());
            if let Some(sha256) = &script.sha256 {
                by_sha256
                    .entry(sha256.as_str())
                    .or_insert(script.id.as_str());
            }
        }
        Self { by_url, by_sha256 }
    }

    /// `url` as the capture reported it; script records hold it as stored.
    pub fn id_of_url(&self, url: &str) -> Option<String> {
        self.by_url
            .get(stored_url(url).as_str())
            .map(|id| (*id).to_string())
    }

    pub fn id_of_content(&self, sha256: &str) -> Option<String> {
        self.by_sha256.get(sha256).map(|id| (*id).to_string())
    }
}

pub fn row(
    index: &RequestIndexRow,
    document: Option<&RequestDocument>,
    scripts: &ScriptIndex,
) -> NetworkRow {
    let url = match document {
        Some(document) => document.full_url.clone(),
        // The query string went with the pack.
        None if index.host.is_empty() => format!("{}:", index.scheme),
        None => format!("{}://{}{}", index.scheme, index.host, index.path),
    };
    let initiator = match document {
        Some(document) => initiator_ref(&document.initiator, scripts),
        None => InitiatorRef {
            kind: index.initiator_kind,
            url: None,
            script_id: None,
            line: None,
            column: None,
        },
    };
    NetworkRow {
        id: request_id(index.seq),
        seq: index.seq,
        page: index.page,
        context: index.context,
        method: document.map_or_else(|| index.method.clone(), |document| document.method.clone()),
        url,
        host: index.host.clone(),
        kind: index.kind,
        mime: index.mime.clone(),
        status: index.status,
        outcome: outcome(index),
        error: index.error.clone(),
        category: index.category,
        vendor: classify::vendor(&index.host).map(str::to_string),
        bidder: index.bidder.clone(),
        first_party: !index.third_party,
        initiator: Some(initiator),
        started_at_ms: index.started_ms,
        duration_ms: index.duration_ms,
        phases: document.and_then(|document| phases(document, index.duration_ms)),
        transferred_bytes: if index.served_from == ServedFrom::Network {
            index.encoded_bytes as f64
        } else {
            0.0
        },
        decoded_bytes: index.decoded_bytes as f64,
        served_from: index.served_from,
        redirect_of: index.redirect_from.map(request_id),
        preflight_of: index.preflight_of.map(request_id),
        has_request_body: index.post_bytes > 0,
        has_response_body: index.body_no.is_some(),
        no_hb_targeting: index.no_hb_targeting,
        ad: index.ad.clone(),
    }
}

pub fn outcome(index: &RequestIndexRow) -> RequestOutcome {
    match index.end {
        RequestEndKind::Finished | RequestEndKind::Redirected => {
            if index.status.is_some_and(|status| status >= 400) {
                RequestOutcome::HttpError
            } else {
                RequestOutcome::Ok
            }
        }
        RequestEndKind::Failed => RequestOutcome::Failed,
        RequestEndKind::Blocked => RequestOutcome::Blocked,
        RequestEndKind::Cancelled => RequestOutcome::Cancelled,
        RequestEndKind::Pending => RequestOutcome::Pending,
    }
}

/// The innermost frame of the stack, or what the browser named instead of
/// one. Lines and columns become 1-based here.
fn initiator_ref(initiator: &StoredInitiator, scripts: &ScriptIndex) -> InitiatorRef {
    match initiator.stack.first() {
        Some(frame) => InitiatorRef {
            kind: initiator.kind,
            url: Some(frame.url.clone()).filter(|url| !url.is_empty()),
            script_id: script_of(frame.script_url.as_deref(), &frame.url, scripts),
            line: Some(frame.line + 1),
            column: Some(frame.column + 1),
        },
        None => InitiatorRef {
            kind: initiator.kind,
            script_id: initiator
                .url
                .as_deref()
                .and_then(|url| scripts.id_of_url(url)),
            url: initiator.url.clone(),
            line: initiator.line.map(|line| line + 1),
            column: initiator.column.map(|column| column + 1),
        },
    }
}

pub fn stack(initiator: &StoredInitiator, scripts: &ScriptIndex) -> Vec<StackFrame> {
    initiator
        .stack
        .iter()
        .map(|frame| StackFrame {
            function: frame.function.clone().filter(|name| !name.is_empty()),
            url: frame.url.clone(),
            script_id: script_of(frame.script_url.as_deref(), &frame.url, scripts),
            line: frame.line + 1,
            column: frame.column + 1,
        })
        .collect()
}

fn script_of(script_url: Option<&str>, url: &str, scripts: &ScriptIndex) -> Option<String> {
    scripts
        .id_of_url(script_url.unwrap_or(url))
        .filter(|_| script_url.is_some() || !url.is_empty())
}

fn phases(document: &RequestDocument, duration_ms: Option<f64>) -> Option<RowPhases> {
    let timing = document.timing?;
    let queued = document.queued_ms.unwrap_or(0.0).max(0.0);
    let connection_start = earliest(&[timing.dns_start, timing.connect_start, timing.ssl_start]);
    let connection_end = latest(&[timing.dns_end, timing.connect_end, timing.ssl_end]);
    let send_start = timing.send_start.unwrap_or(0.0);
    let connect = match (connection_start, connection_end) {
        (Some(start), Some(end)) => (end - start).max(0.0),
        _ => 0.0,
    };
    // Whatever passed before the request went out and was not spent
    // connecting was spent waiting for a turn.
    let stalled = (send_start - connect).max(0.0);
    let headers_end = timing.receive_headers_end.unwrap_or(send_start);
    let wait = (headers_end - timing.send_end.unwrap_or(send_start)).max(0.0);
    let receive = duration_ms.map_or(0.0, |duration| (duration - queued - headers_end).max(0.0));
    Some(RowPhases {
        queue: queued + stalled,
        connect,
        wait,
        receive,
    })
}

pub fn timing_detail(document: &RequestDocument, duration_ms: Option<f64>) -> Option<TimingDetail> {
    let timing: StoredTiming = document.timing?;
    let span = |start: Option<f64>, end: Option<f64>| match (start, end) {
        (Some(start), Some(end)) => Some((end - start).max(0.0)),
        _ => None,
    };
    let queued = document.queued_ms.map(|queued| queued.max(0.0));
    Some(TimingDetail {
        queued_ms: queued,
        dns_ms: span(timing.dns_start, timing.dns_end),
        connect_ms: span(timing.connect_start, timing.connect_end),
        tls_ms: span(timing.ssl_start, timing.ssl_end),
        send_ms: span(timing.send_start, timing.send_end),
        wait_ms: span(timing.send_end, timing.receive_headers_end),
        receive_ms: match (duration_ms, timing.receive_headers_end) {
            (Some(duration), Some(headers_end)) => {
                Some((duration - queued.unwrap_or(0.0) - headers_end).max(0.0))
            }
            _ => None,
        },
        response_at_ms: document.response_ms,
        finished_at_ms: document.ended_ms,
    })
}

fn earliest(values: &[Option<f64>]) -> Option<f64> {
    values.iter().flatten().copied().reduce(f64::min)
}

fn latest(values: &[Option<f64>]) -> Option<f64> {
    values.iter().flatten().copied().reduce(f64::max)
}
