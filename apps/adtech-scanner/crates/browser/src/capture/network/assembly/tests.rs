use adscan_model::BodySkip;
use serde_json::json;

use super::*;

const PAGE: &str = "https://www.pub.example/article";
/// Epoch ms of the browser's monotonic second 1000.
const EPOCH_AT_1000: f64 = 1_700_000_000_000.0;

fn tab(now_ms: f64) -> Source<'static> {
    Source {
        session: "tab",
        reporter: Reporter::Tab,
        target_id: None,
        main_frame_id: Some("main"),
        recording: true,
        page_url: PAGE,
        now_ms,
    }
}

fn child(session: &'static str, reporter: Reporter, now_ms: f64) -> Source<'static> {
    Source {
        session,
        reporter,
        target_id: Some("child-target"),
        main_frame_id: None,
        ..tab(now_ms)
    }
}

/// Epoch ms of a monotonic time in seconds.
fn epoch(mono_seconds: f64) -> f64 {
    EPOCH_AT_1000 + (mono_seconds - 1_000.0) * 1000.0
}

fn request(id: &str, url: &str, at: f64) -> Value {
    json!({
        "requestId": id,
        "loaderId": "loader",
        "documentURL": PAGE,
        "frameId": "main",
        "type": "Fetch",
        "timestamp": at,
        "wallTime": epoch(at) / 1000.0,
        "request": {
            "url": url,
            "method": "GET",
            "headers": { "Accept": "*/*" },
            "initialPriority": "High",
        },
        "initiator": { "type": "script", "stack": { "callFrames": [
            { "functionName": "load", "scriptId": "7", "url": "https://www.pub.example/app.js",
              "lineNumber": 3, "columnNumber": 14 },
        ] } },
    })
}

fn response(id: &str, status: u16, at: f64) -> Value {
    json!({
        "requestId": id,
        "type": "Fetch",
        "timestamp": at,
        "hasExtraInfo": true,
        "response": {
            "url": "ignored",
            "status": status,
            "mimeType": "application/json",
            "protocol": "h2",
            "remoteIPAddress": "203.0.113.9",
            "remotePort": 443,
            "headers": { "content-type": "application/json" },
            "timing": {
                "requestTime": at - 0.080,
                "dnsStart": -1, "dnsEnd": -1, "connectStart": -1, "connectEnd": -1,
                "sslStart": -1, "sslEnd": -1, "sendStart": 1.0, "sendEnd": 1.5,
                "receiveHeadersStart": 70.0, "receiveHeadersEnd": 78.0,
            },
        },
    })
}

fn request_extra(id: &str, cookie: &str) -> Value {
    json!({ "requestId": id, "headers": { "Accept": "*/*", "Cookie": cookie } })
}

fn response_extra(id: &str, cookie: &str) -> Value {
    json!({ "requestId": id, "statusCode": 200, "headers": { "content-type": "application/json", "Set-Cookie": cookie } })
}

fn finished(id: &str, at: f64, encoded: u64) -> Value {
    json!({ "requestId": id, "timestamp": at, "encodedDataLength": encoded })
}

fn failed(id: &str, at: f64, error: &str) -> Value {
    json!({ "requestId": id, "timestamp": at, "type": "Fetch", "errorText": error, "canceled": false })
}

fn ended(outputs: Vec<Output>) -> Vec<Ended> {
    outputs
        .into_iter()
        .filter_map(|output| match output {
            Output::Ended(ended) => Some(*ended),
            _ => None,
        })
        .collect()
}

fn finished_of(outputs: &[Output]) -> Option<Finished> {
    outputs.iter().find_map(|output| match output {
        Output::Finished(finished) => Some(finished.clone()),
        _ => None,
    })
}

/// A page view that started at monotonic second 1000.
fn placed(ended: Ended) -> NetworkRecord {
    ended.into_record(|epoch_ms| (0, epoch_ms - EPOCH_AT_1000), |_, _| None)
}

fn header_of<'a>(headers: &'a [RawHeader], name: &str) -> Option<&'a str> {
    header(headers, name)
}

/// Runs one request with a body through the assembler and returns its record.
fn fetched(assembler: &mut Assembler, id: &str, url: &str, at: f64) -> NetworkRecord {
    let now = epoch(at);
    assembler.observe(
        &tab(now),
        "Network.requestWillBeSent",
        &request(id, url, at),
    );
    assembler.observe(&tab(now), "Network.responseReceived", &{
        let mut response = response(id, 200, at + 0.1);
        response["hasExtraInfo"] = json!(false);
        response
    });
    let outputs = assembler.observe(
        &tab(now),
        "Network.loadingFinished",
        &finished(id, at + 0.2, 300),
    );
    let seq = finished_of(&outputs).unwrap().seq;
    let mut records = ended(assembler.body_settled(seq, CapturedBody::Empty, now));
    assert_eq!(records.len(), 1, "{id}");
    placed(records.remove(0))
}

#[test]
fn a_request_is_assembled_from_its_events_and_handed_over_with_its_body() {
    let mut assembler = Assembler::default();
    let now = epoch(1_002.0);
    let url = "https://api.pub.example/data.json?x=1";

    assert!(
        assembler
            .observe(
                &tab(now),
                "Network.requestWillBeSent",
                &request("1.1", url, 1_002.0)
            )
            .is_empty()
    );
    assembler.observe(
        &tab(now),
        "Network.requestWillBeSentExtraInfo",
        &request_extra("1.1", "sid=abc"),
    );
    assembler.observe(
        &tab(now),
        "Network.responseReceivedExtraInfo",
        &response_extra("1.1", "seen=1"),
    );
    assembler.observe(
        &tab(now),
        "Network.responseReceived",
        &response("1.1", 200, 1_002.1),
    );
    for length in [1_000, 234] {
        assembler.observe(
            &tab(now),
            "Network.dataReceived",
            &json!({ "requestId": "1.1", "dataLength": length, "encodedDataLength": 400 }),
        );
    }
    let outputs = assembler.observe(
        &tab(now),
        "Network.loadingFinished",
        &finished("1.1", 1_002.25, 900),
    );

    let offer = finished_of(&outputs).expect("the body can be read now");
    assert_eq!(
        offer,
        Finished {
            seq: 0,
            session: "tab".into(),
            request_id: "1.1".into(),
            url: url.into(),
            kind: ResourceKind::Fetch,
            mime_type: Some("application/json".into()),
            status: Some(200),
            expected_bytes: 1_234,
            encoded_bytes: 900,
            bid: None,
            initiator_url: Some("https://www.pub.example/app.js".into()),
            in_tab: true,
            main_document: false,
            has_address_on_the_network: true,
            finished_epoch_ms: epoch(1_002.25),
        }
    );
    assert!(ended(outputs).is_empty(), "not before its body is settled");

    let body = CapturedBody::Read {
        bytes: b"{}".to_vec(),
        total_bytes: 2,
    };
    let mut records = ended(assembler.body_settled(0, body.clone(), now));
    assert_eq!(records.len(), 1);
    let record = records.remove(0).into_record(
        |epoch_ms| (2, epoch_ms - EPOCH_AT_1000),
        |session, frame| Some(format!("{session}:{}", frame.script_id)),
    );

    assert_eq!((record.seq, record.page), (0, 2));
    assert_eq!(record.context, RequestContext::Page);
    assert_eq!(record.frame_id.as_deref(), Some("main"));
    assert_eq!(record.target_id, None);
    assert_eq!((record.method.as_str(), record.url.as_str()), ("GET", url));
    assert_eq!(record.priority.as_deref(), Some("High"));
    assert_eq!(
        header_of(&record.request_headers, "cookie"),
        Some("sid=abc"),
        "what was sent, not the provisional headers"
    );
    assert_eq!(
        header_of(&record.response_headers, "set-cookie"),
        Some("seen=1")
    );
    assert_eq!(record.status, Some(200));
    assert_eq!(record.mime_type.as_deref(), Some("application/json"));
    assert_eq!(record.protocol.as_deref(), Some("h2"));
    assert_eq!(record.remote_address.as_deref(), Some("203.0.113.9:443"));
    assert_eq!(record.served_from, ServedFrom::Network);
    assert_eq!(record.started_at_ms, 2_000.0);
    assert_eq!(
        record.response_at_ms.unwrap().round(),
        2_098.0,
        "when the network received the headers, not when the page was told"
    );
    assert_eq!(record.ended_at_ms.unwrap().round(), 2_250.0);
    assert_eq!(record.queued_ms.unwrap().round(), 20.0);
    let timing = record.timing.unwrap();
    assert_eq!((timing.dns_start, timing.send_start), (None, Some(1.0)));
    assert_eq!(timing.receive_headers_end, Some(78.0));
    assert_eq!((record.encoded_bytes, record.decoded_bytes), (900, 1_234));
    assert_eq!(record.end, RequestEnd::Finished);
    assert_eq!(record.body, body);
    assert_eq!((record.redirect_from, record.preflight_of), (None, None));
    assert_eq!(record.initiator.kind, InitiatorKind::Script);
    assert_eq!(
        record.initiator.stack,
        [InitiatorFrame {
            function: Some("load".into()),
            url: "https://www.pub.example/app.js".into(),
            script_url: Some("tab:7".into()),
            line: 3,
            column: 14,
        }]
    );
    assert_eq!((record.post_data, record.post_data_missing), (None, false));
    assert!(assembler.finish().is_empty(), "handed over exactly once");
}

