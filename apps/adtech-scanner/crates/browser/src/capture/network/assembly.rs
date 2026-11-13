//! Assembles the browser's `Network` events into one record per request.
//! No I/O: events and answers go in, what to ask the browser and the
//! finished records come out.
//!
//! What Chrome does, and what follows from it here:
//!
//! - A request id is unique in the whole browser, and the events of one
//!   request arrive on several sessions: a worker's script is announced on
//!   the page's session and finishes on the worker's, an out-of-process
//!   iframe's document is announced on its parent's and finishes on its
//!   own, and the `ExtraInfo` events of a worker's fetches come on the
//!   session of its frame. So requests are keyed by id alone, and a body is
//!   asked for on the session that reported `loadingFinished`: no other has
//!   it.
//! - Every redirect hop reuses the id. Each hop is a record of its own; the
//!   `ExtraInfo` events are matched to hops by position, the way Chrome's
//!   own DevTools do it.
//! - A frame or worker that goes away takes its session with it, and with
//!   the session what it loaded: a body asked for at `loadingFinished` is
//!   then asked for too late, however fast. So a frame or worker is asked,
//!   when it announces a request, to send the body along as it arrives
//!   (`Network.streamResourceContent`). That copy stands in for the body
//!   when the target is gone before it could hand the body over.
//! - The connection to a remote browser carries no message beyond a size,
//!   so a body that could make a larger answer is never asked for
//!   (`reply.rs`). A response of the tab that grows towards that size is
//!   asked to be sent along too, from then on: the copy is all there will
//!   be of it.
//! - `ExtraInfo` may come before `requestWillBeSent` or after
//!   `loadingFinished`, and a preflight may end before the request it
//!   clears the way for is announced. A request that has ended waits a
//!   moment for what is still missing, unless it carries a body.

use std::collections::{HashMap, HashSet, VecDeque};

use adscan_model::{
    CapturedBody, CapturedPostData, InitiatorFrame, InitiatorKind, NetworkInitiator, NetworkRecord,
    RawHeader, RequestContext, RequestEnd, ResourceKind, ResourceTiming, ServedFrom,
};
use base64::Engine;
use serde_json::Value;

use super::super::bid_endpoints::{BidRequest, bid_request};
use super::super::scripts::{initiator_url, without_embedded_payload};
use super::fields::{
    CallFrame, call_frames, header, headers, initiator_kind, position, post_data, resource_kind,
    timing,
};

/// How long an ended request waits for late `ExtraInfo` events, for the
/// request its preflight belongs to, and a navigation for its commit.
const SETTLE_GRACE_MS: f64 = 1_000.0;
/// `ExtraInfo` of a request that is never announced is forgotten after this.
const ORPHAN_PATIENCE_MS: f64 = 10_000.0;
/// A page that opens requests without end must not exhaust memory.
const MAX_OPEN_REQUESTS: usize = 10_000;
/// Request bodies of requests that are still open, together.
const MAX_HELD_POST_BYTES: usize = 32 * 1024 * 1024;
/// A body larger than this is not asked for separately: the answer would
/// have to be held whole to keep its first megabyte.
const MAX_POST_BYTES_TO_ASK_FOR: u64 = 8 * 1024 * 1024;
/// Ids of ended requests that are remembered, so that their late events do
/// not look like new requests.
const REMEMBERED_ENDED: usize = 4_096;
/// Copies of the response bodies that frames and workers are still loading
/// or have not handed over yet, together. A copy that does not fit is given
/// up; its body is then only to be had from the target itself.
const MAX_COPIED_BYTES: usize = 32 * 1024 * 1024;

/// What the way to the browser asks of the assembly. The default is a
/// local browser's: nothing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Limits {
    /// A response that has received this many bytes is asked to be sent
    /// along while it loads, whichever target loads it.
    pub copy_bodies_from: Option<u64>,
    /// A request body is only asked for when its length is known to be
    /// one that may be asked for.
    pub post_length_must_be_known: bool,
}

/// What kind of target a session belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Reporter {
    Tab,
    Iframe,
    Worker,
    ServiceWorker,
}

/// Where an event was reported, and what the visit was showing then.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Source<'a> {
    pub session: &'a str,
    pub reporter: Reporter,
    /// Target of the session, unless it is a tab's own.
    pub target_id: Option<&'a str>,
    /// Main frame of the tab, for a tab's own session.
    pub main_frame_id: Option<&'a str>,
    /// The session belongs to the recorded tab, and that shows a page of
    /// the scanned site.
    pub recording: bool,
    /// Address of the recorded tab's document.
    pub page_url: &'a str,
    /// Epoch ms.
    pub now_ms: f64,
}

/// What the assembler wants done, or has finished.
#[derive(Debug)]
pub(crate) enum Output {
    /// The request has a body that was not sent along: ask
    /// `Network.getRequestPostData` and answer with
    /// [`Assembler::post_data_settled`].
    PostDataWanted {
        seq: u32,
        session: String,
        request_id: String,
    },
    /// A frame or worker announced a request whose answer may be kept, or
    /// a response grew towards what cannot be asked for: ask
    /// `Network.streamResourceContent` and pass what it answers to
    /// [`Assembler::body_copy_started`].
    BodyCopyWanted {
        session: String,
        request_id: String,
    },
    /// A response finished loading. Its body can be read now, and must be
    /// answered for with [`Assembler::body_settled`] either way.
    Finished(Finished),
    Ended(Box<Ended>),
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Finished {
    pub seq: u32,
    /// The session that has the body.
    pub session: String,
    pub request_id: String,
    pub url: String,
    pub kind: ResourceKind,
    pub mime_type: Option<String>,
    pub status: Option<u16>,
    /// Decoded bytes received, else the encoded length.
    pub expected_bytes: u64,
    pub encoded_bytes: u64,
    pub bid: Option<BidRequest>,
    /// URL of the script or document that made the request.
    pub initiator_url: Option<String>,
    /// Reported by a tab's own session: the response is in the page's
    /// process, like every script the page itself runs.
    pub in_tab: bool,
    /// The request loads the document of a tab's main frame.
    pub main_document: bool,
    /// `data:` and `blob:` addresses are their own content.
    pub has_address_on_the_network: bool,
    pub finished_epoch_ms: f64,
}

#[derive(Debug, PartialEq)]
enum Post {
    None,
    Sent(CapturedPostData),
    Asked,
    Missing,
}

#[derive(Debug, PartialEq)]
enum Body {
    /// Not finished, or ended without anything to read.
    None,
    Asked,
    Settled(CapturedBody),
}

/// The response body as the browser sends it along while it loads.
#[derive(Debug, Default)]
struct BodyCopy {
    /// The browser agreed to send the body along and handed over what had
    /// arrived until then.
    started: bool,
    bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq)]
