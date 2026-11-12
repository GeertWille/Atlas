//! What the engine reads out of the visit's requests while they stream past.
//!
//! This is the one place where a request is looked at with its bodies still
//! in memory, and it runs for every scan, whether or not the scan stores
//! network data. Today it keeps what the bid exchanges say; decoding of
//! ad-server calls belongs here too.

use adscan_model::{CapturedBody, NetworkCategory, NetworkRecord, RequestEnd, ResourceKind};

use crate::domain::archive::index::RequestIndexRow;
use crate::domain::network::classify::{is_ad_call, is_prebid_server};
use crate::domain::network::exchange::{AdServerCall, Answer, ExchangeRequest};
use crate::domain::network::openrtb;

/// A bid request's ids are near its start; more is not searched.
const MAX_HAYSTACK_BYTES: usize = 256 * 1024;

#[derive(Debug, Default)]
pub struct RecordAnalysis {
    /// Every bid request of the visit, for matching against Prebid's auctions.
    pub exchanges: Vec<ExchangeRequest>,
    /// Every ad-server ad call, for matching against the slots' requests.
    pub ad_server_calls: Vec<AdServerCall>,
}

impl RecordAnalysis {
    /// Whether the answer of this request is wanted in memory. Asked before
    /// the body is read; the scan's storage setting plays no part in it.
    pub fn wants_body(url: &str, bidder: Option<&str>) -> bool {
        url::Url::parse(url)
            .is_ok_and(|url| is_ad_call(url.host_str().unwrap_or_default(), url.path(), bidder))
    }

    /// Sees one finished request, body included. `row` is what the index
    /// would keep of it.
    pub fn observe(&mut self, record: &NetworkRecord, row: &RequestIndexRow) {
        // A preflight is filed with its request but is neither a bid request
        // nor an ad call.
        if record.kind == ResourceKind::Preflight {
            return;
        }
        match row.category {
            NetworkCategory::Bid => self.exchanges.push(exchange(record, row)),
            NetworkCategory::AdServer => self.ad_server_calls.push(AdServerCall {
                seq: record.seq,
                page: record.page,
                started_ms: record.started_at_ms,
            }),
            _ => {}
        }
    }
}

fn exchange(record: &NetworkRecord, row: &RequestIndexRow) -> ExchangeRequest {
    let post = record
        .post_data
        .as_ref()
        .map(|post| String::from_utf8_lossy(&post.bytes));
    let answer = match &record.body {
        CapturedBody::Read { bytes, .. } => Some(String::from_utf8_lossy(bytes)),
        _ => None,
    };
    let decoded = openrtb::decode(&row.path, post.as_deref(), answer.as_deref(), &|_| None);
    let response = decoded
        .as_ref()
        .and_then(|decoded| decoded.response.as_ref());
    let mut haystack = record.url.clone();
    let mut complete = !record.post_data_missing;
    if let Some(post) = &post {
        haystack.push('\n');
        let cut = (0..=post.len().min(MAX_HAYSTACK_BYTES))
            .rev()
            .find(|at| post.is_char_boundary(*at))
            .unwrap_or(0);
        haystack.push_str(&post[..cut]);
        let sent = record.post_data.as_ref().map_or(0, |post| post.total_bytes);
        // A body that is not text (compressed by the adapter) reads as
        // replacement characters: its ids cannot be seen either.
        complete &= cut == post.len() && post.len() as u64 == sent && !post.contains('\u{FFFD}');
    }
    ExchangeRequest {
        seq: record.seq,
        page: record.page,
        bidder: record.bidder.clone(),
        via_server: is_prebid_server(&row.path)
            || decoded.as_ref().is_some_and(|decoded| {
                decoded.protocol == adscan_model::ExchangeProtocol::PrebidServer
            }),
        started_ms: record.started_at_ms,
        ended_ms: record.ended_at_ms,
        answer: match record.end {
            RequestEnd::Finished | RequestEnd::Redirected => Answer::Answered,
            RequestEnd::Failed { .. } => Answer::Failed,
            RequestEnd::Pending => Answer::Pending,
        },
        haystack,
        complete,
        best_cpm: response.and_then(openrtb::best_price),
        currency: response.and_then(|response| response.currency.clone()),
        nbr: response.and_then(|response| response.nbr),
    }
}