#[test]
fn extra_info_that_comes_first_is_kept_for_its_request() {
    let mut assembler = Assembler::default();
    let now = epoch(1_001.0);
    assembler.observe(
        &tab(now),
        "Network.requestWillBeSentExtraInfo",
        &request_extra("1.2", "early=1"),
    );
    assembler.observe(
        &tab(now),
        "Network.requestWillBeSent",
        &request("1.2", "https://a.example/x", 1_001.0),
    );
    assembler.observe(
        &tab(now),
        "Network.responseReceivedExtraInfo",
        &response_extra("1.2", "r=1"),
    );
    assembler.observe(
        &tab(now),
        "Network.responseReceived",
        &response("1.2", 204, 1_001.1),
    );
    let outputs = assembler.observe(
        &tab(now),
        "Network.loadingFinished",
        &finished("1.2", 1_001.2, 90),
    );
    let seq = finished_of(&outputs).unwrap().seq;
    let record = placed(ended(assembler.body_settled(seq, CapturedBody::Empty, now)).remove(0));
    assert_eq!(
        header_of(&record.request_headers, "cookie"),
        Some("early=1")
    );
    assert_eq!(record.body, CapturedBody::Empty);
}

#[test]
fn an_ended_request_waits_for_late_extra_info_but_not_for_ever() {
    let mut assembler = Assembler::default();
    let at = epoch(1_001.0);
    for id in ["1.3", "1.4"] {
        assembler.observe(
            &tab(at),
            "Network.requestWillBeSent",
            &request(id, "https://a.example/x", 1_001.0),
        );
        assembler.observe(
            &tab(at),
            "Network.responseReceived",
            &response(id, 200, 1_001.1),
        );
        let outputs = assembler.observe(
            &tab(at),
            "Network.loadingFinished",
            &finished(id, 1_001.2, 90),
        );
        let seq = finished_of(&outputs).unwrap().seq;
        let skipped = CapturedBody::Skipped(BodySkip::Type);
        assert!(
            ended(assembler.body_settled(seq, skipped, at)).is_empty(),
            "{id}: the browser announced ExtraInfo that has not come"
        );
    }

    // For the first it arrives a little later.
    assembler.observe(
        &tab(at + 40.0),
        "Network.requestWillBeSentExtraInfo",
        &request_extra("1.3", "late=1"),
    );
    let outputs = assembler.observe(
        &tab(at + 50.0),
        "Network.responseReceivedExtraInfo",
        &response_extra("1.3", "late=2"),
    );
    let first = placed(ended(outputs).remove(0));
    assert_eq!(first.seq, 0);
    assert_eq!(header_of(&first.request_headers, "cookie"), Some("late=1"));
    assert_eq!(
        header_of(&first.response_headers, "set-cookie"),
        Some("late=2")
    );

    // For the second it never does: the next event after the wait frees it.
    let outputs = assembler.observe(
        &tab(at + SETTLE_GRACE_MS + 1.0),
        "Network.requestWillBeSent",
        &request("1.5", "https://a.example/next", 1_003.0),
    );
    let second = placed(ended(outputs).remove(0));
    assert_eq!(second.seq, 1);
    assert_eq!(header_of(&second.request_headers, "accept"), Some("*/*"));
    assert_eq!(header_of(&second.request_headers, "cookie"), None);
    assert_eq!(
        header_of(&second.response_headers, "content-type"),
        Some("application/json")
    );
}

#[test]
fn every_redirect_hop_is_a_record_linked_to_the_hop_before() {
    let mut assembler = Assembler::default();
    let now = epoch(1_010.0);
    let hop = |url: &str, at: f64, from: Option<(&str, bool)>| {
        let mut event = request("9.1", url, at);
        if let Some((location, has_extra_info)) = from {
            event["redirectResponse"] = json!({
                "status": 302,
                "mimeType": "",
                "headers": { "location": location },
                "encodedDataLength": 180,
                "timing": { "requestTime": at - 0.030, "sendStart": 0.5, "sendEnd": 0.6,
                            "receiveHeadersStart": 25.0, "receiveHeadersEnd": 29.0 },
            });
            event["redirectHasExtraInfo"] = json!(has_extra_info);
        }
        event
    };
    let mut records = Vec::new();
    let mut observe = |assembler: &mut Assembler, method: &str, event: Value| {
        let outputs = assembler.observe(&tab(now), method, &event);
        let offer = finished_of(&outputs);
        records.extend(ended(outputs).into_iter().map(placed));
        offer
    };

    observe(
        &mut assembler,
        "Network.requestWillBeSentExtraInfo",
        request_extra("9.1", "hop=1"),
    );
    observe(
        &mut assembler,
        "Network.requestWillBeSent",
        hop("https://sync.a.example/start", 1_010.0, None),
    );
    observe(
        &mut assembler,
        "Network.responseReceivedExtraInfo",
        response_extra("9.1", "set=1"),
    );
    // The second hop is answered from the browser's cache: no ExtraInfo.
    observe(
        &mut assembler,
        "Network.requestWillBeSent",
        hop(
            "https://sync.b.example/match",
            1_010.1,
            Some(("https://sync.b.example/match", true)),
        ),
    );
    observe(
        &mut assembler,
        "Network.requestWillBeSent",
        hop(
            "https://sync.a.example/done.gif",
            1_010.2,
            Some(("https://sync.a.example/done.gif", false)),
        ),
    );
    observe(
        &mut assembler,
        "Network.requestWillBeSentExtraInfo",
        request_extra("9.1", "hop=3"),
    );
    observe(
        &mut assembler,
        "Network.responseReceivedExtraInfo",
        response_extra("9.1", "set=3"),
    );
    observe(
        &mut assembler,
        "Network.responseReceived",
        response("9.1", 200, 1_010.3),
    );
    let offer = observe(
        &mut assembler,
        "Network.loadingFinished",
        finished("9.1", 1_010.35, 43),
    )
    .unwrap();
    assert_eq!(offer.seq, 2, "only the last hop has a body");
    records.extend(
        ended(assembler.body_settled(2, CapturedBody::Skipped(BodySkip::Type), now))
            .into_iter()
            .map(placed),
    );

    records.sort_by_key(|record| record.seq);
    assert_eq!(records.len(), 3);
    let facts: Vec<_> = records
        .iter()
        .map(|record| {
            (
                record.url.as_str(),
                record.status,
                record.redirect_from,
                record.end.clone(),
                header_of(&record.request_headers, "cookie"),
                header_of(&record.response_headers, "set-cookie"),
            )
        })
        .collect();
    assert_eq!(
        facts,
        [
            (
                "https://sync.a.example/start",
                Some(302),
                None,
                RequestEnd::Redirected,
                Some("hop=1"),
                Some("set=1")
            ),
            (
                "https://sync.b.example/match",
                Some(302),
                Some(0),
                RequestEnd::Redirected,
                None,
                None
            ),
            (
                "https://sync.a.example/done.gif",
                Some(200),
                Some(1),
                RequestEnd::Finished,
                Some("hop=3"),
                Some("set=3")
            ),
        ]
    );
    assert_eq!(records[0].body, CapturedBody::Absent);
    assert_eq!(records[0].encoded_bytes, 180);
    assert_eq!(
        header_of(&records[1].response_headers, "location"),
        Some("https://sync.a.example/done.gif")
    );
    assert_eq!(records[0].ended_at_ms.unwrap().round(), 10_100.0);
    assert_eq!(records[1].started_at_ms.round(), 10_100.0);
    assert_eq!(records[0].timing.unwrap().receive_headers_end, Some(29.0));
}

#[test]
fn a_body_is_handed_over_at_once_even_when_earlier_hops_still_wait() {
    let mut assembler = Assembler::default();
    let now = epoch(1_010.0);
    assembler.observe(
        &tab(now),
        "Network.requestWillBeSent",
        &request("9.2", "https://a.example/1", 1_010.0),
    );
    let mut second = request("9.2", "https://a.example/2", 1_010.1);
    second["redirectResponse"] = json!({ "status": 301, "headers": {} });
    second["redirectHasExtraInfo"] = json!(true);
    assert!(ended(assembler.observe(&tab(now), "Network.requestWillBeSent", &second)).is_empty());
    assembler.observe(
        &tab(now),
        "Network.responseReceived",
        &response("9.2", 200, 1_010.2),
    );
    assembler.observe(
        &tab(now),
        "Network.loadingFinished",
        &finished("9.2", 1_010.3, 500),
    );

    let body = CapturedBody::Read {
        bytes: vec![1, 2, 3],
        total_bytes: 3,
    };
    let records = ended(assembler.body_settled(1, body, now));
    assert_eq!(
        records.iter().map(Ended::seq).collect::<Vec<_>>(),
        [0, 1],
        "no ExtraInfo arrived for either hop, and neither is kept back"
    );
}

