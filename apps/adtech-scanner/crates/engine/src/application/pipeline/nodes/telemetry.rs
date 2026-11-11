use adscan_model::{LogKind, LogLevel};
use serde_json::json;

use crate::application::pipeline::audit::StepRun;
use crate::application::pipeline::state::NodeResult;
use crate::domain::{scripts, telemetry};

pub async fn derive_telemetry(run: &mut StepRun<'_>) -> NodeResult {
    let capture = run.state.capture()?;
    let mut derived = telemetry::derive(capture);
    let described = scripts::describe(capture);
    let prebid_version = telemetry::prebid_version(capture);
    derived.metrics.ad_script_count = described.iter().filter(|script| script.record.ad_related).count() as u32;

    let scan_id = run.state.summary.id.clone();
    let records: Vec<_> = described.iter().map(|script| script.record.clone()).collect();
    run.deps
        .scans
        .save_telemetry(&scan_id, &derived.metrics, &derived.auctions, &derived.slots)
        .await?;
    run.deps.scans.save_scripts(&scan_id, &records).await?;

    let undecided = described.iter().filter(|script| script.ambiguous_hints.is_some()).count();
    let summary = format!(
        "{} auctions, {} slots, {} scripts ({} ad-related, {} undecided)",
        derived.auctions.len(),
        derived.slots.len(),
        described.len(),
        derived.metrics.ad_script_count,
        undecided,
    );
    run.log(
        LogLevel::Info,
        LogKind::Message,
        format!("Derived telemetry: {summary}"),
        Some(json!({ "metrics": derived.metrics })),
    )
    .await?;

    run.state.summary.prebid_version = prebid_version;
    run.state.telemetry = Some(derived);
    run.state.scripts = described;
    Ok(summary)
}
