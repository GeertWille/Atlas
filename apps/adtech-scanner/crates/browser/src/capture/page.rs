//! The browser being scanned, as the capture drives it: tab setup, emulation,
//! script injection, evaluation, and the event pump that feeds the recorder.
//! Input is in `input.rs`.

use std::sync::Arc;
use std::time::Duration;

use adscan_model::{ActivityKind, CaptureActivity, VisitorBehavior};
use serde_json::{Value, json};
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::time::Instant;
use url::Url;

use crate::cdp::{Cdp, CdpError, CdpEvent};
use crate::device::Emulation;

use super::error::CaptureError;
use super::injected::{CONSENT_SOURCE, TRACKER_BINDING, TRACKER_FLUSH_EVENT, tracker_script};
use super::interception::{DocumentRequest, Interception, Paused};
use super::recorder::{Recorder, committed_address};
use super::tabs::{NewTab, Tab, Tabs, on_new_tab};
use super::visitor::links::loggable;
use super::visitor::site::Site;
use super::visitor::{Point, Size};

const TAB_PATIENCE: Duration = Duration::from_secs(10);
/// A tab that opens this soon after the visitor's click was opened by it.
const CLICK_OPENS_TAB_WITHIN: Duration = Duration::from_secs(3);

/// Isolated world for the scanner's own evaluations, invisible to page scripts.
const SCANNER_WORLD: &str = "adscan";

/// What every tab of the scan is set up with.
pub(crate) struct TabSetup {
    pub emulation: Emulation,
    /// Force the emulated viewport. Off for a visible desktop browser that is
    /// browsed like a visitor would: its window is the viewport.
    pub override_viewport: bool,
    pub visitor: VisitorBehavior,
}

impl TabSetup {
    /// The commands that turn on recording and emulation and install the
    /// tracker, so that the tab's next document is observed from its first
    /// byte. The tracker script is last.
    fn commands(
        &self,
        interception: &Interception,
        site: Option<&Site>,
    ) -> Vec<(&'static str, Value)> {
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
        ];
        if self.override_viewport {
            commands.push((
                "Emulation.setDeviceMetricsOverride",
                self.emulation.device_metrics(),
            ));
        }
        if let Some(identity) = self.emulation.user_agent_override() {
            commands.push(("Emulation.setUserAgentOverride", identity));
        }
        if let Some(pausing) = interception.fetch_enable() {
            commands.push(("Fetch.enable", pausing));
        }
        commands.push((
            "Page.addScriptToEvaluateOnNewDocument",
            json!({ "source": tracker_script(site) }),
        ));
        commands
    }
}

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
    pub(super) cdp: Arc<Cdp>,
    events: UnboundedReceiver<CdpEvent>,
    tabs: Tabs,
    setup: TabSetup,
    pub recorder: Recorder,
    pub interception: Interception,
    /// Identifier of the tracker script installed in the first tab.
    tracker_script_id: Option<String>,
    /// Where the pointer was last moved to, in viewport coordinates.
    pub(super) pointer: Point,
    expecting_tab_until: Option<Instant>,
    /// A download began since this was last cleared.
    pub download_began: bool,
    crashed: bool,
}

