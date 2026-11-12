//! The scan archive end to end: a capture that streams requests into the
//! scan's sink, what reaches the disk and what does not, and reading it back
//! for the network view and the script viewer. Real SQLite, real pack files
//! in a temporary directory, the real pipeline; only the browser is a double.

mod common;

use std::path::PathBuf;
use std::sync::Arc;

use adscan_engine::application::archive::source::OriginalPosition;
use adscan_engine::application::ports::{ArchiveState, CaptureError, ScanRepository};
use adscan_engine::domain::archive::Location;
use adscan_engine::domain::archive::pack::{Manifest, TRAILER_LEN, manifest_range};
use adscan_engine::infrastructure::sqlite::SqliteStore;
use adscan_model::{
    BodySide, BodyState, CapturedBody, CapturedPostData, DeviceProfile, ExchangeProtocol,
    ExchangeVerdict, HeaderRedaction, InitiatorFrame, InitiatorKind, NetworkAvailability,
    NetworkCategory, NetworkRecord, RequestContext, RequestEnd, RequestOutcome, ResourceKind,
    ScanDetail, ScanStatus, ServedFrom, SettingsPatch, SourceAvailability, SourceFormat,
    SourceView, TrackedAdUnit, TrackedBidRequest, TrackerEvent, TrackerEventKind as Ev,
};
use common::*;

const AUCTION: &str = "auction-demo-0001";
const DOCUMENT_HTML: &str =
    "<!doctype html><title>Publisher</title><p>signed in as reader@example.org</p>";
const COOKIE_SECRET: &str = "s3cr3t-session-value";
const BEARER_SECRET: &str = "tok-9f8e7d6c5b4a";
const INLINE_SCRIPT: &str =
    "window.__pageData={section:'climate'};googletag.cmd.push(function(){});";

fn tracked(code: &str, bidders: &[&str]) -> TrackedAdUnit {
    TrackedAdUnit {
        code: code.into(),
        sizes: strings(&["300x250"]),
        bidders: strings(bidders),
        bids: vec![],
    }
}

fn bid_requested(t: f64, bidder: &str, bids: &[(&str, &str)], with_ids: bool) -> TrackerEvent {
    ev(
        t,
        Ev::BidRequested {
            auction_id: AUCTION.into(),
            bidder: bidder.into(),
            ad_unit_codes: bids.iter().map(|(code, _)| (*code).to_string()).collect(),
            gdpr_applies: None,
            has_consent_string: None,
            schain: vec![],
            bidder_request_id: with_ids.then(|| format!("breq-{bidder}-0001")),
            bids: if with_ids {
                bids.iter()
                    .map(|(code, bid_id)| TrackedBidRequest {
                        bid_id: (*bid_id).into(),
                        ad_unit_code: (*code).into(),
                    })
                    .collect()
            } else {
                Vec::new()
            },
        },
    )
}

fn no_bid(t: f64, bidder: &str, code: &str) -> TrackerEvent {
    ev(
        t,
        Ev::NoBid {
            auction_id: AUCTION.into(),
            bidder: bidder.into(),
            ad_unit_code: code.into(),
        },
    )
}

/// One auction with a 250 ms timeout: openx declines (two ad units, one
/// request), triplelift only sends one of its three bids, rubicon answers
/// after the auction closed.
fn auction_events() -> Vec<TrackerEvent> {
    let mut events = page_setup();
    events.extend([
        ev(
            1_000.0,
            Ev::AuctionInit {
                auction_id: AUCTION.into(),
                timeout: Some(250.0),
                ad_units: vec![
                    tracked("top", &["openx", "triplelift", "rubicon"]),
                    tracked("side", &["openx", "triplelift"]),
                    tracked("footer", &["triplelift"]),
                ],
            },
        ),
        bid_requested(
            1_005.0,
            "openx",
            &[("top", "bid-openx-top-1"), ("side", "bid-openx-side-1")],
            true,
        ),
        bid_requested(
            1_005.0,
            "triplelift",
            &[
                ("top", "bid-tl-top-1"),
                ("side", "bid-tl-side-1"),
                ("footer", "bid-tl-footer-1"),
            ],
            true,
        ),
        bid_requested(1_006.0, "rubicon", &[("top", "unused")], false),
        no_bid(1_010.0, "triplelift", "side"),
        no_bid(1_010.0, "triplelift", "footer"),
        bid_response(1_120.0, AUCTION, "triplelift", "top", 0.4),
        no_bid(1_150.0, "openx", "top"),
        no_bid(1_150.0, "openx", "side"),
        ev(
            1_250.0,
            Ev::BidTimeout {
                auction_id: AUCTION.into(),
                bidder: "rubicon".into(),
                ad_unit_code: "top".into(),
            },
        ),
        ev(
            1_250.0,
            Ev::AuctionEnd {
                auction_id: AUCTION.into(),
            },
        ),
        slot_requested(1_300.0, "top", false),
    ]);
    events
}

fn post(
    mut record: NetworkRecord,
    bidder: &str,
    started: f64,
    ended: Option<f64>,
    body: &str,
) -> NetworkRecord {
    record.method = "POST".into();
    record.kind = ResourceKind::Fetch;
    record.bidder = Some(bidder.into());
    record.started_at_ms = started;
    record.response_at_ms = ended.map(|ended| ended - 10.0);
    record.ended_at_ms = ended;
    record.post_data = Some(CapturedPostData {
        bytes: body.as_bytes().to_vec(),
        total_bytes: body.len() as u64,
    });
    record
}

