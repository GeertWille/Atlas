//! Recognises the bid requests of the common Prebid adapters on the network
//! and reduces their answers to a status and a short reason. No I/O.
//!
//! Hosts and paths were read from the adapter sources of Prebid.js 11.49.0
//! (`modules/<bidder>BidAdapter.js`).

use adscan_model::{ObservedBidResponse, no_bid_reason_name};
use serde_json::Value;
use url::{Host, Url};

/// Longest reason kept per response.
pub(crate) const MAX_REASON_CHARS: usize = 200;
/// A page that hammers its bidders must not exhaust memory.
const MAX_OBSERVATIONS: usize = 2_000;
/// Answers larger than this carry bids, not an explanation.
pub(crate) const MAX_BODY_BYTES: usize = 256 * 1024;

struct Endpoint {
    /// Bidder code of the adapter.
    bidder: &'static str,
    host: &'static str,
    /// The request path starts with this. Empty when the path varies.
    path: &'static str,
}

const fn endpoint(bidder: &'static str, host: &'static str, path: &'static str) -> Endpoint {
    Endpoint { bidder, host, path }
}

const ENDPOINTS: &[Endpoint] = &[
    endpoint("appnexus", "ib.adnxs.com", "/ut/v3/prebid"),
    endpoint("appnexus", "ib.adnxs-simple.com", "/ut/v3/prebid"),
    endpoint(
        "rubicon",
        "fastlane.rubiconproject.com",
        "/a/api/fastlane.json",
    ),
    endpoint(
        "rubicon",
        "prebid-server.rubiconproject.com",
        "/openrtb2/auction",
    ),
    endpoint("ix", "htlb.casalemedia.com", "/openrtb/pbjs"),
    endpoint("pubmatic", "hbopenbid.pubmatic.com", "/translator"),
    endpoint(
        "criteo",
        "grid-bidder.criteo.com",
        "/openrtb_2_5/pbjs/auction/request",
    ),
    endpoint("openx", "rtb.openx.net", "/openrtbb/prebidjs"),
    endpoint("triplelift", "tlx.3lift.com", "/header/auction"),
    endpoint("sovrn", "ap.lijit.com", "/rtb/bid"),
    endpoint("seedtag", "s.seedtag.com", "/c/hb/bid"),
    endpoint("adf", "adx.adform.net", "/adx/openrtb"),
    endpoint("smartadserver", "prg.smartadserver.com", "/prebid/v1"),
    endpoint("teads", "a.teads.tv", "/hb/bid-request"),
    endpoint("improvedigital", "ad.360yield.com", ""),
    endpoint("improvedigital", "ad.360yield-basic.com", ""),
    endpoint("sharethrough", "btlr.sharethrough.com", "/universal/v1"),
    endpoint("amx", "prebid.a-mo.net", "/a/c"),
    endpoint("medianet", "prebid.media.net", "/rtb/prebid"),
    endpoint("ttd", "direct.adsrvr.org", "/bid/bidder/"),
    endpoint("yieldmo", "ads.yieldmo.com", "/exchange/prebid"),
    endpoint("gumgum", "g2.gumgum.com", "/hbid/imp"),
    endpoint("onetag", "onetag-sys.com", "/prebid-request"),
    endpoint("richaudience", "shb.richaudience.com", "/hb/"),
    endpoint("unruly", "targeting.unrulymedia.com", "/unruly_prebid"),
    endpoint("kargo", "krk2.kargo.com", "/api/v1/prebid"),
    endpoint("33across", "ssc.33across.com", "/api/v1/hb"),
];

/// A request to a bidder's endpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BidRequest {
    pub bidder: &'static str,
    pub host: String,
}

