//! Every request of a visit is recorded and handed to the network sink: the
//! page's own, those of its iframes and workers, with their headers, timing,
//! links and, for what the sink admits, their bodies.

mod support;

use std::collections::{BTreeMap, BTreeSet};
use std::net::Ipv4Addr;
use std::sync::Arc;

use adscan_model::{
    BodyAdmission, BodySkip, CapturedBody, DeviceProfile, InitiatorKind, NetworkRecord, PageReach,
    RequestContext, RequestEnd, ResourceKind, ServedFrom, TrackerEventKind as Kind,
    VisitorBehavior,
};
use support::{
    Fixtures, KEPT_BODY_BYTES, LARGE_JSON_BYTES, Reported, Requests, body_text,
    capture_with_requests, header_value,
};

/// Path and query of a record's address.
fn asked(record: &NetworkRecord) -> &str {
    let after_scheme = record.url.split_once("://").map_or("", |(_, rest)| rest);
    after_scheme.find('/').map_or("", |at| &after_scheme[at..])
}

fn path(record: &NetworkRecord) -> &str {
    asked(record).split('?').next().unwrap_or_default()
}

/// The one record asked for exactly `path_and_query`.
#[track_caller]
fn the<'a>(records: &'a [NetworkRecord], path_and_query: &str) -> &'a NetworkRecord {
    let found: Vec<&NetworkRecord> = records
        .iter()
        .filter(|record| asked(record) == path_and_query)
        .collect();
    assert_eq!(
        found.len(),
        1,
        "{path_and_query} should be recorded exactly once, among {:#?}",
        records.iter().map(asked).collect::<Vec<_>>()
    );
    found[0]
}

#[track_caller]
fn assert_every_request_has_one_record(records: &[NetworkRecord]) {
    let numbers: BTreeSet<u32> = records.iter().map(|record| record.seq).collect();
    assert_eq!(
        numbers.len(),
        records.len(),
        "a request was handed over twice"
    );
    assert_eq!(
        numbers.last().copied(),
        Some(records.len() as u32 - 1),
        "requests are numbered without gaps"
    );
}

/// A scrolled desktop visit of the demo site, long enough for its user sync.
async fn demo_visit() -> (adscan_model::Capture, Vec<NetworkRecord>) {
    let site = adscan_browser::start_demo_site().await.unwrap();
    let mut request = support::request(&site.url, DeviceProfile::Desktop);
    request.visitor = VisitorBehavior::Scroll;
    request.observe_seconds = 9;
    capture_with_requests(request).await
}

