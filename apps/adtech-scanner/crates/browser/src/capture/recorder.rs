//! Listens to the page's CDP events and records scripts, console lines and
//! tracker telemetry into the pure collectors.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use futures_util::future::BoxFuture;
use futures_util::stream::FuturesUnordered;
use futures_util::{FutureExt, StreamExt};
use serde_json::{Value, json};

use crate::cdp::{Cdp, CdpError, CdpEvent};

use super::console::ConsoleLog;
use super::injected::{TRACKER_BINDING, TRACKER_SOURCE};
use super::progress::Milestones;
use super::scripts::{Admission, ScriptLibrary, initiator_url, is_page_script_url, response_text};
use super::tracker_log::TrackerLog;

pub(crate) type ProgressSink = Arc<dyn Fn(String) + Send + Sync>;

/// A network request that may turn out to be a script.
struct Request {
    url: String,
    initiator: Option<String>,
    is_script: bool,
    failed_status: bool,
}

/// A script body on its way from the browser.
enum Fetched {
    External {
        url: String,
        initiator: Option<String>,
        loaded_at_epoch_ms: Option<f64>,
        response: Result<Value, CdpError>,
    },
    Inline {
        parsed_at_epoch_ms: f64,
        /// URL of the document or script that contains or created the code.
        creator: String,
        response: Result<Value, CdpError>,
    },
}

pub(crate) struct Recorder {
    pub tracker: TrackerLog,
    pub scripts: ScriptLibrary,
    pub console: ConsoleLog,
    /// URL of the main frame's current document.
    pub document_url: String,
    pub load_fired: bool,
    main_frame_id: String,
    requests: HashMap<String, Request>,
    /// HTTP status of main-frame documents, by loader id.
    document_statuses: HashMap<String, u16>,
    /// Wall-clock start of the latest main-frame document request, epoch ms.
    document_requested_at: Option<f64>,
    /// Epoch ms minus the browser's monotonic clock, to date network events.
    wall_clock_offset_ms: Option<f64>,
    fetches: FuturesUnordered<BoxFuture<'static, Fetched>>,
    milestones: Milestones,
    on_progress: ProgressSink,
    budget_announced: bool,
    malformed_announced: usize,
}

impl Recorder {
    pub(crate) fn new(main_frame_id: String, on_progress: ProgressSink) -> Self {
        Self {
            tracker: TrackerLog::default(),
            scripts: ScriptLibrary::default(),
            console: ConsoleLog::default(),
            document_url: String::new(),
            load_fired: false,
            main_frame_id,
            requests: HashMap::new(),
            document_statuses: HashMap::new(),
            document_requested_at: None,
            wall_clock_offset_ms: None,
            fetches: FuturesUnordered::new(),
            milestones: Milestones::default(),
            on_progress,
            budget_announced: false,
            malformed_announced: 0,
        }
    }

    pub(crate) fn document_status(&self, loader_id: &str) -> Option<u16> {
        self.document_statuses.get(loader_id).copied()
    }

    /// Epoch ms every relative timestamp of the capture is measured from.
    pub(crate) fn navigation_start(&self) -> f64 {
        self.tracker
            .navigation_start()
            .or(self.document_requested_at)
            .unwrap_or_else(epoch_ms_now)
    }

    pub(crate) fn has_pending_bodies(&self) -> bool {
        !self.fetches.is_empty()
    }

    /// Resolves once the next requested script body has been stored; never
    /// resolves while none is pending.
    pub(crate) async fn store_next_body(&mut self) {
        match self.fetches.next().await {
            Some(fetched) => self.store(fetched),
            None => std::future::pending().await,
        }
    }

    pub(crate) fn observe(&mut self, event: &CdpEvent, cdp: &Cdp, session_id: &str) {
        let params = &event.params;
        match event.method.as_str() {
            "Runtime.bindingCalled" if params["name"] == TRACKER_BINDING => {
                self.ingest_tracker_batch(params["payload"].as_str().unwrap_or_default());
            }
            "Runtime.consoleAPICalled" => self.console.record_api_call(params),
            "Log.entryAdded" => self.console.record_log_entry(params),
            "Network.requestWillBeSent" => self.on_request(params),
            "Network.responseReceived" => self.on_response(params),
            "Network.loadingFinished" => self.on_loading_finished(params, cdp, session_id),
            "Network.loadingFailed" => {
                self.requests.remove(params["requestId"].as_str().unwrap_or_default());
            }
            "Debugger.scriptParsed" => self.on_script_parsed(params, cdp, session_id),
            "Page.loadEventFired" => self.load_fired = true,
            "Page.frameNavigated" if params["frame"]["parentId"].is_null() => {
                let frame = &params["frame"];
                self.document_url = format!(
                    "{}{}",
                    frame["url"].as_str().unwrap_or_default(),
                    frame["urlFragment"].as_str().unwrap_or_default()
                );
            }
            "Page.navigatedWithinDocument" if params["frameId"] == self.main_frame_id.as_str() => {
                self.document_url = params["url"].as_str().unwrap_or_default().to_string();
            }
            _ => {}
        }
    }

    fn ingest_tracker_batch(&mut self, payload: &str) {
        let added = self.tracker.ingest(payload);
        for event in &self.tracker.events()[added] {
            if let Some(line) = self.milestones.announce(&event.kind) {
                (self.on_progress)(line);
            }
        }
        if self.tracker.malformed() > self.malformed_announced {
            self.malformed_announced = self.tracker.malformed();
            (self.on_progress)(format!(
                "Ignored {} malformed tracker events so far",
                self.malformed_announced
            ));
        }
    }