/// The bidder a request URL belongs to, if it is a known bid endpoint: one
/// of the adapters' own hosts. For a page served from this machine, as the
/// built-in demo site is, a server on this machine may stand in for an
/// exchange (the demo's does, at its own origin and at a second one), so
/// there the path alone decides. On any real host a path proves nothing: the
/// site chooses what it serves under it.
pub(crate) fn bid_request(url: &str, page_url: &str) -> Option<BidRequest> {
    let url = Url::parse(url).ok()?;
    let host = url.host_str()?.to_ascii_lowercase();
    let path = url.path();
    let on_this_machine = |url: &Url| {
        matches!(url.scheme(), "http" | "https")
            && url.host().is_some_and(|host| is_this_machine(&host))
    };
    let is_local_stand_in =
        on_this_machine(&url) && Url::parse(page_url).is_ok_and(|page| on_this_machine(&page));
    ENDPOINTS
        .iter()
        .find(|endpoint| {
            (endpoint.host == host && path.starts_with(endpoint.path))
                || (is_local_stand_in && !endpoint.path.is_empty() && path == endpoint.path)
        })
        .map(|endpoint| BidRequest {
            bidder: endpoint.bidder,
            host,
        })
}

fn is_this_machine(host: &Host<&str>) -> bool {
    match host {
        Host::Domain(domain) => domain.eq_ignore_ascii_case("localhost"),
        Host::Ipv4(ip) => ip.is_loopback(),
        Host::Ipv6(ip) => ip.is_loopback(),
    }
}

/// Why the endpoint did not bid, when its answer says so. The body is read
/// here and dropped; only this short text leaves the capture.
pub(crate) fn no_bid_reason(bidder: &str, body: &str) -> Option<String> {
    if body.len() > MAX_BODY_BYTES {
        return None;
    }
    let answer: Value = serde_json::from_str(body).ok()?;
    // An answer that carries bids is not a refusal, whatever else it says.
    if answer["seatbid"]
        .as_array()
        .is_some_and(|seats| !seats.is_empty())
    {
        return None;
    }
    let text = |value: &Value| {
        value
            .as_str()
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_string)
    };
    let stated = text(&answer["error"])
        .or_else(|| text(&answer["message"]))
        .map(|error| format!("error: {error}"));
    let reason = stated
        .or_else(|| {
            (bidder == "appnexus")
                .then(|| tag_errors(&answer))
                .flatten()
        })
        .or_else(|| answer["nbr"].as_u64().map(openrtb_no_bid_reason))?;
    Some(short(&reason))
}

/// AppNexus answers per tag; a tag that could not be served names the error.
fn tag_errors(answer: &Value) -> Option<String> {
    let mut errors: Vec<&str> = Vec::new();
    for tag in answer["tags"].as_array()? {
        if let Some(error) = tag["error"].as_str().map(str::trim)
            && !error.is_empty()
            && !errors.contains(&error)
        {
            errors.push(error);
        }
    }
    (!errors.is_empty()).then(|| format!("tag error: {}", errors.join("; ")))
}

fn openrtb_no_bid_reason(code: u64) -> String {
    let name = u32::try_from(code).ok().and_then(no_bid_reason_name);
    let meaning = match name {
        Some(name) => name.to_lowercase(),
        // The list leaves these to each exchange.
        None if code >= 500 => "exchange-specific code".to_string(),
        None => "not a listed code".to_string(),
    };
    format!("no-bid reason {code} ({meaning})")
}