#[test]
fn a_preflight_names_its_request_whichever_is_announced_first() {
    let preflight = |id: &str, announces: &str, at: f64| {
        let mut event = request(id, "https://bid.example/auction", at);
        event["type"] = json!("Other");
        event["request"]["method"] = json!("OPTIONS");
        event["initiator"] = json!({ "type": "preflight", "url": PAGE, "requestId": announces });
        event.as_object_mut().unwrap().remove("frameId");
        event
    };
    let end_preflight = |assembler: &mut Assembler, id: &str, at: f64| {
        let now = epoch(at);
        let mut answer = response(id, 204, at + 0.01);
        answer["type"] = json!("Preflight");
        answer["hasExtraInfo"] = json!(false);
        assembler.observe(&tab(now), "Network.responseReceived", &answer);
        let outputs = assembler.observe(
            &tab(now),
            "Network.loadingFinished",
            &finished(id, at + 0.02, 0),
        );
        let seq = finished_of(&outputs).unwrap().seq;
        ended(assembler.body_settled(seq, CapturedBody::Skipped(BodySkip::Type), now))
    };

    // Chrome announces the preflight, and finishes it, before the request.
    let mut assembler = Assembler::default();
    let now = epoch(1_020.0);
    assembler.observe(
        &tab(now),
        "Network.requestWillBeSent",
        &preflight("P1", "5.5", 1_020.0),
    );
    assert!(
        end_preflight(&mut assembler, "P1", 1_020.0).is_empty(),
        "kept until the request it belongs to is known"
    );
    let outputs = assembler.observe(
        &tab(now),
        "Network.requestWillBeSent",
        &request("5.5", "https://bid.example/auction", 1_020.03),
    );
    let record = placed(ended(outputs).remove(0));
    assert_eq!((record.seq, record.preflight_of), (0, Some(1)));
    assert_eq!(record.kind, ResourceKind::Preflight);
    assert_eq!(record.initiator.kind, InitiatorKind::Preflight);
    assert_eq!(record.context, RequestContext::Page);
    assert_eq!(record.bidder, None);

    // The other order.
    let mut assembler = Assembler::default();
    assembler.observe(
        &tab(now),
        "Network.requestWillBeSent",
        &request("5.6", "https://bid.example/auction", 1_020.0),
    );
    assembler.observe(
        &tab(now),
        "Network.requestWillBeSent",
        &preflight("P2", "5.6", 1_020.01),
    );
    let record = placed(end_preflight(&mut assembler, "P2", 1_020.01).remove(0));
    assert_eq!((record.seq, record.preflight_of), (1, Some(0)));

    // A request that is never announced does not keep its preflight for ever.
    let mut assembler = Assembler::default();
    assembler.observe(
        &tab(now),
        "Network.requestWillBeSent",
        &preflight("P3", "5.7", 1_020.0),
    );
    assert!(end_preflight(&mut assembler, "P3", 1_020.0).is_empty());
    let outputs = assembler.observe(
        &tab(now + SETTLE_GRACE_MS + 30.0),
        "Network.requestWillBeSent",
        &request("5.8", "https://a.example/later", 1_021.5),
    );
    let record = placed(ended(outputs).remove(0));
    assert_eq!((record.seq, record.preflight_of), (0, None));
}

/// Chromium 1243 on a busy page: `requestWillBeSent` of a preflight carried
/// a later timestamp than its own `loadingFinished`.
#[test]
fn a_preflight_announced_after_it_was_sent_starts_when_the_network_sent_it() {
    let preflight = |id: &str, at: f64| {
        let mut event = request(id, "https://bid.example/auction", at);
        event["type"] = json!("Other");
        event["request"]["method"] = json!("OPTIONS");
        event["initiator"] = json!({ "type": "preflight", "url": PAGE, "requestId": "5.5" });
        event
    };
    let mut assembler = Assembler::default();
    let now = epoch(1_020.0);
    assembler.observe(
        &tab(now),
        "Network.requestWillBeSent",
        &request("5.5", "https://bid.example/auction", 1_019.999),
    );

    // Announced at 1020.0040; the network sent it at 1020.0005, had the
    // headers 1.5 ms later and was done at 1020.0021.
    assembler.observe(
        &tab(now),
        "Network.requestWillBeSent",
        &preflight("P1", 1_020.004),
    );
    let mut answer = response("P1", 204, 1_020.004_5);
    answer["type"] = json!("Preflight");
    answer["hasExtraInfo"] = json!(false);
    answer["response"]["timing"]["requestTime"] = json!(1_020.000_5);
    answer["response"]["timing"]["receiveHeadersStart"] = json!(1.4);
    answer["response"]["timing"]["receiveHeadersEnd"] = json!(1.5);
    assembler.observe(&tab(now), "Network.responseReceived", &answer);
    let outputs = assembler.observe(
        &tab(now),
        "Network.loadingFinished",
        &finished("P1", 1_020.002_1, 0),
    );
    let seq = finished_of(&outputs).unwrap().seq;
    let record = placed(
        ended(assembler.body_settled(seq, CapturedBody::Skipped(BodySkip::Type), now)).remove(0),
    );
    assert_eq!(record.kind, ResourceKind::Preflight);
    let close = |actual: f64, expected: f64| (actual - expected).abs() < 1e-3;
    assert!(close(record.started_at_ms, 20_000.5), "{record:?}");
    assert!(
        close(record.response_at_ms.unwrap(), 20_002.0),
        "{record:?}"
    );
    assert!(close(record.ended_at_ms.unwrap(), 20_002.1), "{record:?}");
    assert_eq!(record.queued_ms, Some(0.0));

    // A preflight that fails has no network timing: it ends no earlier
    // than it was announced.
    assembler.observe(
        &tab(now),
        "Network.requestWillBeSent",
        &request("5.6", "https://bid.example/auction", 1_020.1),
    );
    let mut refused = preflight("P2", 1_020.104);
    refused["initiator"]["requestId"] = json!("5.6");
    assembler.observe(&tab(now), "Network.requestWillBeSent", &refused);
    assembler.observe(
        &tab(now),
        "Network.loadingFailed",
        &failed("P2", 1_020.102, "net::ERR_FAILED"),
    );
    let record = assembler
        .finish()
        .into_iter()
        .map(placed)
        .find(|record| record.kind == ResourceKind::Preflight && record.status.is_none())
        .unwrap();
    assert_eq!(record.ended_at_ms, Some(record.started_at_ms));
}

#[test]
fn where_a_response_came_from_is_read_from_the_browser() {
    let served = |change: &dyn Fn(&mut Value), memory_cache: bool| {
        let mut assembler = Assembler::default();
        let now = epoch(1_001.0);
        assembler.observe(
            &tab(now),
            "Network.requestWillBeSent",
            &request("1.1", "https://a.example/x", 1_001.0),
        );
        if memory_cache {
            assembler.observe(
                &tab(now),
                "Network.requestServedFromCache",
                &json!({ "requestId": "1.1" }),
            );
        }
        let mut answer = response("1.1", 200, 1_001.1);
        answer["hasExtraInfo"] = json!(false);
        change(&mut answer["response"]);
        assembler.observe(&tab(now), "Network.responseReceived", &answer);
        let outputs = assembler.observe(
            &tab(now),
            "Network.loadingFinished",
            &finished("1.1", 1_001.2, 0),
        );
        let seq = finished_of(&outputs).unwrap().seq;
        placed(ended(assembler.body_settled(seq, CapturedBody::Empty, now)).remove(0)).served_from
    };
    assert_eq!(served(&|_| {}, false), ServedFrom::Network);
    assert_eq!(served(&|_| {}, true), ServedFrom::MemoryCache);
    assert_eq!(
        served(&|response| response["fromDiskCache"] = json!(true), false),
        ServedFrom::DiskCache
    );
    assert_eq!(
        served(
            &|response| response["fromServiceWorker"] = json!(true),
            false
        ),
        ServedFrom::ServiceWorker
    );
    assert_eq!(
        served(
            &|response| {
                response["fromPrefetchCache"] = json!(true);
                response["fromDiskCache"] = json!(true);
            },
            false
        ),
        ServedFrom::Prefetch
    );
}

#[test]
fn the_events_of_one_request_may_come_from_several_sessions() {
    let mut assembler = Assembler::default();
    let now = epoch(1_030.0);
    // A worker's script: announced by the page, finished by the worker.
    let mut script = request("W1", "https://www.pub.example/worker.js", 1_030.0);
    script["type"] = json!("Script");
    assembler.observe(&tab(now), "Network.requestWillBeSent", &script);
    assembler.observe(
        &tab(now),
        "Network.requestWillBeSentExtraInfo",
        &request_extra("W1", "a=1"),
    );
    let worker = child("worker", Reporter::Worker, now);
    // The same request, should a second session announce it too.
    assert!(
        assembler
            .observe(&worker, "Network.requestWillBeSent", &script)
            .is_empty()
    );
    assembler.observe(
        &worker,
        "Network.responseReceivedExtraInfo",
        &response_extra("W1", "b=1"),
    );
    let mut answer = response("W1", 200, 1_030.1);
    answer["type"] = json!("Script");
    assembler.observe(&worker, "Network.responseReceived", &answer);
    let outputs = assembler.observe(
        &worker,
        "Network.loadingFinished",
        &finished("W1", 1_030.2, 400),
    );

    let offer = finished_of(&outputs).unwrap();
    assert_eq!(
        offer.session, "worker",
        "only the worker's session has the body"
    );
    assert!(!offer.in_tab);
    let mut records = ended(assembler.body_settled(offer.seq, CapturedBody::Empty, now));
    assert_eq!(records.len(), 1, "one request, one record");
    let record = placed(records.remove(0));
    assert_eq!(
        record.context,
        RequestContext::Page,
        "the page asked for it"
    );
    assert_eq!(record.target_id, None);
    assert_eq!(record.kind, ResourceKind::Script);

    // What the worker fetches itself: its ExtraInfo comes on the page's session.
    let mut fetch = request("W2", "https://www.pub.example/config.json", 1_031.0);
    fetch.as_object_mut().unwrap().remove("frameId");
    assembler.observe(&worker, "Network.requestWillBeSent", &fetch);
    assembler.observe(
        &tab(now),
        "Network.requestWillBeSentExtraInfo",
        &request_extra("W2", "c=1"),
    );
    assembler.observe(
        &tab(now),
        "Network.responseReceivedExtraInfo",
        &response_extra("W2", "d=1"),
    );
    assembler.observe(
        &worker,
        "Network.responseReceived",
        &response("W2", 200, 1_031.1),
    );
    let outputs = assembler.observe(
        &worker,
        "Network.loadingFinished",
        &finished("W2", 1_031.2, 400),
    );
    let seq = finished_of(&outputs).unwrap().seq;
    let record = placed(ended(assembler.body_settled(seq, CapturedBody::Empty, now)).remove(0));
    assert_eq!(record.context, RequestContext::Worker);
    assert_eq!(record.target_id.as_deref(), Some("child-target"));
    assert_eq!(record.frame_id, None);
    assert_eq!(header_of(&record.request_headers, "cookie"), Some("c=1"));
}