#[tokio::test]
async fn the_demo_visit_is_recorded_on_every_target_with_links_and_bodies() {
    let (capture, records) = demo_visit().await;
    assert_every_request_has_one_record(&records);
    assert!(records.iter().all(|record| record.page == 0));
    let by_kind = |kind: ResourceKind| records.iter().filter(move |record| record.kind == kind);

    // -- The page itself, with what the browser really sent and received.
    let document = the(&records, "/");
    assert_eq!(document.seq, 0, "the visit starts with its document");
    assert_eq!(
        (document.kind, document.context, document.status),
        (ResourceKind::Document, RequestContext::Page, Some(200))
    );
    assert!(
        document.started_at_ms.abs() < 50.0,
        "{}",
        document.started_at_ms
    );
    assert!(
        body_text(document)
            .unwrap()
            .contains("<title>The tide-powered town")
    );
    assert!(
        header_value(&document.request_headers, "accept-encoding").is_some(),
        "the headers as sent, not the provisional ones: {:?}",
        document.request_headers
    );
    assert!(header_value(&document.request_headers, "user-agent").is_some());
    assert_eq!(
        header_value(&document.response_headers, "cache-control"),
        Some("no-store")
    );
    assert_eq!(document.protocol.as_deref(), Some("http/1.1"));
    assert!(
        document
            .remote_address
            .as_deref()
            .unwrap()
            .starts_with("127.0.0.1:")
    );
    assert!(document.priority.is_some());
    assert_eq!(document.served_from, ServedFrom::Network);
    let timing = document
        .timing
        .expect("a request on the network has its phases");
    assert!(timing.send_start.is_some() && timing.receive_headers_end.is_some());
    assert!(document.queued_ms.is_some());
    let (response_at, ended_at) = (
        document.response_at_ms.unwrap(),
        document.ended_at_ms.unwrap(),
    );
    assert!(document.started_at_ms <= response_at && response_at <= ended_at);
    assert!(document.encoded_bytes > 0 && document.decoded_bytes > 5_000);

    // -- Kept classes come with their body; images and fonts do not.
    let library = the(&records, "/static/ads/prebid.js");
    let held = capture
        .scripts
        .iter()
        .find(|script| script.url.ends_with("/static/ads/prebid.js"))
        .unwrap();
    assert_eq!(body_text(library).as_deref(), Some(held.body.as_str()));
    assert_eq!(held.request_seq, Some(library.seq), "one read serves both");
    assert_eq!(library.initiator.kind, InitiatorKind::Parser);
    let styles = the(&records, "/static/css/site.css");
    assert_eq!(styles.kind, ResourceKind::Stylesheet);
    assert!(body_text(styles).unwrap().contains("@font-face"));
    let index = the(&records, "/api/archive-index.json");
    let CapturedBody::Read { bytes, total_bytes } = &index.body else {
        panic!("{:?}", index.body);
    };
    assert!(*total_bytes > 1024 * 1024 && bytes.len() as u64 == *total_bytes);
    assert_eq!(index.decoded_bytes, *total_bytes);
    assert!(serde_json::from_slice::<serde_json::Value>(bytes).is_ok());
    let image = the(&records, "/static/img/harbour-dusk.png");
    assert_eq!(
        (image.kind, image.mime_type.as_deref(), &image.body),
        (
            ResourceKind::Image,
            Some("image/png"),
            &CapturedBody::Skipped(BodySkip::Type)
        )
    );
    assert_eq!(image.decoded_bytes, 498);
    let font = the(&records, "/static/fonts/meridian-initial.ttf");
    assert_eq!(
        (font.kind, &font.body),
        (ResourceKind::Font, &CapturedBody::Skipped(BodySkip::Type))
    );

    // -- A worker: its script is the page's request, its fetch the worker's.
    let worker_script = the(&records, "/static/js/analytics-worker.js");
    assert_eq!(worker_script.context, RequestContext::Page);
    assert!(body_text(worker_script).unwrap().contains("sampling.json"));
    let rules = the(&records, "/collect/sampling.json");
    assert_eq!(rules.context, RequestContext::Worker);
    assert!(rules.target_id.is_some() && rules.frame_id.is_none());
    assert_eq!(
        body_text(rules).as_deref(),
        Some(r#"{"pageView":1,"scrollDepth":0.5,"engagement":1}"#)
    );
    assert!(
        capture
            .scripts
            .iter()
            .all(|script| !script.url.contains("analytics-worker")),
        "the script library holds what the page itself runs"
    );

    // -- The partner's sync frame, in a process of its own.
    let frame = the(&records, "/usersync/frame.html?bidder=appnexus");
    assert_eq!(
        (frame.kind, frame.context),
        (ResourceKind::Document, RequestContext::Iframe)
    );
    assert!(frame.url.starts_with("http://localhost:"));
    assert_ne!(frame.frame_id, document.frame_id);
    assert!(body_text(frame).unwrap().contains("/usersync/ids.json"));
    let ids = the(&records, "/usersync/ids.json");
    assert_eq!(ids.context, RequestContext::Iframe);
    assert!(
        ids.target_id.is_some(),
        "reported by the frame's own session"
    );
    assert_eq!(ids.frame_id, frame.frame_id);
    assert!(body_text(ids).unwrap().contains("mp-7f3a9c1e5b"));
    let frame_pixel = the(&records, "/usersync/done.gif?from=frame&uid=mp-7f3a9c1e5b");
    assert_eq!(frame_pixel.context, RequestContext::Iframe);
    assert_eq!(ids.initiator.kind, InitiatorKind::Script);
    assert_eq!(
        ids.initiator.stack[0].url, frame.url,
        "asked for by the script in the frame's document"
    );

    // -- The cookie sync: three hops, each a record, linked in order.
    let start = the(&records, "/usersync/start?bidder=rubicon");
    let matched = the(&records, "/usersync/match?partner=meridian");
    let pixel = the(&records, "/usersync/done.gif?uid=mp-7f3a9c1e5b");
    assert_eq!(
        [
            start.redirect_from,
            matched.redirect_from,
            pixel.redirect_from
        ],
        [None, Some(start.seq), Some(matched.seq)]
    );
    assert_eq!(
        [start.status, matched.status, pixel.status],
        [Some(302), Some(302), Some(200)]
    );
    assert_eq!(
        [&start.end, &matched.end, &pixel.end],
        [
            &RequestEnd::Redirected,
            &RequestEnd::Redirected,
            &RequestEnd::Finished
        ]
    );
    assert_eq!(
        [&start.body, &matched.body],
        [&CapturedBody::Absent, &CapturedBody::Absent]
    );
    assert!(
        header_value(&matched.response_headers, "set-cookie")
            .is_some_and(|cookie| cookie.starts_with("uid=mp-7f3a9c1e5b")),
        "{:?}",
        matched.response_headers
    );
    assert!(
        matched.url.starts_with("http://localhost:") && pixel.url.starts_with("http://127.0.0.1:")
    );
    // A redirect is one moment that the record states twice: as the end of
    // the hop before, on the clock that timed that hop, and as the start of
    // the hop after, on the wall clock. How far the two readings are apart
    // depends on how busy the browser is when it takes them, so they are not
    // compared; that the hops follow one another in time is.
    assert!(start.started_at_ms <= start.ended_at_ms.unwrap());
    assert!(matched.started_at_ms <= matched.ended_at_ms.unwrap());
    assert!(
        start.started_at_ms <= matched.started_at_ms
            && matched.started_at_ms <= pixel.started_at_ms
    );
    assert_eq!(pixel.kind, ResourceKind::Image);

    // -- Bid requests: recognised, with the request and the answer.
    let bids: BTreeMap<&str, Vec<&NetworkRecord>> = records
        .iter()
        .filter(|record| record.bidder.is_some())
        .fold(BTreeMap::new(), |mut by_bidder, record| {
            by_bidder
                .entry(record.bidder.as_deref().unwrap())
                .or_default()
                .push(record);
            by_bidder
        });
    assert_eq!(
        bids.keys().copied().collect::<Vec<_>>(),
        [
            "appnexus",
            "criteo",
            "ix",
            "openx",
            "pubmatic",
            "rubicon",
            "triplelift"
        ]
    );
    for (bidder, requests) in &bids {
        for request in requests {
            assert_eq!(request.method, "POST", "{bidder}");
            let sent: serde_json::Value =
                serde_json::from_slice(&request.post_data.as_ref().unwrap().bytes).unwrap();
            assert!(
                sent["id"].is_string() && sent["imp"][0]["id"].is_string(),
                "{bidder}: {sent}"
            );
            let answer: serde_json::Value =
                serde_json::from_str(&body_text(request).unwrap()).unwrap();
            assert!(
                answer["seatbid"].is_array() || answer["nbr"] == 7,
                "{bidder}: {answer}"
            );
            assert!(
                request.initiator.stack.iter().any(|frame| frame
                    .script_url
                    .as_deref()
                    .is_some_and(|url| url.ends_with("/static/ads/prebid.js"))),
                "{bidder}: the stack names the library's script: {:#?}",
                request.initiator.stack
            );
        }
    }
    let openx = bids["openx"][0];
    assert_eq!(
        body_text(openx).as_deref(),
        Some(r#"{"id":"meridian-post","nbr":7}"#)
    );
    assert_eq!(
        bids["triplelift"].len(),
        1,
        "only the ad unit with an inventory code is sent"
    );
    // What the capture says about the answers names the request each came in.
    for answer in &capture.bid_responses {
        let request = &records[answer.request_seq.expect("linked") as usize];
        assert_eq!(request.bidder.as_deref(), Some(answer.bidder.as_str()));
        assert_eq!(request.page, answer.page);
        assert!(
            (request.ended_at_ms.unwrap() - answer.t).abs() < 10.0,
            "{}: {} against {}",
            answer.bidder,
            request.ended_at_ms.unwrap(),
            answer.t
        );
    }
    assert_eq!(
        capture.bid_responses.len(),
        bids.values().map(Vec::len).sum::<usize>()
    );

    // -- The exchange at the second origin is cleared by a preflight first.
    let appnexus = bids["appnexus"][0];
    assert!(appnexus.url.starts_with("http://localhost:"));
    let preflight = by_kind(ResourceKind::Preflight)
        .find(|preflight| preflight.preflight_of == Some(appnexus.seq))
        .expect("the preflight of the first appnexus request");
    assert_eq!(
        (
            preflight.method.as_str(),
            preflight.status,
            preflight.url.as_str()
        ),
        ("OPTIONS", Some(204), appnexus.url.as_str())
    );
    assert_eq!(
        preflight.bidder, None,
        "the preflight is not the bid request"
    );
    assert!(
        header_value(&preflight.request_headers, "access-control-request-method").is_some(),
        "{:?}",
        preflight.request_headers
    );
    assert!(by_kind(ResourceKind::Preflight).all(|preflight| preflight.preflight_of.is_some()));
    for preflight in by_kind(ResourceKind::Preflight) {
        let ended = preflight.ended_at_ms.unwrap();
        assert!(
            preflight.started_at_ms <= preflight.response_at_ms.unwrap()
                && ended >= preflight.started_at_ms,
            "{} started at {}, ended at {ended}",
            preflight.url,
            preflight.started_at_ms
        );
    }

    // -- A bidder slower than the auction: on the network, its answer is late.
    let auction_end: BTreeMap<&str, f64> = capture
        .events
        .iter()
        .filter_map(|event| match &event.kind {
            Kind::AuctionEnd { auction_id, .. } => Some((auction_id.as_str(), event.t)),
            _ => None,
        })
        .collect();
    for late in ["pubmatic", "criteo"] {
        for request in &bids[late] {
            let sent: serde_json::Value =
                serde_json::from_slice(&request.post_data.as_ref().unwrap().bytes).unwrap();
            let closed_at = auction_end[sent["source"]["tid"].as_str().unwrap()];
            let answered_at = request.ended_at_ms.unwrap();
            assert!(
                answered_at > closed_at + 50.0,
                "{late} answered at {answered_at}, its auction closed at {closed_at}"
            );
            assert!(answered_at - request.started_at_ms >= 330.0);
        }
    }
    // The page's clock and the network's agree: a bid request leaves when
    // Prebid says it asks the bidder.
    for event in &capture.events {
        let Kind::BidRequested {
            bidder,
            bidder_request_id,
            ..
        } = &event.kind
        else {
            continue;
        };
        let request_id = bidder_request_id
            .as_deref()
            .expect("the tracker reports it");
        let Some(request) = bids
            .get(bidder.as_str())
            .into_iter()
            .flatten()
            .find(|request| {
                String::from_utf8_lossy(&request.post_data.as_ref().unwrap().bytes)
                    .contains(request_id)
            })
        else {
            // No valid bid: nothing was sent.
            assert_eq!(bidder, "triplelift");
            continue;
        };
        let apart = request.started_at_ms - event.t;
        assert!((-3.0..100.0).contains(&apart), "{bidder}: {apart} ms apart");
    }

    // -- The identity provider hangs up: a failed request, with the reason.
    let identity = records
        .iter()
        .find(|record| path(record) == "/identity/envelope")
        .expect("the identity call");
    let RequestEnd::Failed {
        error,
        cancelled,
        blocked_reason,
    } = &identity.end
    else {
        panic!("{:?}", identity.end);
    };
    assert!(error.starts_with("net::ERR_"), "{error}");
    assert_eq!((*cancelled, blocked_reason), (false, &None));
    assert_eq!(
        (identity.status, &identity.body),
        (None, &CapturedBody::Absent)
    );
    assert!(identity.ended_at_ms.is_some());

    // -- A beacon: metadata and the body that was posted.
    let beacon = records
        .iter()
        .find(|record| path(record) == "/collect")
        .unwrap();
    assert_eq!(
        (beacon.kind, beacon.status),
        (ResourceKind::Ping, Some(204))
    );
    assert!(
        String::from_utf8_lossy(&beacon.post_data.as_ref().unwrap().bytes).contains("page_view")
    );
}

#[tokio::test]
async fn requests_of_every_kind_end_in_the_state_they_were_left_in() {
    let fixtures = Fixtures::start().await;
    // A port nobody listens on.
    let closed = {
        let listener = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        listener.local_addr().unwrap().port()
    };
    let mut request = support::request(
        &fixtures.url(&format!("network-kinds.html?closed={closed}")),
        DeviceProfile::Desktop,
    );
    request.observe_seconds = 4;
    let (capture, records) = capture_with_requests(request).await;
    assert_every_request_has_one_record(&records);
    assert_eq!(
        capture.environment.request_hosts,
        ["127.0.0.1", "localhost"],
        "the closed port and the other site"
    );

    // -- Still open when the visit ended.
    let pending = the(&records, "/never?pending");
    assert_eq!(
        (
            &pending.end,
            pending.status,
            pending.ended_at_ms,
            &pending.body
        ),
        (&RequestEnd::Pending, None, None, &CapturedBody::Absent)
    );
    let stream = the(&records, "/events");
    assert_eq!(
        (stream.kind, &stream.end, stream.status),
        (ResourceKind::EventSource, &RequestEnd::Pending, Some(200))
    );
    assert_eq!(stream.mime_type.as_deref(), Some("text/event-stream"));
    assert!(stream.response_at_ms.is_some() && stream.decoded_bytes > 0);
    let socket = the(&records, "/never?socket");
    assert!(socket.url.starts_with("ws://127.0.0.1:"));
    assert_eq!(
        (socket.kind, &socket.end, &socket.body),
        (
            ResourceKind::WebSocket,
            &RequestEnd::Pending,
            &CapturedBody::Absent
        )
    );
    assert!(header_value(&socket.request_headers, "upgrade").is_some());

    // -- Ended without an answer, each with its reason.
    let failure = |record: &NetworkRecord| match &record.end {
        RequestEnd::Failed {
            error,
            cancelled,
            blocked_reason,
        } => (error.clone(), *cancelled, blocked_reason.clone()),
        other => panic!("{}: {other:?}", record.url),
    };
    assert_eq!(
        failure(the(&records, "/never?aborted")),
        ("net::ERR_ABORTED".to_string(), true, None)
    );
    assert_eq!(
        failure(the(&records, "/refused")),
        ("net::ERR_CONNECTION_REFUSED".to_string(), false, None)
    );
    let (error, cancelled, blocked_reason) = failure(the(&records, "/status/200?no-cors-header"));
    assert_eq!((error.as_str(), cancelled), ("net::ERR_FAILED", false));
    assert_eq!(blocked_reason.as_deref(), Some("MissingAllowOriginHeader"));

    // -- A redirect to an error page: two records, the second an HTTP error.
    let redirect = the(&records, "/redirect?/status/404");
    let missing = the(&records, "/status/404");
    assert_eq!(
        (redirect.status, &redirect.end),
        (Some(302), &RequestEnd::Redirected)
    );
    assert_eq!(
        (missing.status, missing.redirect_from),
        (Some(404), Some(redirect.seq))
    );
    assert_eq!(
        body_text(missing).as_deref(),
        Some("<h1>404 Not Found</h1>")
    );

    // -- Addresses that are their own content: a record, no body.
    let from_blob = records
        .iter()
        .find(|record| record.url.starts_with("blob:"))
        .unwrap();
    assert_eq!(from_blob.body, CapturedBody::Skipped(BodySkip::Type));
    let data_script = records
        .iter()
        .find(|record| record.url.starts_with("data:"))
        .unwrap();
    assert_eq!(data_script.kind, ResourceKind::Script);
    assert_eq!(data_script.body, CapturedBody::Skipped(BodySkip::Type));
    assert!(
        capture
            .scripts
            .iter()
            .any(|script| script.body == "window.fromDataUrl=1"),
        "the script library still reads it"
    );

    // -- Request bodies.
    let beacon = the(&records, "/status/204");
    assert_eq!(
        (beacon.kind, beacon.method.as_str()),
        (ResourceKind::Ping, "POST")
    );
    assert_eq!(beacon.post_data.as_ref().unwrap().bytes, b"beacon=1");
    assert_eq!(beacon.body, CapturedBody::Skipped(BodySkip::Type));
    let posted = the(&records, "/status/200?posted");
    assert_eq!(
        posted.post_data.as_ref().unwrap().bytes,
        br#"{"posted":true}"#
    );
    assert_eq!(body_text(posted).as_deref(), Some("<h1>200 OK</h1>"));

    // -- Frames and their workers, once each, where they were made.
    let document = the(&records, &format!("/network-kinds.html?closed={closed}"));
    let same_site = the(&records, "/network-frame.html?same-site");
    let other_site = the(&records, "/network-frame.html?other-site");
    for frame in [same_site, other_site] {
        assert_eq!(
            (frame.kind, frame.context),
            (ResourceKind::Document, RequestContext::Iframe)
        );
        assert!(frame.frame_id.is_some() && frame.frame_id != document.frame_id);
        assert!(body_text(frame).unwrap().contains("new Worker"));
    }
    let in_same_site = the(&records, "/status/200?frame-same-site");
    assert_eq!(
        (
            in_same_site.context,
            &in_same_site.target_id,
            &in_same_site.frame_id
        ),
        (RequestContext::Iframe, &None, &same_site.frame_id),
        "a frame in the page's own process"
    );
    let in_other_site = the(&records, "/status/200?frame-other-site");
    assert_eq!(in_other_site.context, RequestContext::Iframe);
    assert!(
        in_other_site.target_id.is_some(),
        "a frame in a process of its own"
    );
    assert_eq!(in_other_site.frame_id, other_site.frame_id);
    assert!(in_other_site.url.starts_with("http://localhost:"));
    for which in ["same-site", "other-site"] {
        let script = the(&records, &format!("/network-worker.js?{which}"));
        assert_eq!(script.kind, ResourceKind::Script);
        assert!(body_text(script).unwrap().starts_with("fetch("));
        let fetched = the(&records, &format!("/status/200?worker-{which}"));
        assert_eq!(fetched.context, RequestContext::Worker);
        assert!(fetched.target_id.is_some());
        assert_eq!(body_text(fetched).as_deref(), Some("<h1>200 OK</h1>"));
    }
    let children: BTreeSet<&str> = records
        .iter()
        .filter_map(|record| record.target_id.as_deref())
        .collect();
    assert_eq!(
        children.len(),
        3,
        "one iframe and two workers have a target of their own"
    );
}

#[tokio::test]
async fn shared_and_service_workers_are_recorded_with_what_they_fetch() {
    let fixtures = Fixtures::start().await;
    let mut request = support::request(
        &fixtures.url("network-workers.html"),
        DeviceProfile::Desktop,
    );
    request.observe_seconds = 4;
    let (capture, records) = capture_with_requests(request).await;
    assert_every_request_has_one_record(&records);
    assert_eq!(capture.pages.len(), 1);

    let shared = the(&records, "/status/200?shared-worker");
    assert_eq!(shared.context, RequestContext::Worker);
    assert!(shared.target_id.is_some());
    assert_eq!(body_text(shared).as_deref(), Some("<h1>200 OK</h1>"));
    assert!(
        body_text(the(&records, "/network-shared.js"))
            .unwrap()
            .contains("shared-worker")
    );

    let through = the(&records, "/status/200?through-the-service-worker");
    assert_eq!(
        (through.context, through.served_from, through.status),
        (RequestContext::Page, ServedFrom::ServiceWorker, Some(200))
    );
    assert_eq!(
        body_text(through).as_deref(),
        Some("<h1>200 OK</h1>"),
        "the page's request has the body the service worker passed on"
    );
    assert_eq!(
        through.encoded_bytes, 0,
        "nothing came over the network for it"
    );
    let upstream = the(&records, "/status/200?asked-by-the-service-worker");
    assert_eq!(
        (upstream.context, upstream.served_from, upstream.status),
        (
            RequestContext::ServiceWorker,
            ServedFrom::Network,
            Some(200)
        )
    );
    assert!(upstream.target_id.is_some() && upstream.encoded_bytes > 0);
    assert!(
        records
            .iter()
            .any(|record| path(record) == "/network-sw.js"),
        "the service worker's own script"
    );
}

#[tokio::test]
async fn a_large_body_is_cut_to_what_the_sink_admits_or_not_read_at_all() {
    let fixtures = Fixtures::start().await;
    let visit = |requests: std::sync::Arc<Requests>| {
        let mut request =
            support::request(&fixtures.url("network-large.html"), DeviceProfile::Desktop);
        request.observe_seconds = 2;
        let mut reported = Reported::new();
        reported.hooks.network = Some(requests);
        reported.run(request)
    };

    let keeping = Requests::keeping_text();
    visit(keeping.clone()).await.result.unwrap();
    let records = keeping.records();
    let large = the(&records, "/large.json");
    let CapturedBody::Read { bytes, total_bytes } = &large.body else {
        panic!("{:?}", large.body);
    };
    assert_eq!(
        (bytes.len(), *total_bytes),
        (KEPT_BODY_BYTES, LARGE_JSON_BYTES as u64)
    );
    assert!(bytes.starts_with(br#"{"pad":"xxx"#));
    assert_eq!(large.decoded_bytes, LARGE_JSON_BYTES as u64);
    let (_, expected) = keeping
        .offered()
        .into_iter()
        .find(|(seq, _)| *seq == large.seq)
        .expect("offered before it was read");
    assert_eq!(
        expected, LARGE_JSON_BYTES as u64,
        "the sink is told the size beforehand"
    );

    // A sink that cannot keep up gets the record and no bytes.
    let behind = Requests::admitting(|offer| {
        if offer.expected_bytes > 1024 * 1024 {
            BodyAdmission::Skip(BodySkip::Backlog)
        } else {
            BodyAdmission::Read { max_bytes: 64 }
        }
    });
    visit(behind.clone()).await.result.unwrap();
    let records = behind.records();
    let large = the(&records, "/large.json");
    assert_eq!(large.body, CapturedBody::Skipped(BodySkip::Backlog));
    assert_eq!(
        (large.status, large.decoded_bytes),
        (Some(200), LARGE_JSON_BYTES as u64)
    );
    let page = the(&records, "/network-large.html");
    let CapturedBody::Read { bytes, total_bytes } = &page.body else {
        panic!("{:?}", page.body);
    };
    assert_eq!(bytes.len(), 64);
    assert!(*total_bytes > 64);
    let held: usize = records
        .iter()
        .map(|record| match &record.body {
            CapturedBody::Read { bytes, .. } => bytes.len(),
            _ => 0,
        })
        .sum();
    assert!(held <= 64 * records.len(), "{held} bytes handed over");
}

#[tokio::test]
async fn nothing_of_another_site_is_recorded_once_the_tab_has_left() {
    let fixtures = Fixtures::start().await;
    let mut request =
        support::request(&fixtures.url("network-leaves.html"), DeviceProfile::Desktop);
    request.observe_seconds = 3;
    let (capture, records) = capture_with_requests(request).await;
    assert_every_request_has_one_record(&records);
    assert_eq!(capture.pages.len(), 1);

    assert!(body_text(the(&records, "/network-leaves.html")).is_some());
    assert!(body_text(the(&records, "/status/200?before-leaving")).is_some());
    // Every record names the host of the page view it belongs to, which is
    // what the engine measures "first party" against.
    assert!(
        records.iter().all(|record| record.page_host == "127.0.0.1"),
        "{:?}",
        records
            .iter()
            .map(|record| &record.page_host)
            .collect::<Vec<_>>()
    );
    // The navigation away is a request the page made; what it loaded is not kept.
    let leaving = the(&records, "/network-elsewhere.html");
    assert!(leaving.url.starts_with("http://localhost:"));
    assert_eq!((leaving.kind, leaving.page), (ResourceKind::Document, 0));
    assert!(
        matches!(&leaving.body, CapturedBody::Unavailable(why) if why.contains("left the scanned site")),
        "{:?}",
        leaving.body
    );
    assert!(
        records
            .iter()
            .all(|record| asked(record) != "/status/200?elsewhere"),
        "a request of the other site's page was recorded"
    );
    assert!(
        fixtures
            .requested()
            .iter()
            .any(|asked| asked == "/status/200?elsewhere"),
        "the other site's page did run"
    );
}

/// Chrome discards what a frame loaded when the frame goes. Bodies are asked
/// for the moment they are complete, not at the end of the visit, so they
/// outlive a frame that is removed soon after.
#[tokio::test]
async fn what_a_frame_loaded_is_read_before_the_frame_is_gone() {
    let fixtures = Fixtures::start().await;
    let mut request = support::request(
        &fixtures.url("network-removed-frame.html?400"),
        DeviceProfile::Desktop,
    );
    request.observe_seconds = 2;
    let (_, records) = capture_with_requests(request).await;
    // The page asks for this once it has removed the frame.
    the(&records, "/status/200?frame-removed");
    let in_frame = the(&records, "/status/200?loaded-in-the-frame");
    assert_eq!(in_frame.context, RequestContext::Iframe);
    assert_eq!(body_text(in_frame).as_deref(), Some("<h1>200 OK</h1>"));
    assert!(body_text(the(&records, "/network-reports-back.html")).is_some());
}

/// A frame removed in the same breath as its last response may take that
/// body with it before the scanner can ask. The request is recorded all the
/// same, and says what became of its body.
#[tokio::test]
async fn a_body_the_browser_no_longer_has_is_recorded_as_unavailable() {
    let fixtures = Fixtures::start().await;
    let mut request = support::request(
        &fixtures.url("network-removed-frame.html?0"),
        DeviceProfile::Desktop,
    );
    request.observe_seconds = 2;
    let (_, records) = capture_with_requests(request).await;
    assert_every_request_has_one_record(&records);
    the(&records, "/status/200?frame-removed");
    let in_frame = the(&records, "/status/200?loaded-in-the-frame");
    assert_eq!(
        (in_frame.status, &in_frame.end, in_frame.decoded_bytes),
        (Some(200), &RequestEnd::Finished, 15)
    );
    match &in_frame.body {
        CapturedBody::Read { bytes, .. } => assert_eq!(bytes, b"<h1>200 OK</h1>"),
        CapturedBody::Unavailable(why) => assert!(!why.is_empty()),
        other => panic!("{other:?}"),
    }
}

/// The targets `network-short-lived.html` starts, one after the other.
const SHORT_LIVED: [&str; 3] = ["other-site-frame", "same-site-frame", "worker"];

/// Visits the page whose frames and worker each fetch `asks` and are removed
/// the moment they have their answer, `rounds` times over. `more` is passed
/// on to the page.
async fn short_lived_visit(
    fixtures: &Fixtures,
    asks: &str,
    rounds: usize,
    more: &str,
    requests: Arc<Requests>,
) -> Vec<NetworkRecord> {
    let mut request = support::request(
        &fixtures.url(&format!(
            "network-short-lived.html?rounds={rounds}&asks={}{more}",
            asks.replace('?', "%3F").replace('&', "%26")
        )),
        DeviceProfile::Desktop,
    );
    request.observe_seconds = 5;
    let mut reported = Reported::new();
    reported.hooks.network = Some(requests.clone());
    let outcome = reported.run(request).await;
    outcome.result.unwrap_or_else(|error| {
        panic!("capture failed: {error}\nprogress: {:#?}", outcome.progress)
    });
    let records = requests.records();
    assert_every_request_has_one_record(&records);
    // The page asks for this once the last of them is gone.
    the(&records, "/status/200?all-gone");
    records
}

/// Where a request of one of [`SHORT_LIVED`] was made.
#[track_caller]
fn assert_made_by(record: &NetworkRecord, by: &str) {
    let (context, has_own_target) = match by {
        "other-site-frame" => (RequestContext::Iframe, true),
        "same-site-frame" => (RequestContext::Iframe, false),
        _ => (RequestContext::Worker, true),
    };
    assert_eq!(
        (record.context, record.target_id.is_some()),
        (context, has_own_target),
        "{}",
        record.url
    );
}

/// How long the fixture server takes to answer what short-lived targets
/// ask, as an ad server on the network would. The scanner asks for a copy
/// of a body when the request is announced; an answer that is complete
/// before that question reaches the frame cannot be copied.
const AS_FROM_THE_NETWORK: &str = "wait=200";

/// The body of a record, which must have been read whole.
#[track_caller]
fn whole_body(record: &NetworkRecord) -> &[u8] {
    let CapturedBody::Read { bytes, total_bytes } = &record.body else {
        panic!("{}: {:?}", record.url, record.body);
    };
    assert_eq!(bytes.len() as u64, *total_bytes, "{}", record.url);
    bytes
}

/// Ad frames live for a moment: a creative, a sync, a passback. A frame or
/// worker that is removed in the task that read its last answer takes its
/// session with it, and the body can no longer be asked for. It was sent
/// along while it loaded, so it is recorded all the same.
#[tokio::test]
async fn what_a_frame_or_worker_read_as_it_went_away_is_recorded_with_its_body() {
    const ROUNDS: usize = 3;
    let fixtures = Fixtures::start().await;
    let visit = |size: usize| {
        let fixtures = &fixtures;
        async move {
            let asks = format!("/payload?size={size}&{AS_FROM_THE_NETWORK}");
            let records =
                short_lived_visit(fixtures, &asks, ROUNDS, "", Requests::keeping_text()).await;
            for by in SHORT_LIVED {
                for round in 0..ROUNDS {
                    let query = format!("size={size}&{AS_FROM_THE_NETWORK}&by={by}&round={round}");
                    let fetched = the(&records, &format!("/payload?{query}"));
                    assert_made_by(fetched, by);
                    assert_eq!(
                        (fetched.status, &fetched.end, fetched.decoded_bytes),
                        (Some(200), &RequestEnd::Finished, size as u64),
                        "{query}"
                    );
                    assert!(
                        whole_body(fetched) == support::payload(&query),
                        "{query}: not the bytes that were served"
                    );
                }
            }
        }
    };
    tokio::join!(visit(100), visit(50_000), visit(1_000_000));
}

/// The copy of a body is the body: content encoding undone, the answer at
/// the end of a redirect, and nothing made up for an answer that came out
/// of the cache faster than a copy could be asked for.
#[tokio::test]
async fn a_body_that_outlived_its_frame_or_worker_is_the_one_that_was_served() {
    const ROUNDS: usize = 2;
    let fixtures = Fixtures::start().await;
    let compressed = async {
        let asks = format!("/payload?size=50000&{AS_FROM_THE_NETWORK}&encoding=gzip");
        let records =
            short_lived_visit(&fixtures, &asks, ROUNDS, "", Requests::keeping_text()).await;
        for by in SHORT_LIVED {
            for round in 0..ROUNDS {
                let query =
                    format!("size=50000&{AS_FROM_THE_NETWORK}&encoding=gzip&by={by}&round={round}");
                let fetched = the(&records, &format!("/payload?{query}"));
                assert_eq!(
                    header_value(&fetched.response_headers, "content-encoding"),
                    Some("gzip")
                );
                assert!(
                    fetched.encoded_bytes < 45_000 && fetched.decoded_bytes == 50_000,
                    "{} over the network, {} decoded",
                    fetched.encoded_bytes,
                    fetched.decoded_bytes
                );
                assert!(whole_body(fetched) == support::payload(&query), "{query}");
            }
        }
    };
    let redirected = async {
        let asks = format!("/redirect?/payload?size=50000&{AS_FROM_THE_NETWORK}");
        let records =
            short_lived_visit(&fixtures, &asks, ROUNDS, "", Requests::keeping_text()).await;
        for by in SHORT_LIVED {
            for round in 0..ROUNDS {
                let query = format!("size=50000&{AS_FROM_THE_NETWORK}&by={by}&round={round}");
                let hop = the(&records, &format!("/redirect?/payload?{query}"));
                assert_eq!(
                    (&hop.end, &hop.body),
                    (&RequestEnd::Redirected, &CapturedBody::Absent)
                );
                let fetched = the(&records, &format!("/payload?{query}"));
                assert_eq!(fetched.redirect_from, Some(hop.seq));
                assert!(whole_body(fetched) == support::payload(&query), "{query}");
            }
        }
    };
    let cached = async {
        let asks = format!("/payload?size=50000&{AS_FROM_THE_NETWORK}&cache=1");
        let records = short_lived_visit(
            &fixtures,
            &asks,
            ROUNDS,
            "&times=2",
            Requests::keeping_text(),
        )
        .await;
        for by in SHORT_LIVED {
            for round in 0..ROUNDS {
                let query =
                    format!("size=50000&{AS_FROM_THE_NETWORK}&cache=1&by={by}&round={round}");
                let fetches: Vec<&NetworkRecord> = records
                    .iter()
                    .filter(|record| asked(record) == format!("/payload?{query}"))
                    .collect();
                let [from_network, from_cache] = fetches[..] else {
                    panic!("{query}: {} requests", fetches.len());
                };
                assert_eq!(
                    (from_network.served_from, from_cache.served_from),
                    (ServedFrom::Network, ServedFrom::DiskCache)
                );
                assert!(
                    whole_body(from_network) == support::payload(&query),
                    "{query}"
                );
                match &from_cache.body {
                    CapturedBody::Read { bytes, .. } => {
                        assert!(*bytes == support::payload(&query), "{query}")
                    }
                    CapturedBody::Unavailable(why) => assert!(!why.is_empty()),
                    other => panic!("{query}: {other:?}"),
                }
            }
        }
    };
    tokio::join!(compressed, redirected, cached);
}

/// The sink decides what is read, also of a body that outlived its frame
/// or worker: no more than it admits, and nothing while it cannot keep up.
#[tokio::test]
async fn a_body_that_outlived_its_frame_or_worker_is_cut_or_skipped_as_the_sink_says() {
    let fixtures = Fixtures::start().await;
    let cut_or_skipped = async {
        let behind_for_workers = Requests::admitting(|offer| {
            if offer.url.contains("by=worker") {
                BodyAdmission::Skip(BodySkip::Backlog)
            } else {
                BodyAdmission::Read { max_bytes: 64 }
            }
        });
        let asks = format!("/payload?size=50000&{AS_FROM_THE_NETWORK}");
        let records = short_lived_visit(&fixtures, &asks, 1, "", behind_for_workers).await;
        for by in SHORT_LIVED {
            let query = format!("size=50000&{AS_FROM_THE_NETWORK}&by={by}&round=0");
            let fetched = the(&records, &format!("/payload?{query}"));
            if by == "worker" {
                assert_eq!(fetched.body, CapturedBody::Skipped(BodySkip::Backlog));
                continue;
            }
            assert_eq!(
                fetched.body,
                CapturedBody::Read {
                    bytes: support::payload(&query)[..64].to_vec(),
                    total_bytes: 50_000,
                },
                "{query}"
            );
        }
    };
    let larger_than_is_kept = async {
        let asks = format!("/payload?size={LARGE_JSON_BYTES}&{AS_FROM_THE_NETWORK}");
        let records = short_lived_visit(&fixtures, &asks, 1, "", Requests::keeping_text()).await;
        for by in SHORT_LIVED {
            let query = format!("size={LARGE_JSON_BYTES}&{AS_FROM_THE_NETWORK}&by={by}&round=0");
            let fetched = the(&records, &format!("/payload?{query}"));
            let CapturedBody::Read { bytes, total_bytes } = &fetched.body else {
                panic!("{query}: {:?}", fetched.body);
            };
            assert_eq!(*total_bytes, LARGE_JSON_BYTES as u64, "{query}");
            assert!(
                *bytes == support::payload(&query)[..KEPT_BODY_BYTES],
                "{query}: not the first bytes that were served"
            );
        }
    };
    tokio::join!(cut_or_skipped, larger_than_is_kept);
}

/// A frame or worker that goes away while an answer is still arriving has
/// no body to record: what had arrived until then is not passed off as one.
#[tokio::test]
async fn an_answer_cut_short_by_its_frame_or_worker_going_away_has_no_body() {
    let fixtures = Fixtures::start().await;
    let records = short_lived_visit(
        &fixtures,
        "/payload?size=50000&stall=1",
        2,
        "&unread=1",
        Requests::keeping_text(),
    )
    .await;
    for by in SHORT_LIVED {
        for round in 0..2 {
            let fetched = the(
                &records,
                &format!("/payload?size=50000&stall=1&by={by}&round={round}"),
            );
            assert_eq!(fetched.status, Some(200));
            // A frame in the page's own process reports the request as
            // cancelled; a target of its own is gone before it can.
            assert!(
                matches!(
                    fetched.end,
                    RequestEnd::Pending
                        | RequestEnd::Failed {
                            cancelled: true,
                            ..
                        }
                ),
                "{by}: {:?}",
                fetched.end
            );
            assert_eq!(fetched.body, CapturedBody::Absent, "{by}");
        }
    }
}

/// A route change is reported by the tracker, in a batch that arrives up to
/// a tenth of a second after it; what the page requests in between is
/// already on the new page view.
#[tokio::test]
async fn requests_after_a_route_change_are_on_the_page_view_it_opened() {
    let site = adscan_browser::start_demo_site().await.unwrap();
    let mut request = support::request(&site.url, DeviceProfile::Desktop);
    request.visitor = VisitorBehavior::Browse;
    request.extra_pages = 1;
    request.observe_seconds = 6;
    // This seed's visit follows a link the site routes in place.
    request.seed = 6;
    let (capture, records) = capture_with_requests(request).await;
    assert_every_request_has_one_record(&records);
    assert_eq!(capture.pages.len(), 2, "{:#?}", capture.pages);
    let (entry, routed) = (&capture.pages[0], &capture.pages[1]);
    assert_eq!(routed.reached_by, PageReach::RouteChange);

    // The site's analytics reports the new page view as the route changes.
    let route = routed
        .url
        .trim_start_matches(site.url.trim_end_matches('/'));
    let page_views: Vec<&NetworkRecord> = records
        .iter()
        .filter(|record| {
            path(record) == "/collect"
                && record.post_data.as_ref().is_some_and(|post| {
                    let sent: serde_json::Value = serde_json::from_slice(&post.bytes).unwrap();
                    sent["event"] == "page_view" && sent["properties"]["route"] == route
                })
        })
        .collect();
    assert_eq!(page_views.len(), 1, "{records:#?}");
    assert_eq!(page_views[0].page, 1);
    assert!(
        (0.0..1_000.0).contains(&page_views[0].started_at_ms),
        "{}",
        page_views[0].started_at_ms
    );

    let entry_lasted = entry.ended_at_ms - entry.started_at_ms;
    for record in &records {
        let started = f64::from(record.page) * entry_lasted + record.started_at_ms;
        assert_eq!(
            record.page,
            u32::from(started >= entry_lasted),
            "{} started {} ms into page view {}; the entry page lasted {entry_lasted} ms",
            record.url,
            record.started_at_ms,
            record.page
        );
        assert!(
            record.started_at_ms >= 0.0 || record.page == 0,
            "{}",
            record.url
        );
    }
}

#[tokio::test]
async fn a_visit_of_several_pages_puts_every_request_on_its_page_view() {
    let site = adscan_browser::start_demo_site().await.unwrap();
    let mut request = support::request(&site.url, DeviceProfile::Desktop);
    request.visitor = VisitorBehavior::Browse;
    request.extra_pages = 2;
    request.observe_seconds = 6;
    request.seed = 3;
    let (capture, records) = capture_with_requests(request).await;
    assert_every_request_has_one_record(&records);
    assert!(capture.pages.len() >= 2, "{:#?}", capture.pages);

    for (index, view) in capture.pages.iter().enumerate() {
        let of_view: Vec<&NetworkRecord> = records
            .iter()
            .filter(|record| record.page == index as u32)
            .collect();
        assert!(!of_view.is_empty(), "page view {index} has no requests");
        let lasted = view.ended_at_ms - view.started_at_ms;
        for record in &of_view {
            // A document keeps asking for things until the next one commits,
            // which is a moment after the next page view began.
            assert!(
                record.started_at_ms > -10.0 && record.started_at_ms <= lasted + 1_000.0,
                "{} at {} on page view {index}, which lasted {lasted}",
                record.url,
                record.started_at_ms
            );
        }
        // A page view that loaded a document starts with that document's request.
        if view.reached_by != PageReach::RouteChange {
            let document = of_view
                .iter()
                .find(|record| {
                    record.kind == ResourceKind::Document && record.context == RequestContext::Page
                })
                .unwrap_or_else(|| panic!("page view {index} has no document"));
            assert_eq!(document.url, view.url);
            assert!(
                document.started_at_ms.abs() < 50.0,
                "{}",
                document.started_at_ms
            );
            assert!(body_text(document).unwrap().contains("The Meridian Post"));
        }
    }
    assert!(
        records
            .iter()
            .all(|record| (record.page as usize) < capture.pages.len())
    );

    // A script every page loads is requested, and recorded, once per document.
    let documents = capture
        .pages
        .iter()
        .filter(|view| view.reached_by != PageReach::RouteChange)
        .count();
    let wrapper_loads: Vec<u32> = records
        .iter()
        .filter(|record| path(record) == "/static/ads/hb-wrapper.min.js")
        .map(|record| record.page)
        .collect();
    assert_eq!(wrapper_loads.len(), documents, "{wrapper_loads:?}");
    let wrapper = capture
        .scripts
        .iter()
        .find(|script| script.url.ends_with("/static/ads/hb-wrapper.min.js"))
        .unwrap();
    let first_load = records
        .iter()
        .find(|record| path(record) == "/static/ads/hb-wrapper.min.js")
        .unwrap();
    assert_eq!(wrapper.request_seq, Some(first_load.seq));

    // The answers of the bidders are on the page view of their request.
    for answer in &capture.bid_responses {
        let request = &records[answer.request_seq.unwrap() as usize];
        assert_eq!(request.bidder.as_deref(), Some(answer.bidder.as_str()));
        assert!(
            (request.ended_at_ms.unwrap() + offset(&capture, request.page)
                - answer.t
                - offset(&capture, answer.page))
            .abs()
                < 10.0
        );
    }
}

/// ms from the start of the visit to the start of a page view.
fn offset(capture: &adscan_model::Capture, page: u32) -> f64 {
    capture.pages[page as usize].started_at_ms
}

#[tokio::test]
async fn without_a_sink_nothing_is_read_for_one_and_the_capture_is_as_before() {
    let site = adscan_browser::start_demo_site().await.unwrap();
    let mut request = support::request(&site.url, DeviceProfile::Mobile);
    request.observe_seconds = 3;
    let outcome = support::run(request).await;
    let capture = outcome.result.unwrap();
    assert!(
        capture
            .scripts
            .iter()
            .all(|script| script.request_seq.is_none())
    );
    assert!(
        capture
            .bid_responses
            .iter()
            .all(|answer| answer.request_seq.is_none())
    );
    assert!(!capture.bid_responses.is_empty());
    assert!(
        outcome
            .progress
            .iter()
            .all(|line| !line.starts_with("Recorded ")),
        "{:#?}",
        outcome.progress
    );
}

/// The page leaves its site while the scanner is settling on it: after the
/// load event, before the tracker is confined to the site. The navigation
/// waits for the confinement, so the tracker does nothing on the other site.
/// Without the wait the tab could move to the other site's process with the
/// unconfined tracker, and Chrome then ran a command twice that was on its
/// way: the second run of removing the tracker found nothing to remove and
/// the scan failed with that answer. Several visits at once, because the
/// moment was only hit now and then.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_page_that_leaves_its_site_while_the_scan_settles_on_it_is_still_scanned() {
    let visits: Vec<_> = [0, 0, 5, 10, 20, 40, 80, 150]
        .into_iter()
        .map(|after| {
            tokio::spawn(async move {
                let fixtures = Fixtures::start().await;
                let mut request = support::request(
                    &fixtures.url(&format!("network-leaves-at-once.html?after={after}")),
                    DeviceProfile::Desktop,
                );
                request.observe_seconds = 2;
                let outcome = support::run(request).await;
                let capture = outcome.result.unwrap_or_else(|error| {
                    panic!("leaving after {after} ms: {error}\n{:#?}", outcome.progress)
                });
                assert_eq!(capture.pages.len(), 1, "{:#?}", outcome.progress);
                assert!(
                    outcome
                        .progress
                        .iter()
                        .any(|line| line.starts_with("Left the scanned site")),
                    "{:#?}",
                    outcome.progress
                );
                let told_by_the_other_site: Vec<_> = fixtures
                    .requested()
                    .into_iter()
                    .filter(|path| path.starts_with("/status/200?tracker="))
                    .collect();
                assert_eq!(
                    told_by_the_other_site,
                    ["/status/200?tracker=silent"],
                    "leaving after {after} ms"
                );
            })
        })
        .collect();
    for visit in visits {
        visit.await.unwrap();
    }
}

/// The scan stops pausing document requests once the tracker is confined.
/// It used to do that by switching the interception off, and the browser then
/// let go of a document request that had started and was not yet reported as
/// paused without ever sending it: an iframe stayed empty, and a navigation
/// of the tab never ended, which kept the page from answering the scan for
/// two minutes. The page here starts document requests all through that
/// moment, and the server must see every one of them.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn no_document_request_is_lost_when_the_scan_stops_pausing_them() {
    let visits: Vec<_> = (0..4)
        .map(|_| {
            tokio::spawn(async {
                let fixtures = Fixtures::start().await;
                let mut request = support::request(
                    &fixtures.url("network-frames-while-settling.html"),
                    DeviceProfile::Desktop,
                );
                request.observe_seconds = 2;
                let outcome = support::run(request).await;
                outcome.result.unwrap_or_else(|error| {
                    panic!("capture failed: {error}\n{:#?}", outcome.progress)
                });
                let mut frames: Vec<usize> = fixtures
                    .requested()
                    .iter()
                    .filter_map(|path| path.strip_prefix("/status/200?frame=")?.parse().ok())
                    .collect();
                frames.sort_unstable();
                frames
            })
        })
        .collect();
    for visit in visits {
        let frames = visit.await.unwrap();
        // The last few may still have been on their way when the visit
        // ended; one that was lost leaves a gap before later ones.
        let lost: Vec<usize> = (0..frames.last().map_or(0, |last| last + 1))
            .filter(|frame| frames.binary_search(frame).is_err())
            .collect();
        assert!(
            frames.len() > 4,
            "only {} iframes were requested",
            frames.len()
        );
        assert_eq!(lost, Vec::<usize>::new(), "of {} iframes", frames.len());
    }
}
