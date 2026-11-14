//! The browser being scanned, as the capture drives it: tab setup, emulation,
//! script injection, evaluation, and the event pump that feeds the recorder.
//! Input is in `input.rs`.

use std::sync::Arc;
use std::time::Duration;

use adscan_model::{ActivityKind, CaptureActivity, MAX_POST_DATA_BYTES, VisitorBehavior};
use serde_json::{Value, json};
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::time::Instant;
use url::Url;

use crate::cdp::{Cdp, CdpError, CdpEvent};
use crate::device::Emulation;

use super::error::CaptureError;
use super::injected::{TRACKER_BINDING, TRACKER_FLUSH_EVENT, tracker_script};
use super::interception::{DocumentRequest, Interception, Paused};
use super::network::assembly::Reporter;
use super::network::targets::ChildTargets;
use super::noise::ServiceNoise;
use super::patience::Patience;
use super::recorder::{Recorder, Reported, committed_address};
use super::tabs::{NewTab, Tab, Tabs, on_new_tab};
use super::visitor::links::loggable;
use super::visitor::site::Site;
use super::visitor::{Point, Size};

const TAB_PATIENCE: Duration = Duration::from_secs(10);

/// Isolated world for the scanner's own evaluations, invisible to page scripts.
pub(super) const SCANNER_WORLD: &str = "adscan";

/// How much of what a tab loaded the browser keeps for the scanner to read.
const TAB_BODY_BUFFER_BYTES: u64 = 256 * 1024 * 1024;
/// The same for each iframe and worker below it: bodies are read the moment
/// they are complete, and there can be dozens of such targets.
const CHILD_BODY_BUFFER_BYTES: u64 = 64 * 1024 * 1024;

/// Parameters of `Network.enable`. When the visit's requests are kept,
/// request bodies up to the size that is kept come with the request itself.
fn network_recording(body_buffer_bytes: u64, keeps_requests: bool) -> Value {
    let mut recording = json!({
        "maxTotalBufferSize": body_buffer_bytes,
        "maxResourceBufferSize": 32 * 1024 * 1024,
    });
    if keeps_requests {
        recording["maxPostDataSize"] = json!(MAX_POST_DATA_BYTES);
    }
    recording
}

/// Parameters of `Target.setAutoAttach` for a session: what starts below it
/// (an iframe in a process of its own, a worker) is attached to and held
/// until it is set up.
fn attach_to_what_starts_below() -> Value {
    json!({ "autoAttach": true, "waitForDebuggerOnStart": true, "flatten": true })
}

/// What every tab of the scan is set up with.
pub(crate) struct TabSetup {
    pub emulation: Emulation,
    /// Force the emulated viewport. Off for a visible desktop browser that is
    /// browsed like a visitor would: its window is the viewport.
    pub override_viewport: bool,
    pub visitor: VisitorBehavior,
    /// The browser runs at Browserbase, which keeps a client of its own on
    /// every page: what that client does there is not the page's doing.
    pub hosted_by_service: bool,
    /// The timezone the scan gives the browser, once it has one to give.
    pub timezone: Option<&'static str>,
}