/// One line of printable text, at most `MAX_REASON_CHARS` characters. The
/// text is the endpoint's, so nothing in it may steer how it is displayed.
pub(crate) fn short(text: &str) -> String {
    let is_invisible = |c: char| {
        c.is_control()
            || matches!(c, '\u{200b}'..='\u{200f}' | '\u{2028}'..='\u{202e}' | '\u{2060}'..='\u{2069}' | '\u{feff}')
    };
    let mut line: String = text
        .chars()
        .map(|c| if is_invisible(c) { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if line.chars().count() > MAX_REASON_CHARS {
        line = line.chars().take(MAX_REASON_CHARS - 1).collect();
        line.push('…');
    }
    line
}

struct Observation {
    at_epoch_ms: Option<f64>,
    request: BidRequest,
    status: Option<u16>,
    reason: Option<String>,
    request_seq: Option<u32>,
}

/// What the bidders' endpoints answered during one capture.
#[derive(Default)]
pub(crate) struct BidResponseLog {
    observations: Vec<Observation>,
}

impl BidResponseLog {
    pub(crate) fn record(
        &mut self,
        request: BidRequest,
        at_epoch_ms: Option<f64>,
        status: Option<u16>,
        reason: Option<String>,
        request_seq: Option<u32>,
    ) {
        if self.observations.len() < MAX_OBSERVATIONS {
            self.observations.push(Observation {
                at_epoch_ms,
                request,
                status,
                reason,
                request_seq,
            });
        }
    }

    /// The answers in the order they arrived, without those from before the
    /// visit started (a document that was replaced). `place` turns an epoch
    /// time into the page view showing then and the time since it started.
    pub(crate) fn into_observed(
        self,
        place: impl Fn(f64) -> (u32, f64),
    ) -> Vec<ObservedBidResponse> {
        let mut observed: Vec<ObservedBidResponse> = self
            .observations
            .into_iter()
            .filter_map(|observation| {
                let (page, t) = place(observation.at_epoch_ms?);
                (t >= 0.0).then(|| ObservedBidResponse {
                    page,
                    t,
                    bidder: observation.request.bidder.to_string(),
                    host: observation.request.host,
                    status: observation.status,
                    reason: observation.reason,
                    request_seq: observation.request_seq,
                })
            })
            .collect();
        observed.sort_by(|a, b| a.page.cmp(&b.page).then(a.t.total_cmp(&b.t)));
        observed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = "https://www.pub.example/article";

    fn bidder_of(url: &str) -> Option<&'static str> {
        bid_request(url, PAGE).map(|request| request.bidder)
    }

    #[test]
    fn bid_requests_are_recognised_by_host_and_path() {
        let cases = [
            ("https://ib.adnxs.com/ut/v3/prebid", Some("appnexus")),
            ("https://ib.adnxs-simple.com/ut/v3/prebid", Some("appnexus")),
            (
                "https://fastlane.rubiconproject.com/a/api/fastlane.json?account_id=1",
                Some("rubicon"),
            ),
            ("https://htlb.casalemedia.com/openrtb/pbjs?s=1", Some("ix")),
            (
                "https://hbopenbid.pubmatic.com/translator?source=prebid-client",
                Some("pubmatic"),
            ),
            ("https://rtb.openx.net/openrtbb/prebidjs", Some("openx")),
            ("https://ad.360yield.com/950/pb", Some("improvedigital")),
            ("https://adx.adform.net/adx/openrtb", Some("adf")),
            ("https://direct.adsrvr.org/bid/bidder/acme", Some("ttd")),
            (
                "https://ADS.YIELDMO.COM/exchange/prebidvideo",
                Some("yieldmo"),
            ),
            // Other traffic of the same companies is not a bid request.
            ("https://eus.rubiconproject.com/usync.html", None),
            ("https://ib.adnxs.com/getuid?https://x.test", None),
            ("https://acdn.adnxs.com/prebid/prebid.js", None),
            ("https://match.adsrvr.org/track/rid", None),
            ("https://pub.example/article", None),
            ("not a url", None),
        ];
        for (url, expected) in cases {
            assert_eq!(bidder_of(url), expected, "{url}");
        }
    }

    #[test]
    fn a_path_alone_only_counts_between_a_page_and_a_server_on_this_machine() {
        let demo = "http://127.0.0.1:4455/";
        let on = |url: &str, page: &str| bid_request(url, page).map(|request| request.bidder);
        assert_eq!(
            on("http://127.0.0.1:4455/openrtbb/prebidjs", demo),
            Some("openx")
        );
        assert_eq!(
            on(
                "http://localhost:3000/ut/v3/prebid",
                "http://localhost:3000/demo"
            ),
            Some("appnexus")
        );
        assert_eq!(
            on("http://[::1]:8080/openrtbb/prebidjs", "http://[::1]:8080/"),
            Some("openx")
        );
        // A path that only starts the same is not the endpoint.
        assert_eq!(
            on("http://127.0.0.1:4455/openrtbb/prebidjs/extra", demo),
            None
        );
        // The stand-in may be at another origin of this machine: the demo
        // site's second one, a Prebid Server run locally.
        assert_eq!(
            on("http://127.0.0.1:9999/openrtbb/prebidjs", demo),
            Some("openx")
        );
        assert_eq!(
            on("http://localhost:4456/ut/v3/prebid", demo),
            Some("appnexus")
        );
        assert_eq!(on("ws://127.0.0.1:4455/openrtbb/prebidjs", demo), None);

        // Real hosts: the site, its subdomains, anyone else's server.
        for (url, page) in [
            ("https://bids.pub.example/ut/v3/prebid", PAGE),
            ("https://www.pub.example/ut/v3/prebid", PAGE),
            ("https://www.pub.example/openrtbb/prebidjs", PAGE),
            ("https://evil.example/openrtbb/prebidjs", PAGE),
            ("http://127.0.0.1:4455/openrtbb/prebidjs", PAGE),
            ("https://evil.example/openrtbb/prebidjs", demo),
            (
                "http://192.168.1.20/openrtbb/prebidjs",
                "http://192.168.1.20/",
            ),
            ("http://127.0.0.1:4455/openrtbb/prebidjs", ""),
            ("http://127.0.0.1:4455/openrtbb/prebidjs", "about:blank"),
        ] {
            assert_eq!(on(url, page), None, "{url} on {page}");
        }
        // The adapters' own hosts are recognised wherever the page is.
        assert_eq!(
            on("https://rtb.openx.net/openrtbb/prebidjs", demo),
            Some("openx")
        );
    }

    #[test]
    fn every_endpoint_has_a_lower_case_host_and_a_rooted_path() {
        for endpoint in ENDPOINTS {
            assert!(endpoint.host.contains('.'), "{}", endpoint.bidder);
            assert_eq!(endpoint.host, endpoint.host.to_ascii_lowercase());
            assert!(endpoint.path.is_empty() || endpoint.path.starts_with('/'));
        }
    }

    #[test]
    fn reasons_are_read_from_the_shapes_endpoints_use() {
        let cases = [
            (
                "appnexus",
                r#"{"error":"Member 958 is not active"}"#,
                Some("error: Member 958 is not active"),
            ),
            (
                "appnexus",
                r#"{"version":"3.0.0","tags":[
                    {"uuid":"a","tag_id":1,"nobid":true,"error":"Invalid placement"},
                    {"uuid":"b","tag_id":2,"nobid":true,"error":"Invalid placement"},
                    {"uuid":"c","tag_id":3,"nobid":true,"error":" Size not allowed "}]}"#,
                Some("tag error: Invalid placement; Size not allowed"),
            ),
            // A plain no-bid explains nothing.
            (
                "appnexus",
                r#"{"version":"3.0.0","tags":[{"uuid":"a","tag_id":1,"nobid":true}]}"#,
                None,
            ),
            (
                "openx",
                r#"{"id":"1","nbr":7}"#,
                Some("no-bid reason 7 (blocked publisher or site)"),
            ),
            (
                "ix",
                r#"{"id":"1","nbr":2}"#,
                Some("no-bid reason 2 (invalid request)"),
            ),
            (
                "ix",
                r#"{"id":"1","nbr":504}"#,
                Some("no-bid reason 504 (exchange-specific code)"),
            ),
            // The codes OpenRTB added after 2.5 are named too, not passed
            // off as an exchange's own.
            (
                "ix",
                r#"{"id":"1","nbr":15}"#,
                Some("no-bid reason 15 (insufficient auction time)"),
            ),
            (
                "ix",
                r#"{"id":"1","nbr":11}"#,
                Some("no-bid reason 11 (ads.txt authorization unavailable)"),
            ),
            (
                "ix",
                r#"{"id":"1","nbr":42}"#,
                Some("no-bid reason 42 (not a listed code)"),
            ),
            (
                "pubmatic",
                r#"{"message":"publisher is blocked"}"#,
                Some("error: publisher is blocked"),
            ),
            // Only AppNexus answers per tag.
            ("rubicon", r#"{"tags":[{"error":"x"}]}"#, None),
            (
                "rubicon",
                r#"{"id":"1","seatbid":[{"bid":[{"price":1.2}]}]}"#,
                None,
            ),
            // Bids together with a note are bids.
            (
                "pubmatic",
                r#"{"id":"1","message":"ok","nbr":0,"seatbid":[{"bid":[{"price":1.2}]}]}"#,
                None,
            ),
            (
                "pubmatic",
                r#"{"id":"1","message":"no fill","seatbid":[]}"#,
                Some("error: no fill"),
            ),
            ("rubicon", r#"{"error":""}"#, None),
            ("rubicon", r#"{"error":{"code":7}}"#, None),
            ("rubicon", r#"[{"error":"not an object"}]"#, None),
            ("rubicon", "", None),
            ("rubicon", "<html>Bad gateway</html>", None),
        ];
        for (bidder, body, expected) in cases {
            assert_eq!(
                no_bid_reason(bidder, body).as_deref(),
                expected,
                "{bidder} {body}"
            );
        }
    }

    #[test]
    fn reasons_are_one_short_line_and_large_bodies_are_not_read() {
        let long = format!(r#"{{"error":"line one\n\tline two {}"}}"#, "x".repeat(500));
        let reason = no_bid_reason("ix", &long).unwrap();
        assert!(reason.starts_with("error: line one line two xxx"));
        assert_eq!(reason.chars().count(), MAX_REASON_CHARS);
        assert!(reason.ends_with('…'));

        let huge = format!(r#"{{"error":"e","pad":"{}"}}"#, "x".repeat(MAX_BODY_BYTES));
        assert_eq!(no_bid_reason("ix", &huge), None);
    }

    #[test]
    fn observations_are_dated_from_navigation_start_and_bounded() {
        let request = || bid_request("https://rtb.openx.net/openrtbb/prebidjs", PAGE).unwrap();
        let mut log = BidResponseLog::default();
        log.record(
            request(),
            Some(1_500.0),
            Some(200),
            Some("no-bid reason 7".into()),
            Some(41),
        );
        log.record(
            request(),
            Some(1_200.0),
            None,
            Some("net::ERR_FAILED".into()),
            None,
        );
        // From the document before a reload, and one that could not be dated.
        log.record(request(), Some(900.0), Some(200), None, None);
        log.record(request(), None, Some(200), None, None);

        let observed = log.into_observed(|at| (0, at - 1_000.0));
        assert_eq!(
            observed,
            [
                ObservedBidResponse {
                    page: 0,
                    t: 200.0,
                    bidder: "openx".into(),
                    host: "rtb.openx.net".into(),
                    status: None,
                    reason: Some("net::ERR_FAILED".into()),
                    request_seq: None,
                },
                ObservedBidResponse {
                    page: 0,
                    t: 500.0,
                    bidder: "openx".into(),
                    host: "rtb.openx.net".into(),
                    status: Some(200),
                    reason: Some("no-bid reason 7".into()),
                    request_seq: Some(41),
                },
            ]
        );

        let mut full = BidResponseLog::default();
        for _ in 0..MAX_OBSERVATIONS + 10 {
            full.record(request(), Some(1.0), Some(204), None, None);
        }
        assert_eq!(full.into_observed(|at| (0, at)).len(), MAX_OBSERVATIONS);
    }
}
