use adscan_model::{LogKind, LogLevel};
use serde_json::json;

use crate::application::pipeline::audit::StepRun;
use crate::application::pipeline::state::NodeResult;
use crate::domain::synthesis;

pub async fn synthesize(run: &mut StepRun<'_>) -> NodeResult {
    let before = run.state.findings.len();
    let findings = synthesis::merge_and_rank(std::mem::take(&mut run.state.findings));
    let score = synthesis::health_score(&findings);
    let scan_id = run.state.summary.id.clone();

    let mut overview = match run.state.model_overview.clone() {
        Some(overview) => overview,
        None => synthesis::template_overview(
            &run.state.summary.hostname,
            run.state.summary.device,
            run.state.telemetry()?.pages.len(),
            &run.state.telemetry()?.total.metrics,
            &findings,
            score,
        ),
    };
    if let Some(failure) = &run.state.ai_failure {
        overview.push_str(&format!(
            "\n\n**Warning:** {failure}. The results below come from the deterministic checks and any AI findings recorded before the failure."
        ));
    }
    for caveat in &run.state.caveats {
        overview.push_str(&format!("\n\n**Note:** {caveat}."));
    }

    run.deps.scans.save_findings(&scan_id, &findings).await?;
    run.deps.scans.save_overview(&scan_id, &overview).await?;
    run.state.summary.finding_counts = synthesis::severity_counts(&findings);
    run.state.summary.health_score = Some(score);
    run.deps.scans.save_summary(&run.state.summary).await?;

    let summary = format!(
        "{} findings ({} merged), health score {score}/100",
        findings.len(),
        before - findings.len(),
    );
    run.log(
        LogLevel::Info,
        LogKind::Message,
        summary.clone(),
        Some(json!({ "healthScore": score, "overviewSource": if run.state.model_overview.is_some() { "model" } else { "template" } })),
    )
    .await?;
    run.state.findings = findings;
    Ok(summary)
}
