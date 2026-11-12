//! One page capture, start to finish: launch, emulate, navigate, observe, collect.

mod bid_endpoints;
mod console;
mod error;
mod injected;
mod navigation;
mod page;
mod progress;
mod recorder;
mod scenario;
mod scripts;
mod tracker_log;
mod visitor;

use std::path::Path;
use std::time::Duration;

use adscan_model::{
    Capture, CaptureEnvironment, CaptureRequest, CapturedPage, DeviceProfile, VisitorBehavior,
};
use serde_json::json;
use tokio::time::Instant;
use url::Url;

use crate::cdp::{Chrome, LaunchOptions};
use crate::device::{emulation_for, window_size};
use crate::{BrowserError, CaptureHooks, detect};

use error::CaptureError;
use navigation::{NavigationFailure, host_of, http_failure, network_failure, parse_address};
use page::Page;
use scenario::{
    ScrollAction, ScrollStep, consent_attempt_due, due_step, observation_window, scroll_plan,
    step_distance,
};

const NAVIGATION_TIMEOUT: Duration = Duration::from_secs(30);
/// Ad-heavy pages often never fire `load`; past this point the scan goes on anyway.
const LOAD_TIMEOUT: Duration = Duration::from_secs(15);
const OBSERVATION_TICK: Duration = Duration::from_millis(100);
const BODY_PATIENCE: Duration = Duration::from_secs(3);

pub(crate) async fn run(
    request: CaptureRequest,
    hooks: CaptureHooks,
) -> Result<Capture, BrowserError> {
    let address = parse_address(&request.url).map_err(CaptureError::from)?;
    // Detection runs the browser for its version, which blocks.
    let configured_path = request.chrome_path.clone();
    let browser =
        tokio::task::spawn_blocking(move || detect::detect_browser(configured_path.as_deref()))
            .await
            .ok()
            .flatten()
            .ok_or(BrowserError::NotFound)?;
    let mode = if request.headless {
        "headless"
    } else {
        "visible"
    };
    (hooks.on_progress)(format!("Launching {} ({mode})", browser.version));

    let mut chrome = Chrome::spawn(LaunchOptions {
        executable: Path::new(&browser.path),
        headless: request.headless,
        window_size: window_size(request.device),
    })
    .map_err(CaptureError::from)?;
    let outcome = tokio::select! {
        biased;
        _ = hooks.cancel.cancelled() => Err(CaptureError::Cancelled),
        outcome = scan(&mut chrome, &request, &address, &hooks) => outcome,
    };
    chrome.shutdown().await;
    Ok(outcome?)
}

async fn scan(
    chrome: &mut Chrome,
    request: &CaptureRequest,
    address: &Url,
    hooks: &CaptureHooks,
) -> Result<Capture, CaptureError> {
    let (cdp, events) = crate::cdp::connect(chrome).await?;
    // A scanned page must not be able to put files in the user's Downloads.
    cdp.send(
        None,
        "Browser.setDownloadBehavior",
        json!({ "behavior": "deny" }),
    )
    .await?;
    let version = cdp.send(None, "Browser.getVersion", json!({})).await?;
    let emulation = emulation_for(
        request.device,
        version["userAgent"].as_str().unwrap_or_default(),
        version["product"].as_str().unwrap_or_default(),
    );
    let mut page = Page::attach(cdp, events, emulation, hooks.on_progress.clone()).await?;
    page.prepare().await?;

    (hooks.on_progress)(format!("Opening {address} as {}", page.emulation().label));
    open(&mut page, address, hooks).await?;
    let consent_accepted = observe(&mut page, request, hooks).await?;
    collect(page, request.device, consent_accepted, hooks).await
}

/// Navigates and waits for the load event. A page that never finishes loading
/// is still scanned; a page that cannot be reached is an error.
async fn open(page: &mut Page, address: &Url, hooks: &CaptureHooks) -> Result<(), CaptureError> {
    let host = host_of(address);
    let started = Instant::now();
    let navigation = tokio::time::timeout(
        NAVIGATION_TIMEOUT,
        page.call("Page.navigate", json!({ "url": address.as_str() })),
    )
    .await
    .map_err(|_| NavigationFailure::Timeout {
        host: host.clone(),
        seconds: NAVIGATION_TIMEOUT.as_secs(),
    })??;
    if let Some(error_text) = navigation["errorText"]
        .as_str()
        .filter(|text| !text.is_empty())
    {
        return Err(network_failure(error_text, &host).into());
    }

    let loader_id = navigation["loaderId"].as_str().unwrap_or_default();
    let loaded = page
        .record_until_or(started + LOAD_TIMEOUT, |recorder| recorder.load_fired)
        .await?;
    let status = page.recorder.document_status(loader_id);
    if let Some(failure) = status.and_then(|status| http_failure(status, &host)) {
        return Err(failure.into());
    }
    (hooks.on_progress)(if loaded {
        format!("Page loaded in {:.1}s", started.elapsed().as_secs_f64())
    } else {
        format!(
            "The page is still loading after {} seconds; scanning what has loaded",
            LOAD_TIMEOUT.as_secs()
        )
    });
    Ok(())
}

