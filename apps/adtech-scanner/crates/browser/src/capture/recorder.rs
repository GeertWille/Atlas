//! Listens to the CDP events of the tab being scanned, and of the iframes
//! and workers below it, and records page views, requests, scripts, console
//! lines and tracker telemetry into the pure collectors. Nothing is recorded
//! while the tab shows a page of another site.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use adscan_model::{
    ActivityKind, BodyAdmission, BodyOffer, BodySkip, CaptureActivity, CapturedBody,
    ConsentNoteKind, NetworkSink, PageReach, ResourceKind,
};

use futures_util::future::BoxFuture;
use futures_util::stream::FuturesUnordered;
use futures_util::{FutureExt, StreamExt};
use serde_json::{Value, json};
use url::Url;

use crate::cdp::{Cdp, CdpError, CdpEvent};

use super::bid_endpoints::{
    BidRequest, BidResponseLog, MAX_BODY_BYTES, no_bid_reason, short,
};
use super::console::ConsoleLog;
use super::injected::{TRACKER_BINDING, is_tracker_script};
use super::network::assembly::{Assembler, Ended, Finished, Output, Reporter, Source};
use super::network::fields::captured_body;
use super::pages::{Change, Committed, PageLog};
use super::progress::Milestones;
use super::scripts::{
    Admission, ScriptLibrary, inline_fingerprint, is_page_script_url, response_text,
};
use super::tracker_log::TrackerLog;
use super::visitor::links::loggable;
use super::visitor::site::Site;

pub(crate) type ActivitySink = Arc<dyn Fn(CaptureActivity) + Send + Sync>;

/// The environment report lists at most this many third-party hosts.
const MAX_REQUEST_HOSTS: usize = 500;

/// The session an event was reported on, as the page knows it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Reported<'a> {
    pub session: &'a str,
    pub reporter: Reporter,
    /// Target of the session, unless it is a tab's own.
    pub target_id: Option<&'a str>,
    /// Main frame of the tab, for a tab's own session.
    pub main_frame_id: Option<&'a str>,
    /// The session is the recorded tab's, or belongs to a target below it.
    pub in_recorded_tab: bool,
}

/// A script file the page loaded, for the script library.
struct ScriptLoad {
    url: String,
    initiator: Option<String>,
    loaded_at_epoch_ms: f64,
}

/// A bidder's answer, read for its stated no-bid reason.
struct BidAnswer {
    request: BidRequest,
    status: Option<u16>,
    answered_at_epoch_ms: f64,
}

/// Something on its way from the browser.
enum Fetched {
    /// A response body, read once for everyone who wants it.
    Response {
        seq: u32,
        /// How many bytes the sink admitted, or what the record says instead.
        for_sink: Result<usize, CapturedBody>,
        script: Option<ScriptLoad>,
        bid: Option<BidAnswer>,
        response: Result<Value, CdpError>,
    },
    RequestBody {
        seq: u32,
        response: Result<Value, CdpError>,
    },
    Inline {
        /// The session that parsed the script and the browser's id for it.
        session: String,
        script_id: String,
        parsed_at_epoch_ms: f64,
        /// URL of the document or script that contains or created the code.
        creator: String,
        response: Result<Value, CdpError>,
    },
}

/// How many requests were handed over, and how many of them were made in
/// an out-of-process iframe or a worker.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct RequestCounts {
    pub total: u32,
    pub in_child_targets: u32,
}

/// The full address of the document a `Page.frameNavigated` event reports.
pub(crate) fn committed_address(frame_navigated: &Value) -> String {
    let frame = &frame_navigated["frame"];
    format!(
        "{}{}",
        frame["url"].as_str().unwrap_or_default(),
        frame["urlFragment"].as_str().unwrap_or_default()
    )
}

/// What the scanner saw or did about a consent dialog, and when.
pub(crate) struct ConsentSeen {
    pub at_epoch_ms: f64,
    pub page: Option<u32>,
    pub kind: ConsentNoteKind,
    pub banner: String,
}

