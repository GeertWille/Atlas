use std::sync::Arc;

use tokio::sync::mpsc;

use crate::application::AppError;
use crate::application::pipeline::audit::StepRun;
use crate::application::pipeline::state::NodeResult;
use crate::application::ports::ProgressFn;

pub async fn launch_browser(run: &mut StepRun<'_>) -> NodeResult {
    let browser = run
        .deps
        .capture
        .locate_browser(run.job.settings.chrome_path.clone())
        .await
        .ok_or(AppError::BrowserNotFound)?;
    run.info(format!("Using {} at {}", browser.version, browser.path))
        .await?;
    Ok(format!("{} found", browser.version))
}

pub async fn capture_page(run: &mut StepRun<'_>) -> NodeResult {
    let request = run.job.scan.capture_request(&run.job.settings);
    run.info(format!(
        "Loading {} as {:?}, observing for {} s (auto-scroll {}, accept consent {})",
        request.url,
        request.device,
        request.observe_seconds,
        request.auto_scroll,
        request.accept_consent,
    ))
    .await?;

    let (progress_tx, mut progress_rx) = mpsc::unbounded_channel::<String>();
    let on_progress: ProgressFn = Arc::new(move |line| {
        let _ = progress_tx.send(line);
    });
    let source = Arc::clone(&run.deps.capture);
    let capture = source.capture(request, run.job.cancel.clone(), on_progress);
    tokio::pin!(capture);
    let result = loop {
        tokio::select! {
            result = &mut capture => break result,
            Some(line) = progress_rx.recv() => run.info(line).await?,
        }
    };
    while let Ok(line) = progress_rx.try_recv() {
        run.info(line).await?;
    }
    let capture = result?;

    let summary = format!(
        "Captured {} scripts and {} tracker events from {}",
        capture.scripts.len(),
        capture.events.len(),
        capture.final_url,
    );
    run.state.capture = Some(capture);
    Ok(summary)
}