impl TabSetup {
    /// The commands that turn on recording and emulation and install the
    /// tracker, so that the tab's next document is observed from its first
    /// byte. The tracker script is last.
    fn commands(
        &self,
        interception: &Interception,
        site: Option<&Site>,
        keeps_requests: bool,
    ) -> Vec<(&'static str, Value)> {
        let mut commands = vec![
            ("Page.enable", json!({})),
            ("Runtime.enable", json!({})),
            ("Log.enable", json!({})),
            (
                "Network.enable",
                network_recording(TAB_BODY_BUFFER_BYTES, keeps_requests),
            ),
            ("Debugger.enable", json!({})),
            // An attached debugger must not stop pages that contain `debugger;`.
            ("Debugger.setSkipAllPauses", json!({ "skip": true })),
            ("Target.setAutoAttach", attach_to_what_starts_below()),
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
        if let Some(timezone) = self.timezone {
            commands.push((
                "Emulation.setTimezoneOverride",
                json!({ "timezoneId": timezone }),
            ));
        }
        // A tab set up before the site is known carries the unconfined
        // tracker: its documents are paused whatever the visit needs.
        if site.is_none() || interception.pauses_after_confinement() {
            commands.push(("Fetch.enable", interception.fetch_enable()));
        }
        commands.push((
            "Page.addScriptToEvaluateOnNewDocument",
            json!({ "source": tracker_script(site) }),
        ));
        commands
    }
}

/// Chrome sends a command again when the tab moves to another process while
/// the command is on its way (the page went to another site just then). The
/// first run removed the script; the second finds none and says so. Either
/// way the script is gone, which is what was asked for.
fn was_replayed_removal(error: &CdpError) -> bool {
    matches!(
        error,
        CdpError::Rejected { method: "Page.removeScriptToEvaluateOnNewDocument", message }
            if message == "Script not found"
    )
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
    pub(super) tabs: Tabs,
    /// The iframes and workers below the tabs that have a session of their own.
    children: ChildTargets,
    setup: TabSetup,
    pub recorder: Recorder,
    pub interception: Interception,
    /// Tells what the service that hosts the browser does in a page from
    /// what the page does. `None` for a browser on this machine.
    noise: Option<ServiceNoise>,
    /// How long things that take a round trip to the browser are waited for.
    pub(super) patience: Patience,
    /// Identifier of the tracker script installed in the first tab.
    tracker_script_id: Option<String>,
    /// Network id of a navigation of the recorded tab that was let through
    /// while the tracker was unconfined and has neither committed nor failed.
    entry_navigation: Option<String>,
    /// The tracker of the first tab runs on the scanned site only.
    tracker_is_confined: bool,
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
        // to and held until it is set up. So is a shared worker, which
        // belongs to no tab.
        cdp.send(
            None,
            "Target.setAutoAttach",
            json!({
                "autoAttach": true,
                "waitForDebuggerOnStart": true,
                "flatten": true,
                "filter": [{ "type": "page" }, { "type": "shared_worker" }],
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
            children: ChildTargets::default(),
            noise: setup.hosted_by_service.then(ServiceNoise::default),
            patience: Patience::LOCAL,
            setup,
            interception,
            tracker_script_id: None,
            entry_navigation: None,
            tracker_is_confined: false,
            pointer: Point {
                x: f64::from(viewport.width) * 0.5,
                y: f64::from(viewport.height) * 0.4,
            },
            expecting_tab_until: None,
            download_began: false,
            crashed: false,
        };
        let commands =
            page.setup
                .commands(&page.interception, None, page.recorder.keeps_requests());
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

    pub(super) async fn call_in(
        &mut self,
        session_id: &str,
        method: &'static str,
        params: Value,
    ) -> Result<Value, CaptureError> {
        let response = self.cdp.send(Some(session_id), method, params);
        self.record_until_answered(response).await
    }

    /// Waits for the answer to a command that was already sent, recording
    /// page events meanwhile.
    pub(super) async fn record_until_answered(
        &mut self,
        response: impl Future<Output = Result<Value, CdpError>>,
    ) -> Result<Value, CaptureError> {
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
            Some(session_id) if self.is_service_noise(&session_id, &event) => {}
            Some(session_id) if self.tabs.is_recorded(&session_id) => {
                self.on_recorded_tab_event(&session_id, &event)
            }
            Some(session_id) if self.tabs.contains(&session_id) => {
                self.on_other_tab_event(&session_id, &event)
            }
            Some(session_id) => self.on_child_event(&session_id, &event),
        }
    }

    /// Whether an event of a tab is the doing of the service that hosts the
    /// browser, not of the page.
    fn is_service_noise(&mut self, session_id: &str, event: &CdpEvent) -> bool {
        match &mut self.noise {
            Some(noise) if self.tabs.contains(session_id) => {
                noise.is_noise(session_id, &event.method, &event.params)
            }
            _ => false,
        }
    }

    /// An event of an iframe or worker below a tab, or of a shared worker.
    fn on_child_event(&mut self, session_id: &str, event: &CdpEvent) {
        match event.method.as_str() {
            "Target.attachedToTarget" => self.adopt_child_target(Some(session_id), &event.params),
            "Target.detachedFromTarget" => {
                self.forget_child_target(event.params["sessionId"].as_str().unwrap_or_default())
            }
            method if method.starts_with("Network.") => {
                let Some(child) = self.children.get(session_id) else {
                    return;
                };
                let reported = Reported {
                    session: session_id,
                    reporter: child.reporter,
                    target_id: Some(&child.target_id),
                    main_frame_id: None,
                    in_recorded_tab: child
                        .tab_session
                        .as_deref()
                        .is_none_or(|tab| self.tabs.is_recorded(tab)),
                };
                self.recorder.observe_network(&reported, event, &self.cdp);
            }
            _ => {}
        }
    }

    fn on_browser_event(&mut self, event: &CdpEvent) {
        match event.method.as_str() {
            "Target.attachedToTarget" if event.params["targetInfo"]["type"] == "page" => {
                self.on_tab_opened(&event.params)
            }
            "Target.attachedToTarget" => self.adopt_child_target(None, &event.params),
            "Target.detachedFromTarget" => {
                let session_id = event.params["sessionId"].as_str().unwrap_or_default();
                self.forget_child_target(session_id);
                self.on_tab_closed(session_id);
            }
            "Browser.downloadWillBegin" => self.download_began = true,
            _ => {}
        }
    }

    fn on_recorded_tab_event(&mut self, session_id: &str, event: &CdpEvent) {
        match event.method.as_str() {
            "Inspector.targetCrashed" => self.crashed = true,
            "Fetch.requestPaused" => {
                if self.would_carry_the_unconfined_tracker(session_id, &event.params) {
                    self.confine_tracker(session_id);
                }
                let verdict = self.decide_paused(session_id, &event.params);
                self.answer_paused(session_id, &event.params, verdict);
            }
            "Network.loadingFailed" | "Network.loadingFinished"
                if self.entry_navigation.is_some()
                    && event.params["requestId"].as_str() == self.entry_navigation.as_deref() =>
            {
                self.entry_navigation = None;
                self.recorder.observe(event, &self.cdp, session_id);
            }
            "Page.frameNavigated" if event.params["frame"]["parentId"].is_null() => {
                self.entry_navigation = None;
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
            // The document this tab loads opens a page view of the visit if
            // the visit moves here; until then nothing else of it counts.
            method if method.starts_with("Network.") => {
                let Some(main_frame_id) = self
                    .tabs
                    .by_session(session_id)
                    .map(|tab| tab.main_frame_id.clone())
                else {
                    return;
                };
                let reported = Reported {
                    session: session_id,
                    reporter: Reporter::Tab,
                    target_id: None,
                    main_frame_id: Some(&main_frame_id),
                    in_recorded_tab: false,
                };
                self.recorder.observe_network(&reported, event, &self.cdp);
            }
            _ => {
                self.on_any_tab_event(session_id, event);
            }
        }
    }

    /// What is handled the same way in every tab. Returns whether it was.
    fn on_any_tab_event(&mut self, session_id: &str, event: &CdpEvent) -> bool {
        match event.method.as_str() {
            "Target.attachedToTarget" => self.adopt_child_target(Some(session_id), &event.params),
            "Target.detachedFromTarget" => {
                let child = event.params["sessionId"].as_str().unwrap_or_default();
                self.forget_child_target(child);
                if let Some(tab) = self.tabs.by_session(session_id) {
                    tab.frame_sessions.retain(|session| session != child);
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

    /// Whether a paused document request of the recorded tab could put the
    /// unconfined tracker on another site. Until the entry page has settled,
    /// a navigation is a redirect that decides which site the visit is on,
    /// and takes the unconfined tracker along. From then on the site is
    /// fixed.
    fn would_carry_the_unconfined_tracker(&mut self, session_id: &str, request: &Value) -> bool {
        let replaces_the_page = self.document_request(session_id, request).replaces_the_page;
        if self.tracker_is_confined || !replaces_the_page {
            return false;
        }
        let site_is_fixed = self.recorder.site().is_some() && !self.recorder.pages.is_opening();
        if !site_is_fixed {
            self.entry_navigation = request["networkId"].as_str().map(str::to_string);
        }
        site_is_fixed
    }

    /// A navigation that was let through before the entry page settled is
    /// still on its way: where it ends decides which site the visit is on.
    pub(crate) fn entry_navigation_is_pending(&self) -> bool {
        self.entry_navigation.is_some()
    }

    /// Confines the tracker while a navigation is paused, without waiting
    /// for the answers: Chrome keeps commands back while a tab moves to
    /// another site's process and runs them there before the new document's
    /// first script, so the answers come only once the navigation is let go.
    fn confine_tracker(&mut self, session_id: &str) {
        let Some(site) = self.recorder.site() else {
            return;
        };
        let confined = json!({ "source": tracker_script(Some(site)) });
        self.interception.confine_to_site();
        if let Some(identifier) = self.tracker_script_id.take() {
            self.cdp.notify(
                Some(session_id),
                "Page.removeScriptToEvaluateOnNewDocument",
                json!({ "identifier": identifier }),
            );
        }
        self.cdp.notify(
            Some(session_id),
            "Page.addScriptToEvaluateOnNewDocument",
            confined,
        );
        self.tracker_is_confined = true;
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

    /// The browser detached a session: its target is gone.
    fn forget_child_target(&mut self, session_id: &str) {
        self.children.detached(session_id);
        self.recorder.target_gone(session_id);
    }

    /// Cross-site iframes and workers start paused, so that the emulated
    /// identity and the recording of their requests are in place before
    /// their first request. `parent_session` is the session that reported
    /// the target: a tab's, another child's, or none for a shared worker.
    fn adopt_child_target(&mut self, parent_session: Option<&str>, attached: &Value) {
        let Some(child) = attached["sessionId"].as_str() else {
            return;
        };
        let parent_is_tab = parent_session.is_some_and(|parent| self.tabs.contains(parent));
        let adopted = self
            .children
            .attached(parent_session, parent_is_tab, attached);
        if let Some((_, reporter)) = adopted {
            if reporter == Reporter::Iframe {
                if let Some(identity) = self.setup.emulation.user_agent_override() {
                    self.cdp
                        .notify(Some(child), "Emulation.setUserAgentOverride", identity);
                }
                if let Some(timezone) = self.setup.timezone {
                    self.cdp.notify(
                        Some(child),
                        "Emulation.setTimezoneOverride",
                        json!({ "timezoneId": timezone }),
                    );
                }
                // Consent dialogs are looked for in the iframes of the tab itself.
                if let Some(tab) = parent_session.and_then(|parent| self.tabs.by_session(parent)) {
                    tab.frame_sessions.push(child.to_string());
                }
            }
            if self.recorder.keeps_requests() {
                self.cdp.notify(
                    Some(child),
                    "Network.enable",
                    network_recording(CHILD_BODY_BUFFER_BYTES, true),
                );
            }
            self.cdp.notify(
                Some(child),
                "Target.setAutoAttach",
                attach_to_what_starts_below(),
            );
        }
        self.cdp
            .notify(Some(child), "Runtime.runIfWaitingForDebugger", json!({}));
    }

    // ------------------------------------------------------------------ tabs

    /// The visitor is about to click: a tab that opens now is the click's.
    pub(crate) fn expect_tab(&mut self) {
        self.expecting_tab_until = Some(Instant::now() + self.patience.click_opens_tab);
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
                let commands = self.setup.commands(
                    &self.interception,
                    self.recorder.site(),
                    self.recorder.keeps_requests(),
                );
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
        for child in self.children.tab_closed(session_id) {
            self.recorder.target_gone(&child);
        }
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
    /// leave it. A page that navigated since it settled had its tracker
    /// confined as that navigation was paused.
    pub(crate) async fn confine_to_site(&mut self) -> Result<(), CaptureError> {
        let Some(site) = self.recorder.site().cloned() else {
            return Ok(());
        };
        self.interception.confine_to_site();
        if !self.tracker_is_confined {
            if let Some(identifier) = self.tracker_script_id.take() {
                let removed = self
                    .call(
                        "Page.removeScriptToEvaluateOnNewDocument",
                        json!({ "identifier": identifier }),
                    )
                    .await;
                match removed {
                    Err(CaptureError::Protocol(error)) if was_replayed_removal(&error) => {}
                    other => drop(other?),
                }
            }
            // A navigation paused while the removal was awaited confined
            // the tracker already.
            if !self.tracker_is_confined {
                self.call(
                    "Page.addScriptToEvaluateOnNewDocument",
                    json!({ "source": tracker_script(Some(&site)) }),
                )
                .await?;
                self.tracker_is_confined = true;
            }
        }
        if !self.interception.pauses_after_confinement() {
            self.call("Fetch.enable", Interception::fetch_pause_nothing())
                .await?;
        }
        Ok(())
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
        self.evaluate_in_frame(&session_id, &main_frame_id, expression)
            .await
    }

    /// Evaluates in the scanner's isolated world of one frame, reached
    /// through the session that frame belongs to. `None` when the frame was
    /// navigating or is gone.
    pub(super) async fn evaluate_in_frame(
        &mut self,
        session_id: &str,
        frame_id: &str,
        expression: &str,
    ) -> Result<Option<Value>, CaptureError> {
        let Some(evaluation) = self
            .evaluation_in_frame(session_id, frame_id, expression)
            .await?
        else {
            return Ok(None);
        };
        let evaluated = self
            .call_in(session_id, "Runtime.evaluate", evaluation)
            .await;
        Ok(tolerate_rejection(evaluated)?.map(|mut result| result["result"]["value"].take()))
    }

    /// The `Runtime.evaluate` parameters that run `expression` in the
    /// scanner's isolated world of a frame. `None` when the frame has no
    /// world to run in.
    pub(super) async fn evaluation_in_frame(
        &mut self,
        session_id: &str,
        frame_id: &str,
        expression: &str,
    ) -> Result<Option<Value>, CaptureError> {
        let world = self
            .call_in(
                session_id,
                "Page.createIsolatedWorld",
                json!({ "frameId": frame_id, "worldName": SCANNER_WORLD }),
            )
            .await;
        Ok(tolerate_rejection(world)?.map(|world| {
            json!({
                "expression": expression,
                "returnByValue": true,
                "awaitPromise": true,
                "contextId": world["executionContextId"],
            })
        }))
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

    /// Gives the recorded tab, and every tab and frame set up from here
    /// on, this timezone.
    pub(crate) async fn set_timezone(
        &mut self,
        timezone: &'static str,
    ) -> Result<(), CaptureError> {
        self.setup.timezone = Some(timezone);
        self.call(
            "Emulation.setTimezoneOverride",
            json!({ "timezoneId": timezone }),
        )
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_answer_to_a_replayed_removal_is_taken_for_done() {
        let rejected = |method: &'static str, message: &str| CdpError::Rejected {
            method,
            message: message.to_string(),
        };
        assert!(was_replayed_removal(&rejected(
            "Page.removeScriptToEvaluateOnNewDocument",
            "Script not found"
        )));
        assert!(!was_replayed_removal(&rejected(
            "Page.removeScriptToEvaluateOnNewDocument",
            "Session closed"
        )));
        assert!(!was_replayed_removal(&rejected(
            "Page.addScriptToEvaluateOnNewDocument",
            "Script not found"
        )));
        assert!(!was_replayed_removal(&CdpError::Closed));
    }
}
