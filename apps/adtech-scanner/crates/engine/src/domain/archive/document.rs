//! The request document: one JSON line per request in a record frame of the
//! pack. Its first seven fields carry the names and meaning of the owner's
//! data-lake request document; the rest is what the network view needs.

use adscan_model::{
    BodySkip, BodyState, CapturedBody, InitiatorKind, NetworkRecord, RequestContext, RequestEnd,
    ResourceKind, ResourceTiming, ServedFrom,
};
use serde::{Deserialize, Serialize};

use super::redact::stored_headers;

pub const DOCUMENT_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestDocument {
    #[serde(rename = "_v")]
    pub version: u32,
    /// `<scan id>:<seq>`.
    pub id: String,
    pub full_url: String,
    #[serde(with = "header_object")]
    pub headers: Vec<(String, String)>,
    pub post_data: Option<String>,
    #[serde(with = "header_object")]
    pub response_headers: Vec<(String, String)>,
    /// Always null here: the body is its own frame, named by `body`.
    pub response_content: Option<String>,

    pub seq: u32,
    pub page: u32,
    pub context: RequestContext,
    pub method: String,
    pub resource_type: ResourceKind,
    pub status: Option<u16>,
    pub mime_type: Option<String>,
    pub protocol: Option<String>,
    pub remote_address: Option<String>,
    pub priority: Option<String>,
    pub frame_id: Option<String>,
    pub target_id: Option<String>,
    pub started_ms: f64,
    pub response_ms: Option<f64>,
    pub ended_ms: Option<f64>,
    pub queued_ms: Option<f64>,
    pub timing: Option<StoredTiming>,
    pub encoded_bytes: u64,
    pub decoded_bytes: u64,
    /// Length of the request body as sent; `post_data` holds at most the cap.
    pub post_bytes: u64,
    pub post_missing: bool,
    pub initiator: StoredInitiator,
    pub redirect_from: Option<u32>,
    pub preflight_of: Option<u32>,
    pub end: StoredEnd,
    pub error: Option<String>,
    pub blocked_reason: Option<String>,
    pub served_from: ServedFrom,
    pub bidder: Option<String>,
    pub headers_redacted: bool,
    pub body_state: BodyState,
    pub body: Option<StoredBody>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StoredEnd {
    Finished,
    Redirected,
    Failed,
    Cancelled,
    Pending,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredBody {
    /// Hex, of the stored bytes.
    pub sha256: String,
    /// `scans/<scan id>.pack.zst#o=<offset>&l=<length>`.
    pub location: String,
    /// Length as the browser reported it.
    pub bytes: u64,
    pub stored_bytes: u64,
    pub kind: String,
    pub truncated: bool,
}

/// CDP `ResourceTiming`, ms after the request's own start.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct StoredTiming {
    pub dns_start: Option<f64>,
    pub dns_end: Option<f64>,
    pub connect_start: Option<f64>,
    pub connect_end: Option<f64>,
    pub ssl_start: Option<f64>,
    pub ssl_end: Option<f64>,
    pub send_start: Option<f64>,
    pub send_end: Option<f64>,
    pub receive_headers_start: Option<f64>,
    pub receive_headers_end: Option<f64>,
    pub worker_start: Option<f64>,
    pub worker_ready: Option<f64>,
}

impl From<ResourceTiming> for StoredTiming {
    fn from(timing: ResourceTiming) -> Self {
        Self {
            dns_start: timing.dns_start,
            dns_end: timing.dns_end,
            connect_start: timing.connect_start,
            connect_end: timing.connect_end,
            ssl_start: timing.ssl_start,
            ssl_end: timing.ssl_end,
            send_start: timing.send_start,
            send_end: timing.send_end,
            receive_headers_start: timing.receive_headers_start,
            receive_headers_end: timing.receive_headers_end,
            worker_start: timing.worker_start,
            worker_ready: timing.worker_ready,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredInitiator {
    pub kind: InitiatorKind,
    pub url: Option<String>,
    /// 0-based, as the browser reports them.
    pub line: Option<u32>,
    pub column: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stack: Vec<StoredFrame>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredFrame {
    pub function: Option<String>,
    pub url: String,
    pub script_url: Option<String>,
    pub line: u32,
    pub column: u32,
}

impl RequestDocument {
    /// The document of `record`, with its headers as they may be stored.
    /// `body` is where the writer put the response body, if it kept one.
    pub fn of(
        scan_id: &str,
        record: &NetworkRecord,
        full_headers: bool,
        body_state: BodyState,
        body: Option<StoredBody>,
    ) -> Self {
        let (end, error, blocked_reason) = match &record.end {
            RequestEnd::Finished => (StoredEnd::Finished, None, None),
            RequestEnd::Redirected => (StoredEnd::Redirected, None, None),
            RequestEnd::Pending => (StoredEnd::Pending, None, None),
            RequestEnd::Failed {
                error,
                cancelled,
                blocked_reason,
            } => (
                if *cancelled {
                    StoredEnd::Cancelled
                } else {
                    StoredEnd::Failed
                },
                Some(error.clone()),
                blocked_reason.clone(),
            ),
        };
        Self {
            version: DOCUMENT_VERSION,
            id: format!("{scan_id}:{}", record.seq),
            full_url: record.url.clone(),
            headers: stored_headers(&record.request_headers, full_headers),
            post_data: record
                .post_data
                .as_ref()
                .map(|post| String::from_utf8_lossy(&post.bytes).into_owned()),
            response_headers: stored_headers(&record.response_headers, full_headers),
            response_content: None,
            seq: record.seq,
            page: record.page,
            context: record.context,
            method: record.method.clone(),
            resource_type: record.kind,
            status: record.status,
            mime_type: record.mime_type.clone(),
            protocol: record.protocol.clone(),
            remote_address: record.remote_address.clone(),
            priority: record.priority.clone(),
            frame_id: record.frame_id.clone(),
            target_id: record.target_id.clone(),
            started_ms: record.started_at_ms,
            response_ms: record.response_at_ms,
            ended_ms: record.ended_at_ms,
            queued_ms: record.queued_ms,
            timing: record.timing.map(StoredTiming::from),
            encoded_bytes: record.encoded_bytes,
            decoded_bytes: record.decoded_bytes,
            post_bytes: record.post_data.as_ref().map_or(0, |post| post.total_bytes),
            post_missing: record.post_data_missing,
            initiator: StoredInitiator {
                kind: record.initiator.kind,
                url: record.initiator.url.clone(),
                line: record.initiator.line,
                column: record.initiator.column,
                stack: record
                    .initiator
                    .stack
                    .iter()
                    .map(|frame| StoredFrame {
                        function: frame.function.clone(),
                        url: frame.url.clone(),
                        script_url: frame.script_url.clone(),
                        line: frame.line,
                        column: frame.column,
                    })
                    .collect(),
            },
            redirect_from: record.redirect_from,
            preflight_of: record.preflight_of,
            end,
            error,
            blocked_reason,
            served_from: record.served_from,
            bidder: record.bidder.clone(),
            headers_redacted: !full_headers,
            body_state,
            body,
        }
    }

    /// Whether the stored request body is shorter than what was sent.
    pub fn post_truncated(&self) -> bool {
        self.post_data
            .as_ref()
            .is_some_and(|post| (post.len() as u64) < self.post_bytes)
    }
}

/// What becomes of a body the capture did not hand over as bytes.
pub fn state_without_bytes(body: &CapturedBody, end: &RequestEnd) -> BodyState {
    match body {
        CapturedBody::Empty => BodyState::Empty,
        CapturedBody::Skipped(BodySkip::Type | BodySkip::NotStored) => BodyState::NotKeptType,
        CapturedBody::Skipped(BodySkip::ScanLimit) => BodyState::ScanLimit,
        CapturedBody::Skipped(BodySkip::Backlog) => BodyState::Dropped,
        CapturedBody::Absent if *end == RequestEnd::Pending => BodyState::Pending,
        CapturedBody::Absent | CapturedBody::Unavailable(_) => BodyState::Unavailable,
        CapturedBody::Read { bytes, total_bytes } => {
            if bytes.is_empty() {
                BodyState::Empty
            } else if (bytes.len() as u64) < *total_bytes {
                BodyState::Truncated
            } else {
                BodyState::Kept
            }
        }
    }
}

/// Headers as one JSON object, like the data lake's documents. A name that
/// occurs more than once keeps its values in order, joined by a line feed.
mod header_object {
    use std::fmt;

    use serde::de::{MapAccess, Visitor};
    use serde::ser::SerializeMap;
    use serde::{Deserializer, Serializer};

    pub fn serialize<S: Serializer>(
        headers: &[(String, String)],
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        let mut joined: Vec<(&str, String)> = Vec::with_capacity(headers.len());
        for (name, value) in headers {
            match joined.iter_mut().find(|(seen, _)| seen == name) {
                Some((_, values)) => {
                    values.push('\n');
                    values.push_str(value);
                }
                None => joined.push((name, value.clone())),
            }
        }
        let mut map = serializer.serialize_map(Some(joined.len()))?;
        for (name, value) in joined {
            map.serialize_entry(name, &value)?;
        }
        map.end()
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Vec<(String, String)>, D::Error> {
        struct Headers;
        impl<'de> Visitor<'de> for Headers {
            type Value = Vec<(String, String)>;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("an object of header names and values")
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut headers = Vec::new();
                while let Some((name, values)) = map.next_entry::<String, String>()? {
                    headers.extend(
                        values
                            .split('\n')
                            .map(|value| (name.clone(), value.to_string())),
                    );
                }
                Ok(headers)
            }
        }
        deserializer.deserialize_map(Headers)
    }
}

#[cfg(test)]
mod tests {
    use adscan_model::{CapturedPostData, NetworkInitiator, RawHeader};

    use super::*;

    pub(crate) fn record(seq: u32) -> NetworkRecord {
        NetworkRecord {
            seq,
            page: 0,
            context: RequestContext::Page,
            frame_id: Some("F1".into()),
            target_id: None,
            method: "POST".into(),
            url: "https://rtb.openx.net/openrtbb/prebidjs?x=1".into(),
            kind: ResourceKind::Fetch,
            priority: Some("High".into()),
            request_headers: vec![
                RawHeader {
                    name: "Cookie".into(),
                    value: "i=abcdef".into(),
                },
                RawHeader {
                    name: "Content-Type".into(),
                    value: "text/plain".into(),
                },
            ],
            post_data: Some(CapturedPostData {
                bytes: br#"{"id":"1"}"#.to_vec(),
                total_bytes: 10,
            }),
            post_data_missing: false,
            initiator: NetworkInitiator {
                kind: InitiatorKind::Script,
                url: Some("https://pub.example/prebid.js".into()),
                line: Some(0),
                column: Some(2345),
                stack: Vec::new(),
            },
            redirect_from: None,
            preflight_of: None,
            status: Some(200),
            mime_type: Some("application/json".into()),
            protocol: Some("h2".into()),
            remote_address: Some("203.0.113.9:443".into()),
            response_headers: vec![RawHeader {
                name: "set-cookie".into(),
                value: "a=1; Path=/\nb=22".into(),
            }],
            served_from: ServedFrom::Network,
            started_at_ms: 1834.2,
            response_at_ms: Some(2015.0),
            ended_at_ms: Some(2046.6),
            queued_ms: Some(1.5),
            timing: Some(ResourceTiming {
                send_start: Some(0.3),
                send_end: Some(0.5),
                receive_headers_end: Some(180.2),
                ..ResourceTiming::default()
            }),
            encoded_bytes: 1812,
            decoded_bytes: 28,
            end: RequestEnd::Finished,
            bidder: Some("openx".into()),
            body: CapturedBody::Absent,
        }
    }

    #[test]
    fn a_document_survives_the_json_line_it_is_stored_as() {
        let body = StoredBody {
            sha256: "ab".repeat(32),
            location: "scans/s1.pack.zst#o=10&l=20".into(),
            bytes: 28,
            stored_bytes: 28,
            kind: "json".into(),
            truncated: false,
        };
        let document = RequestDocument::of("s1", &record(412), false, BodyState::Kept, Some(body));
        let line = serde_json::to_string(&document).unwrap();
        assert!(!line.contains('\n'), "one document is one line");
        assert_eq!(
            serde_json::from_str::<RequestDocument>(&line).unwrap(),
            document
        );
    }

    #[test]
    fn the_data_lake_fields_are_present_under_their_names() {
        let document = RequestDocument::of("s1", &record(412), false, BodyState::Unavailable, None);
        let json: serde_json::Value = serde_json::to_value(&document).unwrap();
        assert_eq!(json["_v"], 1);
        assert_eq!(json["id"], "s1:412");
        assert_eq!(
            json["fullUrl"],
            "https://rtb.openx.net/openrtbb/prebidjs?x=1"
        );
        assert_eq!(json["headers"]["Content-Type"], "text/plain");
        assert_eq!(json["postData"], r#"{"id":"1"}"#);
        // Two cookies of one response are one member, as CDP joins them.
        assert_eq!(
            json["responseHeaders"]["set-cookie"],
            "a=[1 bytes]; Path=/\nb=[2 bytes]"
        );
        assert!(json["responseContent"].is_null());
    }

    #[test]
    fn credentials_are_not_in_the_document_unless_full_headers_are_stored() {
        let redacted = serde_json::to_string(&RequestDocument::of(
            "s",
            &record(1),
            false,
            BodyState::Empty,
            None,
        ))
        .unwrap();
        assert!(!redacted.contains("abcdef"));
        assert!(redacted.contains("i=[6 bytes]"));
        let full = serde_json::to_string(&RequestDocument::of(
            "s",
            &record(1),
            true,
            BodyState::Empty,
            None,
        ))
        .unwrap();
        assert!(full.contains("i=abcdef"));
    }

    #[test]
    fn a_failed_or_cancelled_request_keeps_its_error() {
        let mut failed = record(3);
        failed.end = RequestEnd::Failed {
            error: "net::ERR_ABORTED".into(),
            cancelled: true,
            blocked_reason: None,
        };
        let document = RequestDocument::of("s", &failed, false, BodyState::Unavailable, None);
        assert_eq!(document.end, StoredEnd::Cancelled);
        assert_eq!(document.error.as_deref(), Some("net::ERR_ABORTED"));
    }

    #[test]
    fn every_way_a_body_can_be_missing_has_its_own_state() {
        let finished = RequestEnd::Finished;
        for (body, end, expected) in [
            (CapturedBody::Empty, &finished, BodyState::Empty),
            (
                CapturedBody::Skipped(BodySkip::Type),
                &finished,
                BodyState::NotKeptType,
            ),
            (
                CapturedBody::Skipped(BodySkip::ScanLimit),
                &finished,
                BodyState::ScanLimit,
            ),
            (
                CapturedBody::Skipped(BodySkip::Backlog),
                &finished,
                BodyState::Dropped,
            ),
            (
                CapturedBody::Unavailable("gone".into()),
                &finished,
                BodyState::Unavailable,
            ),
            (
                CapturedBody::Absent,
                &RequestEnd::Redirected,
                BodyState::Unavailable,
            ),
            (
                CapturedBody::Absent,
                &RequestEnd::Pending,
                BodyState::Pending,
            ),
            (
                CapturedBody::Read {
                    bytes: vec![1, 2],
                    total_bytes: 2,
                },
                &finished,
                BodyState::Kept,
            ),
            (
                CapturedBody::Read {
                    bytes: vec![1, 2],
                    total_bytes: 9,
                },
                &finished,
                BodyState::Truncated,
            ),
            (
                CapturedBody::Read {
                    bytes: Vec::new(),
                    total_bytes: 0,
                },
                &finished,
                BodyState::Empty,
            ),
        ] {
            assert_eq!(state_without_bytes(&body, end), expected, "{body:?}");
        }
    }

    #[test]
    fn a_cut_request_body_says_so() {
        let mut long = record(5);
        long.post_data = Some(CapturedPostData {
            bytes: b"abc".to_vec(),
            total_bytes: 3_000_000,
        });
        let document = RequestDocument::of("s", &long, false, BodyState::Empty, None);
        assert!(document.post_truncated());
        assert_eq!(document.post_bytes, 3_000_000);
    }
}