enum Preflight {
    No,
    /// The preflight's request is announced by this request id, not yet seen.
    Awaiting(String),
    Of(u32),
}

#[derive(Debug)]
struct Response {
    status: Option<u16>,
    mime_type: Option<String>,
    protocol: Option<String>,
    remote_address: Option<String>,
    headers: Vec<RawHeader>,
    served_from: ServedFrom,
    at_epoch_ms: f64,
    /// Monotonic ms the browser started work on the request, and the phases.
    timing: Option<(f64, ResourceTiming)>,
}

#[derive(Debug)]
struct End {
    how: RequestEnd,
    at_epoch_ms: f64,
    encoded_bytes: u64,
    /// When the assembler learnt of it, epoch ms.
    seen_ms: f64,
}

/// One request, or one hop of a redirected one, as far as it is known.
#[derive(Debug)]
struct Flight {
    seq: u32,
    /// Position in its chain of redirects.
    hop: usize,
    session: String,
    context: RequestContext,
    frame_id: Option<String>,
    target_id: Option<String>,
    method: String,
    url: String,
    kind: ResourceKind,
    priority: Option<String>,
    request_headers: Vec<RawHeader>,
    post: Post,
    initiator_kind: InitiatorKind,
    initiator_address: Option<String>,
    initiator_line: Option<u32>,
    initiator_column: Option<u32>,
    stack: Vec<CallFrame>,
    /// URL of the script or document behind the request.
    made_by: Option<String>,
    redirect_from: Option<u32>,
    preflight: Preflight,
    bid: Option<BidRequest>,
    main_document: bool,
    started_epoch_ms: f64,
    /// The browser's monotonic clock at the start, in ms. A WebSocket has
    /// none until its handshake is sent.
    started_mono_ms: Option<f64>,
    response: Option<Response>,
    memory_cache: bool,
    decoded_bytes: u64,
    end: Option<End>,
    body: Body,
    copy: Option<BodyCopy>,
    /// The browser was asked to send the body along, whatever came of it.
    copy_asked: bool,
}

impl Flight {
    /// Epoch ms of a later event of the request, from the browser's
    /// monotonic `timestamp` in seconds.
    fn epoch_of(&self, timestamp: &Value, now_ms: f64) -> f64 {
        match (self.started_mono_ms, timestamp.as_f64()) {
            (Some(started), Some(seconds)) => self.started_epoch_ms + seconds * 1000.0 - started,
            _ => now_ms,
        }
    }

    /// The response `response` (a CDP `Network.Response`), reported by an
    /// event with this `timestamp`. Its headers arrived when the network
    /// says so; the event itself is sent when the page gets round to it,
    /// which can be after the whole body has loaded.
    fn response(&self, response: &Value, timestamp: &Value, now_ms: f64) -> Response {
        let timing = timing(&response["timing"]);
        let headers_at =
            timing
                .zip(self.started_mono_ms)
                .and_then(|((request_time_ms, phases), started)| {
                    let after_start = request_time_ms + phases.receive_headers_end? - started;
                    Some(self.started_epoch_ms + after_start.max(0.0))
                });
        response_of(
            response,
            headers_at.unwrap_or_else(|| self.epoch_of(timestamp, now_ms)),
            timing,
        )
    }

    /// Chrome stamps the announcement of a CORS preflight when the browser
    /// process hears of it, which can be after the network service sent it
    /// and even after it finished (measured on Chromium 1243: up to a few
    /// ms). The network's own start is the true one then.
    fn start_no_later_than_the_network(&mut self, response: &Value) {
        if self.kind != ResourceKind::Preflight {
            return;
        }
        let (Some(announced), Some((request_time_ms, _))) =
            (self.started_mono_ms, timing(&response["timing"]))
        else {
            return;
        };
        let announced_late_by = announced - request_time_ms;
        if announced_late_by > 0.0 {
            self.started_mono_ms = Some(request_time_ms);
            self.started_epoch_ms -= announced_late_by;
        }
    }

    fn is_complete(&self) -> bool {
        self.end.is_some() && self.body != Body::Asked && self.post != Post::Asked
    }

    fn carries_a_body(&self) -> bool {
        matches!(self.body, Body::Settled(CapturedBody::Read { .. }))
    }

    fn owes_a_record(&self) -> bool {
        self.body == Body::Asked || self.carries_a_body()
    }

    fn held_post_bytes(&self) -> usize {
        match &self.post {
            Post::Sent(data) => data.bytes.len(),
            _ => 0,
        }
    }

    fn copied_bytes(&self) -> usize {
        self.copy.as_ref().map_or(0, |copy| copy.bytes.len())
    }

    /// Whether a copy of the answer is worth asking for: it may be a body
    /// that is kept, or a bidder's answer.
    fn wants_its_body_copied(&self) -> bool {
        self.bid.is_some() || may_have_a_kept_body(self.kind)
    }

    /// Adds a piece of the body, as base64, to the copy. A copy that would
    /// take more than `room` bytes, or cannot be read, is given up.
    fn copy_more(&mut self, piece: &str, room: usize) {
        let Some(copy) = &mut self.copy else {
            return;
        };
        let fits = base64::decoded_len_estimate(piece.len()) <= room;
        let base64 = base64::engine::general_purpose::STANDARD;
        if !(fits && base64.decode_vec(piece, &mut copy.bytes).is_ok()) {
            self.copy = None;
        }
    }

    fn end(&mut self, how: RequestEnd, at_epoch_ms: f64, encoded_bytes: u64, now_ms: f64) {
        self.end = Some(End {
            how,
            // Without a response there is no network start to correct a
            // late announcement with; the request still did not end before
            // it began.
            at_epoch_ms: at_epoch_ms.max(self.started_epoch_ms),
            encoded_bytes,
            seen_ms: now_ms,
        });
    }
}