impl Page {
    /// Attaches to the tab Chrome opened at launch and sets it up.
    pub(crate) async fn attach(
        cdp: Arc<Cdp>,
        mut events: UnboundedReceiver<CdpEvent>,
        setup: TabSetup,
        interception: Interception,
        recorder: impl FnOnce(String) -> Recorder,
    ) -> Result<Self, CaptureError> {
        // Every page target, the first tab and any opened later, is attached
        // to and held until it is set up.
        cdp.send(
            None,
            "Target.setAutoAttach",
            json!({
                "autoAttach": true,
                "waitForDebuggerOnStart": true,
                "flatten": true,
                "filter": [{ "type": "page" }],
            }),
        )
        .await?;
        let first = first_tab(&mut events).await?;
        let viewport = setup.emulation.viewport;
        let mut page = Self {
            cdp,
            events,
            recorder: recorder(first.main_frame_id.clone()),
            tabs: Tabs::starting_with(first),
            setup,
            interception,
            tracker_script_id: None,
            pointer: Point {
                x: f64::from(viewport.width) * 0.5,
                y: f64::from(viewport.height) * 0.4,
            },
            expecting_tab_until: None,
            download_began: false,
            crashed: false,
        };
        let commands = page.setup.commands(&page.interception, None);
        let mut installed = Value::Null;
        for (method, params) in commands {
            installed = page.call(method, params).await?;
        }
        page.tracker_script_id = installed["identifier"].as_str().map(str::to_string);
        // The tab may have been created after the browser was told to hold
        // new ones.
        page.call("Runtime.runIfWaitingForDebugger", json!({}))
            .await?;
        Ok(page)
    }

    fn recorded_session(&self) -> Result<String, CaptureError> {
        self.tabs
            .recorded()
            .map(|tab| tab.session_id.clone())
            .ok_or(CaptureError::TabClosed)
    }

