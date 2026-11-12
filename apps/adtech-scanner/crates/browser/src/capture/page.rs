//! The browser tab being scanned: session setup, emulation, script injection,
//! input, and the event pump that feeds the recorder.

use std::time::Duration;

use serde_json::{Value, json};
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::time::Instant;

use crate::cdp::{Cdp, CdpError, CdpEvent};
use crate::device::Emulation;

use super::error::CaptureError;
use super::injected::{CONSENT_SOURCE, TRACKER_BINDING, TRACKER_FLUSH_EVENT, TRACKER_SOURCE};
use super::recorder::{ProgressSink, Recorder};
use super::scenario::swipe_lengths;

const TAB_PATIENCE: Duration = Duration::from_secs(10);

/// Where on the screen a swipe starts, as a share of the viewport height.
const SWIPE_START: f64 = 0.8;
const SWIPE_FRAMES: u32 = 12;
const SWIPE_FRAME_INTERVAL: Duration = Duration::from_millis(16);
const SWIPE_REST: Duration = Duration::from_millis(80);

/// Isolated world for the scanner's own evaluations, invisible to page scripts.
const SCANNER_WORLD: &str = "adscan";

#[derive(Debug, Clone, Copy)]
pub(crate) struct ScrollPosition {
    pub top: f64,
    pub viewport_height: f64,
    pub page_height: f64,
}

impl ScrollPosition {
    pub(crate) fn remaining(&self) -> f64 {
        (self.page_height - self.viewport_height - self.top).max(0.0)
    }

    /// 0-1 share of the scrollable height reached after scrolling `distance` more.
    pub(crate) fn depth_after(&self, distance: f64) -> f64 {
        let scrollable = (self.page_height - self.viewport_height).max(1.0);
        ((self.top + distance) / scrollable).clamp(0.0, 1.0)
    }
}

pub(crate) struct Page {
    cdp: Cdp,
    events: UnboundedReceiver<CdpEvent>,
    session_id: String,
    main_frame_id: String,
    /// Sessions of cross-site iframes, where consent banners often live.
    frame_sessions: Vec<String>,
    emulation: Emulation,
    crashed: bool,
    pub recorder: Recorder,
}

impl Page {
    /// Attaches to the tab Chrome opened at launch.
    pub(crate) async fn attach(
        cdp: Cdp,
        events: UnboundedReceiver<CdpEvent>,
        emulation: Emulation,
        on_progress: ProgressSink,
    ) -> Result<Self, CaptureError> {
        let target_id = initial_tab(&cdp).await?;
        let attached = cdp
            .send(
                None,
                "Target.attachToTarget",
                json!({ "targetId": target_id, "flatten": true }),
            )
            .await?;
        let session_id = attached["sessionId"]
            .as_str()
            .ok_or(CaptureError::NoPage)?
            .to_string();
        let frame_tree = cdp
            .send(Some(&session_id), "Page.getFrameTree", json!({}))
            .await?;
        let main_frame_id = frame_tree["frameTree"]["frame"]["id"]
            .as_str()
            .ok_or(CaptureError::NoPage)?
            .to_string();
        Ok(Self {
            cdp,
            events,
            session_id,
            recorder: Recorder::new(main_frame_id.clone(), on_progress),
            main_frame_id,
            frame_sessions: Vec::new(),
            emulation,
            crashed: false,
        })
    }