/// Watches the loaded page for the requested time, accepting the consent
/// banner and scrolling as planned. Returns whether consent was accepted.
async fn observe(
    page: &mut Page,
    request: &CaptureRequest,
    hooks: &CaptureHooks,
) -> Result<bool, CaptureError> {
    let window = observation_window(request.observe_seconds);
    let plan = if request.visitor == VisitorBehavior::Scroll {
        scroll_plan(window)
    } else {
        Vec::new()
    };
    (hooks.on_progress)(format!(
        "Observing the page for {} seconds",
        window.as_secs()
    ));

    let loaded_at = Instant::now();
    let mut window_started = loaded_at;
    let mut next_step = 0;
    let mut consent_attempts = 0;
    let mut consent_accepted = false;
    while window_started.elapsed() < window {
        let wants_consent = request.accept_consent && !consent_accepted;
        if wants_consent && consent_attempt_due(loaded_at.elapsed(), consent_attempts) {
            consent_attempts += 1;
            if let Some(banner) = page.accept_consent().await? {
                (hooks.on_progress)(format!("Accepted the consent banner ({banner})"));
                consent_accepted = true;
                // Bidding only starts once consent is given: observe from here.
                window_started = Instant::now();
                next_step = 0;
            }
            continue;
        }
        match due_step(&plan, next_step, window_started.elapsed()) {
            Some(due) => {
                next_step = due + 1;
                perform(page, &plan[due], hooks).await?;
            }
            None => {
                let window_ends = window_started + window;
                page.record_until(window_ends.min(Instant::now() + OBSERVATION_TICK))
                    .await?;
            }
        }
    }
    Ok(consent_accepted)
}

async fn perform(
    page: &mut Page,
    step: &ScrollStep,
    hooks: &CaptureHooks,
) -> Result<(), CaptureError> {
    match step.action {
        ScrollAction::Down { stride, steps_left } => {
            let Some(position) = page.scroll_position().await? else {
                return Ok(());
            };
            if position.remaining() < 1.0 {
                return Ok(());
            }
            let distance = step_distance(
                position.viewport_height,
                position.remaining(),
                steps_left,
                stride,
            )
            .min(position.remaining());
            page.scroll_down(distance).await?;
            (hooks.on_progress)(format!(
                "Scrolling ({:.0}%)",
                position.depth_after(distance) * 100.0
            ));
        }
        ScrollAction::BackToTop => {
            page.scroll_to_top().await?;
            (hooks.on_progress)("Scrolling back to the top".into());
        }
    }
    Ok(())
}

async fn collect(
    mut page: Page,
    device: DeviceProfile,
    consent_accepted: bool,
    hooks: &CaptureHooks,
) -> Result<Capture, CaptureError> {
    page.flush_tracker().await?;
    let page_title = page.title().await?;
    page.settle_bodies(BODY_PATIENCE).await?;

    let emulation = page.emulation().clone();
    let recorder = page.recorder;
    let navigation_start = recorder.navigation_start();
    let page_load_ms = recorder.tracker.page_load_ms();
    let max_scroll_depth = recorder.tracker.max_scroll_depth();
    let events = recorder.tracker.into_events();
    let entry_page = CapturedPage {
        title: page_title.clone(),
        ended_at_ms: events.iter().map(|event| event.t).fold(0.0, f64::max),
        load_ms: page_load_ms,
        max_scroll_depth,
        ..CapturedPage::entry(recorder.document_url.clone())
    };
    let capture = Capture {
        final_url: recorder.document_url,
        page_title,
        device,
        viewport: emulation.viewport,
        user_agent: emulation.user_agent,
        navigation_start,
        page_load_ms,
        max_scroll_depth,
        scripts: recorder.scripts.into_captured(navigation_start),
        console: recorder.console.into_lines(navigation_start),
        events,
        consent_accepted,
        bid_responses: recorder.bid_responses.into_observed(navigation_start),
        pages: vec![entry_page],
        environment: CaptureEnvironment::default(),
    };
    (hooks.on_progress)(format!(
        "Captured {} scripts, {} tracker events and {} console lines",
        capture.scripts.len(),
        capture.events.len(),
        capture.console.len()
    ));
    Ok(capture)
}