#[test]
fn the_frame_and_the_session_decide_where_a_request_was_made() {
    let mut assembler = Assembler::default();
    let mut in_frame = request("1.1", "https://ads.example/pixel", 1_001.0);
    in_frame["frameId"] = json!("child-frame");
    let record = {
        let now = epoch(1_001.0);
        assembler.observe(&tab(now), "Network.requestWillBeSent", &in_frame);
        assembler.observe(
            &tab(now),
            "Network.loadingFailed",
            &failed("1.1", 1_001.1, "net::ERR_FAILED"),
        );
        placed(assembler.finish().remove(0))
    };
    assert_eq!(
        record.context,
        RequestContext::Iframe,
        "a frame of the page's own process"
    );
    assert_eq!(record.target_id, None);

    for (reporter, context) in [
        (Reporter::Iframe, RequestContext::Iframe),
        (Reporter::Worker, RequestContext::Worker),
        (Reporter::ServiceWorker, RequestContext::ServiceWorker),
    ] {
        let now = epoch(1_002.0);
        let source = child("child", reporter, now);
        assembler.observe(
            &source,
            "Network.requestWillBeSent",
            &request("2.1", "https://ads.example/x", 1_002.0),
        );
        let record = placed(assembler.finish().remove(0));
        assert_eq!(record.context, context);
        assert_eq!(record.target_id.as_deref(), Some("child-target"));
    }
}

#[test]
fn failures_keep_their_reason_and_a_failed_bid_request_is_told() {
    let mut assembler = Assembler::default();
    let now = epoch(1_040.0);
    let blocked = {
        let mut event = failed("1.2", 1_040.1, "net::ERR_BLOCKED_BY_CLIENT");
        event["blockedReason"] = json!("inspector");
        event
    };
    let refused_by_cors = {
        let mut event = failed("1.3", 1_040.1, "net::ERR_FAILED");
        event["corsErrorStatus"] =
            json!({ "corsError": "MissingAllowOriginHeader", "failedParameter": "" });
        event
    };
    let aborted = {
        let mut event = failed("1.4", 1_040.1, "net::ERR_ABORTED");
        event["canceled"] = json!(true);
        event
    };
    let failures = [
        failed("1.1", 1_040.1, "net::ERR_CONNECTION_REFUSED"),
        blocked,
        refused_by_cors,
        aborted,
    ];
    for (index, failure) in failures.iter().enumerate() {
        let id = failure["requestId"].as_str().unwrap();
        let url = if index == 0 {
            "https://rtb.openx.net/openrtbb/prebidjs".to_string()
        } else {
            format!("https://a.example/{index}")
        };
        assembler.observe(
            &tab(now),
            "Network.requestWillBeSent",
            &request(id, &url, 1_040.0),
        );
        assembler.observe(&tab(now), "Network.loadingFailed", failure);
    }
    let failures = assembler.finish();
    assert_eq!(failures.len(), 4);
    let (bid, failed_at, why) = failures[0].failed_bid().expect("a bid request");
    assert_eq!(
        (bid.bidder, why.as_str()),
        ("openx", "net::ERR_CONNECTION_REFUSED")
    );
    assert_eq!(failed_at.round(), epoch(1_040.1).round());
    assert!(failures[1].failed_bid().is_none(), "not a bid request");

    let ends: Vec<RequestEnd> = failures
        .into_iter()
        .map(|ended| placed(ended).end)
        .collect();
    let failed = |error: &str, cancelled: bool, blocked_reason: Option<&str>| RequestEnd::Failed {
        error: error.into(),
        cancelled,
        blocked_reason: blocked_reason.map(str::to_string),
    };
    assert_eq!(
        ends,
        [
            failed("net::ERR_CONNECTION_REFUSED", false, None),
            failed("net::ERR_BLOCKED_BY_CLIENT", false, Some("inspector")),
            failed("net::ERR_FAILED", false, Some("MissingAllowOriginHeader")),
            failed("net::ERR_ABORTED", true, None),
        ]
    );
}

#[test]
fn a_failed_request_has_no_body_and_is_handed_over_once_its_headers_are_known() {
    let mut assembler = Assembler::default();
    let now = epoch(1_040.0);
    assembler.observe(
        &tab(now),
        "Network.requestWillBeSent",
        &request("1.1", "https://a.example/x", 1_040.0),
    );
    assert!(
        ended(assembler.observe(
            &tab(now),
            "Network.loadingFailed",
            &failed("1.1", 1_040.1, "net::ERR_TIMED_OUT")
        ))
        .is_empty()
    );
    let outputs = assembler.observe(
        &tab(now),
        "Network.requestWillBeSentExtraInfo",
        &request_extra("1.1", "sent=1"),
    );
    let record = placed(ended(outputs).remove(0));
    assert_eq!((record.status, record.response_at_ms), (None, None));
    assert_eq!(record.body, CapturedBody::Absent);
    assert_eq!(record.ended_at_ms.unwrap().round(), 40_100.0);
    assert_eq!(header_of(&record.request_headers, "cookie"), Some("sent=1"));
    assert!(record.response_headers.is_empty());
}

#[test]
fn what_is_still_open_at_the_end_is_handed_over_as_pending_in_order() {
    let mut assembler = Assembler::default();
    let now = epoch(1_050.0);
    // Never answered.
    assembler.observe(
        &tab(now),
        "Network.requestWillBeSent",
        &request("1.1", "https://a.example/never", 1_050.0),
    );
    // Answered, still streaming.
    let mut stream = request("1.2", "https://a.example/events", 1_050.1);
    stream["type"] = json!("EventSource");
    assembler.observe(&tab(now), "Network.requestWillBeSent", &stream);
    let mut answer = response("1.2", 200, 1_050.2);
    answer["type"] = json!("EventSource");
    assembler.observe(&tab(now), "Network.responseReceived", &answer);
    // Finished, but the browser never handed the body over.
    assembler.observe(
        &tab(now),
        "Network.requestWillBeSent",
        &request("1.3", "https://a.example/slow-body", 1_050.2),
    );
    assembler.observe(
        &tab(now),
        "Network.loadingFinished",
        &finished("1.3", 1_050.3, 10),
    );

    let records: Vec<NetworkRecord> = assembler.finish().into_iter().map(placed).collect();
    assert_eq!(records.iter().map(|r| r.seq).collect::<Vec<_>>(), [0, 1, 2]);
    assert_eq!(records[0].end, RequestEnd::Pending);
    assert_eq!((records[0].ended_at_ms, records[0].status), (None, None));
    assert_eq!(records[0].body, CapturedBody::Absent);
    assert_eq!(records[1].end, RequestEnd::Pending);
    assert_eq!(records[1].kind, ResourceKind::EventSource);
    assert_eq!(records[1].status, Some(200));
    assert!(records[1].response_at_ms.is_some() && records[1].ended_at_ms.is_none());
    assert_eq!(records[2].end, RequestEnd::Finished);
    assert!(matches!(records[2].body, CapturedBody::Unavailable(_)));

    assert!(assembler.finish().is_empty());
    // An answer that comes after the end changes nothing.
    assert!(
        assembler
            .body_settled(2, CapturedBody::Empty, now)
            .is_empty()
    );
}

#[test]
fn nothing_is_recorded_while_the_tab_shows_another_site() {
    let mut assembler = Assembler::default();
    let now = epoch(1_060.0);
    let elsewhere = Source {
        recording: false,
        ..tab(now)
    };
    assembler.observe(
        &elsewhere,
        "Network.requestWillBeSentExtraInfo",
        &request_extra("1.1", "x=1"),
    );
    assembler.observe(
        &elsewhere,
        "Network.requestWillBeSent",
        &request("1.1", "https://other.example/x", 1_060.0),
    );
    assembler.observe(
        &elsewhere,
        "Network.responseReceived",
        &response("1.1", 200, 1_060.1),
    );
    let outputs = assembler.observe(
        &elsewhere,
        "Network.loadingFinished",
        &finished("1.1", 1_060.2, 10),
    );
    assert!(outputs.is_empty());
    assembler.observe(
        &elsewhere,
        "Network.webSocketCreated",
        &json!({ "requestId": "1.2", "url": "wss://other.example/" }),
    );
    assert!(assembler.finish().is_empty());

    // What is only ever heard of through ExtraInfo is forgotten again.
    let mut assembler = Assembler::default();
    assembler.observe(
        &elsewhere,
        "Network.requestWillBeSentExtraInfo",
        &request_extra("1.3", "x=1"),
    );
    assembler.observe(
        &Source {
            now_ms: now + ORPHAN_PATIENCE_MS + 1.0,
            ..elsewhere
        },
        "Network.requestWillBeSentExtraInfo",
        &request_extra("1.4", "x=1"),
    );
    assert_eq!(assembler.chains.len(), 1);
}

