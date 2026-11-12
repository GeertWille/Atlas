//! Throwaway: capture cost with and without a network sink. Run by hand.
mod support;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use adscan_model::{
    BodyAdmission, BodyOffer, BodySkip, CaptureProxy, CapturedBody, DeviceProfile, NetworkRecord,
    NetworkSink, ProxyScheme, ResourceKind, TrackerEventKind, VisitorBehavior,
};

#[derive(Default)]
struct Counting {
    records: AtomicU64,
    child: AtomicU64,
    bodies: AtomicU64,
    body_bytes: AtomicU64,
    unavailable: AtomicU64,
    pending: AtomicU64,
    in_flight: AtomicU64,
    peak_in_flight: AtomicU64,
}

impl NetworkSink for Counting {
    fn admit_body(&self, offer: &BodyOffer<'_>) -> BodyAdmission {
        let text = offer.mime_type.is_none_or(|m| {
            m.starts_with("text/")
                || m.contains("json")
                || m.contains("javascript")
                || m.contains("xml")
        });
        let kind = matches!(
            offer.kind,
            ResourceKind::Document
                | ResourceKind::Script
                | ResourceKind::Stylesheet
                | ResourceKind::Xhr
                | ResourceKind::Fetch
                | ResourceKind::Other
        );
        if kind && text {
            let now = self
                .in_flight
                .fetch_add(offer.expected_bytes, Ordering::Relaxed)
                + offer.expected_bytes;
            self.peak_in_flight.fetch_max(now, Ordering::Relaxed);
            BodyAdmission::Read {
                max_bytes: 5 * 1024 * 1024,
            }
        } else {
            BodyAdmission::Skip(BodySkip::Type)
        }
    }
    fn record(&self, record: NetworkRecord) {
        self.records.fetch_add(1, Ordering::Relaxed);
        if record.target_id.is_some() {
            self.child.fetch_add(1, Ordering::Relaxed);
        }
        match &record.body {
            CapturedBody::Read { bytes, .. } => {
                self.bodies.fetch_add(1, Ordering::Relaxed);
                self.body_bytes
                    .fetch_add(bytes.len() as u64, Ordering::Relaxed);
            }
            CapturedBody::Unavailable(_) => {
                self.unavailable.fetch_add(1, Ordering::Relaxed);
            }
            _ => {}
        }
        if matches!(record.end, adscan_model::RequestEnd::Pending) {
            self.pending.fetch_add(1, Ordering::Relaxed);
        }
    }
}

fn cpu_ms() -> f64 {
    let stat = std::fs::read_to_string("/proc/self/stat").unwrap();
    let fields: Vec<&str> = stat
        .rsplit_once(')')
        .unwrap()
        .1
        .split_whitespace()
        .collect();
    let ticks: f64 = fields[11].parse::<f64>().unwrap() + fields[12].parse::<f64>().unwrap();
    ticks * 10.0
}

fn peak_rss_mb() -> f64 {
    let status = std::fs::read_to_string("/proc/self/status").unwrap();
    let line = status.lines().find(|l| l.starts_with("VmHWM")).unwrap();
    line.split_whitespace()
        .nth(1)
        .unwrap()
        .parse::<f64>()
        .unwrap()
        / 1024.0
}

#[tokio::test]
async fn perf() {
    let Ok(mode) = std::env::var("PERF_SINK") else {
        return;
    };
    let seconds: u32 = std::env::var("PERF_SECONDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(9);
    let site = adscan_browser::start_demo_site().await.unwrap();
    let url = std::env::var("PERF_URL").unwrap_or_else(|_| site.url.clone());
    let mut request = support::request(&url, DeviceProfile::Desktop);
    request.visitor = VisitorBehavior::Scroll;
    request.observe_seconds = seconds;
    if let Ok(proxy) = std::env::var("PERF_PROXY_PORT") {
        request.proxy = CaptureProxy::Custom {
            scheme: ProxyScheme::Http,
            host: "127.0.0.1".into(),
            port: proxy.parse().unwrap(),
            username: None,
        };
    }
    let sink = Arc::new(Counting::default());
    let mut reported = support::Reported::new();
    if mode == "1" {
        reported.hooks.network = Some(sink.clone());
    }
    let (cpu0, t0) = (cpu_ms(), Instant::now());
    let outcome = reported.run(request).await;
    let (cpu, wall) = (cpu_ms() - cpu0, t0.elapsed().as_millis());
    let capture = match outcome.result {
        Ok(c) => c,
        Err(e) => {
            println!("PERF failed: {e} {:?}", outcome.progress);
            return;
        }
    };
    let first_auction = capture
        .events
        .iter()
        .find(|e| matches!(e.kind, TrackerEventKind::AuctionInit { .. }))
        .map(|e| e.t.round());
    let recorded = outcome
        .progress
        .iter()
        .find(|l| l.starts_with("Recorded"))
        .cloned()
        .unwrap_or_default();
    println!(
        "PERF sink={mode} wall={wall}ms cpu={cpu:.0}ms peak_rss={:.0}MB load={:?} first_auction={:?} scripts={} events={} hosts={} | records={} child={} bodies={} body_mb={:.1} unavailable={} pending={} peak_admitted_mb={:.1} | {recorded}",
        peak_rss_mb(),
        capture.page_load_ms.map(|v| v.round()),
        first_auction,
        capture.scripts.len(),
        capture.events.len(),
        capture.environment.request_hosts.len(),
        sink.records.load(Ordering::Relaxed),
        sink.child.load(Ordering::Relaxed),
        sink.bodies.load(Ordering::Relaxed),
        sink.body_bytes.load(Ordering::Relaxed) as f64 / 1e6,
        sink.unavailable.load(Ordering::Relaxed),
        sink.pending.load(Ordering::Relaxed),
        sink.peak_in_flight.load(Ordering::Relaxed) as f64 / 1e6,
    );
}
