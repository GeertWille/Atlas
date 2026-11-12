use adscan_model::{Capture, LogKind, LogLevel};
use serde_json::json;

use crate::application::archive::analysis::RecordAnalysis;
use crate::application::pipeline::audit::StepRun;
use crate::application::pipeline::state::NodeResult;
use crate::application::ports::RequestAdLink;
use crate::domain::network::exchange;
use crate::domain::{environment, scripts, telemetry};

pub async fn derive_telemetry(run: &mut StepRun<'_>) -> NodeResult {
    let capture = run.state.capture()?;
    let mut derived = telemetry::derive(capture);
    let mut described = scripts::describe(capture);
    let archived = run.state.archived;
    let ad_links = link_network(&mut derived, capture, &run.state.network, archived);
    if !archived {
        for script in &mut described {
            script.record.request_id = None;
        }
    }
    let prebid_version = telemetry::prebid_version(capture);
    let scan = &run.job.scan;
    let environment = environment::detect(capture, scan.visitor, scan.seed, scan.proxy.usage());
    derived.total.metrics.ad_script_count = described
        .iter()
        .filter(|script| script.record.ad_related)
        .count() as u32;

    let scan_id = run.state.summary.id.clone();
    let records: Vec<_> = described
        .iter()
        .map(|script| script.record.clone())
        .collect();
    run.deps
        .scans
        .save_telemetry(
            &scan_id,
            &derived.total.metrics,
            &derived.pages,
            &derived.total.auctions,
            &derived.total.slots,
        )
        .await?;
    run.deps
        .scans
        .save_environment(&scan_id, &environment)
        .await?;
    run.deps.scans.save_scripts(&scan_id, &records).await?;
    if archived {
        run.deps
            .archive
            .index
            .save_ad_links(&scan_id, ad_links)
            .await?;
    }

    let undecided = described
        .iter()
        .filter(|script| script.ambiguous_hints.is_some())
        .count();
    let summary = format!(
        "{} page view{}, {} auctions, {} slots, {} scripts ({} ad-related, {} undecided)",
        derived.pages.len(),
        if derived.pages.len() == 1 { "" } else { "s" },
        derived.total.auctions.len(),
        derived.total.slots.len(),
        described.len(),
        derived.total.metrics.ad_script_count,
        undecided,
    );
    run.log(
        LogLevel::Info,
        LogKind::Message,
        format!("Derived telemetry: {summary}"),
        Some(json!({
            "metrics": derived.total.metrics,
            "pages": derived.pages,
            "environment": environment,
        })),
    )
    .await?;

    run.state.summary.prebid_version = prebid_version;
    run.state.telemetry = Some(derived);
    run.state.environment = Some(environment);
    run.state.scripts = described;
    Ok(summary)
}

/// Ties the bids and slot requests of the telemetry to the requests seen on
/// the network. The per-page copies get the same links as the totals.
fn link_network(
    derived: &mut telemetry::ScanTelemetry,
    capture: &Capture,
    network: &RecordAnalysis,
    archived: bool,
) -> Vec<RequestAdLink> {
    let announcements = exchange::announcements(&capture.events);
    let page_starts: Vec<f64> = derived
        .pages
        .iter()
        .map(|page| page.started_at_ms)
        .collect();
    let scopes = std::iter::once(&mut derived.total).chain(&mut derived.by_page);
    let mut links = Vec::new();
    for (index, scope) in scopes.enumerate() {
        let found = exchange::link(
            &mut scope.auctions,
            &announcements,
            &network.exchanges,
            &page_starts,
        );
        if archived {
            exchange::link_slots(&mut scope.slots, &network.ad_server_calls);
        } else {
            exchange::forget_requests(&mut scope.auctions);
        }
        if index == 0 {
            links = found;
        }
    }
    links
}