fn navigation(id: &str, url: &str, at: f64) -> Value {
    let mut event = request(id, url, at);
    event["type"] = json!("Document");
    event["loaderId"] = json!(id);
    event["initiator"] = json!({ "type": "other" });
    event
}

#[test]
fn a_navigation_is_handed_over_when_its_document_commits_on_the_site() {
    let mut assembler = Assembler::default();
    let now = epoch(1_070.0);
    // Before the visit has reached its site nothing is recorded yet.
    let arriving = Source {
        recording: false,
        ..tab(now)
    };
    assembler.observe(
        &arriving,
        "Network.requestWillBeSent",
        &navigation("NAV", "http://pub.example/", 1_070.0),
    );
    let mut second = navigation("NAV", "https://www.pub.example/", 1_070.1);
    second["redirectResponse"] = json!({ "status": 301, "headers": {} });
    second["redirectHasExtraInfo"] = json!(false);
    assert!(
        ended(assembler.observe(&arriving, "Network.requestWillBeSent", &second)).is_empty(),
        "the first hop waits: its page view does not exist yet"
    );
    let mut answer = response("NAV", 200, 1_070.2);
    answer["type"] = json!("Document");
    answer["hasExtraInfo"] = json!(false);
    assembler.observe(&arriving, "Network.responseReceived", &answer);

    let released = ended(assembler.navigation_committed("NAV", true, now));
    assert_eq!(released.iter().map(Ended::seq).collect::<Vec<_>>(), [0]);
    let outputs = assembler.observe(
        &tab(now),
        "Network.loadingFinished",
        &finished("NAV", 1_070.3, 2_000),
    );
    let offer = finished_of(&outputs).unwrap();
    assert!(offer.main_document);
    let document =
        placed(ended(assembler.body_settled(offer.seq, CapturedBody::Empty, now)).remove(0));
    assert_eq!((document.seq, document.redirect_from), (1, Some(0)));
    assert_eq!(document.kind, ResourceKind::Document);
}

#[test]
fn a_navigation_that_never_shows_a_page_of_the_site_leaves_no_record() {
    // Another tab, or the recorded one while it is on another site.
    let now = epoch(1_080.0);
    let elsewhere = Source {
        recording: false,
        ..tab(now)
    };
    let mut assembler = Assembler::default();
    assembler.observe(
        &elsewhere,
        "Network.requestWillBeSent",
        &navigation("N1", "https://other.example/", 1_080.0),
    );
    assert!(assembler.navigation_committed("N1", false, now).is_empty());
    assert!(assembler.finish().is_empty(), "committed on another site");

    assembler.observe(
        &elsewhere,
        "Network.requestWillBeSent",
        &navigation("N2", "https://other.example/2", 1_081.0),
    );
    assembler.observe(
        &elsewhere,
        "Network.loadingFailed",
        &failed("N2", 1_081.1, "net::ERR_ABORTED"),
    );
    let later = Source {
        now_ms: now + SETTLE_GRACE_MS + 2_000.0,
        ..tab(now)
    };
    assert!(
        assembler
            .observe(
                &later,
                "Network.requestWillBeSent",
                &request("1.1", "https://a.example/x", 1_083.0)
            )
            .is_empty()
    );
    assert_eq!(assembler.finish().len(), 1, "only the page's own request");

    assembler.observe(
        &elsewhere,
        "Network.requestWillBeSent",
        &navigation("N3", "https://other.example/3", 1_084.0),
    );
    assert!(
        assembler.finish().is_empty(),
        "still loading when the capture ended"
    );
}

#[test]
fn a_navigation_away_from_the_site_is_a_request_of_the_page_that_made_it() {
    let mut assembler = Assembler::default();
    let now = epoch(1_090.0);
    // Refused by the navigation guard, or a download: it never commits.
    assembler.observe(
        &tab(now),
        "Network.requestWillBeSent",
        &navigation("N1", "https://other.example/", 1_090.0),
    );
    assembler.observe(
        &tab(now),
        "Network.loadingFailed",
        &failed("N1", 1_090.1, "net::ERR_ABORTED"),
    );
    let outputs = assembler.observe(
        &tab(now + SETTLE_GRACE_MS + 1.0),
        "Network.requestWillBeSent",
        &request("1.1", "https://a.example/x", 1_092.0),
    );
    let refused = placed(ended(outputs).remove(0));
    assert_eq!(refused.url, "https://other.example/");
    assert!(matches!(refused.end, RequestEnd::Failed { .. }));

    // One that does commit, on the other site.
    assembler.observe(
        &tab(now),
        "Network.requestWillBeSent",
        &navigation("N2", "https://other.example/2", 1_093.0),
    );
    let mut answer = response("N2", 200, 1_093.1);
    answer["hasExtraInfo"] = json!(false);
    assembler.observe(&tab(now), "Network.responseReceived", &answer);
    assert!(ended(assembler.navigation_committed("N2", false, now)).is_empty());
    let left = assembler.finish();
    assert_eq!(left.len(), 2);
    assert_eq!(
        placed(left.into_iter().last().unwrap()).url,
        "https://other.example/2"
    );
}

#[test]
fn a_websocket_is_one_record_from_its_creation_to_its_close() {
    let mut assembler = Assembler::default();
    let now = epoch(1_100.0);
    let created = json!({
        "requestId": "7.1",
        "url": "wss://live.pub.example/feed",
        "initiator": { "type": "script", "stack": { "callFrames": [
            { "functionName": "connect", "scriptId": "4", "url": "https://www.pub.example/live.js",
              "lineNumber": 7, "columnNumber": 2 } ] } },
    });
    assembler.observe(&tab(now), "Network.webSocketCreated", &created);
    assembler.observe(
        &tab(now),
        "Network.webSocketWillSendHandshakeRequest",
        &json!({ "requestId": "7.1", "timestamp": 1_100.5, "wallTime": epoch(1_100.5) / 1000.0,
                 "request": { "headers": { "Upgrade": "websocket", "Origin": "https://www.pub.example" } } }),
    );
    assembler.observe(
        &tab(now),
        "Network.webSocketHandshakeResponseReceived",
        &json!({ "requestId": "7.1", "timestamp": 1_100.6,
                 "response": { "status": 101, "statusText": "Switching Protocols", "headers": { "Upgrade": "websocket" } } }),
    );
    for _ in 0..3 {
        let frame = json!({ "requestId": "7.1", "timestamp": 1_100.7, "response": { "opcode": 1, "payloadData": "tick" } });
        assert!(
            assembler
                .observe(&tab(now), "Network.webSocketFrameReceived", &frame)
                .is_empty()
        );
    }
    let outputs = assembler.observe(
        &tab(now),
        "Network.webSocketClosed",
        &json!({ "requestId": "7.1", "timestamp": 1_102.0 }),
    );
    assert!(
        finished_of(&outputs).is_none(),
        "a socket has no body to read"
    );
    let socket = placed(ended(outputs).remove(0));
    assert_eq!(socket.kind, ResourceKind::WebSocket);
    assert_eq!(
        (socket.method.as_str(), socket.url.as_str()),
        ("GET", "wss://live.pub.example/feed")
    );
    assert_eq!(socket.status, Some(101));
    assert_eq!(
        header_of(&socket.request_headers, "upgrade"),
        Some("websocket")
    );
    assert_eq!(socket.started_at_ms.round(), 100_500.0);
    assert_eq!(socket.response_at_ms.unwrap().round(), 100_600.0);
    assert_eq!(socket.ended_at_ms.unwrap().round(), 102_000.0);
    assert_eq!(socket.end, RequestEnd::Finished);
    assert_eq!(socket.body, CapturedBody::Absent);
    assert_eq!(
        socket.initiator.stack[0].function.as_deref(),
        Some("connect")
    );

    // One that is still open when the capture ends.
    assembler.observe(
        &tab(now),
        "Network.webSocketCreated",
        &json!({ "requestId": "7.2", "url": "wss://live.pub.example/2" }),
    );
    let open = placed(assembler.finish().remove(0));
    assert_eq!(
        (open.kind, open.end),
        (ResourceKind::WebSocket, RequestEnd::Pending)
    );
}

