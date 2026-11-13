use std::sync::Arc;

use adscan_model::{
    ActivityKind, BrowserbaseCredentials, BrowserbaseRegion, CaptureActivity, CaptureBrowser,
    CaptureProxy, CaptureRequest, LogKind, LogLevel, RemoteSessionEvent, RemoteUsage, ScanEvent,
    VisitorBehavior,
};
use serde_json::json;
use tokio::sync::mpsc;

use crate::application::AppError;
use crate::application::archive::recording::{Recording, RecordingStart};
use crate::application::pipeline::audit::StepRun;
use crate::application::pipeline::state::{NodeError, NodeResult};
use crate::application::ports::{ActivityFn, BrowserbaseAccess, CaptureSession, RemoteSessionFn};
use crate::domain::DomainError;
use crate::domain::scan::{BrowserbaseSessionIds, ResolvedBrowser};

pub async fn launch_browser(run: &mut StepRun<'_>) -> NodeResult {
    if let ResolvedBrowser::Browserbase { region } = run.job.scan.browser {
        // No browser on this machine is needed, or looked for.
        let region = region.as_str();
        let project = match &run.job.settings.browserbase.project_name {
            Some(name) => format!(" (project {name})"),
            None => String::new(),
        };
        run.info(format!("Using Browserbase in {region}{project}"))
            .await?;
        return Ok(format!("Browserbase, {region}"));
    }
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
    let scan = &run.job.scan;
    let remote = match scan.browser {
        ResolvedBrowser::Local => None,
        ResolvedBrowser::Browserbase { region } => {
            let credentials = run
                .job
                .browserbase
                .clone()
                .ok_or(DomainError::BrowserbaseNotSetUp)
                .map_err(AppError::from)?;
            Some((region, credentials))
        }
    };
    if let Some((region, _)) = &remote
        && scan.proxy.system_proxy_dropped
    {
        run.info(format!(
            "The system proxy of this Mac does not apply to a Browserbase browser: the visit leaves from Browserbase's {} region.",
            region.as_str()
        ))
        .await?;
    }

    // Listening from here on: a visit that is finished while it waits for
    // the browser profile ends as soon as its page has loaded, instead of
    // losing the request.
    let finish = run.job.finish.arm();
    // Held until the browser is gone, so the next scan finds the profile free.
    let _profile_turn = if scan.persistent_profile {
        let profile = Arc::clone(match remote {
            Some(_) => &run.job.browserbase_profile,
            None => &run.job.scan_profile,
        });
        if profile.available_permits() == 0 {
            run.info("Waiting for another scan to release the browser profile")
                .await?;
        }
        tokio::select! {
            permit = profile.acquire_owned() => Some(permit.map_err(|_| NodeError::Cancelled)?),
            () = run.job.cancel.cancelled() => return Err(NodeError::Cancelled),
        }
    } else {
        None
    };

    let (session_tx, mut session_rx) = mpsc::unbounded_channel::<RemoteSessionEvent>();
    let (session_ids, browserbase) = match &remote {
        Some((_, credentials)) => {
            let on_session: RemoteSessionFn = Arc::new(move |event| {
                let _ = session_tx.send(event);
            });
            let context_id = if scan.persistent_profile {
                Some(kept_profile_context(run, credentials).await?)
            } else {
                None
            };
            let ids = BrowserbaseSessionIds {
                install_id: run.deps.remote_sessions.install_id().await?,
                scan_id: run.state.summary.id.clone(),
                context_id,
            };
            let access = BrowserbaseAccess {
                credentials: credentials.clone(),
                on_session,
            };
            (Some(ids), Some(access))
        }
        None => (None, None),
    };
    let request = scan.capture_request(&run.job.settings, session_ids);
    let start = start_line(&request, scan.proxy.usage().name);
    run.info(start).await?;

    let (activity_tx, mut activity_rx) = mpsc::unbounded_channel::<CaptureActivity>();
    let on_activity: ActivityFn = Arc::new(move |activity| {
        let _ = activity_tx.send(activity);
    });
    let recording = start_recording(run).await?;
    let session = CaptureSession {
        cancel: run.job.cancel.clone(),
        finish,
        on_activity,
        proxy_password: run.job.proxy_password.clone(),
        network: Some(recording.sink()),
        browserbase,
    };
    let source = Arc::clone(&run.deps.capture);
    let capture = source.capture(request, session);
    tokio::pin!(capture);
    let result = loop {
        tokio::select! {
            result = &mut capture => break result,
            Some(activity) = activity_rx.recv() => record(run, activity).await?,
            Some(event) = session_rx.recv() => remote_session_event(run, event).await?,
        }
    };
    run.job.finish.disarm();
    while let Ok(activity) = activity_rx.try_recv() {
        record(run, activity).await?;
    }
    while let Ok(event) = session_rx.try_recv() {
        remote_session_event(run, event).await?;
    }
    // Whatever became of the browser, nothing is left to show of it.
    run.job.remote_session.ended();
    let capture = match result {
        Ok(capture) => capture,
        Err(error) => {
            recording.abort();
            // Best effort: what stays behind is swept at the next start.
            let _ = run.deps.archive.remove(&run.state.summary.id).await;
            return Err(error.into());
        }
    };
    for script in &capture.scripts {
        recording.store_script(&script.body);
    }
    finish_recording(run, recording).await?;

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

/// The Browserbase context that holds the kept profile of the scan's device,
/// made on first use. The caller holds the profile's turn.
async fn kept_profile_context(
    run: &StepRun<'_>,
    credentials: &BrowserbaseCredentials,
) -> Result<String, NodeError> {
    let device = run.job.scan.options.device;
    let ledger = &run.deps.remote_sessions;
    let now = run.deps.clock.now_ms();
    let known = ledger
        .contexts()
        .await?
        .into_iter()
        .find(|context| context.device == device);
    let context_id = match known {
        Some(context) => context.context_id,
        None => {
            let context_id = run
                .deps
                .browserbase
                .create_context(credentials)
                .await
                .map_err(AppError::from)?;
            ledger.save_context(device, &context_id, now).await?;
            context_id
        }
    };
    ledger.context_used(device, now).await?;
    Ok(context_id)
}

/// What the capture says about its Browserbase session: kept on record so
/// that a session the app does not see end can be released at the next
/// start, and booked as the scan's usage when it has ended.
async fn remote_session_event(
    run: &mut StepRun<'_>,
    event: RemoteSessionEvent,
) -> Result<(), NodeError> {
    let now = run.deps.clock.now_ms();
    match event {
        RemoteSessionEvent::Created { session_id } => {
            run.deps
                .remote_sessions
                .session_created(&session_id, now)
                .await?;
            run.job.remote_session.started(&session_id);
        }
        RemoteSessionEvent::LiveViewReady { session_id } => {
            let Some(credentials) = &run.job.browserbase else {
                return Ok(());
            };
            match run
                .deps
                .browserbase
                .open_live_view(credentials, &session_id)
                .await
            {
                Ok(()) => {
                    run.info(
                        "Opened Browserbase's live view in your browser: browse there, then finish the visit here",
                    )
                    .await?;
                }
                Err(error) => {
                    run.warn(format!(
                        "Browserbase's live view could not be opened ({error}). Use \"Open live view\" to try again."
                    ))
                    .await?;
                }
            }
        }
        RemoteSessionEvent::Released {
            session_id,
            session_seconds,
            proxy_bytes,
        } => {
            run.deps
                .remote_sessions
                .session_released(&session_id, now)
                .await?;
            run.job.remote_session.ended();
            run.state.remote_session_ended = Some((session_id, now));
            if let ResolvedBrowser::Browserbase { region } = run.job.scan.browser {
                book_remote_usage(run, region, session_seconds, proxy_bytes).await?;
            }
        }
    }
    Ok(())
}

async fn book_remote_usage(
    run: &mut StepRun<'_>,
    region: BrowserbaseRegion,
    session_seconds: Option<f64>,
    proxy_bytes: Option<f64>,
) -> Result<(), NodeError> {
    run.state.summary.remote = Some(RemoteUsage {
        region,
        session_seconds,
        proxy_bytes,
    });
    run.deps.scans.save_summary(&run.state.summary).await?;
    run.deps.events.emit(ScanEvent::ScanUpdated {
        summary: run.state.summary.clone(),
    });
    Ok(())
}

/// Every capture is recorded for the analysis of its bid and ad-server
/// calls; with "keep network data" on, the recording is also the scan's
/// archive.
async fn start_recording(run: &mut StepRun<'_>) -> Result<Recording, NodeError> {
    let settings = &run.job.settings;
    let start = RecordingStart {
        scan_id: run.state.summary.id.clone(),
        url: run.state.summary.url.clone(),
        scan_host: run.state.summary.hostname.clone(),
        started_at: run.state.summary.started_at,
        full_headers: settings.store_full_headers,
    };
    let unstored =
        |reason: String| AppError::Internal(format!("the capture could not be recorded: {reason}"));
    if !settings.keep_network_data {
        run.deps
            .archive
            .index
            .begin(
                &run.state.summary.id,
                None,
                false,
                run.state.summary.started_at,
            )
            .await?;
        return Ok(Recording::start(None, start).await.map_err(unstored)?);
    }
    match Recording::start(Some(&run.deps.archive), start.clone()).await {
        Ok(recording) => {
            run.state.archived = true;
            Ok(recording)
        }
        Err(error) => {
            // A scan is worth more than its archive: go on without one.
            run.warn(format!("Network data is not stored for this scan: {error}"))
                .await?;
            let _ = run.deps.archive.remove(&run.state.summary.id).await;
            Ok(Recording::start(None, start).await.map_err(unstored)?)
        }
    }
}

async fn finish_recording(run: &mut StepRun<'_>, recording: Recording) -> Result<(), NodeError> {
    let scan_id = run.state.summary.id.clone();
    let recorded = recording.finish().await.map_err(|error| {
        AppError::Internal(format!("the capture could not be recorded: {error}"))
    })?;
    run.state.network = recorded.analysis;
    match recorded.stored {
        None => {}
        Some(Ok(totals)) => {
            let record = run.deps.archive.index.record(&scan_id).await?;
            let (requests, bodies) =
                record.map_or((0, 0), |record| (record.request_count, record.body_count));
            run.info(format!(
                "Stored {requests} requests and {bodies} bodies in the scan's archive ({} KB){}",
                totals.pack_bytes / 1024,
                if totals.truncated {
                    "; some bodies were left out because the scan reached its size cap"
                } else {
                    ""
                },
            ))
            .await?;
            let evicted = run
                .deps
                .archive
                .enforce_limit(run.job.settings.archive_limit_bytes, Some(&scan_id))
                .await?;
            if evicted > 0 {
                run.info(format!(
                    "Removed the network data of {evicted} older scan{} to stay under the storage limit",
                    if evicted == 1 { "" } else { "s" }
                ))
                .await?;
            }
        }
        Some(Err(error)) => {
            run.state.archived = false;
            run.warn(format!(
                "Network data could not be stored for this scan: {error}"
            ))
            .await?;
            let _ = run.deps.archive.remove(&scan_id).await;
        }
    }
    Ok(())
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
    let named = |host: &str| match &proxy_name {
        Some(name) => format!("proxy \"{name}\" ({host})"),
        None => format!("proxy {host}"),
    };
    let (browser, proxy) = match &request.browser {
        CaptureBrowser::Local => (
            if request.headless {
                "headless".to_string()
            } else {
                "browser shown".to_string()
            },
            match &request.proxy {
                CaptureProxy::None => "no proxy".to_string(),
                CaptureProxy::System => "system proxy".to_string(),
                CaptureProxy::Custom { host, .. } => named(host),
            },
        ),
        CaptureBrowser::Browserbase(cloud) => (
            format!("Browserbase browser in {}", cloud.region.as_str()),
            match (&cloud.location, &request.proxy) {
                (Some(_), _) => format!(
                    "Browserbase residential exit, asked for {}",
                    proxy_name.as_deref().unwrap_or("no place")
                ),
                (None, CaptureProxy::Custom { host, .. }) => {
                    format!("{}, given to Browserbase", named(host))
                }
                (None, CaptureProxy::None | CaptureProxy::System) => {
                    "no proxy: leaves from the cloud region".to_string()
                }
            },
        ),
    };
    format!(
        "Loading {} as {:?}: {visit} (seed {}, {browser}, {} profile, accept consent {}, {proxy})",
        request.url,
        request.device,
        request.seed,
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
        ActivityKind::RemoteBrowser => "browser",
    };
    // What went wrong with the remote browser, among what it merely did.
    let remote_trouble = activity.kind == ActivityKind::RemoteBrowser
        && activity.data.as_ref().is_some_and(|detail| {
            matches!(
                detail["event"].as_str(),
                Some("connectionLost" | "releaseUnconfirmed")
            )
        });
    let level = if activity.kind == ActivityKind::Blocked || remote_trouble {
        LogLevel::Warn
    } else {
        LogLevel::Info
    };
    let data = json!({ "activity": name, "page": activity.page, "detail": activity.data });
    run.log(level, LogKind::Visitor, activity.message, Some(data))
        .await
}