/// What the browser saw during the visit: the document, the wrapper, an
/// image, three bid exchanges, a redirect chain, a preflight, the ad call,
/// a blocked request and one that never finished.
fn network() -> Vec<NetworkRecord> {
    let mut document = with_body(
        net(
            0,
            "https://pub.example/?utm_source=newsletter&session=abc123",
        ),
        ResourceKind::Document,
        "text/html; charset=utf-8",
        DOCUMENT_HTML,
    );
    document.request_headers = vec![
        header("User-Agent", "Mozilla/5.0 (Macintosh)"),
        header("Cookie", &format!("sid={COOKIE_SECRET}; theme=dark")),
    ];
    document.response_headers = vec![
        header("content-type", "text/html; charset=utf-8"),
        header(
            "set-cookie",
            &format!("sid={COOKIE_SECRET}; Path=/; HttpOnly\nab=1; Max-Age=60"),
        ),
    ];
    document.initiator.kind = InitiatorKind::Other;

    let mut wrapper = with_body(
        net(1, "https://pub.example/js/ads.js"),
        ResourceKind::Script,
        "application/javascript",
        WRAPPER_SOURCE,
    );
    wrapper.initiator.kind = InitiatorKind::Parser;
    wrapper.initiator.url = Some("https://pub.example/".into());
    wrapper.initiator.line = Some(11);

    let image = with_body(
        net(2, "https://pub.example/img/logo.png"),
        ResourceKind::Image,
        "image/png",
        "\u{0}PNG-not-really",
    );

    let mut openx = with_body(
        post(
            net(3, "https://rtb.openx.net/openrtbb/prebidjs"),
            "openx",
            1_010.0,
            Some(1_150.0),
            r#"{"id":"breq-openx-0001","tmax":250,"imp":[{"id":"bid-openx-top-1","tagid":"top","banner":{"format":[{"w":300,"h":250}]}},{"id":"bid-openx-side-1","banner":{"w":300,"h":250}}],"user":{"ext":{"consent":"CPconsentstring"}}}"#,
        ),
        ResourceKind::Fetch,
        "application/json",
        r#"{"id":"meridian-post","nbr":7}"#,
    );
    openx.request_headers = vec![
        header("Content-Type", "text/plain"),
        header("Authorization", &format!("Bearer {BEARER_SECRET}")),
    ];
    openx.initiator.kind = InitiatorKind::Script;
    openx.initiator.stack = vec![InitiatorFrame {
        function: Some("askEndpoint".into()),
        url: "https://pub.example/js/ads.js".into(),
        script_url: Some("https://pub.example/js/ads.js".into()),
        line: 0,
        column: WRAPPER_SOURCE.find("pbjs.requestBids").unwrap() as u32,
    }];

    let mut sync_first = net(4, "https://sync.partner.example/setuid?uid=42");
    sync_first.kind = ResourceKind::Image;
    sync_first.status = Some(302);
    sync_first.end = RequestEnd::Redirected;
    sync_first.body = CapturedBody::Absent;
    let mut sync_second = with_body(
        net(5, "https://sync.other.example/match?uid=42"),
        ResourceKind::Image,
        "image/gif",
        "GIF89a",
    );
    sync_second.redirect_from = Some(4);

    let mut preflight = net(6, "https://rtb.openx.net/openrtbb/prebidjs");
    preflight.method = "OPTIONS".into();
    preflight.kind = ResourceKind::Preflight;
    preflight.preflight_of = Some(3);
    preflight.initiator.kind = InitiatorKind::Preflight;
    preflight.started_at_ms = 962.0;
    preflight.ended_at_ms = Some(1_007.0);
    preflight.status = Some(204);

    let mut ad_call = with_body(
        net(
            7,
            "https://securepubads.g.doubleclick.net/gampad/ads?iu=/123/top&prev_scp=pos%3Dtop&sz=728x90",
        ),
        ResourceKind::Xhr,
        "text/plain",
        "{\"/123/top\":[\"html\",0]}",
    );
    ad_call.started_at_ms = 1_304.0;
    ad_call.ended_at_ms = Some(1_446.0);

    let mut blocked = net(8, "https://tracker.blocked.example/pixel.gif");
    blocked.kind = ResourceKind::Image;
    blocked.status = None;
    blocked.response_at_ms = None;
    blocked.timing = None;
    blocked.end = RequestEnd::Failed {
        error: "net::ERR_BLOCKED_BY_CLIENT".into(),
        cancelled: false,
        blocked_reason: Some("inspector".into()),
    };
    blocked.body = CapturedBody::Absent;

    let mut pending = net(9, "https://stream.example.net/events");
    pending.kind = ResourceKind::EventSource;
    pending.status = None;
    pending.response_at_ms = None;
    pending.ended_at_ms = None;
    pending.end = RequestEnd::Pending;
    pending.body = CapturedBody::Absent;

    let triplelift = with_body(
        post(
            net(10, "https://tlx.3lift.com/header/auction?lib=prebid"),
            "triplelift",
            1_008.0,
            Some(1_120.0),
            r#"{"id":"breq-triplelift-0001","imp":[{"id":"bid-tl-top-1","tagid":"top_inv"}]}"#,
        ),
        ResourceKind::Fetch,
        "application/json",
        r#"{"id":"1","cur":"USD","seatbid":[{"seat":"triplelift","bid":[{"impid":"bid-tl-top-1","price":0.4,"w":300,"h":250,"adomain":["shoes.example"],"crid":"cr-1"}]}]}"#,
    );

    let mut rubicon = with_body(
        net(
            11,
            "https://fastlane.rubiconproject.com/a/api/fastlane.json?account_id=1001&size_id=15",
        ),
        ResourceKind::Xhr,
        "application/json",
        r#"{"status":"ok","ads":[{"status":"ok","cpm":1.2}]}"#,
    );
    rubicon.bidder = Some("rubicon".into());
    rubicon.started_at_ms = 1_012.0;
    rubicon.response_at_ms = Some(1_320.0);
    rubicon.ended_at_ms = Some(1_334.0);

    // Records arrive in the order the requests ended, not in order of `seq`.
    vec![
        document,
        preflight,
        wrapper,
        image,
        triplelift,
        openx,
        sync_first,
        sync_second,
        rubicon,
        ad_call,
        blocked,
        pending,
    ]
}

fn visit_capture() -> adscan_model::Capture {
    let mut capture = capture_with(DeviceProfile::Desktop, auction_events(), 1.0);
    capture.bid_responses = vec![adscan_model::ObservedBidResponse {
        page: 0,
        t: 1_150.0,
        bidder: "openx".into(),
        host: "rtb.openx.net".into(),
        status: Some(200),
        reason: Some("nbr 7".into()),
        request_seq: Some(3),
    }];
    capture.scripts[2].request_seq = Some(1);
    capture.scripts.push(adscan_model::CapturedScript {
        url: "inline:1".into(),
        inline: true,
        body: INLINE_SCRIPT.into(),
        loaded_at_ms: Some(40.0),
        initiator: Some("https://pub.example/".into()),
        pages: vec![0],
        request_seq: None,
    });
    capture
}

fn recorded_harness() -> Harness {
    let harness = harness(
        CaptureBehavior::Return(Box::new(visit_capture())),
        "http://127.0.0.1:1",
        None,
    );
    *harness.capture.network.lock().unwrap() = network();
    harness
        .files
        .answer(SITE_ADS_TXT_URL, text_file(200, SITE_ADS_TXT));
    harness
}

/// Like [`recorded_harness`], with the database as a file next to the
/// packs, so that a test can read everything the app wrote.
fn harness_on_disk() -> Harness {
    let data_dir = Arc::new(tempfile::tempdir().unwrap());
    let store = SqliteStore::open(data_dir.path()).unwrap();
    let harness = harness_in(
        CaptureBehavior::Return(Box::new(visit_capture())),
        store,
        data_dir,
    );
    *harness.capture.network.lock().unwrap() = network();
    harness
        .files
        .answer(SITE_ADS_TXT_URL, text_file(200, SITE_ADS_TXT));
    harness
}

async fn recorded_scan(harness: &Harness) -> ScanDetail {
    let detail = run_scan(harness, scan_options(false)).await;
    assert_eq!(
        detail.summary.status,
        ScanStatus::Completed,
        "{:?}",
        detail.summary.error
    );
    detail
}

fn pack_path(harness: &Harness, scan_id: &str) -> PathBuf {
    harness
        .data_dir
        .path()
        .join("archive/scans")
        .join(format!("{scan_id}.pack.zst"))
}

/// Everything a pack holds, decompressed, as one text.
fn pack_content(harness: &Harness, scan_id: &str) -> String {
    let bytes = std::fs::read(pack_path(harness, scan_id)).unwrap();
    String::from_utf8_lossy(&zstd::stream::decode_all(bytes.as_slice()).unwrap()).into_owned()
}

fn holds(bytes: &[u8], needle: &str) -> bool {
    bytes
        .windows(needle.len())
        .any(|window| window == needle.as_bytes())
}

/// Every file under the data directory, as bytes: the database with its
/// write-ahead log, and the packs.
fn everything_on_disk(harness: &Harness) -> Vec<(PathBuf, Vec<u8>)> {
    let mut files = Vec::new();
    let mut pending = vec![harness.data_dir.path().to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else {
                files.push((path.clone(), std::fs::read(&path).unwrap()));
            }
        }
    }
    files
}

async fn all_rows(harness: &Harness, scan_id: &str) -> Vec<adscan_model::NetworkRow> {
    harness
        .network
        .list(scan_id, &[], None, None)
        .await
        .unwrap()
        .rows
}