#[test]
fn a_request_body_comes_with_the_request_or_is_asked_for() {
    let mut assembler = Assembler::default();
    let now = epoch(1_110.0);
    let post = |id: &str, body: Option<&str>| {
        let mut event = request(id, "https://rtb.openx.net/openrtbb/prebidjs", 1_110.0);
        event["request"]["method"] = json!("POST");
        event["request"]["hasPostData"] = json!(true);
        if let Some(body) = body {
            event["request"]["postData"] = json!(body);
        }
        event
    };
    let fail = |assembler: &mut Assembler, id: &str| {
        assembler.observe(
            &tab(now),
            "Network.requestWillBeSentExtraInfo",
            &request_extra(id, "c=1"),
        );
        assembler.observe(
            &tab(now),
            "Network.loadingFailed",
            &failed(id, 1_110.1, "net::ERR_FAILED"),
        )
    };

    // Sent along.
    assert!(
        assembler
            .observe(
                &tab(now),
                "Network.requestWillBeSent",
                &post("1.1", Some("{\"id\":\"a\"}"))
            )
            .is_empty()
    );
    let inline = placed(ended(fail(&mut assembler, "1.1")).remove(0));
    assert_eq!(inline.bidder.as_deref(), Some("openx"));
    let data = inline.post_data.unwrap();
    assert_eq!(
        (data.bytes.as_slice(), data.total_bytes),
        (b"{\"id\":\"a\"}".as_slice(), 10)
    );
    assert!(!inline.post_data_missing);

    // Asked for, and the request waits for the answer.
    let outputs = assembler.observe(&tab(now), "Network.requestWillBeSent", &post("1.2", None));
    assert!(matches!(
        &outputs[..],
        [Output::PostDataWanted { seq: 1, session, request_id }] if session == "tab" && request_id == "1.2"
    ));
    assert!(ended(fail(&mut assembler, "1.2")).is_empty());
    let asked = placed(ended(assembler.post_data_settled(1, Some("a=1&b=2"), now)).remove(0));
    assert_eq!(asked.post_data.unwrap().bytes, b"a=1&b=2");

    // The browser no longer has it.
    assembler.observe(&tab(now), "Network.requestWillBeSent", &post("1.3", None));
    fail(&mut assembler, "1.3");
    let lost = placed(ended(assembler.post_data_settled(2, None, now)).remove(0));
    assert_eq!((lost.post_data, lost.post_data_missing), (None, true));

    // An upload far beyond what is kept is not asked for at all.
    let mut upload = post("1.4", None);
    upload["request"]["headers"]["Content-Length"] = json!("73400320");
    assert!(
        assembler
            .observe(&tab(now), "Network.requestWillBeSent", &upload)
            .is_empty()
    );
    let upload = placed(ended(fail(&mut assembler, "1.4")).remove(0));
    assert_eq!((upload.post_data, upload.post_data_missing), (None, true));

    // A request without a body.
    let plain = fetched(&mut assembler, "1.5", "https://a.example/x", 1_111.0);
    assert_eq!((plain.post_data, plain.post_data_missing), (None, false));
}

#[test]
fn request_bodies_held_for_open_requests_are_bounded() {
    let mut assembler = Assembler::default();
    let now = epoch(1_120.0);
    let body = "x".repeat(adscan_model::MAX_POST_DATA_BYTES);
    let slots = MAX_HELD_POST_BYTES / body.len();
    for index in 0..slots + 2 {
        let mut event = request(&format!("1.{index}"), "https://a.example/upload", 1_120.0);
        event["request"]["method"] = json!("POST");
        event["request"]["hasPostData"] = json!(true);
        event["request"]["postData"] = json!(body);
        assembler.observe(&tab(now), "Network.requestWillBeSent", &event);
    }
    assert_eq!(assembler.held_post_bytes, MAX_HELD_POST_BYTES);
    let records: Vec<NetworkRecord> = assembler.finish().into_iter().map(placed).collect();
    let kept = records
        .iter()
        .filter(|record| record.post_data.is_some())
        .count();
    let missing = records
        .iter()
        .filter(|record| record.post_data_missing)
        .count();
    assert_eq!((kept, missing), (slots, 2));
}

#[test]
fn more_open_requests_than_are_followed_at_once_are_counted_not_kept() {
    let mut assembler = Assembler::default();
    let now = epoch(1_130.0);
    for index in 0..MAX_OPEN_REQUESTS + 5 {
        assembler.observe(
            &tab(now),
            "Network.requestWillBeSent",
            &request(&format!("1.{index}"), "https://a.example/poll", 1_130.0),
        );
    }
    assert_eq!(assembler.overflowed(), 5);
    assert_eq!(assembler.open_flights, MAX_OPEN_REQUESTS);
    // One ends: there is room for one more.
    assembler.observe(
        &tab(now),
        "Network.loadingFailed",
        &failed("1.0", 1_130.1, "net::ERR_FAILED"),
    );
    let later = tab(now + SETTLE_GRACE_MS + 1.0);
    assembler.observe(
        &later,
        "Network.requestWillBeSent",
        &request("2.1", "https://a.example/next", 1_132.0),
    );
    assert_eq!(assembler.overflowed(), 5);
    assert_eq!(assembler.finish().len(), MAX_OPEN_REQUESTS);
}

#[test]
fn addresses_that_are_their_own_content_are_recorded_without_it() {
    let mut assembler = Assembler::default();
    let now = epoch(1_140.0);
    let data_url = format!("data:text/javascript;base64,{}", "QUJD".repeat(200));
    let mut script = request("1.1", &data_url, 1_140.0);
    script["type"] = json!("Script");
    assembler.observe(&tab(now), "Network.requestWillBeSent", &script);
    let outputs = assembler.observe(
        &tab(now),
        "Network.loadingFinished",
        &finished("1.1", 1_140.0, 0),
    );
    let offer = finished_of(&outputs).unwrap();
    assert!(!offer.has_address_on_the_network);
    assert!(offer.url.starts_with("data:text/javascript;base64,[") && offer.url.len() < 80);

    assembler.observe(
        &tab(now),
        "Network.requestWillBeSent",
        &request("1.2", "blob:https://www.pub.example/3f6c", 1_140.0),
    );
    let outputs = assembler.observe(
        &tab(now),
        "Network.loadingFinished",
        &finished("1.2", 1_140.1, 0),
    );
    assert!(!finished_of(&outputs).unwrap().has_address_on_the_network);
}

#[test]
fn late_events_of_a_request_that_was_handed_over_start_nothing_new() {
    let mut assembler = Assembler::default();
    let record = fetched(&mut assembler, "1.1", "https://a.example/x", 1_150.0);
    assert_eq!(record.seq, 0);
    let now = epoch(1_151.0);
    for (method, event) in [
        (
            "Network.requestWillBeSentExtraInfo",
            request_extra("1.1", "late=1"),
        ),
        (
            "Network.dataReceived",
            json!({ "requestId": "1.1", "dataLength": 10 }),
        ),
        (
            "Network.requestWillBeSent",
            request("1.1", "https://a.example/x", 1_151.0),
        ),
        ("Network.loadingFinished", finished("1.1", 1_151.0, 1)),
    ] {
        assert!(
            assembler.observe(&tab(now), method, &event).is_empty(),
            "{method}"
        );
    }
    assert!(assembler.chains.is_empty());
    assert_eq!(
        fetched(&mut assembler, "1.2", "https://a.example/y", 1_152.0).seq,
        1
    );
}

#[test]
fn a_bid_request_is_recognised_and_its_preflight_is_not() {
    let mut assembler = Assembler::default();
    let bid = fetched(
        &mut assembler,
        "1.1",
        "https://hbopenbid.pubmatic.com/translator?source=prebid-client",
        1_160.0,
    );
    assert_eq!(bid.bidder.as_deref(), Some("pubmatic"));
    let other = fetched(
        &mut assembler,
        "1.2",
        "https://ads.pubmatic.com/AdServer/js/user_sync.html",
        1_161.0,
    );
    assert_eq!(other.bidder, None);
}

#[test]
fn a_redirect_whose_first_hop_went_unseen_does_not_get_that_hops_cookies() {
    // The first hop was made while the tab showed another site: its
    // announcement is not followed, its `ExtraInfo` arrives all the same.
    // Then the tab is on the scanned site and the second hop is announced.
    let mut assembler = Assembler::default();
    let elsewhere = Source {
        recording: false,
        ..tab(epoch(1_000.0))
    };
    assembler.observe(
        &elsewhere,
        "Network.requestWillBeSent",
        &request("7.1", "https://other.example/start", 1_000.0),
    );
    assembler.observe(
        &elsewhere,
        "Network.requestWillBeSentExtraInfo",
        &request_extra("7.1", "of_the_first_hop=secret"),
    );
    assembler.observe(
        &elsewhere,
        "Network.responseReceivedExtraInfo",
        &response_extra("7.1", "set_by_the_first_hop=secret"),
    );
    let now = epoch(1_000.5);
    let mut second = request("7.1", "https://www.pub.example/landed", 1_000.5);
    second["redirectResponse"] = response("7.1", 302, 1_000.4)["response"].clone();
    assembler.observe(&tab(now), "Network.requestWillBeSent", &second);
    assembler.observe(
        &tab(now),
        "Network.requestWillBeSentExtraInfo",
        &request_extra("7.1", "of_the_second_hop=1"),
    );
    assembler.observe(
        &tab(now),
        "Network.responseReceived",
        &response("7.1", 200, 1_000.6),
    );
    let outputs = assembler.observe(
        &tab(now),
        "Network.loadingFinished",
        &finished("7.1", 1_000.7, 200),
    );
    let seq = finished_of(&outputs).unwrap().seq;
    let mut records = ended(assembler.body_settled(seq, CapturedBody::Empty, now));
    assert_eq!(records.len(), 1);
    let record = placed(records.remove(0));
    assert_eq!(record.url, "https://www.pub.example/landed");
    // Which `ExtraInfo` belongs to this hop cannot be counted off, so it
    // has the headers of its own announcement and nobody's cookies.
    assert_eq!(header_of(&record.request_headers, "Cookie"), None);
    assert_eq!(header_of(&record.request_headers, "Accept"), Some("*/*"));
    assert_eq!(header_of(&record.response_headers, "Set-Cookie"), None);
    assert!(assembler.chains.is_empty());
}

