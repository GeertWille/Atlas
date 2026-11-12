//! Reads the pieces of the browser's `Network` events into the hand-over
//! types. No I/O.

use adscan_model::{
    CapturedBody, CapturedPostData, InitiatorKind, MAX_INITIATOR_FRAMES, MAX_POST_DATA_BYTES,
    RawHeader, ResourceKind, ResourceTiming,
};
use base64::Engine;
use serde_json::Value;

use crate::cdp::CdpError;

use super::super::bid_endpoints::short;

/// A CDP `Headers` object as a list.
pub(crate) fn headers(object: &Value) -> Vec<RawHeader> {
    let Some(object) = object.as_object() else {
        return Vec::new();
    };
    object
        .iter()
        .map(|(name, value)| RawHeader {
            name: name.clone(),
            value: match value {
                Value::String(text) => text.clone(),
                other => other.to_string(),
            },
        })
        .collect()
}

pub(crate) fn header<'a>(headers: &'a [RawHeader], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|header| header.name.eq_ignore_ascii_case(name))
        .map(|header| header.value.as_str())
}

/// CDP `Network.ResourceType`.
pub(crate) fn resource_kind(cdp_type: &str) -> ResourceKind {
    match cdp_type {
        "Document" => ResourceKind::Document,
        "Stylesheet" => ResourceKind::Stylesheet,
        "Image" => ResourceKind::Image,
        "Media" => ResourceKind::Media,
        "Font" => ResourceKind::Font,
        "Script" => ResourceKind::Script,
        "XHR" => ResourceKind::Xhr,
        "Fetch" => ResourceKind::Fetch,
        "EventSource" => ResourceKind::EventSource,
        "WebSocket" => ResourceKind::WebSocket,
        "Ping" => ResourceKind::Ping,
        "Preflight" => ResourceKind::Preflight,
        _ => ResourceKind::Other,
    }
}

/// CDP `Network.Initiator.type`.
pub(crate) fn initiator_kind(cdp_type: &str) -> InitiatorKind {
    match cdp_type {
        "parser" => InitiatorKind::Parser,
        "script" => InitiatorKind::Script,
        "preload" => InitiatorKind::Preload,
        "preflight" => InitiatorKind::Preflight,
        _ => InitiatorKind::Other,
    }
}

/// One frame of an initiator stack, with the browser's own id of its script.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CallFrame {
    pub function: Option<String>,
    pub url: String,
    /// Only unique within the session that reported it.
    pub script_id: String,
    pub line: u32,
    pub column: u32,
}

/// The call frames of a CDP `Runtime.StackTrace`, innermost first, followed
/// by those of its async parents, at most [`MAX_INITIATOR_FRAMES`].
pub(crate) fn call_frames(stack: &Value) -> Vec<CallFrame> {
    let mut frames = Vec::new();
    let mut stack = stack;
    while stack.is_object() && frames.len() < MAX_INITIATOR_FRAMES {
        let room = MAX_INITIATOR_FRAMES - frames.len();
        let of_this_stack = stack["callFrames"].as_array().into_iter().flatten();
        frames.extend(of_this_stack.take(room).map(|frame| {
            CallFrame {
                function: frame["functionName"]
                    .as_str()
                    .filter(|name| !name.is_empty())
                    .map(str::to_string),
                url: frame["url"].as_str().unwrap_or_default().to_string(),
                script_id: frame["scriptId"].as_str().unwrap_or_default().to_string(),
                line: position(&frame["lineNumber"]).unwrap_or(0),
                column: position(&frame["columnNumber"]).unwrap_or(0),
            }
        }));
        stack = &stack["parent"];
    }
    frames
}

/// A 0-based line or column as CDP reports it.
pub(crate) fn position(value: &Value) -> Option<u32> {
    value.as_f64().filter(|at| *at >= 0.0).map(|at| at as u32)
}

/// CDP `Network.ResourceTiming`: when the browser started work on the
/// request, in ms on its monotonic clock, and the phases after that.
pub(crate) fn timing(timing: &Value) -> Option<(f64, ResourceTiming)> {
    let request_time_ms = timing["requestTime"].as_f64()? * 1000.0;
    let at = |key: &str| timing[key].as_f64().filter(|ms| *ms >= 0.0);
    Some((
        request_time_ms,
        ResourceTiming {
            dns_start: at("dnsStart"),
            dns_end: at("dnsEnd"),
            connect_start: at("connectStart"),
            connect_end: at("connectEnd"),
            ssl_start: at("sslStart"),
            ssl_end: at("sslEnd"),
            send_start: at("sendStart"),
            send_end: at("sendEnd"),
            receive_headers_start: at("receiveHeadersStart"),
            receive_headers_end: at("receiveHeadersEnd"),
            worker_start: at("workerStart"),
            worker_ready: at("workerReady"),
        },
    ))
}