pub(crate) struct Recorder {
    pub consent_notes: Vec<ConsentSeen>,
    /// How many looks in a row found a consent dialog without an accept
    /// control.
    pub dialog_without_accept_seen: u32,
    pub tracker: TrackerLog,
    pub pages: PageLog,
    pub scripts: ScriptLibrary,
    pub console: ConsoleLog,
    pub bid_responses: BidResponseLog,
    pub load_fired: bool,
    /// Main-frame document requests seen so far, redirects not counted. A
    /// visitor that clicked a link watches this for the navigation to begin.
    pub document_requests: u32,
    /// The scanned site: that of the first page the visit reaches.
    site: Option<Site>,
    /// The main frame shows a page of the scanned site.
    on_site: bool,
    /// URL of the main frame's current document, on the site or not.
    document_url: String,
    main_frame_id: String,
    requests: Assembler,
    /// Where finished requests go. `None`: the scan keeps no network data.
    sink: Option<Arc<dyn NetworkSink>>,
    request_counts: RequestCounts,
    /// The library's name of each inline and runtime script, by the session
    /// that parsed it and the browser's script id.
    inline_scripts: HashMap<String, HashMap<String, String>>,
    /// HTTP status of main-frame documents, by loader id.
    document_statuses: HashMap<String, u16>,
    /// Wall-clock start of the latest main-frame document request, epoch ms.
    document_requested_at: Option<f64>,
    fetches: FuturesUnordered<BoxFuture<'static, Fetched>>,
    third_party_hosts: BTreeSet<String>,
    milestones: Milestones,
    on_activity: ActivitySink,
    budget_announced: bool,
    overflow_announced: bool,
    malformed_announced: usize,
}

impl Recorder {
    pub(crate) fn new(
        main_frame_id: String,
        pages: PageLog,
        on_activity: ActivitySink,
        sink: Option<Arc<dyn NetworkSink>>,
    ) -> Self {
        Self {
            consent_notes: Vec::new(),
            dialog_without_accept_seen: 0,
            tracker: TrackerLog::default(),
            pages,
            scripts: ScriptLibrary::default(),
            console: ConsoleLog::default(),
            bid_responses: BidResponseLog::default(),
            load_fired: false,
            document_requests: 0,
            site: None,
            on_site: false,
            document_url: String::new(),
            main_frame_id,
            requests: Assembler::default(),
            sink,
            request_counts: RequestCounts::default(),
            inline_scripts: HashMap::new(),
            document_statuses: HashMap::new(),
            document_requested_at: None,
            fetches: FuturesUnordered::new(),
            third_party_hosts: BTreeSet::new(),
            milestones: Milestones::default(),
            on_activity,
            budget_announced: false,
            overflow_announced: false,
            malformed_announced: 0,
        }
    }

    /// Recording moves to another tab: its main frame is the one to watch,
    /// and `url` is what it shows.
    pub(crate) fn switch_tab(&mut self, main_frame_id: String, url: &str) {
        self.main_frame_id = main_frame_id;
        self.document_url = url.to_string();
        self.on_site = self.site.is_some() && self.is_on_site(url);
        self.load_fired = false;
    }

    /// Whether the recorded tab shows a page of the scanned site.
    pub(crate) fn is_recording(&self) -> bool {
        self.on_site
    }

    pub(crate) fn site(&self) -> Option<&Site> {
        self.site.as_ref()
    }

    /// Whether `url` is a page of the scanned site. Before the visit has
    /// reached its first page, every web address may still become that.
    pub(crate) fn is_on_site(&self, url: &str) -> bool {
        match (&self.site, Url::parse(url)) {
            (Some(site), Ok(url)) => site.contains(&url),
            (None, Ok(url)) => Site::of(&url).is_some(),
            (_, Err(_)) => false,
        }
    }

    pub(crate) fn document_url(&self) -> &str {
        &self.document_url
    }

    /// Host names outside the scanned site that its pages requested.
    pub(crate) fn third_party_hosts(&self) -> Vec<String> {
        self.third_party_hosts.iter().cloned().collect()
    }

    pub(crate) fn report(&self, activity: CaptureActivity) {
        (self.on_activity)(activity);
    }

    fn progress(&self, message: String) {
        self.report(CaptureActivity::progress(message));
    }

    pub(crate) fn document_status(&self, loader_id: &str) -> Option<u16> {
        self.document_statuses.get(loader_id).copied()
    }

    /// Whether the visit's requests are recorded, on every target.
    pub(crate) fn keeps_requests(&self) -> bool {
        self.sink.is_some()
    }

    pub(crate) fn request_counts(&self) -> RequestCounts {
        self.request_counts
    }

    pub(crate) fn has_pending_bodies(&self) -> bool {
        !self.fetches.is_empty()
    }