    /// Turns on recording and emulation and installs the tracker, so the first
    /// navigation is already observed from its first byte.
    pub(crate) async fn prepare(&mut self) -> Result<(), CaptureError> {
        let mut commands = vec![
            ("Page.enable", json!({})),
            ("Runtime.enable", json!({})),
            ("Log.enable", json!({})),
            (
                "Network.enable",
                json!({ "maxTotalBufferSize": 256 * 1024 * 1024, "maxResourceBufferSize": 32 * 1024 * 1024 }),
            ),
            ("Debugger.enable", json!({})),
            // An attached debugger must not stop pages that contain `debugger;`.
            ("Debugger.setSkipAllPauses", json!({ "skip": true })),
            (
                "Target.setAutoAttach",
                json!({ "autoAttach": true, "waitForDebuggerOnStart": true, "flatten": true }),
            ),
            (
                "Emulation.setDeviceMetricsOverride",
                self.emulation.device_metrics(),
            ),
            (
                "Emulation.setTouchEmulationEnabled",
                json!({ "enabled": self.emulation.touch, "maxTouchPoints": 5 }),
            ),
            // A reader's tab has focus; a headless or background window has
            // none, and viewability scripts hold ads back for unfocused pages.
            (
                "Emulation.setFocusEmulationEnabled",
                json!({ "enabled": true }),
            ),
            ("Runtime.addBinding", json!({ "name": TRACKER_BINDING })),
            (
                "Page.addScriptToEvaluateOnNewDocument",
                json!({ "source": TRACKER_SOURCE }),
            ),
        ];
        if let Some(identity) = self.emulation.user_agent_override() {
            commands.push(("Emulation.setUserAgentOverride", identity));
        }
        for (method, params) in commands {
            self.call(method, params).await?;
        }
        Ok(())
    }

    /// Runs a command on the tab, recording page events while it waits.
    pub(crate) async fn call(
        &mut self,
        method: &'static str,
        params: Value,
    ) -> Result<Value, CaptureError> {
        let session_id = self.session_id.clone();
        self.call_in(&session_id, method, params).await
    }

    async fn call_in(
        &mut self,
        session_id: &str,
        method: &'static str,
        params: Value,
    ) -> Result<Value, CaptureError> {
        let response = self.cdp.send(Some(session_id), method, params);
        tokio::pin!(response);
        loop {
            tokio::select! {
                biased;
                result = &mut response => return Ok(result?),
                progressed = self.pump() => progressed?,
            }
        }
    }

    /// Records page events until `deadline`.
    pub(crate) async fn record_until(&mut self, deadline: Instant) -> Result<(), CaptureError> {
        self.record_until_or(deadline, |_| false).await.map(drop)
    }

    /// Records page events until `done` holds or `deadline` passes; returns
    /// whether `done` held.
    pub(crate) async fn record_until_or(
        &mut self,
        deadline: Instant,
        done: impl Fn(&Recorder) -> bool,
    ) -> Result<bool, CaptureError> {
        loop {
            if done(&self.recorder) {
                return Ok(true);
            }
            tokio::select! {
                biased;
                progressed = self.pump() => progressed?,
                _ = tokio::time::sleep_until(deadline) => return Ok(done(&self.recorder)),
            }
        }
    }

    /// Handles one page event or one arrived script body.
    async fn pump(&mut self) -> Result<(), CaptureError> {
        tokio::select! {
            biased;
            event = self.events.recv() => self.handle(event.ok_or(CdpError::Closed)?),
            _ = self.recorder.store_next_body() => {}
        }
        if self.crashed {
            Err(CaptureError::PageCrashed)
        } else {
            Ok(())
        }
    }

    fn handle(&mut self, event: CdpEvent) {
        if event.session_id.as_deref() != Some(self.session_id.as_str()) {
            return;
        }
        match event.method.as_str() {
            "Target.attachedToTarget" => self.adopt_child_target(&event.params),
            "Target.detachedFromTarget" => {
                self.frame_sessions
                    .retain(|session| event.params["sessionId"] != session.as_str());
            }
            // A page that opens alert() would otherwise never finish loading.
            "Page.javascriptDialogOpening" => {
                self.cdp.notify(
                    Some(&self.session_id),
                    "Page.handleJavaScriptDialog",
                    json!({ "accept": true }),
                );
            }
            "Debugger.paused" => {
                self.cdp
                    .notify(Some(&self.session_id), "Debugger.resume", json!({}))
            }
            "Inspector.targetCrashed" => self.crashed = true,
            _ => self.recorder.observe(&event, &self.cdp, &self.session_id),
        }
    }

