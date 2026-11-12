use adscan_model::{LogKind, LogLevel, ScriptCategory};
use serde_json::json;

use crate::application::pipeline::audit::StepRun;
use crate::application::pipeline::graph::analysis_selection;
use crate::application::pipeline::state::{NodeResult, SourceFile};
use crate::application::ports::FormatMethod;
use crate::domain::chunks::split_lines;
use crate::domain::guide::{publisher_ranges, telemetry_literals};

pub async fn beautify_scripts(run: &mut StepRun<'_>) -> NodeResult {
    let selection = analysis_selection(run.state);
    let placements = telemetry_literals(run.state.telemetry()?);
    let mut sources = Vec::with_capacity(selection.len());
    let mut total_lines = 0;
    let mut reflowed = 0;
    for script_index in selection {
        run.check_cancelled()?;
        let body = run.state.body(script_index)?.to_string();
        let formatted = run.deps.formatter.format(body).await;
        if formatted.method != FormatMethod::Parser {
            reflowed += 1;
        }
        total_lines += formatted.lines.len();
        let record = &run.state.scripts[script_index].record;
        let message = format!(
            "{} {}: {} lines ({:?})",
            record.id,
            record.url,
            formatted.lines.len(),
            formatted.method,
        );
        let data = json!({ "scriptId": record.id, "lines": formatted.lines.len() });
        // A stock Prebid build drowns every search in library internals, so
        // only the stretches that can be the publisher's own code stay in
        // the default search. The whole file remains readable by id.
        let default_search = (record.category == ScriptCategory::Prebid).then(|| {
            publisher_ranges(
                &formatted.lines,
                &split_lines(&formatted.lines),
                &placements,
            )
        });
        run.log(LogLevel::Debug, LogKind::Message, message, Some(data))
            .await?;
        sources.push(SourceFile {
            script_index,
            lines: formatted.lines,
            default_search,
        });
    }
    let summary = format!(
        "Beautified {} scripts into {} lines ({} without a full parse)",
        sources.len(),
        total_lines,
        reflowed,
    );
    run.state.sources = sources;
    Ok(summary)
}
