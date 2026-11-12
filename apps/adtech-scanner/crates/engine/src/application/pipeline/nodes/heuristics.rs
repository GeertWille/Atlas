use serde_json::json;

use crate::application::pipeline::audit::StepRun;
use crate::application::pipeline::state::NodeResult;
use crate::domain::heuristics::{self, AUTOMATION_RULE, CONSENT_WAIT_RULE, RuleInput};

pub async fn run_rules(run: &mut StepRun<'_>) -> NodeResult {
    let scripts = run.state.script_records();
    let hits = heuristics::evaluate(&RuleInput {
        capture: run.state.capture()?,
        telemetry: run.state.telemetry()?,
        diagnostics: run.state.diagnostics.as_ref(),
        scripts: &scripts,
        environment: run.state.environment.as_ref(),
    });
    let rule_count = heuristics::all_rules().len();
    let hit_count = hits.len();
    for hit in hits {
        // Whoever writes the overview, the reader must learn what in this
        // scan may be the scan's own doing. Only what the evidence supports:
        // the automation note is for bidders that never bid.
        if hit.rule == AUTOMATION_RULE && !hit.finding.affected_bidders.is_empty() {
            run.state.caveats.push(format!(
                "{} never bid in this scan. Scripted visits (scroll, \"Browse like a visitor\") are automated \
                 by design and the pages can see that, so a bidder may have filtered the scan; before treating \
                 one as broken, run the scan again as \"I'll browse myself\"",
                hit.finding.affected_bidders.join(", ")
            ));
        }
        if hit.rule == CONSENT_WAIT_RULE {
            let account = hit.finding.description.trim_end_matches('.').to_string();
            run.warn(account.clone()).await?;
            run.state.caveats.push(account);
        }
        run.add_finding(hit.finding, json!({ "rule": hit.rule }))
            .await?;
    }
    Ok(format!(
        "{rule_count} rules evaluated, {hit_count} findings"
    ))
}