/// A navigation of a tab's main frame is held until the document commits:
/// only then is it known which page view it opens, and whether that is a
/// page of the scanned site.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Hold {
    /// Asked for while the recorded tab showed the scanned site.
    started_recording: bool,
}

/// The hops of one request id, and the `ExtraInfo` events seen for it.
#[derive(Debug, Default)]
struct Chain {
    /// Hops not handed over yet, oldest first.
    hops: VecDeque<Flight>,
    hops_seen: usize,
    /// By hop. `None` where the browser said there is none.
    request_extras: Vec<Option<Vec<RawHeader>>>,
    response_extras: Vec<Option<Vec<RawHeader>>>,
    /// The last hop has ended, and not with a redirect.
    closed: bool,
    hold: Option<Hold>,
    /// The first hop that was followed is not the request's first: hops
    /// before it went unseen (the tab was on another site, or too many
    /// requests were open), so `ExtraInfo` cannot be counted off against
    /// the hops any more. Rather no cookies on a record than another
    /// request's.
    extras_misaligned: bool,
}

impl Chain {
    /// The hop that is still loading.
    fn current(&mut self) -> Option<&mut Flight> {
        self.hops.back_mut().filter(|flight| flight.end.is_none())
    }

    /// The browser reported a response of hop `hop` that has no `ExtraInfo`
    /// events. Any that did arrive belong to later hops.
    fn hop_has_no_extra_info(&mut self, hop: usize) {
        for extras in [&mut self.request_extras, &mut self.response_extras] {
            extras.insert(hop.min(extras.len()), None);
        }
    }

    fn extras_are_known(&self, flight: &Flight) -> bool {
        self.extras_misaligned
            || (self.request_extras.len() > flight.hop
                && (self.response_extras.len() > flight.hop || flight.response.is_none()))
    }

    /// The `ExtraInfo` headers of hop `hop`, request and response.
    fn take_extras(&mut self, hop: usize) -> (Option<Vec<RawHeader>>, Option<Vec<RawHeader>>) {
        if self.extras_misaligned {
            return (None, None);
        }
        let take =
            |extras: &mut Vec<Option<Vec<RawHeader>>>| extras.get_mut(hop).and_then(Option::take);
        (
            take(&mut self.request_extras),
            take(&mut self.response_extras),
        )
    }
}

/// A request as it ended, ready to be placed on its page view.
#[derive(Debug)]
pub(crate) struct Ended {
    flight: Flight,
    request_extra: Option<Vec<RawHeader>>,
    response_extra: Option<Vec<RawHeader>>,
}

impl Ended {
    pub(crate) fn seq(&self) -> u32 {
        self.flight.seq
    }

    /// Made in an out-of-process iframe or a worker, not in the tab's own
    /// process.
    pub(crate) fn is_of_child_target(&self) -> bool {
        self.flight.target_id.is_some()
    }

    /// A bid request that got no answer: the bidder, when it failed (epoch
    /// ms) and why.
    pub(crate) fn failed_bid(&self) -> Option<(&BidRequest, f64, String)> {
        let end = self.flight.end.as_ref()?;
        let RequestEnd::Failed {
            error,
            blocked_reason,
            ..
        } = &end.how
        else {
            return None;
        };
        let failure = match blocked_reason {
            Some(blocked_by) => format!("blocked ({blocked_by})"),
            None => error.clone(),
        };
        Some((self.flight.bid.as_ref()?, end.at_epoch_ms, failure))
    }

    /// The record. `place` turns an epoch time into the page view showing
    /// then and the time since it started; `kept_script` names the captured
    /// script a stack frame runs in, given the session that reported the
    /// frame.
    pub(crate) fn into_record(
        self,
        place: impl Fn(f64) -> (u32, f64),
        kept_script: impl Fn(&str, &CallFrame) -> Option<String>,
    ) -> NetworkRecord {
        let flight = self.flight;
        let (page, started_at_ms) = place(flight.started_epoch_ms);
        let since_page_start = |epoch_ms: f64| started_at_ms + epoch_ms - flight.started_epoch_ms;
        let stack = flight
            .stack
            .iter()
            .map(|frame| InitiatorFrame {
                function: frame.function.clone(),
                url: frame.url.clone(),
                script_url: kept_script(&flight.session, frame),
                line: frame.line,
                column: frame.column,
            })
            .collect();
        let (end, ended_at_ms, encoded_bytes) = match flight.end {
            Some(end) => (
                end.how,
                Some(since_page_start(end.at_epoch_ms)),
                end.encoded_bytes,
            ),
            None => (RequestEnd::Pending, None, 0),
        };
        let response = flight.response;
        let served_from = match response.as_ref().map(|response| response.served_from) {
            Some(ServedFrom::Network) | None if flight.memory_cache => ServedFrom::MemoryCache,
            Some(served_from) => served_from,
            None => ServedFrom::Network,
        };
        let timing = response.as_ref().and_then(|response| response.timing);
        let (post_data, post_data_missing) = match flight.post {
            Post::Sent(data) => (Some(data), false),
            Post::None => (None, false),
            Post::Asked | Post::Missing => (None, true),
        };
        NetworkRecord {
            seq: flight.seq,
            page,
            page_host: String::new(),
            context: flight.context,
            frame_id: flight.frame_id,
            target_id: flight.target_id,
            method: flight.method,
            url: flight.url,
            kind: flight.kind,
            priority: flight.priority,
            request_headers: self.request_extra.unwrap_or(flight.request_headers),
            post_data,
            post_data_missing,
            initiator: NetworkInitiator {
                kind: flight.initiator_kind,
                url: flight.initiator_address,
                line: flight.initiator_line,
                column: flight.initiator_column,
                stack,
            },
            redirect_from: flight.redirect_from,
            preflight_of: match flight.preflight {
                Preflight::Of(seq) => Some(seq),
                Preflight::No | Preflight::Awaiting(_) => None,
            },
            status: response.as_ref().and_then(|response| response.status),
            mime_type: response
                .as_ref()
                .and_then(|response| response.mime_type.clone()),
            protocol: response
                .as_ref()
                .and_then(|response| response.protocol.clone()),
            remote_address: response
                .as_ref()
                .and_then(|response| response.remote_address.clone()),
            response_at_ms: response.as_ref().map(|response| {
                let at = since_page_start(response.at_epoch_ms);
                ended_at_ms.map_or(at, |ended| at.min(ended))
            }),
            response_headers: self
                .response_extra
                .or(response.map(|response| response.headers))
                .unwrap_or_default(),
            served_from,
            started_at_ms,
            ended_at_ms,
            queued_ms: timing
                .zip(flight.started_mono_ms)
                .map(|((request_time_ms, _), started)| (request_time_ms - started).max(0.0)),
            timing: timing.map(|(_, phases)| phases),
            encoded_bytes,
            decoded_bytes: flight.decoded_bytes,
            end,
            bidder: flight.bid.map(|bid| bid.bidder.to_string()),
            body: match flight.body {
                Body::Settled(body) => body,
                Body::None => CapturedBody::Absent,
                Body::Asked => CapturedBody::Unavailable(
                    "the capture ended before the browser handed the body over".into(),
                ),
            },
        }
    }
}

