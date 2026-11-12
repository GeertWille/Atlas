use adscan_model::{LogKind, LogLevel};
use serde_json::json;

use crate::application::pipeline::audit::StepRun;
use crate::application::pipeline::state::NodeResult;
use crate::domain::{environment, scripts, telemetry};

pub async fn derive_telemetry(run: &mut StepRun<'_>) -> NodeResult {
    let capture = run.state.capture()?;
    let mut derived = telemetry::derive(capture);
    let described = scripts::describe(capture);
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