    /// Cross-site iframes and workers start paused so the emulated identity is
    /// in place before their first request.
    fn adopt_child_target(&mut self, params: &Value) {
        let Some(child) = params["sessionId"].as_str() else {
            return;
        };
        if params["targetInfo"]["type"] == "iframe" {
            if let Some(identity) = self.emulation.user_agent_override() {
                self.cdp
                    .notify(Some(child), "Emulation.setUserAgentOverride", identity);
            }
            self.frame_sessions.push(child.to_string());
        }
        self.cdp
            .notify(Some(child), "Runtime.runIfWaitingForDebugger", json!({}));
    }

    /// Evaluates in the scanner's isolated world of the main frame. `None` when
    /// the page was navigating and had no context to run in.
    pub(crate) async fn evaluate(
        &mut self,
        expression: &str,
    ) -> Result<Option<Value>, CaptureError> {
        let world = self
            .call(
                "Page.createIsolatedWorld",
                json!({ "frameId": self.main_frame_id, "worldName": SCANNER_WORLD }),
            )
            .await;
        let Some(world) = tolerate_rejection(world)? else {
            return Ok(None);
        };
        let session_id = self.session_id.clone();
        self.evaluate_in(&session_id, expression, Some(&world["executionContextId"]))
            .await
    }

    async fn evaluate_in(
        &mut self,
        session_id: &str,
        expression: &str,
        context_id: Option<&Value>,
    ) -> Result<Option<Value>, CaptureError> {
        let mut params =
            json!({ "expression": expression, "returnByValue": true, "awaitPromise": true });
        if let Some(context_id) = context_id {
            params["contextId"] = context_id.clone();
        }
        let evaluated = self.call_in(session_id, "Runtime.evaluate", params).await;
        Ok(tolerate_rejection(evaluated)?.map(|mut result| result["result"]["value"].take()))
    }

    /// Tries to accept a consent banner in the page or one of its iframes.
    /// Returns the name of the banner that was accepted.
    pub(crate) async fn accept_consent(&mut self) -> Result<Option<String>, CaptureError> {
        if let Some(Value::String(banner)) = self.evaluate(CONSENT_SOURCE).await? {
            return Ok(Some(banner));
        }
        for session_id in self.frame_sessions.clone() {
            if let Some(Value::String(banner)) =
                self.evaluate_in(&session_id, CONSENT_SOURCE, None).await?
            {
                return Ok(Some(banner));
            }
        }
        Ok(None)
    }

    pub(crate) async fn scroll_position(&mut self) -> Result<Option<ScrollPosition>, CaptureError> {
        let measured = self
            .evaluate(
                "(() => { const page = document.scrollingElement || document.documentElement; \
                 return [window.scrollY, window.innerHeight, page ? page.scrollHeight : 0]; })()",
            )
            .await?;
        let number = |index: usize| measured.as_ref().and_then(|values| values[index].as_f64());
        Ok(match (number(0), number(1), number(2)) {
            (Some(top), Some(viewport_height), Some(page_height)) => Some(ScrollPosition {
                top,
                viewport_height,
                page_height,
            }),
            _ => None,
        })
    }

    /// Scrolls with real input events (finger swipes on touch devices, the
    /// mouse wheel otherwise) so the page sees what a reader would cause.
    pub(crate) async fn scroll_down(&mut self, distance: f64) -> Result<(), CaptureError> {
        let x = f64::from(self.emulation.viewport.width) / 2.0;
        let viewport_height = f64::from(self.emulation.viewport.height);
        if !self.emulation.touch {
            let wheel = json!({ "type": "mouseWheel", "x": x, "y": viewport_height / 2.0, "deltaX": 0, "deltaY": distance });
            return tolerate_rejection(self.call("Input.dispatchMouseEvent", wheel).await)
                .map(drop);
        }
        for swipe in swipe_lengths(distance, viewport_height) {
            self.swipe_up(x, viewport_height * SWIPE_START, swipe)
                .await?;
        }
        Ok(())
    }