#[derive(Debug, Default)]
pub(crate) struct Assembler {
    limits: Limits,
    chains: HashMap<String, Chain>,
    next_seq: u32,
    open_flights: usize,
    held_post_bytes: usize,
    copied_bytes: usize,
    /// Request id by `seq`, for answers that are awaited.
    bodies_asked: HashMap<u32, String>,
    posts_asked: HashMap<u32, String>,
    /// Preflight request ids by the id of the request they announce.
    preflights_awaiting: HashMap<String, Vec<String>>,
    /// Requests that ended and wait for something, with the epoch ms from
    /// which they no longer do. In that order.
    revisit: VecDeque<(f64, String)>,
    /// Request ids only `ExtraInfo` has named, with the epoch ms from which
    /// they are forgotten unless the request was announced. In that order.
    unannounced: VecDeque<(f64, String)>,
    ended: HashSet<String>,
    ended_order: VecDeque<String>,
    /// Requests left out because too many were open.
    overflowed: u32,
}

impl Assembler {
    pub(crate) fn within(limits: Limits) -> Self {
        Self {
            limits,
            ..Self::default()
        }
    }

    pub(crate) fn overflowed(&self) -> u32 {
        self.overflowed
    }

    /// Takes one `Network` event.
    pub(crate) fn observe(
        &mut self,
        source: &Source<'_>,
        method: &str,
        params: &Value,
    ) -> Vec<Output> {
        let mut out = Vec::new();
        let Some(request_id) = params["requestId"].as_str() else {
            return out;
        };
        if method == "Network.dataReceived" {
            // The busiest event by far, and it settles nothing.
            let room = MAX_COPIED_BYTES - self.copied_bytes;
            let copy_from = self.limits.copy_bodies_from;
            let Some(flight) = self.current(request_id) else {
                return out;
            };
            flight.decoded_bytes += params["dataLength"].as_u64().unwrap_or(0);
            let copied_before = flight.copied_bytes();
            if let Some(piece) = params["data"].as_str() {
                flight.copy_more(piece, room);
            }
            let copied = flight.copied_bytes();
            let grows_large = copy_from.is_some_and(|from| flight.decoded_bytes >= from);
            if grows_large && !flight.copy_asked && flight.wants_its_body_copied() {
                flight.copy_asked = true;
                flight.copy = Some(BodyCopy::default());
                out.push(Output::BodyCopyWanted {
                    session: source.session.to_string(),
                    request_id: request_id.to_string(),
                });
            }
            self.copied_bytes = self.copied_bytes - copied_before + copied;
            return out;
        }
        self.revisit_due(source.now_ms, &mut out);
        if self.ended.contains(request_id) {
            return out;
        }
        match method {
            "Network.requestWillBeSent" => self.on_request(source, request_id, params, &mut out),
            "Network.requestWillBeSentExtraInfo" => {
                let extra = Some(headers(&params["headers"]));
                self.chain_for_extras(request_id, source.now_ms)
                    .request_extras
                    .push(extra);
            }
            "Network.responseReceivedExtraInfo" => {
                let extra = Some(headers(&params["headers"]));
                self.chain_for_extras(request_id, source.now_ms)
                    .response_extras
                    .push(extra);
            }
            "Network.responseReceived" => self.on_response(request_id, params, source.now_ms),
            "Network.requestServedFromCache" => {
                if let Some(flight) = self.current(request_id) {
                    flight.memory_cache = true;
                }
            }
            "Network.loadingFinished" => self.on_finished(source, request_id, params, &mut out),
            "Network.loadingFailed" => self.on_failed(request_id, params, source.now_ms),
            "Network.webSocketCreated" => self.on_socket_created(source, request_id, params),
            "Network.webSocketWillSendHandshakeRequest" => {
                if let Some(flight) = self.current(request_id) {
                    flight.started_mono_ms = params["timestamp"].as_f64().map(|s| s * 1000.0);
                    if let Some(wall_seconds) = params["wallTime"].as_f64() {
                        flight.started_epoch_ms = wall_seconds * 1000.0;
                    }
                    flight.request_headers = headers(&params["request"]["headers"]);
                }
            }
            "Network.webSocketHandshakeResponseReceived" => {
                let now_ms = source.now_ms;
                if let Some(flight) = self.current(request_id) {
                    let response = &params["response"];
                    let at = flight.epoch_of(&params["timestamp"], now_ms);
                    flight.response = Some(response_of(response, at, None));
                }
            }
            "Network.webSocketClosed" => {
                let now_ms = source.now_ms;
                if let Some(chain) = self.chains.get_mut(request_id)
                    && let Some(flight) = chain.current()
                {
                    let at = flight.epoch_of(&params["timestamp"], now_ms);
                    flight.end(RequestEnd::Finished, at, 0, now_ms);
                    chain.closed = true;
                    chain.hop_has_no_extra_info(0);
                }
            }
            _ => return out,
        }
        self.settle(request_id, source.now_ms, &mut out);
        out
    }