#[test]
fn events_the_browser_sends_twice_make_one_record_and_one_body_request() {
    let mut assembler = Assembler::default();
    let now = epoch(1_200.0);
    let mut outputs = Vec::new();
    for (method, event) in [
        (
            "Network.requestWillBeSent",
            request("9.1", "https://a.example/x", 1_200.0),
        ),
        (
            "Network.requestWillBeSent",
            request("9.1", "https://a.example/x", 1_200.0),
        ),
        ("Network.responseReceived", {
            let mut response = response("9.1", 200, 1_200.1);
            response["hasExtraInfo"] = json!(false);
            response
        }),
        ("Network.responseReceived", {
            let mut response = response("9.1", 200, 1_200.1);
            response["hasExtraInfo"] = json!(false);
            response
        }),
        ("Network.loadingFinished", finished("9.1", 1_200.2, 300)),
        ("Network.loadingFinished", finished("9.1", 1_200.2, 300)),
        (
            "Network.loadingFailed",
            failed("9.1", 1_200.3, "net::ERR_ABORTED"),
        ),
    ] {
        outputs.extend(assembler.observe(&tab(now), method, &event));
    }
    let asked: Vec<&Output> = outputs
        .iter()
        .filter(|output| matches!(output, Output::Finished(_)))
        .collect();
    assert_eq!(asked.len(), 1, "the body is asked for once");
    let seq = finished_of(&outputs).unwrap().seq;
    let mut records = ended(assembler.body_settled(seq, CapturedBody::Empty, now));
    // Answering twice hands nothing over twice.
    assert!(
        assembler
            .body_settled(seq, CapturedBody::Empty, now)
            .is_empty()
    );
    assert_eq!(records.len(), 1);
    let record = placed(records.remove(0));
    assert_eq!((record.seq, &record.end), (0, &RequestEnd::Finished));
    assert_eq!(assembler.open_flights, 0);
    assert!(assembler.finish().is_empty());
}

#[test]
fn a_request_whose_target_dies_is_pending_at_the_end_and_its_lost_body_is_said_so() {
    let mut assembler = Assembler::default();
    let worker = |now| child("worker", Reporter::Worker, now);
    let now = epoch(1_300.0);
    // One request never ends: the worker was terminated mid-flight.
    assembler.observe(
        &worker(now),
        "Network.requestWillBeSent",
        &request("5.1", "https://a.example/stream", 1_300.0),
    );
    // Another finished, but the worker was gone before its body was read.
    assembler.observe(
        &worker(now),
        "Network.requestWillBeSent",
        &request("5.2", "https://a.example/data.json", 1_300.0),
    );
    assembler.observe(&worker(now), "Network.responseReceived", &{
        let mut response = response("5.2", 200, 1_300.1);
        response["hasExtraInfo"] = json!(false);
        response
    });
    let outputs = assembler.observe(
        &worker(now),
        "Network.loadingFinished",
        &finished("5.2", 1_300.2, 300),
    );
    assert_eq!(finished_of(&outputs).unwrap().session, "worker");

    let records: Vec<NetworkRecord> = assembler.finish().into_iter().map(placed).collect();
    assert_eq!(records.len(), 2);
    assert_eq!(
        (&records[0].end, &records[0].body, records[0].ended_at_ms),
        (&RequestEnd::Pending, &CapturedBody::Absent, None)
    );
    assert_eq!(records[1].end, RequestEnd::Finished);
    assert!(matches!(&records[1].body, CapturedBody::Unavailable(_)));
    assert_eq!(records[0].context, RequestContext::Worker);
    assert_eq!(assembler.open_flights, 0);
}

#[test]
fn a_request_id_that_comes_back_after_its_request_ended_is_not_a_second_request() {
    // Ids are the browser's and are not given out twice; events that name
    // an ended one are late or replayed, whatever they claim to start.
    let mut assembler = Assembler::default();
    let first = fetched(&mut assembler, "3.1", "https://a.example/one", 1_400.0);
    let now = epoch(1_405.0);
    let mut again = request("3.1", "https://a.example/two", 1_405.0);
    again["redirectResponse"] = response("3.1", 302, 1_404.9)["response"].clone();
    for (method, event) in [
        ("Network.requestWillBeSent", again),
        (
            "Network.webSocketCreated",
            json!({ "requestId": "3.1", "url": "wss://a.example/s" }),
        ),
        ("Network.responseReceived", response("3.1", 200, 1_405.1)),
        (
            "Network.loadingFailed",
            failed("3.1", 1_405.2, "net::ERR_FAILED"),
        ),
    ] {
        assert!(
            assembler.observe(&tab(now), method, &event).is_empty(),
            "{method}"
        );
    }
    assert!(assembler.finish().is_empty());
    assert_eq!(first.seq, 0);
    assert_eq!(assembler.next_seq, 1, "no number was spent on it");
}

fn base64_of(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// A piece of a response body as the browser reports it; `sent_along` when
/// it was asked to send the body with it.
fn data(id: &str, piece: &[u8], sent_along: bool) -> Value {
    let mut received = json!({ "requestId": id, "timestamp": 1_000.2, "dataLength": piece.len(), "encodedDataLength": piece.len() });
    if sent_along {
        received["data"] = json!(base64_of(piece));
    }
    received
}

fn copies_wanted(outputs: &[Output]) -> Vec<(&str, &str)> {
    outputs
        .iter()
        .filter_map(|output| match output {
            Output::BodyCopyWanted {
                session,
                request_id,
            } => Some((session.as_str(), request_id.as_str())),
            _ => None,
        })
        .collect()
}

#[test]
fn a_frame_or_worker_is_asked_to_send_along_what_may_be_kept_and_the_tab_is_not() {
    let mut assembler = Assembler::default();
    let now = epoch(1_000.0);
    let announce =
        |assembler: &mut Assembler, source: &Source<'_>, id: &str, kind: &str, url: &str| {
            let mut announced = request(id, url, 1_000.0);
            announced["type"] = json!(kind);
            assembler.observe(source, "Network.requestWillBeSent", &announced)
        };
    let frame = child("frame", Reporter::Iframe, now);
    let worker = child("worker", Reporter::Worker, now);

    let outputs = announce(
        &mut assembler,
        &frame,
        "F.1",
        "Fetch",
        "https://ads.example/config.json",
    );
    assert_eq!(copies_wanted(&outputs), [("frame", "F.1")]);
    let outputs = announce(
        &mut assembler,
        &worker,
        "W.1",
        "Script",
        "https://ads.example/lib.js",
    );
    assert_eq!(copies_wanted(&outputs), [("worker", "W.1")]);
    for (id, kind) in [
        ("F.2", "Image"),
        ("F.3", "Media"),
        ("F.4", "Font"),
        ("F.5", "EventSource"),
    ] {
        let outputs = announce(
            &mut assembler,
            &frame,
            id,
            kind,
            "https://ads.example/creative",
        );
        assert_eq!(copies_wanted(&outputs), [], "{kind}");
    }
    // A bid request is read whatever the browser takes its answer for.
    let outputs = announce(
        &mut assembler,
        &frame,
        "F.6",
        "Image",
        "https://ib.adnxs.com/ut/v3/prebid",
    );
    assert_eq!(copies_wanted(&outputs), [("frame", "F.6")]);
    // The tab is there as long as the visit: its bodies are read from it.
    let outputs = announce(
        &mut assembler,
        &tab(now),
        "T.1",
        "Fetch",
        "https://ads.example/config.json",
    );
    assert_eq!(copies_wanted(&outputs), []);
}

/// Announces a fetch in a frame and returns its `seq` once it finished with
/// `pieces` as its body, each reported at `sent_along`.
fn finished_in_a_frame(assembler: &mut Assembler, id: &str, pieces: &[(&[u8], bool)]) -> u32 {
    let frame = child("frame", Reporter::Iframe, epoch(1_000.0));
    assembler.observe(
        &frame,
        "Network.requestWillBeSent",
        &request(id, "https://ads.example/config.json", 1_000.0),
    );
    assembler.observe(
        &frame,
        "Network.responseReceived",
        &response(id, 200, 1_000.1),
    );
    for (piece, sent_along) in pieces {
        assembler.observe(
            &frame,
            "Network.dataReceived",
            &data(id, piece, *sent_along),
        );
    }
    let outputs = assembler.observe(
        &frame,
        "Network.loadingFinished",
        &finished(id, 1_000.3, 40),
    );
    finished_of(&outputs).expect("the body can be read").seq
}

#[test]
fn a_body_that_was_sent_along_is_whole_once_its_start_and_every_piece_are_in() {
    // The answer to the question arrives after pieces that were sent later.
    let mut assembler = Assembler::default();
    let seq = finished_in_a_frame(
        &mut assembler,
        "F.1",
        &[(b"{\"ads\":", false), (b"[1,2,", true), (b"3]}", true)],
    );
    assembler.body_copy_started("F.1", &base64_of(b"{\"ads\":"));
    assert_eq!(
        assembler.take_body_copy(seq).as_deref(),
        Some(&b"{\"ads\":[1,2,3]}"[..])
    );
    assert_eq!(assembler.take_body_copy(seq), None, "handed over once");

    // The browser never said that it sends the body along: its first piece
    // is missing, and nothing says so but the length.
    let mut assembler = Assembler::default();
    let seq = finished_in_a_frame(
        &mut assembler,
        "F.1",
        &[(b"{\"ads\":", false), (b"[1,2,3]}", true)],
    );
    assert_eq!(assembler.take_body_copy(seq), None);

    // It said so, but what it had until then is not all that had arrived.
    let mut assembler = Assembler::default();
    let seq = finished_in_a_frame(
        &mut assembler,
        "F.1",
        &[(b"{\"ads\":", false), (b"[1,2,3]}", true)],
    );
    assembler.body_copy_started("F.1", &base64_of(b"{\"ad"));
    assert_eq!(assembler.take_body_copy(seq), None);

    // An empty answer is whole without a single piece.
    let mut assembler = Assembler::default();
    let seq = finished_in_a_frame(&mut assembler, "F.1", &[]);
    assembler.body_copy_started("F.1", "");
    assert_eq!(assembler.take_body_copy(seq), Some(Vec::new()));
}