    async fn swipe_up(&mut self, x: f64, from_y: f64, length: f64) -> Result<(), CaptureError> {
        let finger_at = |y: f64| json!([{ "x": x, "y": y }]);
        let touch = |kind: &str, points: Value| json!({ "type": kind, "touchPoints": points });
        tolerate_rejection(
            self.call(
                "Input.dispatchTouchEvent",
                touch("touchStart", finger_at(from_y)),
            )
            .await,
        )?;
        for frame in 1..=SWIPE_FRAMES {
            let y = from_y - length * f64::from(frame) / f64::from(SWIPE_FRAMES);
            tolerate_rejection(
                self.call("Input.dispatchTouchEvent", touch("touchMove", finger_at(y)))
                    .await,
            )?;
            self.record_until(Instant::now() + SWIPE_FRAME_INTERVAL)
                .await?;
        }
        // Resting the finger before lifting it avoids a fling of unknown length.
        self.record_until(Instant::now() + SWIPE_REST).await?;
        tolerate_rejection(
            self.call("Input.dispatchTouchEvent", touch("touchEnd", json!([])))
                .await,
        )
        .map(drop)
    }

    pub(crate) async fn scroll_to_top(&mut self) -> Result<(), CaptureError> {
        self.evaluate("window.scrollTo({ top: 0, behavior: 'smooth' })")
            .await
            .map(drop)
    }

    /// Makes the tracker hand over what it still buffers. Its batches arrive
    /// before the evaluation returns, so they are recorded when this resolves.
    pub(crate) async fn flush_tracker(&mut self) -> Result<(), CaptureError> {
        self.evaluate(&format!(
            "window.dispatchEvent(new Event('{TRACKER_FLUSH_EVENT}'))"
        ))
        .await?;
        while let Ok(event) = self.events.try_recv() {
            self.handle(event);
        }
        Ok(())
    }

    pub(crate) async fn title(&mut self) -> Result<Option<String>, CaptureError> {
        let title = self.evaluate("document.title").await?;
        Ok(title
            .and_then(|title| title.as_str().map(|title| title.trim().to_string()))
            .filter(|title| !title.is_empty()))
    }

    /// Gives script bodies still in flight a last chance to arrive.
    pub(crate) async fn settle_bodies(&mut self, patience: Duration) -> Result<(), CaptureError> {
        let deadline = Instant::now() + patience;
        self.record_until_or(deadline, |recorder| !recorder.has_pending_bodies())
            .await
            .map(drop)
    }

    pub(crate) fn emulation(&self) -> &Emulation {
        &self.emulation
    }
}

/// The tab Chrome opens at launch. The DevTools endpoint can be up a moment
/// before that tab exists.
async fn initial_tab(cdp: &Cdp) -> Result<String, CaptureError> {
    let deadline = Instant::now() + TAB_PATIENCE;
    loop {
        let targets = cdp.send(None, "Target.getTargets", json!({})).await?;
        let tab = targets["targetInfos"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|target| target["type"] == "page")
            .and_then(|target| target["targetId"].as_str());
        match tab {
            Some(target_id) => return Ok(target_id.to_string()),
            None if Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(50)).await
            }
            None => return Err(CaptureError::NoPage),
        }
    }
}

/// The page refusing a command (it navigated away, the frame is gone) is not a
/// failed capture; a dead connection is.
fn tolerate_rejection(result: Result<Value, CaptureError>) -> Result<Option<Value>, CaptureError> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(CaptureError::Protocol(CdpError::Rejected { .. } | CdpError::Timeout(_))) => Ok(None),
        Err(error) => Err(error),
    }
}
