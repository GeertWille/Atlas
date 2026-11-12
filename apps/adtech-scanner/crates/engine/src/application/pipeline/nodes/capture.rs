use std::sync::Arc;

use adscan_model::{
    ActivityKind, CaptureActivity, CaptureProxy, CaptureRequest, LogKind, LogLevel, VisitorBehavior,
};
use serde_json::json;
use tokio::sync::mpsc;

use crate::application::AppError;
use crate::application::pipeline::audit::StepRun;
use crate::application::pipeline::state::{NodeError, NodeResult};
use crate::application::ports::{ActivityFn, CaptureSession};

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
    let start = start_line(&request, run.job.scan.proxy.usage().name);
    run.info(start).await?;

    // Listening from here on: a visit that is finished while it waits for
    // the scan profile ends as soon as its page has loaded, instead of
    // losing the request.
    let finish = run.job.finish.arm();
    // Held until the browser is gone, so the next scan finds the profile free.
    let _profile_turn = if request.persistent_profile {
        let scan_profile = Arc::clone(&run.job.scan_profile);
        if scan_profile.available_permits() == 0 {
            run.info("Waiting for another scan to release the scan profile")
                .await?;
        }
        tokio::select! {
            permit = scan_profile.acquire_owned() => Some(permit.map_err(|_| NodeError::Cancelled)?),
            () = run.job.cancel.cancelled() => return Err(NodeError::Cancelled),
        }
    } else {
        None
    };

    let (activity_tx, mut activity_rx) = mpsc::unbounded_channel::<CaptureActivity>();
    let on_activity: ActivityFn = Arc::new(move |activity| {
        let _ = activity_tx.send(activity);
    });
    let session = CaptureSession {
        cancel: run.job.cancel.clone(),
        finish,
        on_activity,
        proxy_password: run.job.proxy_password.clone(),
    };
    let source = Arc::clone(&run.deps.capture);
    let capture = source.capture(request, session);
    tokio::pin!(capture);
    let result = loop {
        tokio::select! {
            result = &mut capture => break result,
            Some(activity) = activity_rx.recv() => record(run, activity).await?,
        }
    };
    run.job.finish.disarm();
    while let Ok(activity) = activity_rx.try_recv() {
        record(run, activity).await?;
    }
    let capture = result?;

    let summary = format!(
        "Captured {} page view{}, {} scripts and {} tracker events from {}",
        capture.pages.len(),
        if capture.pages.len() == 1 { "" } else { "s" },
        capture.scripts.len(),
        capture.events.len(),
        capture.final_url,
    );
    run.state.capture = Some(capture);
    Ok(summary)
}

fn start_line(request: &CaptureRequest, proxy_name: Option<String>) -> String {
    let visit = match request.visitor {
        VisitorBehavior::None => {
            format!("observing for {} s without input", request.observe_seconds)
        }
        VisitorBehavior::Scroll => format!("scrolling for {} s", request.observe_seconds),
        VisitorBehavior::Browse => format!(
            "browsing like a visitor, {} s per page and up to {} further page{}",
            request.observe_seconds,
            request.extra_pages,
            if request.extra_pages == 1 { "" } else { "s" },
        ),
        VisitorBehavior::Manual => "a person browses; recording until they finish".to_string(),
    };
    let proxy = match &request.proxy {
        CaptureProxy::None => "no proxy".to_string(),
        CaptureProxy::System => "system proxy".to_string(),
        CaptureProxy::Custom { host, .. } => match proxy_name {
            Some(name) => format!("proxy \"{name}\" ({host})"),
            None => format!("proxy {host}"),
        },
    };
    format!(
        "Loading {} as {:?}: {visit} (seed {}, {}, {} profile, accept consent {}, {proxy})",
        request.url,
        request.device,
        request.seed,
        if request.headless {
            "headless"
        } else {
            "browser shown"
        },
        if request.persistent_profile {
            "persistent"
        } else {
            "throwaway"
        },
        request.accept_consent,
    )
}

/// Every activity of the capture becomes an audit-trail entry. What the
/// visitor did is kept apart from plain progress so the visit can be read
/// back, live and afterwards.
async fn record(run: &mut StepRun<'_>, activity: CaptureActivity) -> Result<(), NodeError> {
    let name = match activity.kind {
        ActivityKind::Progress => {
            return run
                .log(
                    LogLevel::Info,
                    LogKind::Message,
                    activity.message,
                    activity.data,
                )
                .await;
        }
        ActivityKind::PageView => "pageView",
        ActivityKind::Action => "action",
        ActivityKind::LinkChoice => "linkChoice",
        ActivityKind::Blocked => "blocked",
        ActivityKind::Environment => "environment",
    };
    let level = if activity.kind == ActivityKind::Blocked {
        LogLevel::Warn
    } else {
        LogLevel::Info
    };
    let data = json!({ "activity": name, "page": activity.page, "detail": activity.data });
    run.log(level, LogKind::Visitor, activity.message, Some(data))
        .await
}