    /// Runs a command on the recorded tab, recording page events while it waits.
    pub(crate) async fn call(
        &mut self,
        method: &'static str,
        params: Value,
    ) -> Result<Value, CaptureError> {
        let session_id = self.recorded_session()?;
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

    /// Records page events for `time`.
    pub(crate) async fn record_for(&mut self, time: Duration) -> Result<(), CaptureError> {
        self.record_until(Instant::now() + time).await
    }

    /// Records page events until `done` holds or `deadline` passes; returns
    /// whether `done` held.
    pub(crate) async fn record_until_or(
        &mut self,
        deadline: Instant,
        done: impl Fn(&Self) -> bool,
    ) -> Result<bool, CaptureError> {
        loop {
            if done(self) {
                return Ok(true);
            }
            tokio::select! {
                biased;
                progressed = self.pump() => progressed?,
                _ = tokio::time::sleep_until(deadline) => return Ok(done(self)),
            }
        }
    }

    /// Handles one browser event or one arrived script body.
    async fn pump(&mut self) -> Result<(), CaptureError> {
        tokio::select! {
            biased;
            event = self.events.recv() => self.handle(event.ok_or(CdpError::Closed)?),
            _ = self.recorder.store_next_body() => {}
        }
        if self.crashed {
            Err(CaptureError::PageCrashed)
        } else if self.tabs.recorded().is_none() {
            Err(CaptureError::TabClosed)
        } else {
            Ok(())
        }
    }

    fn handle(&mut self, event: CdpEvent) {
        match event.session_id.clone() {
            None => self.on_browser_event(&event),
            Some(session_id) if self.tabs.is_recorded(&session_id) => {
                self.on_recorded_tab_event(&session_id, &event)
            }
            Some(session_id) if self.tabs.contains(&session_id) => {
                self.on_other_tab_event(&session_id, &event)
            }
            // Iframes and workers: set up when they attached, then left alone.
            Some(_) => {}
        }
    }

    fn on_browser_event(&mut self, event: &CdpEvent) {
        match event.method.as_str() {
            "Target.attachedToTarget" => self.on_tab_opened(&event.params),
            "Target.detachedFromTarget" => {
                self.on_tab_closed(event.params["sessionId"].as_str().unwrap_or_default());
            }
            "Browser.downloadWillBegin" => self.download_began = true,
            _ => {}
        }
    }

    fn on_recorded_tab_event(&mut self, session_id: &str, event: &CdpEvent) {
        match event.method.as_str() {
            "Inspector.targetCrashed" => self.crashed = true,
            "Fetch.requestPaused" => {
                let verdict = self.decide_paused(session_id, &event.params);
                self.answer_paused(session_id, &event.params, verdict);
            }
            "Page.frameNavigated" if event.params["frame"]["parentId"].is_null() => {
                self.note_tab_address(session_id, &event.params);
                self.recorder.observe(event, &self.cdp, session_id);
            }
            _ if self.on_any_tab_event(session_id, event) => {}
            _ => self.recorder.observe(event, &self.cdp, session_id),
        }
    }

    /// A tab that is open but not recorded: one the visitor's click just
    /// opened, or another tab of a person browsing.
    fn on_other_tab_event(&mut self, session_id: &str, event: &CdpEvent) {
        match event.method.as_str() {
            "Fetch.requestPaused" => self.on_first_document_of_new_tab(session_id, &event.params),
            "Page.frameNavigated" if event.params["frame"]["parentId"].is_null() => {
                let url = self.note_tab_address(session_id, &event.params);
                let person_went_there =
                    self.setup.visitor == VisitorBehavior::Manual && self.recorder.is_on_site(&url);
                if person_went_there {
                    self.record_tab(session_id);
                    self.recorder.observe(event, &self.cdp, session_id);
                }
            }
            _ => {
                self.on_any_tab_event(session_id, event);
            }
        }
    }

    /// What is handled the same way in every tab. Returns whether it was.
    fn on_any_tab_event(&mut self, session_id: &str, event: &CdpEvent) -> bool {
        match event.method.as_str() {
            "Target.attachedToTarget" => self.adopt_child_target(session_id, &event.params),
            "Target.detachedFromTarget" => {
                if let Some(tab) = self.tabs.by_session(session_id) {
                    tab.frame_sessions
                        .retain(|session| event.params["sessionId"] != session.as_str());
                }
            }
            "Page.javascriptDialogOpening" => self.on_dialog(session_id, &event.params),
            "Debugger.paused" => self
                .cdp
                .notify(Some(session_id), "Debugger.resume", json!({})),
            "Fetch.authRequired" => {
                let answer = self.interception.on_auth_required(&event.params);
                self.cdp.notify(
                    Some(session_id),
                    "Fetch.continueWithAuth",
                    json!({ "requestId": event.params["requestId"], "authChallengeResponse": answer }),
                );
            }
            "Fetch.requestPaused" => {
                self.answer_paused(session_id, &event.params, Paused::Continue)
            }
            "Page.downloadWillBegin" => self.download_began = true,
            _ => return false,
        }
        true
    }

    /// What a paused document request of a tab is, for the navigation guard.
    fn document_request<'a>(
        &mut self,
        session_id: &str,
        request: &'a Value,
    ) -> DocumentRequest<'a> {
        DocumentRequest {
            url: request["request"]["url"].as_str().unwrap_or_default(),
            method: request["request"]["method"].as_str().unwrap_or("GET"),
            replaces_the_page: self
                .tabs
                .by_session(session_id)
                .is_some_and(|tab| request["frameId"] == tab.main_frame_id.as_str()),
        }
    }

    /// Decides a paused document request of the recorded tab and puts a
    /// refusal into the audit trail.
    fn decide_paused(&mut self, session_id: &str, request: &Value) -> Paused {
        let document = self.document_request(session_id, request);
        let verdict = self
            .interception
            .on_request_paused(&document, self.recorder.site());
        if let Paused::Refuse(refusal) = verdict {
            let host = Url::parse(document.url)
                .ok()
                .and_then(|url| url.host_str().map(str::to_string))
                .unwrap_or_default();
            let (reason, message) = refusal.report(&host);
            self.report_blocked(reason, &message, Some(document.url));
        }
        verdict
    }

    fn answer_paused(&self, session_id: &str, request: &Value, verdict: Paused) {
        let request_id = &request["requestId"];
        match verdict {
            Paused::Continue => self.cdp.notify(
                Some(session_id),
                "Fetch.continueRequest",
                json!({ "requestId": request_id }),
            ),
            // Aborted, like a navigation the reader stopped: the page that
            // is showing stays.
            Paused::Refuse(_) => self.cdp.notify(
                Some(session_id),
                "Fetch.failRequest",
                json!({ "requestId": request_id, "errorReason": "Aborted" }),
            ),
        }
    }

    /// Remembers what a tab shows and returns the address.
    fn note_tab_address(&mut self, session_id: &str, frame_navigated: &Value) -> String {
        let url = committed_address(frame_navigated);
        if let Some(tab) = self.tabs.by_session(session_id) {
            tab.url.clone_from(&url);
        }
        url
    }

    /// A pop-up alert would otherwise keep the page from ever finishing; a
    /// question is answered with "no", because "yes" may be a purchase. A
    /// person browsing answers their dialogs themselves.
    fn on_dialog(&mut self, session_id: &str, dialog: &Value) {
        if self.setup.visitor == VisitorBehavior::Manual {
            return;
        }
        let kind = dialog["type"].as_str().unwrap_or("alert");
        let accept = matches!(kind, "alert" | "beforeunload");
        self.cdp.notify(
            Some(session_id),
            "Page.handleJavaScriptDialog",
            json!({ "accept": accept }),
        );
        if !accept {
            self.recorder.report(CaptureActivity {
                kind: ActivityKind::Blocked,
                message: format!("Dismissed a {kind} dialog of the page"),
                page: self.recorder.pages.current(),
                data: Some(json!({ "reason": "dialog" })),
            });
        }
    }

    /// Cross-site iframes and workers start paused so the emulated identity is
    /// in place before their first request.
    fn adopt_child_target(&mut self, tab_session: &str, params: &Value) {
        let Some(child) = params["sessionId"].as_str() else {
            return;
        };
        if params["targetInfo"]["type"] == "iframe" {
            if let Some(identity) = self.setup.emulation.user_agent_override() {
                self.cdp
                    .notify(Some(child), "Emulation.setUserAgentOverride", identity);
            }
            if let Some(tab) = self.tabs.by_session(tab_session) {
                tab.frame_sessions.push(child.to_string());
            }
        }
        self.cdp
            .notify(Some(child), "Runtime.runIfWaitingForDebugger", json!({}));
    }

    // ------------------------------------------------------------------ tabs

    /// The visitor is about to click: a tab that opens now is the click's.
    pub(crate) fn expect_tab(&mut self) {
        self.expecting_tab_until = Some(Instant::now() + CLICK_OPENS_TAB_WITHIN);
    }

    fn on_tab_opened(&mut self, attached: &Value) {
        let target = &attached["targetInfo"];
        let (Some(session_id), Some(target_id)) =
            (attached["sessionId"].as_str(), target["targetId"].as_str())
        else {
            return;
        };
        let expected = self
            .expecting_tab_until
            .is_some_and(|until| Instant::now() < until);
        match on_new_tab(self.setup.visitor, expected) {
            NewTab::Close => {
                self.close_tab(target_id);
                self.report_blocked("popup", "Closed a tab the page opened", None);
            }
            NewTab::Watch => {
                // Queued, not awaited: a held tab answers nothing until it is
                // let go, which is the last command.
                let commands = self
                    .setup
                    .commands(&self.interception, self.recorder.site());
                for (method, params) in commands {
                    self.cdp.notify(Some(session_id), method, params);
                }
                self.cdp.notify(
                    Some(session_id),
                    "Runtime.runIfWaitingForDebugger",
                    json!({}),
                );
                self.tabs.add(Tab {
                    target_id: target_id.to_string(),
                    session_id: session_id.to_string(),
                    // The main frame of a page target carries the target's id.
                    main_frame_id: target_id.to_string(),
                    frame_sessions: Vec::new(),
                    url: String::new(),
                });
            }
        }
    }

    /// The tab a click of the scripted visitor opened asks for its first
    /// document. When the visit may load it, the visit moves into that tab;
    /// otherwise the tab is closed before the request is sent.
    fn on_first_document_of_new_tab(&mut self, session_id: &str, request: &Value) {
        let document = self.document_request(session_id, request);
        if self.setup.visitor != VisitorBehavior::Browse || !document.replaces_the_page {
            return self.answer_paused(session_id, request, Paused::Continue);
        }
        let Some(target_id) = self
            .tabs
            .by_session(session_id)
            .map(|tab| tab.target_id.clone())
        else {
            return;
        };
        let verdict = self.interception.verdict(&document, self.recorder.site());
        if let Paused::Refuse(_) = verdict {
            self.answer_paused(session_id, request, verdict);
            self.close_tab(&target_id);
            self.report_blocked(
                "popup",
                "Closed a new tab that was leaving the scanned site",
                Some(document.url),
            );
            return;
        }
        let left_behind = self.tabs.recorded().map(|tab| tab.target_id.clone());
        self.record_tab(session_id);
        self.answer_paused(session_id, request, Paused::Continue);
        if let Some(left_behind) = left_behind {
            self.close_tab(&left_behind);
        }
    }

    fn record_tab(&mut self, session_id: &str) {
        if let Some(tab) = self.tabs.record(session_id) {
            self.recorder
                .switch_tab(tab.main_frame_id.clone(), &tab.url);
        }
    }

    fn on_tab_closed(&mut self, session_id: &str) {
        if let Some(successor) = self.tabs.closed(session_id) {
            self.recorder
                .switch_tab(successor.main_frame_id.clone(), &successor.url);
        }
    }

    fn close_tab(&self, target_id: &str) {
        self.cdp
            .notify(None, "Target.closeTarget", json!({ "targetId": target_id }));
    }

    pub(crate) fn report_blocked(&self, reason: &str, message: &str, url: Option<&str>) {
        let mut detail = json!({ "reason": reason });
        if let Some(url) = url.and_then(|url| Url::parse(url).ok()) {
            detail["url"] = json!(loggable(&url));
            detail["host"] = json!(url.host_str());
        }
        self.recorder.report(CaptureActivity {
            kind: ActivityKind::Blocked,
            message: message.to_string(),
            page: self.recorder.pages.current(),
            data: Some(detail),
        });
    }

    // ------------------------------------------------------------ evaluation

    /// The visit has reached its first page: from here on the tracker runs
    /// on that page's site only, and a visitor that follows links cannot
    /// leave it.
    pub(crate) async fn confine_to_site(&mut self) -> Result<(), CaptureError> {
        let Some(site) = self.recorder.site().cloned() else {
            return Ok(());
        };
        self.interception.confine_to_site();
        if let Some(identifier) = self.tracker_script_id.take() {
            self.call(
                "Page.removeScriptToEvaluateOnNewDocument",
                json!({ "identifier": identifier }),
            )
            .await?;
        }
        self.call(
            "Page.addScriptToEvaluateOnNewDocument",
            json!({ "source": tracker_script(Some(&site)) }),
        )
        .await
        .map(drop)
    }

    /// Evaluates in the scanner's isolated world of the main frame. `None` when
    /// the page was navigating and had no context to run in.
    pub(crate) async fn evaluate(
        &mut self,
        expression: &str,
    ) -> Result<Option<Value>, CaptureError> {
        let Some((session_id, main_frame_id)) = self
            .tabs
            .recorded()
            .map(|tab| (tab.session_id.clone(), tab.main_frame_id.clone()))
        else {
            return Err(CaptureError::TabClosed);
        };
        let world = self
            .call_in(
                &session_id,
                "Page.createIsolatedWorld",
                json!({ "frameId": main_frame_id, "worldName": SCANNER_WORLD }),
            )
            .await;
        let Some(world) = tolerate_rejection(world)? else {
            return Ok(None);
        };
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
        let frame_sessions = self
            .tabs
            .recorded()
            .map(|tab| tab.frame_sessions.clone())
            .unwrap_or_default();
        for session_id in frame_sessions {
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

    pub(crate) async fn scroll_to_top(&mut self) -> Result<(), CaptureError> {
        self.evaluate("window.scrollTo({ top: 0, behavior: 'smooth' })")
            .await
            .map(drop)
    }

    /// Makes the tracker hand over what it still buffers. Its batches arrive
    /// before the evaluation returns, so they are recorded when this resolves.
    pub(crate) async fn flush_tracker(&mut self) -> Result<(), CaptureError> {
        // On a page of another site there is no tracker, and nothing of the
        // scanner's is done there.
        if !self.recorder.is_recording() {
            return Ok(());
        }
        self.evaluate(&format!(
            "window.dispatchEvent(new Event('{TRACKER_FLUSH_EVENT}'))"
        ))
        .await?;
        while let Ok(event) = self.events.try_recv() {
            self.handle(event);
        }
        Ok(())
    }

    /// Reads the title of the page view the visit is on, unless it is known.
    pub(crate) async fn note_title(&mut self) -> Result<(), CaptureError> {
        let Some(page) = self.recorder.pages.current() else {
            return Ok(());
        };
        if self.recorder.pages.has_title(page) || !self.recorder.is_recording() {
            return Ok(());
        }
        let title = self.evaluate("document.title").await?;
        let title = title
            .and_then(|title| title.as_str().map(|title| title.trim().to_string()))
            .filter(|title| !title.is_empty());
        // The page may have moved on while its title was being read.
        if let Some(title) = title.filter(|_| self.recorder.pages.current() == Some(page)) {
            self.recorder.pages.set_title(page, title);
        }
        Ok(())
    }

    /// Gives script bodies still in flight a last chance to arrive.
    pub(crate) async fn settle_bodies(&mut self, patience: Duration) -> Result<(), CaptureError> {
        let deadline = Instant::now() + patience;
        self.record_until_or(deadline, |page| !page.recorder.has_pending_bodies())
            .await
            .map(drop)
    }

    pub(crate) fn emulation(&self) -> &Emulation {
        &self.setup.emulation
    }

    pub(crate) fn overrides_viewport(&self) -> bool {
        self.setup.override_viewport
    }

    /// The size input coordinates are valid in, until the page reports its own.
    pub(crate) fn emulated_viewport(&self) -> Size {
        let viewport = self.setup.emulation.viewport;
        Size {
            width: f64::from(viewport.width),
            height: f64::from(viewport.height),
        }
    }

    /// What was recorded, without another word to the browser: for when the
    /// browser is gone.
    pub(crate) fn into_recorder(self) -> Recorder {
        self.recorder
    }
}

/// The tab Chrome opens at launch, as the browser announces it once it is
/// told to attach to every page.
async fn first_tab(events: &mut UnboundedReceiver<CdpEvent>) -> Result<Tab, CaptureError> {
    let deadline = Instant::now() + TAB_PATIENCE;
    loop {
        let event = tokio::time::timeout_at(deadline, events.recv())
            .await
            .map_err(|_| CaptureError::NoPage)?
            .ok_or(CdpError::Closed)?;
        let target = &event.params["targetInfo"];
        let is_tab = event.method == "Target.attachedToTarget"
            && event.session_id.is_none()
            && target["type"] == "page";
        if let (true, Some(session_id), Some(target_id)) = (
            is_tab,
            event.params["sessionId"].as_str(),
            target["targetId"].as_str(),
        ) {
            return Ok(Tab {
                target_id: target_id.to_string(),
                session_id: session_id.to_string(),
                // The main frame of a page target carries the target's id.
                main_frame_id: target_id.to_string(),
                frame_sessions: Vec::new(),
                url: String::new(),
            });
        }
    }
}

/// The page refusing a command (it navigated away, the frame is gone) is not a
/// failed capture; a dead connection is.
pub(super) fn tolerate_rejection(
    result: Result<Value, CaptureError>,
) -> Result<Option<Value>, CaptureError> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(CaptureError::Protocol(CdpError::Rejected { .. } | CdpError::Timeout(_))) => Ok(None),
        Err(error) => Err(error),
    }
}