// ---------------------------------------------------------------------------
// Writing during a capture
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_scan_stores_every_request_and_the_kept_bodies_in_one_sealed_pack() {
    let harness = recorded_harness();
    let detail = recorded_scan(&harness).await;
    let scan_id = &detail.summary.id;

    let network = detail.network.clone().unwrap();
    assert_eq!(network.availability, NetworkAvailability::Available);
    assert_eq!(network.request_count, 12);
    assert!(network.headers_redacted && !network.truncated);
    let on_disk = std::fs::metadata(pack_path(&harness, scan_id))
        .unwrap()
        .len();
    assert_eq!(network.stored_bytes, on_disk as f64);

    let record = harness
        .archive
        .index
        .record(scan_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(record.state, ArchiveState::Sealed);
    // document, wrapper, three bid answers, the ad call's answer, and the
    // four other scripts of the capture plus the inline one.
    assert_eq!(record.body_count, 11);

    // The audit trail says what was stored.
    let logs = harness.scans.logs(scan_id, None).await.unwrap();
    assert!(
        logs.iter().any(|entry| entry
            .message
            .starts_with("Stored 12 requests and 11 bodies")),
        "{:?}",
        logs.iter().map(|entry| &entry.message).collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn a_sealed_pack_is_a_plain_zstd_file_that_names_its_own_content() {
    let harness = recorded_harness();
    let detail = recorded_scan(&harness).await;
    let scan_id = &detail.summary.id;

    // Any zstd tool reads it front to back, trailer included.
    let content = pack_content(&harness, scan_id);
    assert!(content.contains(DOCUMENT_HTML));
    assert!(
        content
            .contains(r#""fullUrl":"https://pub.example/?utm_source=newsletter&session=abc123""#)
    );

    // From the file alone: the trailer names the manifest, the manifest
    // names every frame.
    let packs = harness.archive.packs.list().await.unwrap();
    assert_eq!(packs.len(), 1);
    let file = packs[0].file.clone();
    assert_eq!(file, format!("scans/{scan_id}.pack.zst"));
    let trailer = harness
        .archive
        .packs
        .read(&Location {
            file: file.clone(),
            offset: packs[0].bytes - TRAILER_LEN as u64,
            length: TRAILER_LEN as u32,
        })
        .await
        .unwrap();
    let (offset, length) = manifest_range(&trailer).expect("a sealed pack ends with its trailer");
    let frame = harness
        .archive
        .packs
        .read(&Location {
            file: file.clone(),
            offset,
            length,
        })
        .await
        .unwrap();
    let manifest: Manifest =
        serde_json::from_slice(&zstd::stream::decode_all(frame.as_slice()).unwrap()).unwrap();
    assert_eq!(manifest.scan_id, *scan_id);
    assert_eq!(manifest.url, detail.summary.url);
    assert!(manifest.headers_redacted);
    assert_eq!(manifest.request_count, 12);
    assert_eq!(manifest.bodies.len(), 11);
    assert_eq!(
        manifest
            .records
            .iter()
            .map(|records| records.count)
            .sum::<u32>(),
        12
    );
    for body in &manifest.bodies {
        let frame = harness
            .archive
            .packs
            .read(&Location {
                file: file.clone(),
                offset: body.offset,
                length: body.length,
            })
            .await
            .unwrap();
        let raw = zstd::stream::decode_all(frame.as_slice()).unwrap();
        assert_eq!(raw.len() as u64, body.raw_length);
    }
}

#[tokio::test]
async fn a_body_loaded_twice_is_stored_once() {
    let harness = recorded_harness();
    let mut records = network();
    // The same wrapper again from an iframe, and a second copy of the answer.
    let mut again = with_body(
        net(20, "https://cdn.other.example/copy/ads.js"),
        ResourceKind::Script,
        "text/javascript",
        WRAPPER_SOURCE,
    );
    again.context = RequestContext::Iframe;
    records.push(again);
    *harness.capture.network.lock().unwrap() = records;
    let detail = recorded_scan(&harness).await;

    let record = harness
        .archive
        .index
        .record(&detail.summary.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!((record.request_count, record.body_count), (13, 11));
    let content = pack_content(&harness, &detail.summary.id);
    assert_eq!(
        content
            .matches("function isMobile(){return window.innerWidth>768}")
            .count(),
        1
    );

    // Both requests read the one body.
    for id in ["r1", "r20"] {
        let body = harness
            .network
            .body(&detail.summary.id, id, BodySide::Response, None, None)
            .await
            .unwrap();
        assert_eq!(body.text.as_deref(), Some(WRAPPER_SOURCE), "{id}");
    }
}

// ---------------------------------------------------------------------------
// What is and what is not stored
// ---------------------------------------------------------------------------

#[tokio::test]
async fn credentials_in_headers_never_reach_the_disk_but_urls_and_bodies_do() {
    let harness = harness_on_disk();
    let detail = recorded_scan(&harness).await;
    let content = pack_content(&harness, &detail.summary.id);

    // Stored as captured: URLs with their query, documents, payloads.
    for kept in [
        "session=abc123",
        "signed in as reader@example.org",
        "bid-openx-top-1",
        "CPconsentstring",
        r#""nbr":7"#,
        "Mozilla/5.0 (Macintosh)",
    ] {
        assert!(content.contains(kept), "{kept} should be in the pack");
    }
    // Cookie names and lengths, the authorization scheme, cookie attributes.
    assert!(content.contains("sid=[20 bytes]; theme=[4 bytes]"));
    assert!(content.contains("sid=[20 bytes]; Path=/; HttpOnly\\nab=[1 bytes]; Max-Age=60"));
    assert!(content.contains("Bearer [16 bytes]"));

    // The values themselves are nowhere: not in the pack, not in SQLite,
    // not in any other file of the data directory.
    let files = everything_on_disk(&harness);
    assert!(
        files
            .iter()
            .any(|(path, _)| path.extension().is_some_and(|ext| ext == "sqlite3"))
    );
    for secret in [COOKIE_SECRET, BEARER_SECRET] {
        assert!(!content.contains(secret), "{secret} is in the pack");
        for (path, bytes) in &files {
            assert!(!holds(bytes, secret), "{secret} is in {}", path.display());
        }
    }
}

#[tokio::test]
async fn images_media_and_fonts_are_recorded_without_their_bodies() {
    let harness = recorded_harness();
    let mut records = network();
    records.push(with_body(
        net(21, "https://cdn.pub.example/fonts/serif.woff2"),
        ResourceKind::Font,
        "font/woff2",
        "wOF2-font-bytes",
    ));
    records.push(with_body(
        net(22, "https://video.pub.example/clip.mp4"),
        ResourceKind::Media,
        "video/mp4",
        "ftypisom-video-bytes",
    ));
    *harness.capture.network.lock().unwrap() = records;
    let detail = recorded_scan(&harness).await;
    let scan_id = &detail.summary.id;

    let content = pack_content(&harness, scan_id);
    for body in [
        "PNG-not-really",
        "GIF89a",
        "wOF2-font-bytes",
        "ftypisom-video-bytes",
    ] {
        assert!(!content.contains(body), "{body} was stored");
    }
    for id in ["r2", "r5", "r21", "r22"] {
        let detail = harness.network.request(scan_id, id).await.unwrap();
        assert_eq!(detail.response_body.state, BodyState::NotKeptType, "{id}");
        assert_eq!(detail.response_body.stored_bytes, 0.0);
        assert!(detail.response_body.bytes > 0.0, "the size is metadata");
        assert!(!detail.row.has_response_body);
        let body = harness
            .network
            .body(scan_id, id, BodySide::Response, None, None)
            .await
            .unwrap();
        assert_eq!((body.state, body.text), (BodyState::NotKeptType, None));
    }
}

#[tokio::test]
async fn the_database_holds_no_query_string_no_header_and_no_body() {
    let harness = harness_on_disk();
    let detail = recorded_scan(&harness).await;
    let (rows, _) = harness
        .archive
        .index
        .rows(&detail.summary.id, &[], 0, 100)
        .await
        .unwrap();
    let document = rows.iter().find(|row| row.seq == 0).unwrap();
    assert_eq!(
        (document.host.as_str(), document.path.as_str()),
        ("pub.example", "/")
    );
    assert_eq!(
        document.query_len,
        "utm_source=newsletter&session=abc123".len() as u32
    );

    let database: Vec<u8> = everything_on_disk(&harness)
        .into_iter()
        .filter(|(path, _)| !path.to_string_lossy().ends_with(".pack.zst"))
        .flat_map(|(_, bytes)| bytes)
        .collect();
    let holds = |needle: &str| holds(&database, needle);
    for absent in [
        "session=abc123",
        "utm_source",
        "Mozilla/5.0 (Macintosh)",
        "signed in as reader@example.org",
        "bid-openx-top-1\",\"tagid\"",
        "CPconsentstring",
    ] {
        assert!(!holds(absent), "{absent} is in the database");
    }
    assert!(
        holds("rtb.openx.net"),
        "host names are what the index is for"
    );
}

#[tokio::test]
async fn api_keys_and_proxy_passwords_are_never_written_with_a_scan() {
    const API_KEY: &str = "sk-ant-archive-test-key";
    const PROXY_PASSWORD: &str = "pr0xy-p4ssw0rd-archive";
    let harness = harness_on_disk();
    harness
        .secrets
        .store_api_key(adscan_model::ApiProvider::Anthropic, API_KEY)
        .await
        .unwrap();
    let settings = harness
        .settings
        .save_proxy_profile(adscan_model::ProxyProfileDraft {
            id: None,
            name: "Belgium".into(),
            scheme: adscan_model::ProxyScheme::Http,
            host: "proxy.example".into(),
            port: 8080,
            username: Some("scanner".into()),
        })
        .await
        .unwrap();
    let proxy_id = settings.proxies[0].id.clone();
    harness
        .settings
        .set_proxy_password(&proxy_id, PROXY_PASSWORD)
        .await
        .unwrap();
    // An AI scan, so that the key is handed to the pipeline; the model API
    // is unreachable and the scan completes without it.
    let mut options = scan_options(true);
    options.proxy = Some(adscan_model::ProxyChoice::Profile {
        id: proxy_id,
        name: None,
        host: None,
    });
    let detail = run_scan(&harness, options).await;
    assert_eq!(detail.summary.status, ScanStatus::Completed);
    assert_eq!(
        harness
            .capture
            .proxy_passwords
            .lock()
            .unwrap()
            .last()
            .unwrap()
            .as_deref(),
        Some(PROXY_PASSWORD),
        "the capture did get the password"
    );

    let content = pack_content(&harness, &detail.summary.id);
    let files = everything_on_disk(&harness);
    assert!(
        files
            .iter()
            .any(|(path, _)| path.extension().is_some_and(|ext| ext == "sqlite3"))
    );
    for secret in [API_KEY, PROXY_PASSWORD] {
        assert!(!content.contains(secret));
        for (path, bytes) in &files {
            assert!(!holds(bytes, secret), "{secret} is in {}", path.display());
        }
    }
}

#[tokio::test]
async fn store_full_headers_keeps_the_values_for_new_scans_only() {
    let harness = recorded_harness();
    let redacted = recorded_scan(&harness).await;
    harness
        .settings
        .update(SettingsPatch {
            store_full_headers: Some(true),
            ..SettingsPatch::default()
        })
        .await
        .unwrap();
    let full = recorded_scan(&harness).await;

    assert!(!pack_content(&harness, &redacted.summary.id).contains(COOKIE_SECRET));
    let content = pack_content(&harness, &full.summary.id);
    assert!(content.contains(&format!("sid={COOKIE_SECRET}; theme=dark")));
    assert!(content.contains(&format!("Bearer {BEARER_SECRET}")));
    assert!(!full.network.unwrap().headers_redacted);
    assert!(redacted.network.unwrap().headers_redacted);

    let shown = harness
        .network
        .request(&full.summary.id, "r0")
        .await
        .unwrap();
    let cookie = shown
        .request_headers
        .iter()
        .find(|header| header.name == "Cookie")
        .unwrap();
    assert_eq!(cookie.redaction, None);
    assert!(cookie.value.contains(COOKIE_SECRET));
    let earlier = harness
        .network
        .request(&redacted.summary.id, "r0")
        .await
        .unwrap();
    let cookie = earlier
        .request_headers
        .iter()
        .find(|header| header.name == "Cookie")
        .unwrap();
    assert_eq!(cookie.redaction, Some(HeaderRedaction::Cookie));
    assert_eq!(cookie.value, "sid=[20 bytes]; theme=[4 bytes]");
}

#[tokio::test]
async fn with_keep_network_data_off_nothing_is_written_and_the_scan_says_so() {
    let harness = recorded_harness();
    harness
        .settings
        .update(SettingsPatch {
            keep_network_data: Some(false),
            ..SettingsPatch::default()
        })
        .await
        .unwrap();
    let detail = recorded_scan(&harness).await;
    let scan_id = &detail.summary.id;

    assert!(
        !harness.data_dir.path().join("archive").exists(),
        "no pack, no directory"
    );
    let network = detail.network.clone().unwrap();
    assert_eq!(network.availability, NetworkAvailability::Disabled);
    assert_eq!((network.request_count, network.stored_bytes), (0, 0.0));
    let listed = harness
        .network
        .list(scan_id, &[], None, None)
        .await
        .unwrap();
    assert_eq!(
        (listed.rows.len(), listed.total, listed.next_cursor),
        (0, 0, None)
    );
    let summary = harness.network.summary(scan_id).await.unwrap();
    assert_eq!(summary.archive.availability, NetworkAvailability::Disabled);
    assert_eq!(summary.total_requests, 0);
    assert_eq!(
        harness
            .network
            .request(scan_id, "r3")
            .await
            .unwrap_err()
            .code(),
        adscan_model::ErrorCode::NotFound
    );
    let source = harness
        .sources
        .get(scan_id, "s3", SourceView::Formatted, None)
        .await
        .unwrap();
    assert_eq!(
        (source.availability, source.text),
        (SourceAvailability::NotRetained, None)
    );

    // The capture is still read for its bid exchanges: the verdicts are
    // there, without a request to open. Nothing points at a row.
    assert_eq!(*harness.capture.had_sink.lock().unwrap(), Some(true));
    let exchange = bid(&detail, "top", "rubicon").exchange.clone().unwrap();
    assert_eq!(exchange.verdict, ExchangeVerdict::LateOnNetwork);
    assert_eq!(
        (exchange.request_id, exchange.late_by_ms),
        (None, Some(84.0))
    );
    assert!(
        detail
            .scripts
            .iter()
            .all(|script| script.request_id.is_none())
    );
    assert!(
        detail
            .slots
            .iter()
            .flat_map(|slot| &slot.requests)
            .all(|request| request.request_id.is_none())
    );
}

// ---------------------------------------------------------------------------
// Reading it back
// ---------------------------------------------------------------------------

#[tokio::test]
async fn rows_come_in_chunks_ordered_by_start_time() {
    let harness = recorded_harness();
    let detail = recorded_scan(&harness).await;
    let scan_id = &detail.summary.id;

    let first = harness
        .network
        .list(scan_id, &[], None, Some(5))
        .await
        .unwrap();
    assert_eq!((first.rows.len(), first.total), (5, 12));
    let second = harness
        .network
        .list(scan_id, &[], first.next_cursor.as_deref(), Some(5))
        .await
        .unwrap();
    let third = harness
        .network
        .list(scan_id, &[], second.next_cursor.as_deref(), Some(5))
        .await
        .unwrap();
    assert_eq!(
        (
            second.rows.len(),
            third.rows.len(),
            third.next_cursor.clone()
        ),
        (5, 2, None)
    );

    let rows: Vec<_> = [first.rows, second.rows, third.rows].concat();
    let starts: Vec<f64> = rows.iter().map(|row| row.started_at_ms).collect();
    assert!(
        starts.windows(2).all(|pair| pair[0] <= pair[1]),
        "{starts:?}"
    );
    let mut ids: Vec<&str> = rows.iter().map(|row| row.id.as_str()).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), 12, "every request once");

    assert_eq!(
        harness
            .network
            .list(scan_id, &[], Some("not-a-cursor"), None)
            .await
            .unwrap_err()
            .code(),
        adscan_model::ErrorCode::InvalidInput
    );
    assert_eq!(
        harness
            .network
            .list("no-such-scan", &[], None, None)
            .await
            .unwrap_err()
            .code(),
        adscan_model::ErrorCode::NotFound
    );
}

#[tokio::test]
async fn a_row_says_what_the_request_was_and_how_it_went() {
    let harness = recorded_harness();
    let detail = recorded_scan(&harness).await;
    let rows = all_rows(&harness, &detail.summary.id).await;
    let row = |id: &str| rows.iter().find(|row| row.id == id).unwrap();

    let document = row("r0");
    assert_eq!(
        document.url,
        "https://pub.example/?utm_source=newsletter&session=abc123"
    );
    assert_eq!(
        (document.kind, document.category),
        (ResourceKind::Document, NetworkCategory::Site)
    );
    assert!(document.first_party && document.has_response_body && !document.has_request_body);
    assert_eq!(
        (document.outcome, document.status),
        (RequestOutcome::Ok, Some(200))
    );
    assert_eq!(document.transferred_bytes, 300.0);
    assert_eq!(document.duration_ms, Some(100.0));
    let phases = document.phases.clone().unwrap();
    assert_eq!((phases.connect, phases.wait), (25.0, 51.0));
    assert_eq!(
        phases.queue + phases.connect + phases.wait + phases.receive,
        99.0
    );

    let wrapper = row("r1");
    assert_eq!(wrapper.category, NetworkCategory::Site);
    let initiator = wrapper.initiator.clone().unwrap();
    assert_eq!(
        (initiator.kind, initiator.line),
        (InitiatorKind::Parser, Some(12))
    );

    let openx = row("r3");
    assert_eq!(
        (openx.category, openx.bidder.as_deref()),
        (NetworkCategory::Bid, Some("openx"))
    );
    assert_eq!(openx.vendor.as_deref(), Some("OpenX"));
    assert!(!openx.first_party && openx.has_request_body && openx.has_response_body);
    let initiator = openx.initiator.clone().unwrap();
    assert_eq!(
        initiator.script_id.as_deref(),
        Some("s3"),
        "the wrapper of the capture"
    );
    assert_eq!(initiator.line, Some(1));

    assert_eq!(row("r5").redirect_of.as_deref(), Some("r4"));
    assert_eq!(row("r4").status, Some(302));
    assert_eq!(row("r6").preflight_of.as_deref(), Some("r3"));
    assert_eq!(row("r6").kind, ResourceKind::Preflight);

    let ad_call = row("r7");
    assert_eq!(ad_call.category, NetworkCategory::AdServer);
    assert!(ad_call.no_hb_targeting, "the ad call carried no hb_ keys");
    assert_eq!(ad_call.ad, None);

    let blocked = row("r8");
    assert_eq!(blocked.outcome, RequestOutcome::Blocked);
    assert_eq!(blocked.error.as_deref(), Some("inspector"));
    assert_eq!(blocked.phases, None);

    let pending = row("r9");
    assert_eq!(
        (pending.outcome, pending.duration_ms),
        (RequestOutcome::Pending, None)
    );
    assert_eq!(pending.served_from, ServedFrom::Network);
}

#[tokio::test]
async fn a_request_opens_with_headers_timing_chain_stack_and_bodies() {
    let harness = recorded_harness();
    let detail = recorded_scan(&harness).await;
    let scan_id = &detail.summary.id;

    let openx = harness.network.request(scan_id, "r3").await.unwrap();
    assert!(openx.available);
    assert_eq!(openx.protocol.as_deref(), Some("h2"));
    assert_eq!(openx.remote_address.as_deref(), Some("203.0.113.9:443"));
    let authorization = &openx.request_headers[1];
    assert_eq!(
        (
            authorization.name.as_str(),
            authorization.value.as_str(),
            authorization.redaction
        ),
        (
            "Authorization",
            "Bearer [16 bytes]",
            Some(HeaderRedaction::Credential)
        )
    );
    assert_eq!(openx.request_headers[0].redaction, None);
    assert_eq!(openx.preflight_ms, Some(45.0));
    assert_eq!(openx.stack.len(), 1);
    assert_eq!(openx.stack[0].function.as_deref(), Some("askEndpoint"));
    assert_eq!(openx.stack[0].script_id.as_deref(), Some("s3"));
    assert_eq!(openx.request_body.state, BodyState::Kept);
    assert_eq!(openx.request_body.mime.as_deref(), Some("text/plain"));
    assert_eq!(openx.response_body.state, BodyState::Kept);
    assert_eq!(openx.response_body.stored_bytes, 30.0);
    let timing = openx.timing.unwrap();
    assert_eq!(
        (timing.dns_ms, timing.tls_ms, timing.wait_ms),
        (Some(5.0), Some(15.0), Some(51.0))
    );

    let document = harness.network.request(scan_id, "r0").await.unwrap();
    let set_cookies: Vec<&str> = document
        .response_headers
        .iter()
        .filter(|header| header.name == "set-cookie")
        .map(|header| header.value.as_str())
        .collect();
    assert_eq!(
        set_cookies,
        [
            "sid=[20 bytes]; Path=/; HttpOnly",
            "ab=[1 bytes]; Max-Age=60"
        ]
    );

    // Both hops of the chain, whichever hop is opened.
    for id in ["r4", "r5"] {
        let hop = harness.network.request(scan_id, id).await.unwrap();
        let chain: Vec<(&str, Option<u16>)> = hop
            .redirect_chain
            .iter()
            .map(|hop| (hop.request_id.as_str(), hop.status))
            .collect();
        assert_eq!(chain, [("r4", Some(302)), ("r5", Some(200))], "{id}");
        assert_eq!(
            hop.redirect_chain[0].url,
            "https://sync.partner.example/setuid?uid=42"
        );
    }
    assert!(document.redirect_chain.is_empty());

    // A script's response says which script of the scan it is.
    let wrapper = harness.network.request(scan_id, "r1").await.unwrap();
    assert_eq!(wrapper.response_body.script_id.as_deref(), Some("s3"));

    let pending = harness.network.request(scan_id, "r9").await.unwrap();
    assert_eq!(pending.response_body.state, BodyState::Pending);
    let redirect = harness.network.request(scan_id, "r4").await.unwrap();
    assert_eq!(redirect.response_body.state, BodyState::Unavailable);

    for unknown in ["r999", "s1", "", "r-1"] {
        assert_eq!(
            harness
                .network
                .request(scan_id, unknown)
                .await
                .unwrap_err()
                .code(),
            adscan_model::ErrorCode::NotFound,
            "{unknown}"
        );
    }
}

#[tokio::test]
async fn a_body_is_read_by_byte_range() {
    let harness = recorded_harness();
    let detail = recorded_scan(&harness).await;
    let scan_id = &detail.summary.id;

    let whole = harness
        .network
        .body(scan_id, "r1", BodySide::Response, None, None)
        .await
        .unwrap();
    assert_eq!(whole.text.as_deref(), Some(WRAPPER_SOURCE));
    assert_eq!((whole.offset, whole.next_offset), (0.0, None));
    assert_eq!(whole.total_bytes, WRAPPER_SOURCE.len() as f64);

    let mut offset = Some(0.0);
    let mut read = String::new();
    let mut chunks = 0;
    while let Some(at) = offset {
        let chunk = harness
            .network
            .body(scan_id, "r1", BodySide::Response, Some(at), Some(200.0))
            .await
            .unwrap();
        assert_eq!(chunk.state, BodyState::Kept);
        read.push_str(chunk.text.as_deref().unwrap());
        offset = chunk.next_offset;
        chunks += 1;
    }
    assert_eq!(read, WRAPPER_SOURCE);
    assert_eq!(chunks, WRAPPER_SOURCE.len().div_ceil(200));

    let payload = harness
        .network
        .body(scan_id, "r3", BodySide::Request, None, None)
        .await
        .unwrap();
    assert!(
        payload
            .text
            .unwrap()
            .starts_with(r#"{"id":"breq-openx-0001""#)
    );
    let none = harness
        .network
        .body(scan_id, "r0", BodySide::Request, None, None)
        .await
        .unwrap();
    assert_eq!((none.state, none.text), (BodyState::Empty, None));
    let past_the_end = harness
        .network
        .body(scan_id, "r1", BodySide::Response, Some(1e9), None)
        .await
        .unwrap();
    assert_eq!(past_the_end.text.as_deref(), Some(""));
    assert_eq!(past_the_end.next_offset, None);
}

#[tokio::test]
async fn the_summary_counts_and_points_at_the_requests_worth_opening() {
    let harness = recorded_harness();
    let detail = recorded_scan(&harness).await;
    let summary = harness.network.summary(&detail.summary.id).await.unwrap();

    assert_eq!(summary.archive.availability, NetworkAvailability::Available);
    assert_eq!(summary.total_requests, 12);
    assert_eq!(summary.transferred_bytes, 3_600.0);
    assert_eq!((summary.failed, summary.blocked_or_cancelled), (0, 1));
    assert_eq!(
        (
            summary.late_bids,
            summary.unanswered_bids,
            summary.from_cache
        ),
        (1, 0, 0)
    );
    let count = |category: NetworkCategory| {
        summary
            .categories
            .iter()
            .find(|count| count.category == category)
            .map_or(0, |count| count.requests)
    };
    assert_eq!(
        count(NetworkCategory::Bid),
        4,
        "three bid calls and the preflight of one"
    );
    assert_eq!(count(NetworkCategory::AdServer), 1);
    assert_eq!(count(NetworkCategory::Site), 3);
    assert_eq!(
        count(NetworkCategory::Identity),
        3,
        "the redirect chain and the blocked pixel"
    );
    assert_eq!(count(NetworkCategory::OtherThirdParty), 1);
    assert_eq!(
        summary
            .categories
            .iter()
            .map(|count| count.requests)
            .sum::<u32>(),
        12
    );
    assert_eq!(summary.pages.len(), 1);
    assert_eq!(summary.pages[0].requests, 12);
    assert_eq!(summary.vendors[0].vendor, "OpenX");
    assert_eq!(summary.vendors[0].requests, 2);

    let notable: Vec<&str> = summary.notable.iter().map(|row| row.id.as_str()).collect();
    // The late bid answer, the blocked request, and the slowest request
    // before the first auction.
    assert_eq!(notable, ["r11", "r8", "r0"]);
    assert!(
        summary.notable[0].url.contains("account_id=1001"),
        "notable rows are whole rows"
    );
}

// ---------------------------------------------------------------------------
// Bid exchanges
// ---------------------------------------------------------------------------

fn bid<'a>(detail: &'a ScanDetail, unit: &str, bidder: &str) -> &'a adscan_model::Bid {
    detail.auctions[0]
        .ad_units
        .iter()
        .find(|candidate| candidate.code == unit)
        .unwrap()
        .bids
        .iter()
        .find(|bid| bid.bidder == bidder)
        .unwrap()
}

#[tokio::test]
async fn bids_are_tied_to_their_requests_with_a_verdict() {
    let harness = recorded_harness();
    let detail = recorded_scan(&harness).await;

    // Both openx bids travelled in one request, found by their ids.
    for unit in ["top", "side"] {
        let exchange = bid(&detail, unit, "openx").exchange.clone().unwrap();
        assert_eq!(exchange.request_id.as_deref(), Some("r3"), "{unit}");
        assert!(exchange.exact);
        assert_eq!(exchange.verdict, ExchangeVerdict::InTime);
        assert_eq!(
            (exchange.late_by_ms, exchange.network_ms),
            (Some(-100.0), Some(140.0))
        );
    }

    // Prebid announced three triplelift bids; one left the page.
    let sent = bid(&detail, "top", "triplelift").exchange.clone().unwrap();
    assert_eq!(
        (sent.request_id.as_deref(), sent.verdict),
        (Some("r10"), ExchangeVerdict::InTime)
    );
    for unit in ["side", "footer"] {
        let exchange = bid(&detail, unit, "triplelift").exchange.clone().unwrap();
        assert_eq!(exchange.verdict, ExchangeVerdict::NotSent, "{unit}");
        assert_eq!(exchange.request_id, None);
    }

    // No id from the tracker: matched by timing, and late under the 250 ms
    // timeout.
    let late = bid(&detail, "top", "rubicon").exchange.clone().unwrap();
    assert_eq!(late.request_id.as_deref(), Some("r11"));
    assert!(!late.exact);
    assert_eq!(
        (late.verdict, late.late_by_ms),
        (ExchangeVerdict::LateOnNetwork, Some(84.0))
    );

    // The same links on the rows.
    let rows = all_rows(&harness, &detail.summary.id).await;
    let ad = |id: &str| {
        rows.iter()
            .find(|row| row.id == id)
            .unwrap()
            .ad
            .clone()
            .unwrap()
    };
    assert_eq!(ad("r3").ad_units, ["top", "side"]);
    assert_eq!(
        (ad("r3").nbr, ad("r3").cpm, ad("r3").auction_id.as_str()),
        (Some(7), None, AUCTION)
    );
    assert_eq!(
        (ad("r10").ad_units, ad("r10").cpm),
        (vec!["top".to_string()], Some(0.4))
    );
    assert_eq!(ad("r11").verdict, ExchangeVerdict::LateOnNetwork);
    assert!(rows.iter().filter(|row| row.ad.is_some()).count() == 3);

    // The slot's ad request found its ad call.
    let top = detail
        .slots
        .iter()
        .find(|slot| slot.element_id == "top")
        .unwrap();
    assert_eq!(top.requests[0].request_id.as_deref(), Some("r7"));

    // The bidder check that quotes openx's answer names the request.
    let openx = detail
        .bidder_diagnostics
        .as_ref()
        .unwrap()
        .bidders
        .iter()
        .find(|diagnosis| diagnosis.bidder == "openx")
        .unwrap();
    let reason = openx
        .checks
        .iter()
        .find(|check| check.code == "response-reason")
        .unwrap();
    assert_eq!(reason.request_ids, ["r3"]);

    // The wrapper's script record names the request that delivered it.
    let wrapper = detail
        .scripts
        .iter()
        .find(|script| script.id == "s3")
        .unwrap();
    assert_eq!(wrapper.request_id.as_deref(), Some("r1"));
    assert_eq!(wrapper.initiator.as_deref(), Some("https://pub.example/"));
    assert_eq!(wrapper.sha256.as_ref().unwrap().len(), 64);
}

#[tokio::test]
async fn a_bid_exchange_is_decoded_with_its_ad_units() {
    let harness = recorded_harness();
    let detail = recorded_scan(&harness).await;
    let scan_id = &detail.summary.id;

    let openx = harness.network.request(scan_id, "r3").await.unwrap();
    let decoded = openx.decoded.unwrap();
    assert_eq!(decoded.protocol, ExchangeProtocol::OpenRtb);
    let request = decoded.request.unwrap();
    assert_eq!(request.tmax_ms, Some(250.0));
    let imps: Vec<(&str, Option<&str>)> = request
        .imps
        .iter()
        .map(|imp| (imp.id.as_str(), imp.ad_unit.as_deref()))
        .collect();
    assert_eq!(
        imps,
        [
            ("bid-openx-top-1", Some("top")),
            ("bid-openx-side-1", Some("side"))
        ]
    );
    assert!(request.consent_string_present);
    let response = decoded.response.unwrap();
    assert_eq!(
        (response.nbr, response.nbr_meaning.as_deref()),
        (Some(7), Some("Blocked Publisher or Site"))
    );

    let context = openx.ad_context.unwrap();
    assert_eq!(context.bidder.as_deref(), Some("openx"));
    assert_eq!(context.timeout_ms, Some(250.0));
    assert_eq!(
        (context.auction_started_at_ms, context.auction_ended_at_ms),
        (Some(1_000.0), Some(1_250.0))
    );
    let units: Vec<&str> = context
        .units
        .iter()
        .map(|unit| unit.ad_unit.as_str())
        .collect();
    assert_eq!(units, ["top", "side"]);
    assert!(
        context
            .units
            .iter()
            .all(|unit| unit.status == adscan_model::BidStatus::NoBid)
    );

    let triplelift = harness
        .network
        .request(scan_id, "r10")
        .await
        .unwrap()
        .decoded
        .unwrap();
    let bid = &triplelift.response.unwrap().seats[0].bids[0];
    assert_eq!((bid.ad_unit.as_deref(), bid.price), (Some("top"), 0.4));

    // Not OpenRTB: no decoder, raw only.
    assert_eq!(
        harness
            .network
            .request(scan_id, "r11")
            .await
            .unwrap()
            .decoded,
        None
    );
    // Not a bid exchange at all.
    let document = harness.network.request(scan_id, "r0").await.unwrap();
    assert_eq!((document.decoded, document.ad_context), (None, None));
}

// ---------------------------------------------------------------------------
// Script sources
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_script_opens_from_the_archive_as_captured_and_formatted() {
    let harness = recorded_harness();
    let detail = recorded_scan(&harness).await;
    let scan_id = &detail.summary.id;

    let original = harness
        .sources
        .get(
            scan_id,
            "s3",
            SourceView::Original,
            Some(OriginalPosition {
                line: 1,
                column: 80,
            }),
        )
        .await
        .unwrap();
    assert_eq!(original.availability, SourceAvailability::Available);
    assert_eq!(original.text.as_deref(), Some(WRAPPER_SOURCE));
    assert_eq!(
        (original.format, original.line_count, original.focus_line),
        (SourceFormat::Unformatted, 1, Some(1))
    );
    assert!(original.anchors.is_empty() && original.regions.is_empty());
    assert!(
        original
            .ad_calls
            .iter()
            .any(|call| call.name == "requestBids" && call.line == 1)
    );

    let column = WRAPPER_SOURCE.find("function isMobile").unwrap() as u32 + 1;
    let formatted = harness
        .sources
        .get(
            scan_id,
            "s3",
            SourceView::Formatted,
            Some(OriginalPosition { line: 1, column }),
        )
        .await
        .unwrap();
    assert_eq!(formatted.format, SourceFormat::Parser);
    assert!(formatted.line_count > 20);
    let text = formatted.text.unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len() as u32, formatted.line_count);
    let focus = formatted.focus_line.unwrap() as usize;
    assert!(
        lines[focus - 1].contains("function isMobile()"),
        "{}",
        lines[focus - 1]
    );
    let request_bids = formatted
        .ad_calls
        .iter()
        .find(|call| call.name == "requestBids")
        .unwrap();
    assert!(lines[request_bids.line as usize - 1].contains("pbjs.requestBids("));

    // An inline script has no request; its source is kept all the same.
    let inline = detail.scripts.iter().find(|script| script.inline).unwrap();
    let source = harness
        .sources
        .get(scan_id, &inline.id, SourceView::Original, None)
        .await
        .unwrap();
    assert_eq!(source.text.as_deref(), Some(INLINE_SCRIPT));

    assert_eq!(
        harness
            .sources
            .get(scan_id, "s99", SourceView::Formatted, None)
            .await
            .unwrap_err()
            .code(),
        adscan_model::ErrorCode::NotFound
    );
}

// ---------------------------------------------------------------------------
// Deleting, evicting, clearing, sweeping
// ---------------------------------------------------------------------------

#[tokio::test]
async fn deleting_a_scan_deletes_its_rows_and_its_pack() {
    let harness = recorded_harness();
    let kept = recorded_scan(&harness).await;
    let deleted = recorded_scan(&harness).await;
    assert!(pack_path(&harness, &deleted.summary.id).exists());

    harness.scans.delete(&deleted.summary.id).await.unwrap();

    assert!(!pack_path(&harness, &deleted.summary.id).exists());
    assert_eq!(
        harness
            .archive
            .index
            .record(&deleted.summary.id)
            .await
            .unwrap(),
        None
    );
    let (rows, total) = harness
        .archive
        .index
        .rows(&deleted.summary.id, &[], 0, 10)
        .await
        .unwrap();
    assert_eq!((rows.len(), total), (0, 0));
    // The other scan is untouched.
    assert!(pack_path(&harness, &kept.summary.id).exists());
    assert_eq!(all_rows(&harness, &kept.summary.id).await.len(), 12);
    let storage = harness.settings.archive_storage().await.unwrap();
    assert_eq!(storage.scans, 1);
    assert_eq!(storage.stored_bytes, kept.network.unwrap().stored_bytes);

    harness.scans.delete_all().await.unwrap();
    assert!(harness.archive.packs.list().await.unwrap().is_empty());
    assert_eq!(harness.settings.archive_storage().await.unwrap().scans, 0);
}

#[tokio::test]
async fn the_oldest_archives_are_evicted_to_stay_under_the_limit_and_keep_their_request_list() {
    let harness = recorded_harness();
    let oldest = recorded_scan(&harness).await;
    let middle = recorded_scan(&harness).await;
    let newest = recorded_scan(&harness).await;
    let one = oldest.network.clone().unwrap().stored_bytes;
    let before = harness.settings.archive_storage().await.unwrap();
    assert_eq!((before.scans, before.evicted_scans), (3, 0));
    assert_eq!(before.oldest_at, Some(oldest.summary.started_at));

    // Room for two packs and a bit.
    let evicted = harness
        .archive
        .enforce_limit(one * 2.5, None)
        .await
        .unwrap();
    assert_eq!(evicted, 1);

    let after = harness.settings.archive_storage().await.unwrap();
    assert_eq!((after.scans, after.evicted_scans), (2, 1));
    assert_eq!(after.oldest_at, Some(middle.summary.started_at));
    assert!(!pack_path(&harness, &oldest.summary.id).exists());
    assert!(pack_path(&harness, &newest.summary.id).exists());

    // The evicted scan still lists its requests, without what the pack held.
    let detail = harness.scans.get(&oldest.summary.id).await.unwrap();
    let network = detail.network.unwrap();
    assert_eq!(network.availability, NetworkAvailability::Evicted);
    assert_eq!(
        (
            network.request_count,
            network.body_count,
            network.stored_bytes
        ),
        (12, 0, 0.0)
    );
    let rows = all_rows(&harness, &oldest.summary.id).await;
    assert_eq!(rows.len(), 12);
    let document = rows.iter().find(|row| row.id == "r0").unwrap();
    assert_eq!(
        document.url, "https://pub.example/",
        "the query string went with the pack"
    );
    assert!(!document.has_response_body && document.phases.is_none());
    assert!(
        rows.iter()
            .find(|row| row.id == "r11")
            .unwrap()
            .ad
            .is_some(),
        "links are kept"
    );
    let opened = harness
        .network
        .request(&oldest.summary.id, "r3")
        .await
        .unwrap();
    assert!(!opened.available);
    assert!(opened.request_headers.is_empty());
    assert_eq!(opened.response_body.state, BodyState::Evicted);
    assert_eq!(opened.request_body.state, BodyState::Evicted);
    assert_eq!(opened.decoded, None);
    let body = harness
        .network
        .body(&oldest.summary.id, "r3", BodySide::Response, None, None)
        .await
        .unwrap();
    assert_eq!((body.state, body.text), (BodyState::Evicted, None));
    let source = harness
        .sources
        .get(&oldest.summary.id, "s3", SourceView::Formatted, None)
        .await
        .unwrap();
    assert_eq!(source.availability, SourceAvailability::Evicted);
}

#[tokio::test]
async fn a_new_scan_evicts_older_ones_but_never_itself_and_a_lower_limit_acts_at_once() {
    let harness = recorded_harness();
    let first = recorded_scan(&harness).await;
    let one = first.network.clone().unwrap().stored_bytes;
    // The limit cannot be set this low through the settings; the stored
    // settings are what a scan reads.
    let mut settings = harness.settings.get().await.unwrap();
    settings.archive_limit_bytes = one * 0.5;
    adscan_engine::application::ports::SettingsRepository::save(&harness.store, &settings)
        .await
        .unwrap();

    let second = recorded_scan(&harness).await;
    assert_eq!(
        harness
            .scans
            .get(&first.summary.id)
            .await
            .unwrap()
            .network
            .unwrap()
            .availability,
        NetworkAvailability::Evicted
    );
    assert_eq!(
        harness
            .scans
            .get(&second.summary.id)
            .await
            .unwrap()
            .network
            .unwrap()
            .availability,
        NetworkAvailability::Available,
        "the scan just written keeps its archive even when it alone is over the limit"
    );
    let logs = harness.scans.logs(&second.summary.id, None).await.unwrap();
    assert!(logs.iter().any(|entry| {
        entry
            .message
            .contains("Removed the network data of 1 older scan")
    }));

    // Raising the limit through the settings evicts nothing; the bounds hold.
    let storage_limit = |bytes: f64| SettingsPatch {
        archive_limit_bytes: Some(bytes),
        ..SettingsPatch::default()
    };
    assert!(
        harness
            .settings
            .update(storage_limit(1024.0))
            .await
            .is_err()
    );
    let updated = harness
        .settings
        .update(storage_limit(200.0 * 1024.0 * 1024.0))
        .await
        .unwrap();
    assert_eq!(updated.archive_limit_bytes, 200.0 * 1024.0 * 1024.0);
    assert_eq!(
        harness
            .settings
            .archive_storage()
            .await
            .unwrap()
            .limit_bytes,
        200.0 * 1024.0 * 1024.0
    );
    assert_eq!(harness.settings.archive_storage().await.unwrap().scans, 1);
}

#[tokio::test]
async fn clearing_removes_all_network_data_and_leaves_the_reports() {
    let harness = recorded_harness();
    let first = recorded_scan(&harness).await;
    let second = recorded_scan(&harness).await;

    let storage = harness.settings.clear_archive().await.unwrap();
    assert_eq!((storage.scans, storage.stored_bytes), (0, 0.0));
    assert!(harness.archive.packs.list().await.unwrap().is_empty());
    for scan in [&first, &second] {
        let detail = harness.scans.get(&scan.summary.id).await.unwrap();
        assert_eq!(detail.summary.status, ScanStatus::Completed);
        assert_eq!(detail.auctions.len(), 1, "the report stays");
        let network = detail.network.unwrap();
        assert_eq!(
            (network.availability, network.request_count),
            (NetworkAvailability::Evicted, 0)
        );
        assert!(all_rows(&harness, &scan.summary.id).await.is_empty());
    }
}

#[tokio::test]
async fn a_cancelled_scan_and_a_failed_capture_leave_no_archive() {
    let harness = harness(
        CaptureBehavior::BlockUntilCancelled,
        "http://127.0.0.1:1",
        None,
    );
    *harness.capture.network.lock().unwrap() = network();
    let started = harness.scans.start(scan_options(false)).await.unwrap();
    for _ in 0..200 {
        if harness.capture.had_sink.lock().unwrap().is_some() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    harness.scans.cancel(&started.id).await.unwrap();
    let detail = wait_for_end(&harness, &started.id).await;
    assert_eq!(detail.summary.status, ScanStatus::Cancelled);
    assert_eq!(detail.network, None);
    assert!(harness.archive.packs.list().await.unwrap().is_empty());
    assert_eq!(
        harness.archive.index.record(&started.id).await.unwrap(),
        None
    );

    let failing = common::harness(
        CaptureBehavior::Fail(CaptureError::Navigation(
            "net::ERR_NAME_NOT_RESOLVED".into(),
        )),
        "http://127.0.0.1:1",
        None,
    );
    *failing.capture.network.lock().unwrap() = network();
    let detail = run_scan(&failing, scan_options(false)).await;
    assert_eq!(detail.summary.status, ScanStatus::Failed);
    assert_eq!(detail.network, None);
    assert!(failing.archive.packs.list().await.unwrap().is_empty());
}

#[tokio::test]
async fn the_start_up_sweep_cleans_up_after_a_crash() {
    let data_dir = Arc::new(tempfile::tempdir().unwrap());
    let database = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(database.path()).unwrap();
    let harness = harness_in(
        CaptureBehavior::Return(Box::new(visit_capture())),
        store.clone(),
        Arc::clone(&data_dir),
    );
    *harness.capture.network.lock().unwrap() = network();
    let sealed = recorded_scan(&harness).await;
    let removed_by_user = recorded_scan(&harness).await;
    let crashed = recorded_scan(&harness).await;

    // A scan that was still capturing when the app died: its archive is
    // registered as being written, with rows and a pack that has no trailer.
    rusqlite::Connection::open(database.path().join("adtech-scanner.sqlite3"))
        .unwrap()
        .execute(
            "UPDATE scan_archives SET state = 'writing' WHERE scan_id = ?1",
            [&crashed.summary.id],
        )
        .unwrap();
    let pack = pack_path(&harness, &crashed.summary.id);
    let bytes = std::fs::read(&pack).unwrap();
    std::fs::write(&pack, &bytes[..bytes.len() - 100]).unwrap();
    // A pack whose scan was deleted between the rows and the file.
    let orphan = data_dir.path().join("archive/scans/deleted-scan.pack.zst");
    std::fs::write(&orphan, b"left behind").unwrap();
    // A sealed archive whose file the user removed.
    std::fs::remove_file(pack_path(&harness, &removed_by_user.summary.id)).unwrap();
    drop(harness);

    // The app starts again.
    let restarted = harness_in(CaptureBehavior::NoBrowser, store, Arc::clone(&data_dir));
    let report = restarted.archive.sweep().await.unwrap();
    assert_eq!(
        (report.unfinished, report.orphans, report.missing),
        (1, 1, 1)
    );

    assert!(!pack.exists() && !orphan.exists());
    assert_eq!(
        restarted
            .archive
            .index
            .record(&crashed.summary.id)
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        restarted
            .scans
            .get(&crashed.summary.id)
            .await
            .unwrap()
            .network,
        None
    );
    let evicted = restarted
        .scans
        .get(&removed_by_user.summary.id)
        .await
        .unwrap()
        .network
        .unwrap();
    assert_eq!(
        (evicted.availability, evicted.request_count),
        (NetworkAvailability::Evicted, 12)
    );
    // What was fine stays fine and still reads after the restart.
    assert!(pack_path(&restarted, &sealed.summary.id).exists());
    assert_eq!(all_rows(&restarted, &sealed.summary.id).await.len(), 12);
    let source = restarted
        .sources
        .get(&sealed.summary.id, "s3", SourceView::Original, None)
        .await
        .unwrap();
    assert_eq!(source.text.as_deref(), Some(WRAPPER_SOURCE));
    // A second sweep finds nothing to do.
    let again = restarted.archive.sweep().await.unwrap();
    assert_eq!((again.unfinished, again.orphans, again.missing), (0, 0, 0));
}

// ---------------------------------------------------------------------------
// Damage
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_damaged_pack_is_reported_and_never_misread() {
    let harness = recorded_harness();
    let detail = recorded_scan(&harness).await;
    let scan_id = &detail.summary.id;
    let (rows, _) = harness
        .archive
        .index
        .rows(scan_id, &[], 0, 100)
        .await
        .unwrap();
    let wrapper = rows.iter().find(|row| row.seq == 1).unwrap();
    let body = harness
        .archive
        .index
        .body(scan_id, wrapper.body_no.unwrap())
        .await
        .unwrap()
        .unwrap();

    // One flipped bit inside the wrapper's frame.
    let path = pack_path(&harness, scan_id);
    let mut bytes = std::fs::read(&path).unwrap();
    bytes[(body.pack_offset + u64::from(body.pack_length) / 2) as usize] ^= 0x10;
    std::fs::write(&path, &bytes).unwrap();
    let error = harness
        .network
        .body(scan_id, "r1", BodySide::Response, None, None)
        .await
        .unwrap_err();
    assert_eq!(error.code(), adscan_model::ErrorCode::Storage);
    // Other frames are independent: the document still reads.
    let document = harness
        .network
        .body(scan_id, "r0", BodySide::Response, None, None)
        .await
        .unwrap();
    assert_eq!(document.text.as_deref(), Some(DOCUMENT_HTML));

    // The file cut short: ranges past the end fail, they are not read short.
    std::fs::write(&path, &bytes[..40]).unwrap();
    let error = harness
        .archive
        .packs
        .read(&Location {
            file: format!("scans/{scan_id}.pack.zst"),
            offset: 20,
            length: 100,
        })
        .await
        .unwrap_err();
    assert!(
        matches!(
            error,
            adscan_engine::application::ports::PackError::OutOfRange(_)
        ),
        "{error:?}"
    );
    assert!(
        harness
            .network
            .body(scan_id, "r0", BodySide::Response, None, None)
            .await
            .is_err()
    );
    // The list still answers, from the index alone.
    let rows = all_rows(&harness, scan_id).await;
    assert_eq!(rows.len(), 12);
    assert_eq!(
        rows.iter().find(|row| row.id == "r0").unwrap().url,
        "https://pub.example/"
    );
    let opened = harness.network.request(scan_id, "r0").await.unwrap();
    assert!(!opened.available);
}

// ---------------------------------------------------------------------------
// Scans without an archive
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_scan_from_before_the_archive_reads_as_not_recorded() {
    let harness = recorded_harness();
    let detail = recorded_scan(&harness).await;
    let scan_id = &detail.summary.id;
    // As an older version of the app left it: no archive of any kind.
    harness.archive.remove(scan_id).await.unwrap();
    let mut scripts = detail.scripts.clone();
    for script in &mut scripts {
        script.sha256 = None;
        script.request_id = None;
    }
    harness.store.save_scripts(scan_id, &scripts).await.unwrap();

    let old = harness.scans.get(scan_id).await.unwrap();
    assert_eq!(old.network, None);
    assert_eq!(old.auctions.len(), 1, "the report itself reads as before");
    let summary = harness.network.summary(scan_id).await.unwrap();
    assert_eq!(
        summary.archive.availability,
        NetworkAvailability::NotRecorded
    );
    assert_eq!((summary.total_requests, summary.notable.len()), (0, 0));
    assert!(all_rows(&harness, scan_id).await.is_empty());
    assert_eq!(
        harness
            .network
            .body(scan_id, "r1", BodySide::Response, None, None)
            .await
            .unwrap_err()
            .code(),
        adscan_model::ErrorCode::NotFound
    );
    let source = harness
        .sources
        .get(scan_id, "s3", SourceView::Formatted, None)
        .await
        .unwrap();
    assert_eq!(
        (source.availability, source.text, source.line_count),
        (SourceAvailability::NotRecorded, None, 0)
    );
}