    /// Resolves once the next body asked of the browser has been stored;
    /// never resolves while none is pending.
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
            "Runtime.consoleAPICalled" if self.on_site => self.console.record_api_call(params),
            "Log.entryAdded" if self.on_site => self.console.record_log_entry(params),
            method if method.starts_with("Network.") => {
                match method {
                    "Network.requestWillBeSent" => self.on_document_request(params),
                    "Network.responseReceived" => self.on_document_response(params),
                    _ => {}
                }
                let main_frame_id = self.main_frame_id.clone();
                let tab = Reported {
                    session: session_id,
                    reporter: Reporter::Tab,
                    target_id: None,
                    main_frame_id: Some(&main_frame_id),
                    in_recorded_tab: true,
                };
                self.observe_network(&tab, event, cdp);
            }
            "Debugger.scriptParsed" if self.on_site => {
                self.on_script_parsed(params, cdp, session_id)
            }
            "Page.loadEventFired" => self.load_fired = true,
            "Page.frameNavigated" if params["frame"]["parentId"].is_null() => {
                let restored = params["type"] == "BackForwardCacheRestore";
                let loader_id = params["frame"]["loaderId"].as_str().unwrap_or_default();
                self.on_document_committed(committed_address(params), loader_id, restored);
            }
            "Page.navigatedWithinDocument" if params["frameId"] == self.main_frame_id.as_str() => {
                self.document_url = params["url"].as_str().unwrap_or_default().to_string();
            }
            _ => {}
        }
    }

    fn on_document_committed(&mut self, url: String, loader_id: &str, restored: bool) {
        self.document_url = url;
        let web_address = Url::parse(&self.document_url)
            .ok()
            .and_then(|url| Some((Site::of(&url)?, url)));
        let Some((site_of_document, url)) = web_address else {
            // The tab's blank start page, an error page: not part of a visit.
            self.on_site = false;
            return;
        };
        // Until the entry page has settled, redirects decide which site the
        // visit is on. From then on it is fixed.
        let entry_is_open = self.pages.current().is_none_or(|page| page == 0);
        if self.site.is_none() || (entry_is_open && self.pages.is_opening()) {
            self.site = Some(site_of_document);
        }
        self.on_site = self.site.as_ref().is_some_and(|site| site.contains(&url));
        let now = epoch_ms_now();
        let change = self.pages.document_committed(Committed {
            url: &self.document_url,
            requested_at: self.document_requested_at.take().unwrap_or(now),
            on_site: self.on_site,
            restored,
            now,
        });
        match change {
            Change::Opened(page) => self.announce_page_view(page),
            Change::Replaced(page) => {
                self.tracker.drop_page(page);
                self.announce_page_view(page);
            }
            Change::LeftSite => {
                let host = url.host_str().unwrap_or_default().to_string();
                self.report(CaptureActivity {
                    kind: ActivityKind::Blocked,
                    message: format!("Left the scanned site ({host}): not recorded"),
                    page: self.pages.current(),
                    data: Some(
                        json!({ "reason": "off-site", "url": loggable(&url), "host": host }),
                    ),
                });
            }
            Change::OverCap | Change::None => {}
        }
        self.announce_page_cap();
        // After the page view exists: the document's request opens it.
        let released = self
            .requests
            .navigation_committed(loader_id, self.on_site, now);
        self.hand_over(released);
    }

    fn announce_page_view(&self, page: u32) {
        let address = self.pages.url(page).unwrap_or_default();
        let url = Url::parse(address).ok();
        let reached_by = self.pages.reached_by(page).unwrap_or(PageReach::Entry);
        let how = match reached_by {
            PageReach::Entry => "entry page",
            PageReach::LinkClick => "followed link",
            PageReach::RouteChange => "route change",
            PageReach::UserNavigation => "opened by the user",
            PageReach::Redirect => "redirect",
            PageReach::Reload => "reload",
        };
        let path = url.as_ref().map_or(address, |url| url.path());
        self.report(CaptureActivity {
            kind: ActivityKind::PageView,
            message: format!("Page {}: {path} ({how})", page + 1),
            page: Some(page),
            data: Some(json!({
                "index": page,
                "url": url.as_ref().map_or_else(|| address.to_string(), loggable),
                "title": null,
                "reachedBy": reached_by,
                "startedAtMs": self.pages.started_at_ms(page),
                "link": self.pages.link(page),
            })),
        });
    }

    fn announce_page_cap(&mut self) {
        if self.pages.take_cap_notice() {
            self.report(CaptureActivity {
                kind: ActivityKind::Blocked,
                message: format!(
                    "The visit has its maximum of {} page views; what follows is recorded on the last one",
                    self.pages.len()
                ),
                page: self.pages.current(),
                data: Some(json!({ "reason": "page-cap" })),
            });
        }
    }

    fn ingest_tracker_batch(&mut self, payload: &str) {
        let ingested = self.tracker.ingest(payload, &mut self.pages);
        for change in ingested.changes {
            if let Change::Opened(page) = change {
                self.announce_page_view(page);
            }
        }
        self.announce_page_cap();
        for event in &self.tracker.events()[ingested.events] {
            if let Some(line) = self.milestones.announce(&event.kind) {
                self.report(CaptureActivity {
                    page: Some(event.page),
                    ..CaptureActivity::progress(line)
                });
            }
        }
        if self.tracker.malformed() > self.malformed_announced {
            self.malformed_announced = self.tracker.malformed();
            self.progress(format!(
                "Ignored {} malformed tracker events so far",
                self.malformed_announced
            ));
        }
    }

    /// Takes a `Network` event of any session of the visit.
    pub(crate) fn observe_network(&mut self, reported: &Reported<'_>, event: &CdpEvent, cdp: &Cdp) {
        let recording = reported.in_recorded_tab && self.on_site;
        if recording
            && event.method == "Network.requestWillBeSent"
            && let Some(url) = event.params["request"]["url"].as_str()
        {
            self.note_third_party(url);
        }
        let source = Source {
            session: reported.session,
            reporter: reported.reporter,
            target_id: reported.target_id,
            main_frame_id: reported.main_frame_id,
            recording,
            page_url: &self.document_url,
            now_ms: epoch_ms_now(),
        };
        let outputs = self
            .requests
            .observe(&source, &event.method, &event.params);
        for output in outputs {
            match output {
                Output::PostDataWanted {
                    seq,
                    session,
                    request_id,
                } => self.ask_for_request_body(seq, &session, &request_id, cdp),
                Output::Finished(finished) => self.on_response_finished(finished, cdp),
                Output::Ended(ended) => self.hand_over_one(*ended),
            }
        }
    }

    /// The start of a main-frame document request of the recorded tab.
    fn on_document_request(&mut self, params: &Value) {
        let is_main_document =
            params["type"] == "Document" && params["frameId"] == self.main_frame_id.as_str();
        if is_main_document && params["redirectResponse"].is_null() {
            self.document_requested_at =
                params["wallTime"].as_f64().map(|seconds| seconds * 1000.0);
            self.document_requests += 1;
            self.load_fired = false;
        }
    }

    fn note_third_party(&mut self, url: &str) {
        if self.third_party_hosts.len() >= MAX_REQUEST_HOSTS {
            return;
        }
        let Ok(url) = Url::parse(url) else { return };
        let is_web = matches!(url.scheme(), "http" | "https" | "ws" | "wss");
        let is_own = self.site.as_ref().is_some_and(|site| {
            // The site is known by its web address; a socket to it is the site's too.
            let mut as_web = url.clone();
            let scheme = if matches!(url.scheme(), "https" | "wss") {
                "https"
            } else {
                "http"
            };
            as_web.set_scheme(scheme).is_ok() && site.contains(&as_web)
        });
        if let Some(host) = url.host_str().filter(|_| is_web && !is_own) {
            self.third_party_hosts.insert(host.to_ascii_lowercase());
        }
    }

    fn on_document_response(&mut self, params: &Value) {
        let Some(request_id) = params["requestId"].as_str() else {
            return;
        };
        let status = params["response"]["status"]
            .as_u64()
            .and_then(|status| u16::try_from(status).ok());
        let is_main_document =
            params["type"] == "Document" && params["frameId"] == self.main_frame_id.as_str();
        if let Some(status) = status.filter(|_| is_main_document) {
            self.document_statuses
                .insert(request_id.to_string(), status);
        }
    }

    fn ask_for_request_body(&mut self, seq: u32, session: &str, request_id: &str, cdp: &Cdp) {
        if self.sink.is_none() {
            let settled = self.requests.post_data_settled(seq, None, epoch_ms_now());
            return self.hand_over(settled);
        }
        let response = cdp.send(
            Some(session),
            "Network.getRequestPostData",
            json!({ "requestId": request_id }),
        );
        self.fetches.push(
            response
                .map(move |response| Fetched::RequestBody { seq, response })
                .boxed(),
        );
    }

    /// A response finished loading. Its body is asked for at once, when
    /// anyone wants it: Chrome discards bodies as the page keeps loading,
    /// and with the frame that loaded them.
    fn on_response_finished(&mut self, finished: Finished, cdp: &Cdp) {
        let showing_the_site =
            self.on_site || (finished.main_document && self.is_on_site(&finished.url));
        let for_sink = match &self.sink {
            None => Err(CapturedBody::Absent),
            Some(_) if !finished.has_address_on_the_network => {
                Err(CapturedBody::Skipped(BodySkip::Type))
            }
            Some(_) if !showing_the_site => Err(CapturedBody::Unavailable(
                "the tab had left the scanned site".into(),
            )),
            Some(sink) => {
                let admission = sink.admit_body(&BodyOffer {
                    seq: finished.seq,
                    kind: finished.kind,
                    mime_type: finished.mime_type.as_deref(),
                    status: finished.status,
                    expected_bytes: finished.expected_bytes,
                });
                match admission {
                    BodyAdmission::Read { max_bytes } => Ok(max_bytes),
                    BodyAdmission::Skip(why) => Err(CapturedBody::Skipped(why)),
                }
            }
        };
        let request_seq = self.sink.is_some().then_some(finished.seq);
        let failed_status = finished.status.is_some_and(|status| status >= 400);
        let is_page_script = finished.in_tab
            && finished.kind == ResourceKind::Script
            && !failed_status
            && is_page_script_url(&finished.url);
        let script = is_page_script.then(|| ScriptLoad {
            url: finished.url.clone(),
            initiator: finished.initiator_url.clone(),
            loaded_at_epoch_ms: finished.finished_epoch_ms,
        });
        let mut bid = finished.bid.map(|request| BidAnswer {
            request,
            status: finished.status,
            answered_at_epoch_ms: finished.finished_epoch_ms,
        });
        if finished.encoded_bytes > MAX_BODY_BYTES as u64
            && let Some(answer) = bid.take()
        {
            // An answer this large carries bids, not an explanation.
            self.bid_responses.record(
                answer.request,
                Some(answer.answered_at_epoch_ms),
                answer.status,
                None,
                request_seq,
            );
        }
        let for_sink = match for_sink {
            Err(instead) if script.is_none() && bid.is_none() => {
                // Nobody wants it read.
                let settled = self
                    .requests
                    .body_settled(finished.seq, instead, epoch_ms_now());
                return self.hand_over(settled);
            }
            wanted => wanted,
        };
        let response = cdp.send(
            Some(&finished.session),
            "Network.getResponseBody",
            json!({ "requestId": finished.request_id }),
        );
        let seq = finished.seq;
        self.fetches.push(
            response
                .map(move |response| Fetched::Response {
                    seq,
                    for_sink,
                    script,
                    bid,
                    response,
                })
                .boxed(),
        );
    }

    fn hand_over(&mut self, outputs: Vec<Output>) {
        for output in outputs {
            if let Output::Ended(ended) = output {
                self.hand_over_one(*ended);
            }
        }
    }

    /// A request has ended: it goes to the sink, placed on its page view.
    fn hand_over_one(&mut self, ended: Ended) {
        let request_seq = self.sink.is_some().then_some(ended.seq());
        if let Some((bid, failed_at, failure)) = ended.failed_bid() {
            // A bid request that never got an answer: blocked, refused or aborted.
            self.bid_responses.record(
                bid.clone(),
                Some(failed_at),
                None,
                Some(short(&failure)),
                request_seq,
            );
        }
        let Some(sink) = &self.sink else {
            return;
        };
        self.request_counts.total += 1;
        if ended.is_of_child_target() {
            self.request_counts.in_child_targets += 1;
        }
        let record = ended.into_record(
            |epoch_ms| self.pages.place_at(epoch_ms),
            |session, frame| {
                self.inline_scripts
                    .get(session)
                    .and_then(|scripts| scripts.get(&frame.script_id))
                    .cloned()
                    .or_else(|| self.scripts.external_url(&frame.url))
            },
        );
        sink.record(record);
    }

    /// The capture is over: hands over what is still open, as it stands.
    pub(crate) fn finish_requests(&mut self) {
        for ended in self.requests.finish() {
            self.hand_over_one(ended);
        }
        self.announce_overflow();
    }

    fn announce_overflow(&mut self) {
        let left_out = self.requests.overflowed();
        if left_out > 0 && !self.overflow_announced && self.sink.is_some() {
            self.overflow_announced = true;
            self.progress(format!(
                "The page kept more requests open than the scan follows at once; \
                 at least {left_out} were not recorded"
            ));
        }
    }

    /// External files are captured from the network; this picks up the scripts
    /// that never touch it: `<script>` blocks in the main frame's HTML and code
    /// the page creates at run time.
    fn on_script_parsed(&mut self, params: &Value, cdp: &Cdp, session_id: &str) {
        let context = &params["executionContextAuxData"];
        let in_page_world =
            context["isDefault"] == true && context["frameId"] == self.main_frame_id.as_str();
        if !in_page_world || !(is_inline_block(params) || is_substantial_runtime_script(params)) {
            return;
        }
        let Some(script_id) = params["scriptId"].as_str().map(str::to_string) else {
            return;
        };
        let session = session_id.to_string();
        let parsed_at_epoch_ms = epoch_ms_now();
        let creator = params["stackTrace"]["callFrames"][0]["url"]
            .as_str()
            .filter(|url| !url.is_empty())
            .unwrap_or(&self.document_url)
            .to_string();
        let response = cdp.send(
            Some(session_id),
            "Debugger.getScriptSource",
            json!({ "scriptId": script_id }),
        );
        self.fetches.push(
            response
                .map(move |response| Fetched::Inline {
                    session,
                    script_id,
                    parsed_at_epoch_ms,
                    creator,
                    response,
                })
                .boxed(),
        );
    }

    fn store(&mut self, fetched: Fetched) {
        let admission = match fetched {
            Fetched::Response {
                seq,
                for_sink,
                script,
                bid,
                response,
            } => {
                let request_seq = self.sink.is_some().then_some(seq);
                let text = || response.as_ref().ok().and_then(response_text);
                if let Some(answer) = bid {
                    let reason = text().and_then(|body| no_bid_reason(answer.request.bidder, &body));
                    self.bid_responses.record(
                        answer.request,
                        Some(answer.answered_at_epoch_ms),
                        answer.status,
                        reason,
                        request_seq,
                    );
                }
                let admission = script.and_then(|script| {
                    Some(self.scripts.add_external(
                        script.url,
                        text()?,
                        Some(script.loaded_at_epoch_ms),
                        script.initiator,
                        request_seq,
                    ))
                });
                let body = match for_sink {
                    Ok(max_bytes) => captured_body(response, max_bytes),
                    Err(instead) => instead,
                };
                let settled = self.requests.body_settled(seq, body, epoch_ms_now());
                self.hand_over(settled);
                admission
            }
            Fetched::RequestBody { seq, response } => {
                let text = response.ok().and_then(|mut result| match result["postData"].take() {
                    Value::String(text) => Some(text),
                    _ => None,
                });
                let settled =
                    self.requests
                        .post_data_settled(seq, text.as_deref(), epoch_ms_now());
                self.hand_over(settled);
                None
            }
            Fetched::Inline {
                session,
                script_id,
                parsed_at_epoch_ms,
                creator,
                response,
            } => {
                let Some(Value::String(body)) = response
                    .ok()
                    .map(|mut result| result["scriptSource"].take())
                else {
                    return;
                };
                if is_tracker_script(&body) {
                    return;
                }
                let fingerprint = inline_fingerprint(&body);
                let admission =
                    self.scripts
                        .add_inline(body, Some(parsed_at_epoch_ms), Some(creator));
                if let Some(held_as) = self.scripts.inline_url(fingerprint) {
                    self.inline_scripts
                        .entry(session)
                        .or_default()
                        .insert(script_id, held_as.to_string());
                }
                Some(admission)
            }
        };
        if admission == Some(Admission::OverBudget) && !self.budget_announced {
            self.budget_announced = true;
            self.progress(
                "Script memory budget reached; further script bodies are not kept".into(),
            );
        }
        self.announce_overflow();
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
    script_parsed["url"].as_str().is_some_and(str::is_empty)
        && script_parsed["length"].as_u64().unwrap_or(0) >= MIN_CHARS
}

pub(crate) fn epoch_ms_now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs_f64() * 1000.0)
        .unwrap_or_default()
}
