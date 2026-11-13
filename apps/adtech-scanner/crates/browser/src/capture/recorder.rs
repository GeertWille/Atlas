//! Listens to the CDP events of the tab being scanned, and of the iframes
//! and workers below it, and records page views, requests, scripts, console
//! lines and tracker telemetry into the pure collectors. Nothing is recorded
//! while the tab shows a page of another site.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use adscan_model::{
    ActivityKind, BodyAdmission, BodyOffer, BodySkip, CaptureActivity, CapturedBody,
    ConsentNoteKind, NetworkSink, PageReach, ResourceKind,
};

use futures_util::future::BoxFuture;
use futures_util::stream::FuturesUnordered;
use futures_util::{FutureExt, StreamExt};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use url::Url;

use crate::cdp::{Cdp, CdpError, CdpEvent};

use super::bid_endpoints::{BidRequest, BidResponseLog, MAX_BODY_BYTES, no_bid_reason, short};
use super::clock::CaptureClock;
use super::console::ConsoleLog;
use super::injected::{TRACKER_BINDING, is_tracker_script};
use super::network::assembly::{Assembler, Ended, Finished, Limits, Output, Reporter, Source};
use super::network::fields::{body_of, captured_body};
use super::network::reply::{BodySize, ReplyLimit};
use super::noise::is_service_script;
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

/// The address every request of a browser behind a Browserbase proxy
/// reports as its server's: the proxy's own, inside the service.
const SERVICE_PROXY_ADDRESS: &str = "10.255.255.1";

/// What the recording has to know about the way to its browser.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BrowserLink {
    /// The browser is hosted by a service that puts scripts of its own
    /// into every page.
    pub hosted_by_service: bool,
    /// The largest answer the connection carries.
    pub reply_limit: ReplyLimit,
    /// The browser reaches the web through a proxy of the service that
    /// hosts it, and reports that proxy as the server of every request.
    pub behind_service_proxy: bool,
}

impl BrowserLink {
    /// Chrome on this machine, on its debugging pipe.
    pub(crate) const LOCAL: Self = Self {
        hosted_by_service: false,
        reply_limit: ReplyLimit::Unlimited,
        behind_service_proxy: false,
    };

    fn limits(self) -> Limits {
        Limits {
            copy_bodies_from: self.reply_limit.copies_bodies_from(),
            post_length_must_be_known: self.reply_limit != ReplyLimit::Unlimited,
        }
    }
}

/// Whether `address` (`ip:port`) is the service's proxy, not a server.
fn is_service_proxy(address: &str) -> bool {
    address
        .strip_prefix(SERVICE_PROXY_ADDRESS)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with(':'))
}

/// The answer to the question for a response body.
enum BodyAnswer {
    Browser(Result<Value, CdpError>),
    /// Never asked: the answer could have been more than the connection
    /// to the browser carries.
    TooLarge {
        total_bytes: u64,
    },
}

/// Scripts in the page's own world that the service hosting the browser
/// put there. They are no part of the scan's scripts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct ServiceScripts {
    pub count: u32,
    pub chars: u64,
}