#[test]
fn a_redirected_request_keeps_its_copy_for_the_answer_at_the_end() {
    let mut assembler = Assembler::default();
    let frame = child("frame", Reporter::Iframe, epoch(1_000.0));
    let outputs = assembler.observe(
        &frame,
        "Network.requestWillBeSent",
        &request("F.1", "https://ads.example/go", 1_000.0),
    );
    assert_eq!(copies_wanted(&outputs).len(), 1);
    let mut second_hop = request("F.1", "https://cdn.example/ad.json", 1_000.1);
    second_hop["redirectResponse"] = json!({ "status": 302, "headers": {} });
    second_hop["redirectHasExtraInfo"] = json!(false);
    let outputs = assembler.observe(&frame, "Network.requestWillBeSent", &second_hop);
    assert_eq!(copies_wanted(&outputs), [], "asked once per request");
    assembler.body_copy_started("F.1", "");

    assembler.observe(
        &frame,
        "Network.responseReceived",
        &response("F.1", 200, 1_000.2),
    );
    assembler.observe(
        &frame,
        "Network.dataReceived",
        &data("F.1", b"{\"ad\":1}", true),
    );
    let outputs = assembler.observe(
        &frame,
        "Network.loadingFinished",
        &finished("F.1", 1_000.3, 40),
    );
    let seq = finished_of(&outputs).unwrap().seq;
    assert_eq!(
        assembler.take_body_copy(seq).as_deref(),
        Some(&b"{\"ad\":1}"[..])
    );
}

#[test]
fn copies_are_given_up_beyond_what_is_held_at_once_and_let_go_with_their_request() {
    let mut assembler = Assembler::default();
    let frame = child("frame", Reporter::Iframe, epoch(1_000.0));
    let megabyte = vec![b'x'; 1024 * 1024];
    let open = |assembler: &mut Assembler, id: &str, megabytes: usize| {
        assembler.observe(
            &frame,
            "Network.requestWillBeSent",
            &request(id, "https://ads.example/large.json", 1_000.0),
        );
        assembler.body_copy_started(id, "");
        assembler.observe(
            &frame,
            "Network.responseReceived",
            &response(id, 200, 1_000.1),
        );
        for _ in 0..megabytes {
            assembler.observe(&frame, "Network.dataReceived", &data(id, &megabyte, true));
        }
    };
    let finish = |assembler: &mut Assembler, id: &str| {
        let outputs = assembler.observe(
            &frame,
            "Network.loadingFinished",
            &finished(id, 1_000.3, 40),
        );
        finished_of(&outputs).unwrap().seq
    };
    let megabytes_held = |assembler: &Assembler| assembler.copied_bytes / megabyte.len();

    open(&mut assembler, "F.1", 20);
    assert_eq!(megabytes_held(&assembler), 20);
    // The second does not fit next to the first: it is given up as a whole.
    open(&mut assembler, "F.2", 20);
    assert_eq!(megabytes_held(&assembler), 20);
    let second = finish(&mut assembler, "F.2");
    assert_eq!(assembler.take_body_copy(second), None);
    let first = finish(&mut assembler, "F.1");
    assert_eq!(
        assembler.take_body_copy(first).map(|body| body.len()),
        Some(20 * megabyte.len())
    );
    assert_eq!(megabytes_held(&assembler), 0);

    // A copy goes when the frame handed the body over itself, when the
    // request failed, and at the end of the capture.
    open(&mut assembler, "F.3", 2);
    let read_from_the_frame = finish(&mut assembler, "F.3");
    open(&mut assembler, "F.4", 3);
    open(&mut assembler, "F.5", 4);
    assert_eq!(megabytes_held(&assembler), 9);
    assembler.body_settled(read_from_the_frame, CapturedBody::Empty, epoch(1_000.4));
    assert_eq!(megabytes_held(&assembler), 7);
    assembler.observe(
        &frame,
        "Network.loadingFailed",
        &failed("F.4", 1_000.4, "net::ERR_ABORTED"),
    );
    assert_eq!(megabytes_held(&assembler), 4);
    assembler.finish();
    assert_eq!(megabytes_held(&assembler), 0);
}

const REMOTE: Limits = Limits {
    copy_bodies_from: Some(4_000),
    post_length_must_be_known: true,
};

fn received(id: &str, bytes: u64) -> Value {
    json!({ "requestId": id, "timestamp": 1_200.1, "dataLength": bytes, "encodedDataLength": bytes })
}

/// The connection to a remote browser cannot carry a large body in one
/// answer: a response of the tab that grows towards that size is asked to
/// be sent along from then on, once.
#[test]
fn a_response_of_the_tab_that_grows_large_is_asked_to_be_sent_along_once() {
    let mut assembler = Assembler::within(REMOTE);
    let now = epoch(1_200.0);
    let mut observe = |method: &str, params: &Value| assembler.observe(&tab(now), method, params);
    observe(
        "Network.requestWillBeSent",
        &request("9.1", "https://cdn.example/bundle.js", 1_200.0),
    );

    assert!(copies_wanted(&observe("Network.dataReceived", &received("9.1", 3_999))).is_empty());
    assert_eq!(
        copies_wanted(&observe("Network.dataReceived", &received("9.1", 1))),
        [("tab", "9.1")]
    );
    assert!(copies_wanted(&observe("Network.dataReceived", &received("9.1", 50_000))).is_empty());

    // An image is never text and never kept: nothing is asked for it.
    let mut image = request("9.2", "https://cdn.example/hero.jpg", 1_200.0);
    image["type"] = json!("Image");
    observe("Network.requestWillBeSent", &image);
    assert!(copies_wanted(&observe("Network.dataReceived", &received("9.2", 90_000))).is_empty());
}

#[test]
fn a_local_browser_is_never_asked_to_send_a_tabs_response_along() {
    let mut assembler = Assembler::default();
    let now = epoch(1_200.0);
    assembler.observe(
        &tab(now),
        "Network.requestWillBeSent",
        &request("9.1", "https://cdn.example/bundle.js", 1_200.0),
    );

    let outputs = assembler.observe(
        &tab(now),
        "Network.dataReceived",
        &received("9.1", 900_000_000),
    );

    assert!(outputs.is_empty());
}

/// A copy that began in the middle of a response is whole when what had
/// arrived before, as the browser hands it over, and every piece since
/// add up to the response.
#[test]
fn a_copy_that_began_while_the_response_loaded_is_the_whole_body() {
    let base64 = |text: &str| base64::engine::general_purpose::STANDARD.encode(text);
    let mut assembler = Assembler::within(REMOTE);
    let now = epoch(1_200.0);
    let first = "a".repeat(4_000);
    assembler.observe(
        &tab(now),
        "Network.requestWillBeSent",
        &request("9.1", "https://cdn.example/bundle.js", 1_200.0),
    );
    let outputs = assembler.observe(&tab(now), "Network.dataReceived", &received("9.1", 4_000));
    assert_eq!(copies_wanted(&outputs), [("tab", "9.1")]);
    // A piece sent along is seen before the answer that started the copy.
    let mut later = received("9.1", 3);
    later["data"] = json!(base64("bcd"));
    assembler.observe(&tab(now), "Network.dataReceived", &later);
    assembler.body_copy_started("9.1", &base64(&first));
    let outputs = assembler.observe(
        &tab(now),
        "Network.loadingFinished",
        &finished("9.1", 1_200.3, 4_100),
    );
    let finished = finished_of(&outputs).unwrap();

    assert_eq!(finished.expected_bytes, 4_003);
    assert_eq!(
        assembler.take_body_copy(finished.seq),
        Some(format!("{first}bcd").into_bytes())
    );
}

/// A request body whose size is not known could be any size: over a
/// connection that carries no large answer it is not asked for.
#[test]
fn a_request_body_of_unknown_length_is_not_asked_of_a_remote_browser() {
    let now = epoch(1_110.0);
    let post = |id: &str, length: Option<&str>| {
        let mut event = request(id, "https://rtb.openx.net/openrtbb/prebidjs", 1_110.0);
        event["request"]["method"] = json!("POST");
        event["request"]["hasPostData"] = json!(true);
        if let Some(length) = length {
            event["request"]["headers"]["Content-Length"] = json!(length);
        }
        event
    };
    let asked_for = |limits: Limits, length: Option<&str>| {
        let mut assembler = Assembler::within(limits);
        let outputs =
            assembler.observe(&tab(now), "Network.requestWillBeSent", &post("1.1", length));
        outputs
            .iter()
            .any(|output| matches!(output, Output::PostDataWanted { .. }))
    };

    assert!(asked_for(Limits::default(), None));
    assert!(!asked_for(REMOTE, None));
    assert!(asked_for(REMOTE, Some("2000000")));
    assert!(!asked_for(REMOTE, Some("9000000")));
    assert!(!asked_for(Limits::default(), Some("9000000")));
}