    /// The answer to [`Output::BodyCopyWanted`]: the browser sends the body
    /// of the request along from here on, and `already_arrived` (base64) is
    /// what it had of it until then. Pieces that were sent along may have
    /// been seen before this answer.
    pub(crate) fn body_copy_started(&mut self, request_id: &str, already_arrived: &str) {
        let room = MAX_COPIED_BYTES - self.copied_bytes;
        // A redirected request carries its copy on to the hop that is answered.
        let Some(flight) = self
            .chains
            .get_mut(request_id)
            .and_then(|chain| chain.hops.back_mut())
        else {
            return;
        };
        let copied_before = flight.copied_bytes();
        if let Some(copy) = &mut flight.copy {
            let since = std::mem::take(&mut copy.bytes);
            copy.started = true;
            flight.copy_more(already_arrived, room);
            if let Some(copy) = &mut flight.copy {
                copy.bytes.extend_from_slice(&since);
            }
        }
        let copied = flight.copied_bytes();
        self.copied_bytes = self.copied_bytes - copied_before + copied;
    }

    /// The body of a finished response whose frame or worker did not hand
    /// it over, when the browser sent all of it along while it loaded: the
    /// bytes as they arrived, content encoding undone.
    pub(crate) fn take_body_copy(&mut self, seq: u32) -> Option<Vec<u8>> {
        let request_id = self.bodies_asked.get(&seq)?.clone();
        let flight = self.flight(&request_id, seq)?;
        let whole_body_expected = flight.decoded_bytes;
        let copy = flight.copy.take()?;
        self.copied_bytes -= copy.bytes.len();
        let is_whole = copy.started && copy.bytes.len() as u64 == whole_body_expected;
        is_whole.then_some(copy.bytes)
    }

    /// The answer to [`Output::Finished`]: the body as it is handed over.
    pub(crate) fn body_settled(
        &mut self,
        seq: u32,
        body: CapturedBody,
        now_ms: f64,
    ) -> Vec<Output> {
        let mut out = Vec::new();
        let Some(request_id) = self.bodies_asked.remove(&seq) else {
            return out;
        };
        if let Some(flight) = self.flight(&request_id, seq) {
            flight.body = Body::Settled(body);
            let copied = flight.copied_bytes();
            flight.copy = None;
            self.copied_bytes -= copied;
        }
        self.settle(&request_id, now_ms, &mut out);
        out
    }

    /// The answer to [`Output::PostDataWanted`]: the request body, or `None`
    /// when the browser did not have it.
    pub(crate) fn post_data_settled(
        &mut self,
        seq: u32,
        text: Option<&str>,
        now_ms: f64,
    ) -> Vec<Output> {
        let mut out = Vec::new();
        let Some(request_id) = self.posts_asked.remove(&seq) else {
            return out;
        };
        let room = MAX_HELD_POST_BYTES.saturating_sub(self.held_post_bytes);
        let post = text
            .map(post_data)
            .filter(|data| data.bytes.len() <= room)
            .map_or(Post::Missing, Post::Sent);
        if let Some(flight) = self.flight(&request_id, seq) {
            flight.post = post;
            let held = flight.held_post_bytes();
            self.held_post_bytes += held;
        }
        self.settle(&request_id, now_ms, &mut out);
        out
    }

    /// The main frame of the recorded tab committed the document that
    /// request `loader_id` loaded. `on_site`: it is a page of the scanned
    /// site.
    pub(crate) fn navigation_committed(
        &mut self,
        loader_id: &str,
        on_site: bool,
        now_ms: f64,
    ) -> Vec<Output> {
        let mut out = Vec::new();
        let Some(chain) = self.chains.get_mut(loader_id) else {
            return out;
        };
        let Some(hold) = chain.hold.take() else {
            return out;
        };
        if on_site || hold.started_recording || chain.hops.iter().any(Flight::owes_a_record) {
            self.settle(loader_id, now_ms, &mut out);
        } else {
            self.forget(loader_id);
        }
        out
    }

    /// When the navigation that request `loader_id` loads was asked for,
    /// epoch ms, while it waits for its document to commit.
    pub(crate) fn navigation_started_at(&self, loader_id: &str) -> Option<f64> {
        let chain = self
            .chains
            .get(loader_id)
            .filter(|chain| chain.hold.is_some())?;
        chain.hops.front().map(|first| first.started_epoch_ms)
    }

    /// The capture is over: every request that was not handed over yet, in
    /// the order they were first seen. What is still loading ends as pending.
    pub(crate) fn finish(&mut self) -> Vec<Ended> {
        let mut remaining: Vec<Ended> = Vec::new();
        for (_, mut chain) in self.chains.drain() {
            let never_shown = chain.hold.is_some_and(|hold| !hold.started_recording)
                && !chain.hops.iter().any(Flight::owes_a_record);
            if never_shown {
                continue;
            }
            while let Some(flight) = chain.hops.pop_front() {
                let (request_extra, response_extra) = chain.take_extras(flight.hop);
                remaining.push(Ended {
                    request_extra,
                    response_extra,
                    flight,
                });
            }
        }
        remaining.sort_by_key(Ended::seq);
        *self = Self {
            next_seq: self.next_seq,
            overflowed: self.overflowed,
            ..Self::default()
        };
        remaining
    }

    fn current(&mut self, request_id: &str) -> Option<&mut Flight> {
        self.chains.get_mut(request_id)?.current()
    }

    fn flight(&mut self, request_id: &str, seq: u32) -> Option<&mut Flight> {
        self.chains
            .get_mut(request_id)?
            .hops
            .iter_mut()
            .find(|flight| flight.seq == seq)
    }

    /// The chain of a request whose `ExtraInfo` just arrived. It may be the
    /// first that is heard of the request.
    fn chain_for_extras(&mut self, request_id: &str, now_ms: f64) -> &mut Chain {
        if !self.chains.contains_key(request_id) {
            self.unannounced
                .push_back((now_ms + ORPHAN_PATIENCE_MS, request_id.to_string()));
        }
        self.chains.entry(request_id.to_string()).or_default()
    }

