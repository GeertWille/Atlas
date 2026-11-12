use serde_json::json;

use crate::application::pipeline::audit::StepRun;
use crate::application::pipeline::state::NodeResult;
use crate::domain::heuristics::{self, AUTOMATION_RULE, RuleInput};

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
        // Whoever writes the overview, the reader must learn that missing
        // bids in this scan may be the scan's own doing.
        if hit.rule == AUTOMATION_RULE {
            run.state.caveats.push(
                "This scan was identifiable as automated traffic, so bidders may have filtered it; \
                 confirm missing bids with a visit a person drives"
                    .to_string(),
            );
        }
        run.add_finding(hit.finding, json!({ "rule": hit.rule }))
            .await?;
    }
    Ok(format!(
        "{rule_count} rules evaluated, {hit_count} findings"
    ))
}