/// A request body as it is handed over: at most [`MAX_POST_DATA_BYTES`].
pub(crate) fn post_data(text: &str) -> CapturedPostData {
    CapturedPostData {
        bytes: leading_text(text, MAX_POST_DATA_BYTES).as_bytes().to_vec(),
        total_bytes: text.len() as u64,
    }
}

/// The result of `Network.getResponseBody` as it is handed over: at most
/// `max_bytes` of the decoded body.
pub(crate) fn captured_body(result: Result<Value, CdpError>, max_bytes: usize) -> CapturedBody {
    let result = match result {
        Ok(result) => result,
        Err(CdpError::Rejected { message, .. }) => {
            return CapturedBody::Unavailable(short(&message));
        }
        Err(error) => return CapturedBody::Unavailable(short(&error.to_string())),
    };
    let Some(body) = result["body"].as_str() else {
        return CapturedBody::Unavailable("the browser answered without a body".into());
    };
    if result["base64Encoded"].as_bool() != Some(true) {
        return read(
            leading_text(body, max_bytes).as_bytes().to_vec(),
            body.len(),
        );
    }
    match base64::engine::general_purpose::STANDARD.decode(body) {
        Ok(mut bytes) => {
            let total_bytes = bytes.len();
            bytes.truncate(max_bytes);
            read(bytes, total_bytes)
        }
        Err(_) => CapturedBody::Unavailable("the browser sent a body that is not base64".into()),
    }
}

fn read(bytes: Vec<u8>, total_bytes: usize) -> CapturedBody {
    if total_bytes == 0 {
        CapturedBody::Empty
    } else {
        CapturedBody::Read {
            bytes,
            total_bytes: total_bytes as u64,
        }
    }
}