    fn on_request(
        &mut self,
        source: &Source<'_>,
        request_id: &str,
        params: &Value,
        out: &mut Vec<Output>,
    ) {
        let request = &params["request"];
        let Some(url) = request["url"].as_str() else {
            return;
        };
        let main_document = params["type"] == "Document"
            && source
                .main_frame_id
                .is_some_and(|main| params["frameId"] == main);
        let started_mono_ms = params["timestamp"].as_f64().map(|seconds| seconds * 1000.0);
        let started_epoch_ms = params["wallTime"]
            .as_f64()
            .map_or(source.now_ms, |seconds| seconds * 1000.0);

        let redirect = &params["redirectResponse"];
        let is_loading = self.current(request_id).is_some();
        if is_loading && redirect.is_null() {
            // The same request, announced a second time.
            return;
        }
        if !is_loading {
            let is_over = self
                .chains
                .get(request_id)
                .is_some_and(|chain| chain.closed);
            if is_over || !(source.recording || main_document) {
                return;
            }
            if self.open_flights >= MAX_OPEN_REQUESTS {
                self.overflowed += 1;
                return;
            }
        }
        let chain = self.chains.entry(request_id.to_string()).or_default();
        let mut redirect_from = None;
        let mut copy = None;
        let mut copy_asked = false;
        match chain.current() {
            Some(previous) => {
                copy = previous.copy.take();
                copy_asked = previous.copy_asked;
                let at = previous.epoch_of(&params["timestamp"], source.now_ms);
                let encoded = redirect["encodedDataLength"].as_f64().unwrap_or(0.0) as u64;
                previous.response =
                    Some(previous.response(redirect, &params["timestamp"], source.now_ms));
                previous.end(RequestEnd::Redirected, at, encoded, source.now_ms);
                redirect_from = Some(previous.seq);
                let hop = previous.hop;
                if params["redirectHasExtraInfo"] == false {
                    chain.hop_has_no_extra_info(hop);
                }
                self.revisit
                    .push_back((source.now_ms + SETTLE_GRACE_MS, request_id.to_string()));
            }
            None => {
                chain.extras_misaligned |= !redirect.is_null();
                if main_document {
                    chain.hold = Some(Hold {
                        started_recording: source.recording,
                    });
                }
            }
        }

        let initiator = &params["initiator"];
        let initiator_kind = initiator_kind(initiator["type"].as_str().unwrap_or_default());
        let is_preflight = initiator_kind == InitiatorKind::Preflight;
        let kind = if is_preflight {
            ResourceKind::Preflight
        } else {
            resource_kind(params["type"].as_str().unwrap_or_default())
        };
        let frame_id = params["frameId"]
            .as_str()
            .filter(|id| !id.is_empty())
            .map(str::to_string);
        let context = match source.reporter {
            Reporter::Tab => match (&frame_id, source.main_frame_id) {
                (Some(frame), Some(main)) if frame != main => RequestContext::Iframe,
                _ => RequestContext::Page,
            },
            Reporter::Iframe => RequestContext::Iframe,
            Reporter::Worker => RequestContext::Worker,
            Reporter::ServiceWorker => RequestContext::ServiceWorker,
        };
        // A CORS preflight goes to the same URL and is not the bid request.
        let bid = bid_request(url, source.page_url).filter(|_| !is_preflight);
        // The tab outlives what it loads; a frame or worker may not.
        let may_go_with_its_body = source.reporter != Reporter::Tab && redirect_from.is_none();
        if may_go_with_its_body && (bid.is_some() || may_have_a_kept_body(kind)) {
            copy = Some(BodyCopy::default());
            copy_asked = true;
            out.push(Output::BodyCopyWanted {
                session: source.session.to_string(),
                request_id: request_id.to_string(),
            });
        }
        let request_headers = headers(&request["headers"]);
        let known_length = chain
            .request_extras
            .get(chain.hops_seen)
            .and_then(Option::as_deref)
            .and_then(|extra| header(extra, "content-length"))
            .or_else(|| header(&request_headers, "content-length"))
            .and_then(|length| length.parse::<u64>().ok());
        let seq = self.next_seq;
        let post = match request["postData"].as_str() {
            Some(text) => {
                let data = post_data(text);
                if self.held_post_bytes + data.bytes.len() <= MAX_HELD_POST_BYTES {
                    Post::Sent(data)
                } else {
                    Post::Missing
                }
            }
            None if request["hasPostData"] != true => Post::None,
            None if known_length.is_some_and(|length| length > MAX_POST_BYTES_TO_ASK_FOR) => {
                Post::Missing
            }
            None if known_length.is_none() && self.limits.post_length_must_be_known => {
                Post::Missing
            }
            None => Post::Asked,
        };
        let flight = Flight {
            seq,
            hop: chain.hops_seen,
            session: source.session.to_string(),
            context,
            frame_id,
            target_id: source.target_id.map(str::to_string),
            method: request["method"].as_str().unwrap_or("GET").to_string(),
            url: without_embedded_payload(url.to_string()),
            kind,
            priority: request["initialPriority"].as_str().map(str::to_string),
            request_headers,
            post,
            initiator_kind,
            initiator_address: initiator["url"]
                .as_str()
                .filter(|url| !url.is_empty())
                .map(|url| without_embedded_payload(url.to_string())),
            initiator_line: position(&initiator["lineNumber"]),
            initiator_column: position(&initiator["columnNumber"]),
            stack: call_frames(&initiator["stack"]),
            made_by: initiator_url(initiator)
                .or_else(|| params["documentURL"].as_str().map(str::to_string)),
            redirect_from,
            preflight: match initiator["requestId"].as_str() {
                Some(announced) if is_preflight => Preflight::Awaiting(announced.to_string()),
                _ => Preflight::No,
            },
            bid,
            main_document,
            started_epoch_ms,
            started_mono_ms,
            response: None,
            memory_cache: false,
            decoded_bytes: 0,
            end: None,
            body: Body::None,
            copy,
            copy_asked,
        };
        self.next_seq += 1;
        self.open_flights += 1;
        self.held_post_bytes += flight.held_post_bytes();
        if flight.post == Post::Asked {
            self.posts_asked.insert(seq, request_id.to_string());
            out.push(Output::PostDataWanted {
                seq,
                session: source.session.to_string(),
                request_id: request_id.to_string(),
            });
        }
        let is_first_hop = chain.hops_seen == 0;
        chain.hops_seen += 1;
        chain.hops.push_back(flight);

        if is_preflight {
            self.link_preflight(request_id, seq);
        }
        if is_first_hop && let Some(preflights) = self.preflights_awaiting.remove(request_id) {
            for preflight_id in preflights {
                if let Some(preflight) = self
                    .chains
                    .get_mut(&preflight_id)
                    .and_then(|chain| chain.hops.back_mut())
                {
                    preflight.preflight = Preflight::Of(seq);
                }
                self.settle(&preflight_id, source.now_ms, out);
            }
        }
    }

