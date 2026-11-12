//! Recognises the bid requests of the common Prebid adapters on the network
//! and reduces their answers to a status and a short reason. No I/O.
//!
//! Hosts and paths were read from the adapter sources of Prebid.js 11.49.0
//! (`modules/<bidder>BidAdapter.js`). The no-bid reason codes are those of
//! OpenRTB 2.5, list 5.24.

use adscan_model::ObservedBidResponse;
use serde_json::Value;
use url::Url;

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
    /// The path belongs to this adapter alone, so it is recognised on any
    /// host: publishers proxy bid endpoints through their own domains.
    unmistakable: bool,
}

const fn endpoint(bidder: &'static str, host: &'static str, path: &'static str) -> Endpoint {
    Endpoint {
        bidder,
        host,
        path,
        unmistakable: false,
    }
}

const ENDPOINTS: &[Endpoint] = &[
    Endpoint {
        unmistakable: true,
        ..endpoint("appnexus", "ib.adnxs.com", "/ut/v3/prebid")
    },
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
    Endpoint {
        unmistakable: true,
        ..endpoint("openx", "rtb.openx.net", "/openrtbb/prebidjs")
    },
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

/// The bidder a request URL belongs to, if it is a known bid endpoint.
pub(crate) fn bid_request(url: &str) -> Option<BidRequest> {
    let url = Url::parse(url).ok()?;
    let host = url.host_str()?.to_ascii_lowercase();
    let path = url.path();
    ENDPOINTS
        .iter()
        .find(|endpoint| {
            (endpoint.host == host && path.starts_with(endpoint.path))
                || (endpoint.unmistakable && path == endpoint.path)
        })
        .map(|endpoint| BidRequest {
            bidder: endpoint.bidder,
            host,
        })
}

/// Why the endpoint did not bid, when its answer says so. The body is read
/// here and dropped; only this short text leaves the capture.
pub(crate) fn no_bid_reason(bidder: &str, body: &str) -> Option<String> {
    if body.len() > MAX_BODY_BYTES {
        return None;
    }
    let answer: Value = serde_json::from_str(body).ok()?;
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
    let meaning = match code {
        0 => "unknown error",
        1 => "technical error",
        2 => "invalid request",
        3 => "known web spider",
        4 => "suspected non-human traffic",
        5 => "cloud, data center or proxy IP",
        6 => "unsupported device",
        7 => "blocked publisher or site",
        8 => "unmatched user",
        _ => "exchange-specific code",
    };
    format!("no-bid reason {code} ({meaning})")
}

/// One line of printable text, at most `MAX_REASON_CHARS` characters.
pub(crate) fn short(text: &str) -> String {
    let mut line: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
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
    ) {
        if self.observations.len() < MAX_OBSERVATIONS {
            self.observations.push(Observation {
                at_epoch_ms,
                request,
                status,
                reason,
            });
        }
    }

    /// Answers of the document that ended up being scanned, in time order.
    pub(crate) fn into_observed(self, navigation_start: f64) -> Vec<ObservedBidResponse> {
        let mut observed: Vec<ObservedBidResponse> = self
            .observations
            .into_iter()
            .filter_map(|observation| {
                let t = observation.at_epoch_ms? - navigation_start;
                (t >= 0.0).then(|| ObservedBidResponse {
                    t,
                    bidder: observation.request.bidder.to_string(),
                    host: observation.request.host,
                    status: observation.status,
                    reason: observation.reason,
                })
            })
            .collect();
        observed.sort_by(|a, b| a.t.total_cmp(&b.t));
        observed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bidder_of(url: &str) -> Option<&'static str> {
        bid_request(url).map(|request| request.bidder)
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
    fn unmistakable_paths_are_recognised_behind_any_host() {
        assert_eq!(
            bidder_of("https://bids.pub.example/ut/v3/prebid"),
            Some("appnexus")
        );
        assert_eq!(
            bidder_of("http://127.0.0.1:4455/openrtbb/prebidjs"),
            Some("openx")
        );
        // A path that only starts the same, or that other adapters share, is not enough.
        assert_eq!(
            bidder_of("https://bids.pub.example/ut/v3/prebid/extra"),
            None
        );
        assert_eq!(bidder_of("https://bids.pub.example/translator"), None);
        assert_eq!(bidder_of("https://bids.pub.example/hb/"), None);
    }

    #[test]
    fn every_endpoint_has_a_host_and_unmistakable_ones_a_path() {
        for endpoint in ENDPOINTS {
            assert!(endpoint.host.contains('.'), "{}", endpoint.bidder);
            assert_eq!(endpoint.host, endpoint.host.to_ascii_lowercase());
            assert!(endpoint.path.is_empty() || endpoint.path.starts_with('/'));
            assert!(!endpoint.unmistakable || endpoint.path.len() > 8);
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
            ("rubicon", r#"{"error":""}"#, None),
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
        let request = || bid_request("https://rtb.openx.net/openrtbb/prebidjs").unwrap();
        let mut log = BidResponseLog::default();
        log.record(
            request(),
            Some(1_500.0),
            Some(200),
            Some("no-bid reason 7".into()),
        );
        log.record(
            request(),
            Some(1_200.0),
            None,
            Some("net::ERR_FAILED".into()),
        );
        // From the document before a reload, and one that could not be dated.
        log.record(request(), Some(900.0), Some(200), None);
        log.record(request(), None, Some(200), None);

        let observed = log.into_observed(1_000.0);
        assert_eq!(
            observed,
            [
                ObservedBidResponse {
                    t: 200.0,
                    bidder: "openx".into(),
                    host: "rtb.openx.net".into(),
                    status: None,
                    reason: Some("net::ERR_FAILED".into()),
                },
                ObservedBidResponse {
                    t: 500.0,
                    bidder: "openx".into(),
                    host: "rtb.openx.net".into(),
                    status: Some(200),
                    reason: Some("no-bid reason 7".into()),
                },
            ]
        );

        let mut full = BidResponseLog::default();
        for _ in 0..MAX_OBSERVATIONS + 10 {
            full.record(request(), Some(1.0), Some(204), None);
        }
        assert_eq!(full.into_observed(0.0).len(), MAX_OBSERVATIONS);
    }
}
