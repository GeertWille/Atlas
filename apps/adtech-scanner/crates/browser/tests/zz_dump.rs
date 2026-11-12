mod support;
use adscan_model::{CapturedBody, DeviceProfile, VisitorBehavior};

fn dump(records: &[adscan_model::NetworkRecord]) {
    for r in records {
        let body = match &r.body {
            CapturedBody::Read { bytes, total_bytes } => {
                format!("Read {}/{}", bytes.len(), total_bytes)
            }
            other => format!("{other:?}"),
        };
        println!(
            "{:>3} p{} {:?}/{:?} {} {} st={:?} t={:.1}..{:?}..{:?} from={:?} pre={:?} tgt={} fr={:?} srv={:?} end={:?} body={} enc={} dec={} q={:?}",
            r.seq,
            r.page,
            r.kind,
            r.context,
            r.method,
            &r.url[..r.url.len().min(90)],
            r.status,
            r.started_at_ms,
            r.response_at_ms.map(|v| v.round()),
            r.ended_at_ms.map(|v| v.round()),
            r.redirect_from,
            r.preflight_of,
            r.target_id.as_deref().map_or("-", |t| &t[..6]),
            r.frame_id.as_deref().map(|f| &f[..6]),
            r.served_from,
            r.end,
            body,
            r.encoded_bytes,
            r.decoded_bytes,
            r.queued_ms.map(|q| q.round())
        );
    }
}

#[tokio::test]
async fn dump_fixture() {
    let which = std::env::var("DUMP").unwrap_or_default();
    if which.is_empty() {
        return;
    }
    let (capture, records) = if which == "demo" || which == "browse" {
        let site = adscan_browser::start_demo_site().await.unwrap();
        let mut request = support::request(&site.url, DeviceProfile::Desktop);
        request.visitor = if which == "demo" {
            VisitorBehavior::Scroll
        } else {
            VisitorBehavior::Browse
        };
        request.extra_pages = 2;
        request.seed = 3;
        request.observe_seconds = if which == "demo" { 9 } else { 6 };
        support::capture_with_requests(request).await
    } else {
        let fixtures = support::Fixtures::start().await;
        let mut request = support::request(&fixtures.url(&which), DeviceProfile::Desktop);
        request.observe_seconds = 4;
        support::capture_with_requests(request).await
    };
    dump(&records);
    for p in &capture.pages {
        println!(
            "PAGE {} {:?} {}..{}",
            p.url, p.reached_by, p.started_at_ms, p.ended_at_ms
        );
    }
    println!(
        "title={:?} hosts={:?}",
        capture.page_title, capture.environment.request_hosts
    );
    for c in &capture.console {
        println!("CONSOLE {} {}", c.level, &c.text[..c.text.len().min(160)]);
    }
}