    /// Finds the request a preflight announces, or notes that it is awaited.
    fn link_preflight(&mut self, preflight_id: &str, preflight_seq: u32) {
        let Some(Preflight::Awaiting(announced)) = self
            .flight(preflight_id, preflight_seq)
            .map(|flight| flight.preflight.clone())
        else {
            return;
        };
        let known = self
            .chains
            .get(&announced)
            .and_then(|chain| chain.hops.back())
            .map(|flight| flight.seq);
        match known {
            Some(seq) => {
                if let Some(flight) = self.flight(preflight_id, preflight_seq) {
                    flight.preflight = Preflight::Of(seq);
                }
            }
            None => self
                .preflights_awaiting
                .entry(announced)
                .or_default()
                .push(preflight_id.to_string()),
        }
    }

    fn on_response(&mut self, request_id: &str, params: &Value, now_ms: f64) {
        let Some(chain) = self.chains.get_mut(request_id) else {
            return;
        };
        let Some(flight) = chain.current() else {
            return;
        };
        flight.start_no_later_than_the_network(&params["response"]);
        flight.response = Some(flight.response(&params["response"], &params["timestamp"], now_ms));
        // The session that answers may know less about the request than the
        // one that announced it: a worker's script is "other" to the worker.
        let answered_as = resource_kind(params["type"].as_str().unwrap_or_default());
        if flight.kind == ResourceKind::Other {
            flight.kind = answered_as;
        }
        let hop = flight.hop;
        if params["hasExtraInfo"] == false {
            chain.hop_has_no_extra_info(hop);
        }
    }

    fn on_finished(
        &mut self,
        source: &Source<'_>,
        request_id: &str,
        params: &Value,
        out: &mut Vec<Output>,
    ) {
        let Some(chain) = self.chains.get_mut(request_id) else {
            return;
        };
        let Some(flight) = chain.current() else {
            return;
        };
        let at = flight.epoch_of(&params["timestamp"], source.now_ms);
        let encoded_bytes = params["encodedDataLength"].as_f64().unwrap_or(0.0) as u64;
        flight.end(RequestEnd::Finished, at, encoded_bytes, source.now_ms);
        flight.body = Body::Asked;
        let has_address_on_the_network =
            !(flight.url.starts_with("data:") || flight.url.starts_with("blob:"));
        out.push(Output::Finished(Finished {
            seq: flight.seq,
            session: source.session.to_string(),
            request_id: request_id.to_string(),
            url: flight.url.clone(),
            kind: flight.kind,
            mime_type: flight
                .response
                .as_ref()
                .and_then(|response| response.mime_type.clone()),
            status: flight
                .response
                .as_ref()
                .and_then(|response| response.status),
            expected_bytes: if flight.decoded_bytes > 0 {
                flight.decoded_bytes
            } else {
                encoded_bytes
            },
            encoded_bytes,
            bid: flight.bid.clone(),
            initiator_url: flight.made_by.clone(),
            in_tab: source.reporter == Reporter::Tab,
            main_document: flight.main_document,
            has_address_on_the_network,
            finished_epoch_ms: at,
        }));
        self.bodies_asked.insert(flight.seq, request_id.to_string());
        chain.closed = true;
        self.revisit
            .push_back((source.now_ms + SETTLE_GRACE_MS, request_id.to_string()));
    }

    fn on_failed(&mut self, request_id: &str, params: &Value, now_ms: f64) {
        let Some(chain) = self.chains.get_mut(request_id) else {
            return;
        };
        let Some(flight) = chain.current() else {
            return;
        };
        let at = flight.epoch_of(&params["timestamp"], now_ms);
        let blocked_reason = params["blockedReason"]
            .as_str()
            .or_else(|| params["corsErrorStatus"]["corsError"].as_str())
            .map(str::to_string);
        flight.end(
            RequestEnd::Failed {
                error: params["errorText"]
                    .as_str()
                    .filter(|text| !text.is_empty())
                    .unwrap_or("the request failed")
                    .to_string(),
                cancelled: params["canceled"] == true,
                blocked_reason,
            },
            at,
            0,
            now_ms,
        );
        let copied = flight.copied_bytes();
        flight.copy = None;
        chain.closed = true;
        self.copied_bytes -= copied;
        self.revisit
            .push_back((now_ms + SETTLE_GRACE_MS, request_id.to_string()));
    }

    fn on_socket_created(&mut self, source: &Source<'_>, request_id: &str, params: &Value) {
        let Some(url) = params["url"].as_str() else {
            return;
        };
        if !source.recording
            || self.open_flights >= MAX_OPEN_REQUESTS
            || self.chains.contains_key(request_id)
        {
            return;
        }
        let initiator = &params["initiator"];
        let flight = Flight {
            seq: self.next_seq,
            hop: 0,
            session: source.session.to_string(),
            context: match source.reporter {
                Reporter::Tab => RequestContext::Page,
                Reporter::Iframe => RequestContext::Iframe,
                Reporter::Worker => RequestContext::Worker,
                Reporter::ServiceWorker => RequestContext::ServiceWorker,
            },
            frame_id: None,
            target_id: source.target_id.map(str::to_string),
            method: "GET".to_string(),
            url: url.to_string(),
            kind: ResourceKind::WebSocket,
            priority: None,
            request_headers: Vec::new(),
            post: Post::None,
            initiator_kind: initiator_kind(initiator["type"].as_str().unwrap_or_default()),
            initiator_address: initiator["url"]
                .as_str()
                .filter(|url| !url.is_empty())
                .map(str::to_string),
            initiator_line: position(&initiator["lineNumber"]),
            initiator_column: position(&initiator["columnNumber"]),
            stack: call_frames(&initiator["stack"]),
            made_by: initiator_url(initiator),
            redirect_from: None,
            preflight: Preflight::No,
            bid: None,
            main_document: false,
            started_epoch_ms: source.now_ms,
            started_mono_ms: None,
            response: None,
            memory_cache: false,
            decoded_bytes: 0,
            end: None,
            body: Body::None,
            copy: None,
            copy_asked: false,
        };
        self.next_seq += 1;
        self.open_flights += 1;
        self.chains.insert(
            request_id.to_string(),
            Chain {
                hops: VecDeque::from([flight]),
                hops_seen: 1,
                ..Chain::default()
            },
        );
    }