/// The longest start of `text` of at most `max_bytes` that ends between two
/// characters, so that a cut body is still text.
fn leading_text(text: &str, max_bytes: usize) -> &str {
    if text.len() <= max_bytes {
        return text;
    }
    let mut end = max_bytes;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn headers_keep_names_and_values_as_reported() {
        let list = headers(&json!({
            "Content-Type": "application/json",
            "set-cookie": "a=1\nb=2",
            "X-Count": 3,
        }));
        assert_eq!(list.len(), 3);
        assert_eq!(header(&list, "content-type"), Some("application/json"));
        assert_eq!(header(&list, "Set-Cookie"), Some("a=1\nb=2"));
        assert_eq!(header(&list, "x-count"), Some("3"));
        assert_eq!(header(&list, "accept"), None);
        assert!(headers(&json!(null)).is_empty());
    }

    #[test]
    fn resource_types_and_initiators_map_to_the_contract() {
        assert_eq!(resource_kind("XHR"), ResourceKind::Xhr);
        assert_eq!(resource_kind("Preflight"), ResourceKind::Preflight);
        assert_eq!(resource_kind("EventSource"), ResourceKind::EventSource);
        for other in [
            "TextTrack",
            "Manifest",
            "Prefetch",
            "CSPViolationReport",
            "",
        ] {
            assert_eq!(resource_kind(other), ResourceKind::Other, "{other}");
        }
        assert_eq!(initiator_kind("parser"), InitiatorKind::Parser);
        assert_eq!(initiator_kind("preflight"), InitiatorKind::Preflight);
        assert_eq!(initiator_kind("SignedExchange"), InitiatorKind::Other);
    }

    #[test]
    fn a_stack_lists_its_own_frames_then_those_of_its_async_parents() {
        let stack = json!({
            "callFrames": [
                { "functionName": "send", "scriptId": "12", "url": "https://a.test/hb.js",
                  "lineNumber": 0, "columnNumber": 4_211 },
                { "functionName": "", "scriptId": "9", "url": "", "lineNumber": 3, "columnNumber": 0 },
            ],
            "parent": {
                "description": "setTimeout",
                "callFrames": [
                    { "functionName": "start", "scriptId": "12", "url": "https://a.test/hb.js",
                      "lineNumber": 0, "columnNumber": 88 },
                ],
            },
        });
        let frames = call_frames(&stack);
        assert_eq!(frames.len(), 3);
        assert_eq!(frames[0].function.as_deref(), Some("send"));
        assert_eq!((frames[0].line, frames[0].column), (0, 4_211));
        assert_eq!(frames[0].script_id, "12");
        assert_eq!(frames[1].function, None);
        assert_eq!(frames[1].url, "");
        assert_eq!(frames[2].function.as_deref(), Some("start"));
        assert!(call_frames(&json!(null)).is_empty());
    }

    #[test]
    fn a_deep_stack_is_cut_at_the_frame_limit() {
        let frame = json!({ "functionName": "f", "scriptId": "1", "url": "u", "lineNumber": 1, "columnNumber": 1 });
        let level =
            |parent: Value| json!({ "callFrames": vec![frame.clone(); 20], "parent": parent });
        let stack = level(level(level(Value::Null)));
        assert_eq!(call_frames(&stack).len(), MAX_INITIATOR_FRAMES);
    }

    #[test]
    fn timing_phases_are_ms_after_the_start_and_absent_where_cdp_says_minus_one() {
        let (request_time_ms, phases) = timing(&json!({
            "requestTime": 60_690.5,
            "dnsStart": -1, "dnsEnd": -1,
            "connectStart": 0.4, "connectEnd": 12.5,
            "sslStart": 3.0, "sslEnd": 12.0,
            "sendStart": 12.6, "sendEnd": 12.9,
            "receiveHeadersStart": 80.0, "receiveHeadersEnd": 81.5,
            "workerStart": -1, "workerReady": -1,
        }))
        .unwrap();
        assert_eq!(request_time_ms, 60_690_500.0);
        assert_eq!(phases.dns_start, None);
        assert_eq!(phases.connect_end, Some(12.5));
        assert_eq!(phases.receive_headers_end, Some(81.5));
        assert_eq!(phases.worker_start, None);
        assert_eq!(timing(&json!(null)), None);
    }

    #[test]
    fn text_bodies_are_handed_over_as_their_utf8_and_cut_between_characters() {
        let whole = captured_body(Ok(json!({ "body": "héllo", "base64Encoded": false })), 100);
        assert_eq!(
            whole,
            CapturedBody::Read {
                bytes: "héllo".as_bytes().to_vec(),
                total_bytes: 6
            }
        );
        // "é" is two bytes: a cut after the first of them moves back.
        let cut = captured_body(Ok(json!({ "body": "héllo", "base64Encoded": false })), 2);
        assert_eq!(
            cut,
            CapturedBody::Read {
                bytes: b"h".to_vec(),
                total_bytes: 6
            }
        );
    }

    #[test]
    fn binary_bodies_are_decoded_and_cut_by_bytes() {
        let body = captured_body(Ok(json!({ "body": "AAECAwQ=", "base64Encoded": true })), 3);
        assert_eq!(
            body,
            CapturedBody::Read {
                bytes: vec![0, 1, 2],
                total_bytes: 5
            }
        );
        assert!(matches!(
            captured_body(Ok(json!({ "body": "***", "base64Encoded": true })), 3),
            CapturedBody::Unavailable(_)
        ));
    }

    #[test]
    fn an_empty_body_and_a_refusal_are_told_apart() {
        assert_eq!(
            captured_body(Ok(json!({ "body": "", "base64Encoded": false })), 10),
            CapturedBody::Empty
        );
        assert_eq!(
            captured_body(Ok(json!({ "body": "", "base64Encoded": true })), 10),
            CapturedBody::Empty
        );
        let refused = captured_body(
            Err(CdpError::Rejected {
                method: "Network.getResponseBody",
                message: "No resource with given identifier found".into(),
            }),
            10,
        );
        assert_eq!(
            refused,
            CapturedBody::Unavailable("No resource with given identifier found".into())
        );
        assert_eq!(
            captured_body(Err(CdpError::Closed), 10),
            CapturedBody::Unavailable("the connection to the browser closed".into())
        );
        assert!(matches!(
            captured_body(Ok(json!({})), 10),
            CapturedBody::Unavailable(_)
        ));
    }

    #[test]
    fn a_post_body_is_cut_at_its_limit_and_keeps_its_length() {
        let small = post_data("{\"a\":1}");
        assert_eq!((small.bytes.len(), small.total_bytes), (7, 7));
        let large = post_data(&"é".repeat(MAX_POST_DATA_BYTES));
        assert_eq!(large.total_bytes, 2 * MAX_POST_DATA_BYTES as u64);
        assert_eq!(large.bytes.len(), MAX_POST_DATA_BYTES);
        assert!(String::from_utf8(large.bytes).is_ok());
    }
}