    fn on_request(&mut self, params: &Value) {
        let (Some(request_id), Some(url)) = (params["requestId"].as_str(), params["request"]["url"].as_str()) else {
            return;
        };
        let wall_ms = params["wallTime"].as_f64().map(|seconds| seconds * 1000.0);
        if let (Some(wall_ms), Some(monotonic)) = (wall_ms, params["timestamp"].as_f64()) {
            self.wall_clock_offset_ms.get_or_insert(wall_ms - monotonic * 1000.0);
        }
        let is_main_document = params["type"] == "Document" && params["frameId"] == self.main_frame_id.as_str();
        if is_main_document && params["redirectResponse"].is_null() {
            self.document_requested_at = wall_ms;
        }
        let initiator = initiator_url(&params["initiator"])
            .or_else(|| params["documentURL"].as_str().map(str::to_string));
        self.requests.insert(
            request_id.to_string(),
            Request { url: url.to_string(), initiator, is_script: params["type"] == "Script", failed_status: false },
        );
    }

    fn on_response(&mut self, params: &Value) {
        let Some(request_id) = params["requestId"].as_str() else { return };
        let status = params["response"]["status"].as_u64().and_then(|status| u16::try_from(status).ok());
        if params["type"] == "Document" && params["frameId"] == self.main_frame_id.as_str() {
            if let Some(status) = status {
                self.document_statuses.insert(request_id.to_string(), status);
            }
        }
        if let Some(request) = self.requests.get_mut(request_id) {
            request.is_script |= params["type"] == "Script";
            request.failed_status = status.is_some_and(|status| status >= 400);
        }
    }

    /// Bodies are requested the moment loading finishes: Chrome evicts them
    /// from its buffer as the page keeps loading.
    fn on_loading_finished(&mut self, params: &Value, cdp: &Cdp, session_id: &str) {
        let Some(request_id) = params["requestId"].as_str() else { return };
        let Some(request) = self.requests.remove(request_id) else { return };
        if !request.is_script || request.failed_status || !is_page_script_url(&request.url) {
            return;
        }
        let loaded_at_epoch_ms = self
            .wall_clock_offset_ms
            .zip(params["timestamp"].as_f64())
            .map(|(offset, monotonic)| offset + monotonic * 1000.0);
        let response = cdp.send(Some(session_id), "Network.getResponseBody", json!({ "requestId": request_id }));
        self.fetches.push(
            response
                .map(move |response| Fetched::External {
                    url: request.url,
                    initiator: request.initiator,
                    loaded_at_epoch_ms,
                    response,
                })
                .boxed(),
        );
    }

    /// External files are captured from the network; this picks up the scripts
    /// that never touch it: `<script>` blocks in the main frame's HTML and code
    /// the page creates at run time.
    fn on_script_parsed(&mut self, params: &Value, cdp: &Cdp, session_id: &str) {
        let context = &params["executionContextAuxData"];
        let in_page_world = context["isDefault"] == true && context["frameId"] == self.main_frame_id.as_str();
        if !in_page_world || !(is_inline_block(params) || is_substantial_runtime_script(params)) {
            return;
        }
        let parsed_at_epoch_ms = epoch_ms_now();
        let creator = params["stackTrace"]["callFrames"][0]["url"]
            .as_str()
            .filter(|url| !url.is_empty())
            .unwrap_or(&self.document_url)
            .to_string();
        let response = cdp.send(
            Some(session_id),
            "Debugger.getScriptSource",
            json!({ "scriptId": params["scriptId"] }),
        );
        self.fetches.push(
            response
                .map(move |response| Fetched::Inline { parsed_at_epoch_ms, creator, response })
                .boxed(),
        );
    }

    fn store(&mut self, fetched: Fetched) {
        let admission = match fetched {
            Fetched::External { url, initiator, loaded_at_epoch_ms, response } => {
                let Some(body) = response.ok().as_ref().and_then(response_text) else { return };
                self.scripts.add_external(url, body, loaded_at_epoch_ms, initiator)
            }
            Fetched::Inline { parsed_at_epoch_ms, creator, response } => {
                let Some(Value::String(body)) = response.ok().map(|mut result| result["scriptSource"].take()) else {
                    return;
                };
                if body == TRACKER_SOURCE {
                    return;
                }
                self.scripts.add_inline(body, Some(parsed_at_epoch_ms), Some(creator))
            }
        };
        if admission == Admission::OverBudget && !self.budget_announced {
            self.budget_announced = true;
            (self.on_progress)("Script memory budget reached; further script bodies are not kept".into());
        }
    }
}

/// Scripts the HTML parser found inside a document start after their
/// `<script>` tag; files and evaluated strings start at 0:0.
fn is_inline_block(script_parsed: &Value) -> bool {
    let offset = |key: &str| script_parsed[key].as_u64().unwrap_or(0);
    let url = script_parsed["url"].as_str().unwrap_or_default();
    is_page_script_url(url) && (offset("startLine") > 0 || offset("startColumn") > 0)
}

/// Injected `<script>` elements, `eval` and `new Function` all arrive without a
/// URL and cannot be told apart. Tag managers ship real ad logic this way, so
/// they are kept, minus the one-liners that only pass data around.
fn is_substantial_runtime_script(script_parsed: &Value) -> bool {
    const MIN_CHARS: u64 = 200;
    script_parsed["url"].as_str().is_some_and(str::is_empty) && script_parsed["length"].as_u64().unwrap_or(0) >= MIN_CHARS
}

fn epoch_ms_now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs_f64() * 1000.0)
        .unwrap_or_default()
}