    /// Hands over the hops of a request that have everything they will get.
    fn settle(&mut self, request_id: &str, now_ms: f64, out: &mut Vec<Output>) {
        let Some(chain) = self.chains.get_mut(request_id) else {
            return;
        };
        if let Some(hold) = chain.hold {
            // A navigation that ended and still did not commit never will:
            // a download, an answer without content, a refused request.
            let waited_for_commit = chain.closed
                && chain
                    .hops
                    .back()
                    .and_then(|flight| flight.end.as_ref())
                    .is_some_and(|end| now_ms >= end.seen_ms + SETTLE_GRACE_MS);
            if !waited_for_commit {
                return;
            }
            if !(hold.started_recording || chain.hops.iter().any(Flight::owes_a_record)) {
                self.forget(request_id);
                return;
            }
            chain.hold = None;
        }
        // A body goes straight on: nothing before it may keep it waiting.
        let hurry = chain.hops.back().is_some_and(Flight::carries_a_body);
        let mut handed_over = Vec::new();
        while let Some(flight) = chain.hops.front() {
            if !flight.is_complete() {
                break;
            }
            let waited = flight
                .end
                .as_ref()
                .is_some_and(|end| now_ms >= end.seen_ms + SETTLE_GRACE_MS);
            let has_everything = chain.extras_are_known(flight)
                && !matches!(flight.preflight, Preflight::Awaiting(_));
            if !(hurry || waited || has_everything) {
                break;
            }
            let Some(flight) = chain.hops.pop_front() else {
                break;
            };
            let (request_extra, response_extra) = chain.take_extras(flight.hop);
            handed_over.push(Ended {
                request_extra,
                response_extra,
                flight,
            });
        }
        let done = chain.closed && chain.hops.is_empty();
        for ended in handed_over {
            self.open_flights -= 1;
            self.held_post_bytes -= ended.flight.held_post_bytes();
            self.copied_bytes -= ended.flight.copied_bytes();
            if let Preflight::Awaiting(announced) = &ended.flight.preflight
                && let Some(waiting) = self.preflights_awaiting.get_mut(announced)
            {
                waiting.retain(|id| id != request_id);
                if waiting.is_empty() {
                    self.preflights_awaiting.remove(announced);
                }
            }
            out.push(Output::Ended(Box::new(ended)));
        }
        if done {
            self.chains.remove(request_id);
            self.remember_ended(request_id);
        }
    }

    /// Looks again at the requests whose wait is over.
    fn revisit_due(&mut self, now_ms: f64, out: &mut Vec<Output>) {
        while let Some((_, request_id)) = self.revisit.pop_front_if(|(due, _)| *due <= now_ms) {
            self.settle(&request_id, now_ms, out);
        }
        while let Some((_, request_id)) = self.unannounced.pop_front_if(|(due, _)| *due <= now_ms) {
            let never_announced = self
                .chains
                .get(&request_id)
                .is_some_and(|chain| chain.hops_seen == 0);
            if never_announced {
                self.chains.remove(&request_id);
            }
        }
    }

    /// Drops a request without a record: a navigation that was never part
    /// of the visit.
    fn forget(&mut self, request_id: &str) {
        if let Some(chain) = self.chains.remove(request_id) {
            for flight in &chain.hops {
                self.open_flights -= 1;
                self.held_post_bytes -= flight.held_post_bytes();
                self.copied_bytes -= flight.copied_bytes();
                self.bodies_asked.remove(&flight.seq);
                self.posts_asked.remove(&flight.seq);
            }
        }
        self.remember_ended(request_id);
    }

    fn remember_ended(&mut self, request_id: &str) {
        if self.ended.insert(request_id.to_string()) {
            self.ended_order.push_back(request_id.to_string());
        }
        if self.ended_order.len() > REMEMBERED_ENDED
            && let Some(oldest) = self.ended_order.pop_front()
        {
            self.ended.remove(&oldest);
        }
    }
}

/// Whether the answer to a request of this kind, as far as its
/// announcement tells, can be a body that is kept: text, not media.
fn may_have_a_kept_body(kind: ResourceKind) -> bool {
    matches!(
        kind,
        ResourceKind::Document
            | ResourceKind::Stylesheet
            | ResourceKind::Script
            | ResourceKind::Xhr
            | ResourceKind::Fetch
            | ResourceKind::Other
    )
}

fn status_of(response: &Value) -> Option<u16> {
    response["status"]
        .as_u64()
        .and_then(|status| u16::try_from(status).ok())
}

/// A CDP `Network.Response`, whose headers arrived at `at_epoch_ms`.
fn response_of(
    response: &Value,
    at_epoch_ms: f64,
    timing: Option<(f64, ResourceTiming)>,
) -> Response {
    let text = |key: &str| {
        response[key]
            .as_str()
            .filter(|text| !text.is_empty())
            .map(str::to_string)
    };
    let served_from = if response["fromServiceWorker"] == true {
        ServedFrom::ServiceWorker
    } else if response["fromPrefetchCache"] == true {
        ServedFrom::Prefetch
    } else if response["fromDiskCache"] == true {
        ServedFrom::DiskCache
    } else {
        ServedFrom::Network
    };
    Response {
        status: status_of(response),
        mime_type: text("mimeType"),
        protocol: text("protocol"),
        remote_address: text("remoteIPAddress").map(|ip| match response["remotePort"].as_u64() {
            Some(port) => format!("{ip}:{port}"),
            None => ip,
        }),
        headers: headers(&response["headers"]),
        served_from,
        at_epoch_ms,
        timing,
    }
}

#[cfg(test)]
mod tests;