/// Response bodies that were not asked for because of their size.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct TooLarge {
    pub count: u32,
    pub largest_bytes: u64,
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
        response: BodyAnswer,
    },
    RequestBody {
        seq: u32,
        response: Result<Value, CdpError>,
    },
    /// The answer of a frame or worker that was asked to send a response
    /// body along while it loads.
    BodyCopy {
        request_id: String,
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
    /// The clock every moment the recording notes is read from.
    pub clock: CaptureClock,
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
    link: BrowserLink,
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
    /// Triggered when the target of a session is gone, by session: what was
    /// asked of it will not be answered.
    targets_gone: HashMap<String, CancellationToken>,
    /// The browser is older than `Network.streamResourceContent` (Chrome
    /// 122): what a frame or worker loaded is lost when it goes away before
    /// it handed the body over.
    cannot_copy_bodies: bool,
    too_large: TooLarge,
    service_scripts: ServiceScripts,
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
            clock: CaptureClock::SHARED,
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
            link: BrowserLink::LOCAL,
            requests: Assembler::default(),
            sink,
            request_counts: RequestCounts::default(),
            inline_scripts: HashMap::new(),
            document_statuses: HashMap::new(),
            document_requested_at: None,
            fetches: FuturesUnordered::new(),
            targets_gone: HashMap::new(),
            cannot_copy_bodies: false,
            too_large: TooLarge::default(),
            service_scripts: ServiceScripts::default(),
            third_party_hosts: BTreeSet::new(),
            milestones: Milestones::default(),
            on_activity,
            budget_announced: false,
            overflow_announced: false,
            malformed_announced: 0,
        }
    }

    /// The same recorder for a browser that is reached over `link`. Before
    /// anything is recorded.
    pub(crate) fn linked_by(mut self, link: BrowserLink) -> Self {
        self.link = link;
        self.requests = Assembler::within(link.limits());
        self
    }

    /// What the service that hosts the browser ran in the pages' own world.
    pub(crate) fn service_scripts(&self) -> ServiceScripts {
        self.service_scripts
    }

    /// Response bodies that were too large to ask the browser for.
    pub(crate) fn too_large_bodies(&self) -> TooLarge {
        self.too_large
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
        let now = self.clock.now_ms();
        let change = self.pages.document_committed(Committed {
            url: &self.document_url,
            // A tab the visit just moved to asked for its document before
            // it was the recorded one.
            requested_at: self
                .document_requested_at
                .take()
                .or_else(|| self.requests.navigation_started_at(loader_id))
                .unwrap_or(now),
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
            now_ms: self.clock.now_ms(),
        };
        let outputs = self.requests.observe(&source, &event.method, &event.params);
        for output in outputs {
            match output {
                Output::PostDataWanted {
                    seq,
                    session,
                    request_id,
                } => self.ask_for_request_body(seq, &session, &request_id, cdp),
                Output::BodyCopyWanted {
                    session,
                    request_id,
                } => self.ask_for_body_copy(&session, request_id, cdp),
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

    /// The target of a session is gone: a closed tab, a removed frame, a
    /// worker that ended.
    pub(crate) fn target_gone(&mut self, session_id: &str) {
        if let Some(gone) = self.targets_gone.remove(session_id) {
            gone.cancel();
        }
    }

    /// Asks the session that has a request for one of its bodies. Chrome
    /// does not always answer a session whose target went away meanwhile;
    /// then the answer is that it is gone.
    fn ask_for_body(
        &mut self,
        cdp: &Cdp,
        session: &str,
        method: &'static str,
        request_id: &str,
    ) -> impl Future<Output = Result<Value, CdpError>> + Send + 'static + use<> {
        let gone = self
            .targets_gone
            .entry(session.to_string())
            .or_default()
            .clone();
        let response = cdp.send(Some(session), method, json!({ "requestId": request_id }));
        async move {
            tokio::select! {
                biased;
                response = response => response,
                _ = gone.cancelled() => Err(CdpError::Rejected {
                    method,
                    message: "the frame or worker that loaded it is gone".into(),
                }),
            }
        }
    }

    fn ask_for_request_body(&mut self, seq: u32, session: &str, request_id: &str, cdp: &Cdp) {
        if self.sink.is_none() {
            let settled = self
                .requests
                .post_data_settled(seq, None, self.clock.now_ms());
            return self.hand_over(settled);
        }
        let response = self.ask_for_body(cdp, session, "Network.getRequestPostData", request_id);
        self.fetches.push(
            response
                .map(move |response| Fetched::RequestBody { seq, response })
                .boxed(),
        );
    }

    /// Asks a frame or worker to send the body of a response along while it
    /// loads, so that the body outlives it. Nothing about the request
    /// changes for the page.
    fn ask_for_body_copy(&mut self, session: &str, request_id: String, cdp: &Cdp) {
        if self.cannot_copy_bodies {
            return;
        }
        let response =
            self.ask_for_body(cdp, session, "Network.streamResourceContent", &request_id);
        self.fetches.push(
            response
                .map(move |response| Fetched::BodyCopy {
                    request_id,
                    response,
                })
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
                    url: &finished.url,
                    bidder: finished.bid.as_ref().map(|bid| bid.bidder),
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
                let settled =
                    self.requests
                        .body_settled(finished.seq, instead, self.clock.now_ms());
                return self.hand_over(settled);
            }
            wanted => wanted,
        };
        let seq = finished.seq;
        let total_bytes = finished.expected_bytes;
        let size = if finished.decoded_size_known {
            BodySize::Decoded(total_bytes)
        } else {
            BodySize::OnTheWire(finished.encoded_bytes)
        };
        let admitted =
            self.link
                .reply_limit
                .admits_body(finished.kind, finished.mime_type.as_deref(), size);
        if !admitted {
            // The answer to the question that set the copy going arrived
            // before the response finished: it is taken first.
            self.store_what_has_arrived();
            return self.store(Fetched::Response {
                seq,
                for_sink,
                script,
                bid,
                response: BodyAnswer::TooLarge { total_bytes },
            });
        }
        let response = self.ask_for_body(
            cdp,
            &finished.session,
            "Network.getResponseBody",
            &finished.request_id,
        );
        self.fetches.push(
            response
                .map(move |response| Fetched::Response {
                    seq,
                    for_sink,
                    script,
                    bid,
                    response: BodyAnswer::Browser(response),
                })
                .boxed(),
        );
    }

    /// Stores every answer of the browser that is here already.
    fn store_what_has_arrived(&mut self) {
        while let Some(Some(fetched)) = self.fetches.next().now_or_never() {
            self.store(fetched);
        }
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
        let behind_service_proxy = self.link.behind_service_proxy;
        let mut record = ended.into_record(
            |epoch_ms| self.pages.place_at(epoch_ms),
            |session, frame| {
                self.inline_scripts
                    .get(session)
                    .and_then(|scripts| scripts.get(&frame.script_id))
                    .cloned()
                    .or_else(|| self.scripts.external_url(&frame.url))
            },
        );
        record.page_host = self
            .pages
            .url(record.page)
            .and_then(|url| Url::parse(url).ok())
            .and_then(|url| url.host_str().map(str::to_ascii_lowercase))
            .unwrap_or_default();
        if behind_service_proxy {
            record
                .remote_address
                .take_if(|address| is_service_proxy(address));
        }
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
        let length = params["length"].as_u64().unwrap_or(0);
        if !self.link.reply_limit.admits_script_source(length) {
            return;
        }
        let Some(script_id) = params["scriptId"].as_str().map(str::to_string) else {
            return;
        };
        let session = session_id.to_string();
        let parsed_at_epoch_ms = self.clock.now_ms();
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
                // The frame or worker is gone, and its session with it: what
                // it sent along while loading is the body then.
                let copied = match &response {
                    BodyAnswer::Browser(Err(CdpError::Rejected { .. }))
                    | BodyAnswer::TooLarge { .. } => self.requests.take_body_copy(seq),
                    BodyAnswer::Browser(_) => None,
                };
                let text = || match (&copied, &response) {
                    (Some(bytes), _) => Some(String::from_utf8_lossy(bytes).into_owned()),
                    (None, BodyAnswer::Browser(Ok(answer))) => response_text(answer),
                    (None, _) => None,
                };
                if let Some(answer) = bid {
                    let reason =
                        text().and_then(|body| no_bid_reason(answer.request.bidder, &body));
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
                if let (BodyAnswer::TooLarge { total_bytes }, None) = (&response, &copied) {
                    self.too_large.count += 1;
                    self.too_large.largest_bytes = self.too_large.largest_bytes.max(*total_bytes);
                }
                let body = match (for_sink, copied, response) {
                    (Ok(max_bytes), Some(bytes), _) => body_of(bytes, max_bytes),
                    (Ok(max_bytes), None, BodyAnswer::Browser(answer)) => {
                        captured_body(answer, max_bytes)
                    }
                    (Ok(_), None, BodyAnswer::TooLarge { total_bytes }) => {
                        CapturedBody::TooLargeForRemote { total_bytes }
                    }
                    (Err(instead), _, _) => instead,
                };
                let settled = self.requests.body_settled(seq, body, self.clock.now_ms());
                self.hand_over(settled);
                admission
            }
            Fetched::RequestBody { seq, response } => {
                let text = response
                    .ok()
                    .and_then(|mut result| match result["postData"].take() {
                        Value::String(text) => Some(text),
                        _ => None,
                    });
                let settled =
                    self.requests
                        .post_data_settled(seq, text.as_deref(), self.clock.now_ms());
                self.hand_over(settled);
                None
            }
            Fetched::BodyCopy {
                request_id,
                response,
            } => {
                match response {
                    Ok(started) => self.requests.body_copy_started(
                        &request_id,
                        started["bufferedData"].as_str().unwrap_or_default(),
                    ),
                    Err(refusal) if refusal.is_unknown_method() => self.cannot_copy_bodies = true,
                    // The response was complete already, or its target is
                    // gone: the body is read as far as the target allows.
                    Err(_) => {}
                }
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
                if self.link.hosted_by_service && is_service_script(&body) {
                    self.service_scripts.count += 1;
                    self.service_scripts.chars += body.chars().count() as u64;
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

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use adscan_model::NetworkRecord;

    use super::*;
    use crate::cdp::fake::{Browser, connected};

    /// A sink that reads up to 1024 bytes of every body.
    #[derive(Default)]
    struct Kept(Mutex<Vec<NetworkRecord>>);

    impl NetworkSink for Kept {
        fn admit_body(&self, _: &BodyOffer<'_>) -> BodyAdmission {
            BodyAdmission::Read { max_bytes: 1024 }
        }

        fn record(&self, record: NetworkRecord) {
            self.0.lock().unwrap().push(record);
        }
    }

    impl Kept {
        fn body_of(&self, url: &str) -> CapturedBody {
            let records = self.0.lock().unwrap();
            let record = records.iter().find(|record| record.url == url);
            record
                .unwrap_or_else(|| panic!("{url} is not recorded"))
                .body
                .clone()
        }
    }

    fn event(method: &str, params: Value) -> CdpEvent {
        CdpEvent {
            method: method.to_string(),
            params,
            session_id: None,
        }
    }

    /// A recorder whose tab shows a page of the site it scans.
    fn recorder_on_a_page(sink: Arc<Kept>, cdp: &Cdp) -> Recorder {
        let mut recorder = Recorder::new(
            "main".to_string(),
            PageLog::new(8, false),
            Arc::new(|_| {}),
            Some(sink),
        );
        let committed = json!({ "frame": { "id": "main", "loaderId": "L.1", "url": "https://www.pub.example/" } });
        recorder.observe(&event("Page.frameNavigated", committed), cdp, "tab");
        assert!(recorder.is_recording());
        recorder
    }

    /// A frame in a process of its own, below the recorded tab.
    const FRAME: Reported<'static> = Reported {
        session: "frame",
        reporter: Reporter::Iframe,
        target_id: Some("frame-target"),
        main_frame_id: None,
        in_recorded_tab: true,
    };

    fn announce(recorder: &mut Recorder, cdp: &Cdp, id: &str, url: &str) {
        let announced = json!({
            "requestId": id, "frameId": "frame-target", "type": "Fetch",
            "timestamp": 1_000.0, "wallTime": 1_700_000_000.0,
            "request": { "url": url, "method": "GET", "headers": {} },
            "initiator": { "type": "script" },
        });
        recorder.observe_network(&FRAME, &event("Network.requestWillBeSent", announced), cdp);
    }

    /// The answer to `id` arrives in the frame: its headers, `pieces` of its
    /// body, each sent along or not, and its end.
    fn answer(recorder: &mut Recorder, cdp: &Cdp, id: &str, pieces: &[(&str, bool)]) {
        let headers = json!({
            "requestId": id, "type": "Fetch", "timestamp": 1_000.1, "hasExtraInfo": false,
            "response": { "status": 200, "mimeType": "application/json", "headers": {} },
        });
        recorder.observe_network(&FRAME, &event("Network.responseReceived", headers), cdp);
        for (piece, sent_along) in pieces {
            let mut received = json!({ "requestId": id, "timestamp": 1_000.2,
                "dataLength": piece.len(), "encodedDataLength": piece.len() });
            if *sent_along {
                received["data"] = json!(base64_of(piece));
            }
            recorder.observe_network(&FRAME, &event("Network.dataReceived", received), cdp);
        }
        let end = json!({ "requestId": id, "timestamp": 1_000.3, "encodedDataLength": 60 });
        recorder.observe_network(&FRAME, &event("Network.loadingFinished", end), cdp);
    }

    fn base64_of(text: &str) -> String {
        base64::Engine::encode(&base64::engine::general_purpose::STANDARD, text)
    }

    /// The next command, which must be `method` for request `id` in the frame.
    async fn asked(browser: &mut Browser, method: &str, id: &str) -> u64 {
        let command = browser.received().await;
        assert_eq!(
            (
                &command["method"],
                &command["sessionId"],
                &command["params"]["requestId"]
            ),
            (&json!(method), &json!("frame"), &json!(id))
        );
        command["id"].as_u64().unwrap()
    }

    #[tokio::test]
    async fn a_body_sent_along_stands_in_when_its_frame_is_gone_before_it_handed_it_over() {
        let (cdp, _events, mut browser) = connected();
        let sink = Arc::new(Kept::default());
        let mut recorder = recorder_on_a_page(sink.clone(), &cdp);
        const URL: &str = "https://ads.example/passback.json";

        announce(&mut recorder, &cdp, "F.1", URL);
        let copy = asked(&mut browser, "Network.streamResourceContent", "F.1").await;
        // The answer was on its way when the question arrived.
        browser
            .say(json!({ "id": copy, "result": { "bufferedData": base64_of("{\"ad\":") } }))
            .await;
        answer(
            &mut recorder,
            &cdp,
            "F.1",
            &[("{\"ad\":", false), ("\"creative\"}", true)],
        );
        asked(&mut browser, "Network.getResponseBody", "F.1").await;
        // The frame is removed; Chrome answers nothing more for it.
        recorder.target_gone("frame");
        while recorder.has_pending_bodies() {
            recorder.store_next_body().await;
        }

        assert_eq!(
            sink.body_of(URL),
            CapturedBody::Read {
                bytes: b"{\"ad\":\"creative\"}".to_vec(),
                total_bytes: 17
            }
        );
    }

    #[tokio::test]
    async fn a_browser_that_cannot_send_bodies_along_is_asked_once_and_read_as_before() {
        let (cdp, _events, mut browser) = connected();
        let sink = Arc::new(Kept::default());
        let mut recorder = recorder_on_a_page(sink.clone(), &cdp);
        const LOST: &str = "https://ads.example/passback.json";
        const READ: &str = "https://ads.example/config.json";

        announce(&mut recorder, &cdp, "F.1", LOST);
        let copy = asked(&mut browser, "Network.streamResourceContent", "F.1").await;
        browser
            .say(json!({ "id": copy, "error": { "code": -32601,
                "message": "'Network.streamResourceContent' wasn't found" } }))
            .await;
        recorder.store_next_body().await;

        // The next request is announced without that question: the next
        // thing asked of the browser is the body of the first.
        announce(&mut recorder, &cdp, "F.2", READ);
        answer(&mut recorder, &cdp, "F.1", &[("{\"ad\":1}", false)]);
        let lost = asked(&mut browser, "Network.getResponseBody", "F.1").await;
        browser
            .say(json!({ "id": lost, "error": { "code": -32001,
                "message": "Session with given id not found." } }))
            .await;
        answer(&mut recorder, &cdp, "F.2", &[("{\"slots\":2}", false)]);
        let read = asked(&mut browser, "Network.getResponseBody", "F.2").await;
        browser
            .say(json!({ "id": read, "result": { "body": "{\"slots\":2}", "base64Encoded": false } }))
            .await;
        while recorder.has_pending_bodies() {
            recorder.store_next_body().await;
        }

        assert_eq!(
            sink.body_of(LOST),
            CapturedBody::Unavailable("Session with given id not found.".into())
        );
        assert_eq!(
            sink.body_of(READ),
            CapturedBody::Read {
                bytes: b"{\"slots\":2}".to_vec(),
                total_bytes: 11
            }
        );
    }
    const MIB: usize = 1024 * 1024;

    /// The recorded tab itself, in a browser at Browserbase.
    const TAB: Reported<'static> = Reported {
        session: "tab",
        reporter: Reporter::Tab,
        target_id: None,
        main_frame_id: Some("main"),
        in_recorded_tab: true,
    };

    fn remote_recorder(sink: Arc<Kept>, cdp: &Cdp, behind_service_proxy: bool) -> Recorder {
        let mut recorder = Recorder::new(
            "main".to_string(),
            PageLog::new(8, false),
            Arc::new(|_| {}),
            Some(sink),
        )
        .linked_by(BrowserLink {
            hosted_by_service: true,
            reply_limit: ReplyLimit::Remote,
            behind_service_proxy,
        });
        let committed = json!({ "frame": { "id": "main", "loaderId": "L.1", "url": "https://www.pub.example/" } });
        recorder.observe(&event("Page.frameNavigated", committed), cdp, "tab");
        recorder
    }

    /// The tab asks for `url` and its answer's headers arrive, from
    /// `server` (`ip`, `port`).
    fn tab_loads(recorder: &mut Recorder, cdp: &Cdp, id: &str, url: &str, server: (&str, u16)) {
        tab_loads_as(
            recorder,
            cdp,
            id,
            url,
            ("Script", "text/javascript"),
            server,
        );
    }

    /// The same for a request of the type `asked_as.0` that is answered
    /// with the MIME type `asked_as.1`.
    fn tab_loads_as(
        recorder: &mut Recorder,
        cdp: &Cdp,
        id: &str,
        url: &str,
        asked_as: (&str, &str),
        server: (&str, u16),
    ) {
        let announced = json!({
            "requestId": id, "frameId": "main", "type": asked_as.0,
            "timestamp": 1_000.0, "wallTime": 1_700_000_000.0,
            "request": { "url": url, "method": "GET", "headers": {} },
            "initiator": { "type": "parser" },
        });
        recorder.observe_network(&TAB, &event("Network.requestWillBeSent", announced), cdp);
        let headers = json!({
            "requestId": id, "type": asked_as.0, "timestamp": 1_000.1, "hasExtraInfo": false,
            "response": { "status": 200, "mimeType": asked_as.1, "headers": {},
                "remoteIPAddress": server.0, "remotePort": server.1 },
        });
        recorder.observe_network(&TAB, &event("Network.responseReceived", headers), cdp);
    }

    fn tab_receives(recorder: &mut Recorder, cdp: &Cdp, id: &str, piece: &str, sent_along: bool) {
        let mut received = json!({ "requestId": id, "timestamp": 1_000.2,
            "dataLength": piece.len(), "encodedDataLength": piece.len() });
        if sent_along {
            received["data"] = json!(base64_of(piece));
        }
        recorder.observe_network(&TAB, &event("Network.dataReceived", received), cdp);
    }

    fn tab_finishes(recorder: &mut Recorder, cdp: &Cdp, id: &str) {
        tab_finishes_after(recorder, cdp, id, 60);
    }

    /// The response ends after `on_the_wire` bytes, headers included.
    fn tab_finishes_after(recorder: &mut Recorder, cdp: &Cdp, id: &str, on_the_wire: usize) {
        let end =
            json!({ "requestId": id, "timestamp": 1_000.3, "encodedDataLength": on_the_wire });
        recorder.observe_network(&TAB, &event("Network.loadingFinished", end), cdp);
    }

    /// The next command the browser gets is `Browser.getVersion`: nothing
    /// was asked of it before.
    async fn assert_nothing_else_was_asked(cdp: &Cdp, browser: &mut Browser) {
        cdp.notify(None, "Browser.getVersion", json!({}));
        assert_eq!(browser.received().await["method"], "Browser.getVersion");
    }

    /// Over the connection to a remote browser an answer of more than
    /// about 50 MiB ends the session. A body that could make one is never
    /// asked for: it was sent along while it loaded, from the moment it
    /// grew large, and that copy is the body.
    #[tokio::test]
    async fn a_large_body_of_a_remote_tab_is_taken_from_its_copy_and_never_asked_for() {
        let (cdp, _events, mut browser) = connected();
        let sink = Arc::new(Kept::default());
        let mut recorder = remote_recorder(sink.clone(), &cdp, false);
        const URL: &str = "https://cdn.example/bundle.js";
        let first = "a".repeat(5 * MIB);
        let rest = "b".repeat(4 * MIB);

        tab_loads(&mut recorder, &cdp, "T.1", URL, ("203.0.113.9", 443));
        tab_receives(&mut recorder, &cdp, "T.1", &first, false);
        let command = browser.received().await;
        assert_eq!(
            (
                &command["method"],
                &command["sessionId"],
                &command["params"]["requestId"]
            ),
            (
                &json!("Network.streamResourceContent"),
                &json!("tab"),
                &json!("T.1")
            )
        );
        browser
            .say(json!({ "id": command["id"], "result": { "bufferedData": base64_of(&first) } }))
            .await;
        tab_receives(&mut recorder, &cdp, "T.1", &rest, true);
        // The browser's answer is here before the response ends, as on the
        // wire: the answer was sent before the end was.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        tab_finishes(&mut recorder, &cdp, "T.1");

        assert_eq!(
            sink.body_of(URL),
            CapturedBody::Read {
                bytes: vec![b'a'; 1024],
                total_bytes: 9 * MIB as u64,
            }
        );
        assert_eq!(recorder.too_large_bodies(), TooLarge::default());
        assert!(!recorder.has_pending_bodies());
        assert_nothing_else_was_asked(&cdp, &mut browser).await;
    }

    #[tokio::test]
    async fn a_large_body_of_a_remote_tab_without_a_whole_copy_is_recorded_as_too_large() {
        let (cdp, _events, mut browser) = connected();
        let sink = Arc::new(Kept::default());
        let mut recorder = remote_recorder(sink.clone(), &cdp, false);
        let large = "a".repeat(9 * MIB);

        // There at once, as from a cache: the copy is asked for and the
        // response has ended before the browser can answer.
        tab_loads(
            &mut recorder,
            &cdp,
            "T.1",
            "https://cdn.example/cached.js",
            ("203.0.113.9", 443),
        );
        tab_receives(&mut recorder, &cdp, "T.1", &large, false);
        tab_finishes(&mut recorder, &cdp, "T.1");
        let late = browser.received().await;
        assert_eq!(late["method"], "Network.streamResourceContent");
        browser
            .say(json!({ "id": late["id"], "error": { "code": -32602,
                "message": "Request with the provided ID has already finished loading" } }))
            .await;
        // A second one, larger still, and a body of ordinary size.
        tab_loads(
            &mut recorder,
            &cdp,
            "T.2",
            "https://cdn.example/larger.js",
            ("203.0.113.9", 443),
        );
        tab_receives(&mut recorder, &cdp, "T.2", &large, false);
        tab_receives(&mut recorder, &cdp, "T.2", &large, false);
        tab_finishes(&mut recorder, &cdp, "T.2");
        let later = browser.received().await;
        assert_eq!(later["method"], "Network.streamResourceContent");
        browser
            .say(json!({ "id": later["id"], "error": { "code": -32602,
                "message": "Request with the provided ID has already finished loading" } }))
            .await;
        tab_loads(
            &mut recorder,
            &cdp,
            "T.3",
            "https://cdn.example/small.js",
            ("203.0.113.9", 443),
        );
        tab_receives(&mut recorder, &cdp, "T.3", "let a = 1;", false);
        tab_finishes(&mut recorder, &cdp, "T.3");
        let read = browser.received().await;
        assert_eq!(
            (&read["method"], &read["params"]["requestId"]),
            (&json!("Network.getResponseBody"), &json!("T.3"))
        );
        browser
            .say(json!({ "id": read["id"], "result": { "body": "let a = 1;", "base64Encoded": false } }))
            .await;
        while recorder.has_pending_bodies() {
            recorder.store_next_body().await;
        }

        assert_eq!(
            sink.body_of("https://cdn.example/cached.js"),
            CapturedBody::TooLargeForRemote {
                total_bytes: 9 * MIB as u64
            }
        );
        assert_eq!(
            sink.body_of("https://cdn.example/larger.js"),
            CapturedBody::TooLargeForRemote {
                total_bytes: 18 * MIB as u64
            }
        );
        assert_eq!(
            sink.body_of("https://cdn.example/small.js"),
            CapturedBody::Read {
                bytes: b"let a = 1;".to_vec(),
                total_bytes: 10
            }
        );
        assert_eq!(
            recorder.too_large_bodies(),
            TooLarge {
                count: 2,
                largest_bytes: 18 * MIB as u64
            }
        );
        assert_nothing_else_was_asked(&cdp, &mut browser).await;
    }

    /// A script source is text, and every character of it can take six
    /// bytes in an answer.
    /// What a response was asked for as does not say how its body comes
    /// back: 10 MiB of SVG, or of text, served to an `<img>` may come as
    /// text, at six times its size. Only an image that also calls itself
    /// one is asked for up to the size that fits as base64.
    #[tokio::test]
    async fn an_image_that_is_answered_as_text_is_held_to_the_limit_for_text() {
        let (cdp, _events, mut browser) = connected();
        let sink = Arc::new(Kept::default());
        let mut recorder = remote_recorder(sink.clone(), &cdp, false);
        let ten_mib = "a".repeat(10 * MIB);
        let server = ("203.0.113.9", 443);
        let loads = [
            ("T.1", "https://cdn.example/drawing.svg", "image/svg+xml"),
            ("T.2", "https://cdn.example/pixel.gif", "text/plain"),
            ("T.3", "https://cdn.example/photo.png", "image/png"),
        ];

        for (id, url, mime_type) in loads {
            tab_loads_as(&mut recorder, &cdp, id, url, ("Image", mime_type), server);
            tab_receives(&mut recorder, &cdp, id, &ten_mib, false);
            tab_finishes(&mut recorder, &cdp, id);
        }

        // Only the one that cannot be text was asked for.
        let read = browser.received().await;
        assert_eq!(
            (&read["method"], &read["params"]["requestId"]),
            (&json!("Network.getResponseBody"), &json!("T.3"))
        );
        browser
            .say(json!({ "id": read["id"], "result": { "body": "iVBORw0KGgo=", "base64Encoded": true } }))
            .await;
        while recorder.has_pending_bodies() {
            recorder.store_next_body().await;
        }
        for url in [loads[0].1, loads[1].1] {
            assert_eq!(
                sink.body_of(url),
                CapturedBody::TooLargeForRemote {
                    total_bytes: 10 * MIB as u64
                },
                "{url}"
            );
        }
        assert!(matches!(
            sink.body_of(loads[2].1),
            CapturedBody::Read { .. }
        ));
        assert_eq!(
            recorder.too_large_bodies(),
            TooLarge {
                count: 2,
                largest_bytes: 10 * MIB as u64
            }
        );
        assert_nothing_else_was_asked(&cdp, &mut browser).await;
    }

    /// A response can end without one byte of its body having been
    /// counted. All that is known of it then is its length on the wire,
    /// and 5 MiB of gzip can be 100 MiB of text: it is not asked for over
    /// a connection that such an answer would end. A local browser is
    /// asked as before.
    #[tokio::test]
    async fn a_body_that_was_never_counted_is_asked_of_a_remote_browser_only_when_small_on_the_wire()
     {
        let server = ("203.0.113.9", 443);
        const COMPRESSED: &str = "https://cdn.example/compressed.js";
        const EMPTY: &str = "https://cdn.example/empty.js";

        let (cdp, _events, mut browser) = connected();
        let sink = Arc::new(Kept::default());
        let mut recorder = remote_recorder(sink.clone(), &cdp, false);
        tab_loads(&mut recorder, &cdp, "T.1", COMPRESSED, server);
        tab_finishes_after(&mut recorder, &cdp, "T.1", 5 * MIB);
        tab_loads(&mut recorder, &cdp, "T.2", EMPTY, server);
        tab_finishes_after(&mut recorder, &cdp, "T.2", 600);

        let read = browser.received().await;
        assert_eq!(
            (&read["method"], &read["params"]["requestId"]),
            (&json!("Network.getResponseBody"), &json!("T.2"))
        );
        browser
            .say(json!({ "id": read["id"], "result": { "body": "", "base64Encoded": false } }))
            .await;
        while recorder.has_pending_bodies() {
            recorder.store_next_body().await;
        }
        assert_eq!(
            sink.body_of(COMPRESSED),
            CapturedBody::TooLargeForRemote {
                total_bytes: 5 * MIB as u64
            }
        );
        assert!(!matches!(
            sink.body_of(EMPTY),
            CapturedBody::TooLargeForRemote { .. }
        ));
        assert_nothing_else_was_asked(&cdp, &mut browser).await;

        // The debugging pipe of a local browser carries an answer of any size.
        let (cdp, _events, mut browser) = connected();
        let mut recorder = recorder_on_a_page(Arc::new(Kept::default()), &cdp);
        tab_loads(&mut recorder, &cdp, "T.1", COMPRESSED, server);
        tab_finishes_after(&mut recorder, &cdp, "T.1", 5 * MIB);
        let read = browser.received().await;
        assert_eq!(
            (&read["method"], &read["params"]["requestId"]),
            (&json!("Network.getResponseBody"), &json!("T.1"))
        );
    }

    #[tokio::test]
    async fn an_inline_script_too_large_for_one_answer_is_not_asked_of_a_remote_browser() {
        let (cdp, _events, mut browser) = connected();
        let mut recorder = remote_recorder(Arc::new(Kept::default()), &cdp, false);
        let parsed = |script_id: &str, length: usize| {
            event(
                "Debugger.scriptParsed",
                json!({
                    "scriptId": script_id, "url": "", "length": length,
                    "startLine": 0, "startColumn": 0,
                    "executionContextAuxData": { "isDefault": true, "frameId": "main" },
                }),
            )
        };

        recorder.observe(&parsed("1", 9 * MIB), &cdp, "tab");
        recorder.observe(&parsed("2", 5_000), &cdp, "tab");

        let asked = browser.received().await;
        assert_eq!(
            (&asked["method"], &asked["params"]["scriptId"]),
            (&json!("Debugger.getScriptSource"), &json!("2"))
        );
        assert_nothing_else_was_asked(&cdp, &mut browser).await;
    }

    /// Browserbase runs scripts of its own in the page's world of every
    /// document. They are counted and kept out of the scan's scripts; the
    /// same text in a local browser would be the page's own.
    #[tokio::test]
    async fn the_services_own_scripts_are_not_the_pages() {
        let parsed = |script_id: &str| {
            event(
                "Debugger.scriptParsed",
                json!({
                    "scriptId": script_id, "url": "", "length": 40_000,
                    "startLine": 0, "startColumn": 0,
                    "executionContextAuxData": { "isDefault": true, "frameId": "main" },
                }),
            )
        };
        let services = format!(
            "(() => {{\n      /* browserbase:stealth-init-script */\n\"use strict\";\n{}\n    }})();",
            "var solved = true;".repeat(20)
        );
        let pages = format!("(() => {{ {} }})();", "window.adsReady = true;".repeat(20));
        for hosted_by_service in [true, false] {
            let (cdp, _events, mut browser) = connected();
            let mut recorder = remote_recorder(Arc::new(Kept::default()), &cdp, false);
            if !hosted_by_service {
                recorder = recorder.linked_by(BrowserLink::LOCAL);
            }
            for (script_id, source) in [("1", &services), ("2", &pages)] {
                recorder.observe(&parsed(script_id), &cdp, "tab");
                let asked = browser.received().await;
                assert_eq!(asked["method"], "Debugger.getScriptSource");
                browser
                    .say(json!({ "id": asked["id"], "result": { "scriptSource": source } }))
                    .await;
                recorder.store_next_body().await;
            }

            let kept = recorder.scripts.into_captured(|_| (0, 0.0));
            if hosted_by_service {
                assert_eq!(kept.len(), 1);
                assert_eq!(kept[0].body, pages);
                assert_eq!(
                    recorder.service_scripts,
                    ServiceScripts {
                        count: 1,
                        chars: services.chars().count() as u64
                    }
                );
            } else {
                assert_eq!(kept.len(), 2);
                assert_eq!(recorder.service_scripts, ServiceScripts::default());
            }
        }
    }

    /// Behind a proxy of Browserbase every request reports the proxy as
    /// its server. That is not where the request went, so it is not said.
    #[tokio::test]
    async fn the_address_of_browserbases_proxy_is_not_handed_over_as_the_servers() {
        assert!(is_service_proxy("10.255.255.1:8091"));
        assert!(is_service_proxy("10.255.255.1"));
        assert!(!is_service_proxy("10.255.255.10:8091"));
        assert!(!is_service_proxy("203.0.113.9:443"));

        for (behind_service_proxy, server, handed_over) in [
            (true, ("10.255.255.1", 8091), None),
            (true, ("203.0.113.9", 443), Some("203.0.113.9:443")),
            (false, ("10.255.255.1", 8091), Some("10.255.255.1:8091")),
        ] {
            let (cdp, _events, mut browser) = connected();
            let sink = Arc::new(Kept::default());
            let mut recorder = remote_recorder(sink.clone(), &cdp, behind_service_proxy);
            tab_loads(
                &mut recorder,
                &cdp,
                "T.1",
                "https://cdn.example/app.js",
                server,
            );
            tab_receives(&mut recorder, &cdp, "T.1", "let a = 1;", false);
            tab_finishes(&mut recorder, &cdp, "T.1");
            let read = browser.received().await;
            browser
                .say(json!({ "id": read["id"], "result": { "body": "let a = 1;", "base64Encoded": false } }))
                .await;
            while recorder.has_pending_bodies() {
                recorder.store_next_body().await;
            }

            let records = sink.0.lock().unwrap();
            assert_eq!(
                records[0].remote_address.as_deref(),
                handed_over,
                "behind the proxy: {behind_service_proxy}, server {server:?}"
            );
        }
    }
}
