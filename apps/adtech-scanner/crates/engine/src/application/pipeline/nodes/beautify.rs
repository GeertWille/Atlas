use adscan_model::{LogKind, LogLevel};
use serde_json::json;

use crate::application::pipeline::audit::StepRun;
use crate::application::pipeline::graph::analysis_selection;
use crate::application::pipeline::state::{NodeResult, SourceFile};
use crate::application::ports::FormatMethod;

pub async fn beautify_scripts(run: &mut StepRun<'_>) -> NodeResult {
    let selection = analysis_selection(run.state);
    let mut sources = Vec::with_capacity(selection.len());
    let mut total_lines = 0;
    let mut reflowed = 0;
    for script_index in selection {
        run.check_cancelled()?;
        let body = run.state.capture()?.scripts[script_index].body.clone();
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
        run.log(LogLevel::Debug, LogKind::Message, message, Some(data))
            .await?;
        sources.push(SourceFile {
            script_index,
            lines: formatted.lines,
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
