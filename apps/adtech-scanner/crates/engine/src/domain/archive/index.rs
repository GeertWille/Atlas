//! The index rows of an archive: what is kept of a body and of a request
//! outside the pack, so that lists, counts and links never open it.

use adscan_model::{
    AdLink, BodyState, InitiatorKind, NetworkCategory, NetworkRecord, RequestContext, RequestEnd,
    ResourceKind, ServedFrom,
};
use url::Url;

use super::kinds::BodyKind;
use crate::domain::network::classify::{self, RequestFacts};

const MAX_PATH_CHARS: usize = 512;
const MAX_ERROR_CHARS: usize = 80;

/// One distinct body of a scan and where its frame is in the pack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BodyIndexRow {
    pub body_no: u32,
    /// Of the stored bytes.
    pub sha256: [u8; 32],
    pub kind: BodyKind,
    /// Stored bytes, before compression.
    pub raw_len: u64,
    pub pack_offset: u64,
    pub pack_length: u32,
    pub truncated: bool,
}

/// What the index keeps of a request: enough to list, sort, count and link
/// without opening the pack. Never a query string, a header or a body.
#[derive(Debug, Clone, PartialEq)]
pub struct RequestIndexRow {
    pub seq: u32,
    pub page: u32,
    pub context: RequestContext,
    pub started_ms: f64,
    /// From the start of the request to the first byte of the answer.
    pub ttfb_ms: Option<f64>,
    pub duration_ms: Option<f64>,
    pub method: String,
    pub scheme: String,
    pub host: String,
    /// Registrable domain of `host`.
    pub site: String,
    /// Without the query string, at most 512 characters.
    pub path: String,
    pub query_len: u32,
    pub kind: ResourceKind,
    pub mime: Option<String>,
    pub status: Option<u16>,
    pub end: RequestEndKind,
    /// Network error or blocked reason, at most 80 characters.
    pub error: Option<String>,
    pub third_party: bool,
    pub served_from: ServedFrom,
    pub no_hb_targeting: bool,
    pub encoded_bytes: u64,
    pub decoded_bytes: u64,
    pub post_bytes: u64,
    pub initiator_kind: InitiatorKind,
    pub redirect_from: Option<u32>,
    pub preflight_of: Option<u32>,
    pub bidder: Option<String>,
    pub category: NetworkCategory,
    pub body_state: BodyState,
    pub body_no: Option<u32>,
    /// The record frame that holds the request's document, and its line.
    pub record_offset: u64,
    pub record_length: u32,
    pub record_index: u32,
    /// Set after the capture, when the request was matched to an auction.
    pub ad: Option<AdLink>,
    /// (id in the request, ad unit code): Prebid's bid ids found in a matched
    /// bid request, so its impressions can be named when it is decoded.
    pub bid_units: Vec<(String, String)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestEndKind {
    Finished,
    Redirected,
    Failed,
    Blocked,
    Cancelled,
    Pending,
}

/// The registrable domain of a host, or the host itself where the public
/// suffix list has no answer (IP addresses, `localhost`).
pub fn site_of(host: &str) -> String {
    psl::domain_str(host).unwrap_or(host).to_string()
}

pub fn sha256_hex(sha256: &[u8; 32]) -> String {
    sha256.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The 32 bytes a hex sha-256 stands for.
pub fn sha256_from_hex(text: &str) -> Option<[u8; 32]> {
    if text.len() != 64 || !text.is_ascii() {
        return None;
    }
    let mut bytes = [0u8; 32];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[index * 2..index * 2 + 2], 16).ok()?;
    }
    Some(bytes)
}

impl RequestIndexRow {
    /// The row of `record`. "First party" is the registrable domain of the
    /// page view the request belongs to; `scan_site`, that of the address the
    /// scan was started with, stands in where the capture named no page. The
    /// frame of the record's document is filled in by the writer once it is
    /// in the pack.
    pub fn of(
        record: &NetworkRecord,
        scan_site: &str,
        body_state: BodyState,
        body_no: Option<u32>,
    ) -> Self {
        let page_site = if record.page_host.is_empty() {
            scan_site.to_string()
        } else {
            site_of(&record.page_host)
        };
        let parsed = Url::parse(&record.url).ok();
        let host = parsed
            .as_ref()
            .and_then(|url| url.host_str())
            .unwrap_or_default()
            .to_string();
        let scheme = parsed
            .as_ref()
            .map_or_else(String::new, |url| url.scheme().to_string());
        // `data:` and `blob:` have no hierarchy; their "path" is the payload.
        let hierarchical = parsed.as_ref().is_some_and(|url| !url.cannot_be_a_base());
        let path: String = parsed
            .as_ref()
            .filter(|_| hierarchical)
            .map_or("", |url| url.path())
            .chars()
            .take(MAX_PATH_CHARS)
            .collect();
        let query = parsed
            .as_ref()
            .and_then(|url| url.query())
            .unwrap_or_default();
        let site = site_of(&host);
        let third_party = site != page_site;
        let category = classify::category(&RequestFacts {
            host: &host,
            path: &path,
            method: &record.method,
            kind: record.kind,
            context: record.context,
            bidder: record.bidder.as_deref(),
            first_party: !third_party,
        });
        let post = record
            .post_data
            .as_ref()
            .map(|post| String::from_utf8_lossy(&post.bytes));
        let (end, error) = match &record.end {
            RequestEnd::Finished => (RequestEndKind::Finished, None),
            RequestEnd::Redirected => (RequestEndKind::Redirected, None),
            RequestEnd::Pending => (RequestEndKind::Pending, None),
            RequestEnd::Failed {
                error,
                cancelled,
                blocked_reason,
            } => (
                if blocked_reason.is_some() {
                    RequestEndKind::Blocked
                } else if *cancelled {
                    RequestEndKind::Cancelled
                } else {
                    RequestEndKind::Failed
                },
                Some(
                    blocked_reason
                        .as_deref()
                        .unwrap_or(error)
                        .chars()
                        .take(MAX_ERROR_CHARS)
                        .collect(),
                ),
            ),
        };
        Self {
            seq: record.seq,
            page: record.page,
            context: record.context,
            started_ms: record.started_at_ms,
            ttfb_ms: record.response_at_ms.map(|at| at - record.started_at_ms),
            duration_ms: record.ended_at_ms.map(|at| at - record.started_at_ms),
            method: record.method.clone(),
            scheme,
            no_hb_targeting: category == NetworkCategory::AdServer
                && record.kind != ResourceKind::Preflight
                && !classify::has_prebid_targeting(query, post.as_deref()),
            query_len: query.len() as u32,
            host,
            site,
            path,
            kind: record.kind,
            mime: record.mime_type.clone(),
            status: record.status,
            end,
            error,
            third_party,
            served_from: record.served_from,
            encoded_bytes: record.encoded_bytes,
            decoded_bytes: record.decoded_bytes,
            post_bytes: record.post_data.as_ref().map_or(0, |post| post.total_bytes),
            initiator_kind: record.initiator.kind,
            redirect_from: record.redirect_from,
            preflight_of: record.preflight_of,
            bidder: record.bidder.clone(),
            category,
            body_state,
            body_no,
            record_offset: 0,
            record_length: 0,
            record_index: 0,
            ad: None,
            bid_units: Vec::new(),
        }
    }
}
