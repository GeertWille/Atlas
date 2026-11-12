//! What a request is for, in ad-tech terms, from its host, path and kind.
//!
//! The host table of the knowledge base says whose host it is; this module
//! adds what the path says, because one host serves several jobs: Xandr's
//! `ib.adnxs.com` answers Prebid's bid requests on `/ut/v3/prebid` and is an
//! ad server on `/ut/v3`, and Google's ad hosts serve the ad call, the
//! creative and the viewability beacons.

use adscan_model::{NetworkCategory, RequestContext, ResourceKind, ScriptCategory};

use crate::domain::knowledge::known_host;

pub struct RequestFacts<'a> {
    pub host: &'a str,
    /// Without the query string.
    pub path: &'a str,
    pub method: &'a str,
    pub kind: ResourceKind,
    pub context: RequestContext,
    /// Bidder code when the capture recognised a Prebid adapter's endpoint.
    pub bidder: Option<&'a str>,
    pub first_party: bool,
}

/// (host suffix, path prefix): the ad call of an ad server.
const AD_SERVER_CALLS: &[(&str, &str)] = &[
    ("doubleclick.net", "/gampad/ads"),
    ("doubleclick.net", "/gampad/adx"),
    ("googlesyndication.com", "/pagead/ads"),
    // Xandr AST (`apntag`) and its older tag endpoints.
    ("adnxs.com", "/ut/v3"),
    ("adnxs.com", "/ut/v2"),
    ("adnxs.com", "/ttj"),
    ("adnxs.com", "/tt"),
    ("adnxs.com", "/ptv"),
    // Equativ / Smart AdServer (`sas.call`, `sas.render`).
    ("smartadserver.com", "/ac"),
    ("smartadserver.com", "/call/"),
    ("smartadserver.com", "/h/"),
    ("smartadserver.com", "/genericpost"),
    ("adform.net", "/adx/"),
    ("adform.net", "/adfscript"),
];

/// (host suffix, path prefix): bid endpoints that are told from the ad-server
/// calls of the same host by their path. Checked before [`AD_SERVER_CALLS`].
const BID_CALLS: &[(&str, &str)] = &[
    ("adnxs.com", "/ut/v3/prebid"),
    ("smartadserver.com", "/prebid"),
    ("adform.net", "/adx/openrtb"),
    ("amazon-adsystem.com", "/e/dtb/bid"),
];

/// Path fragments of auction endpoints on hosts the tables do not know:
/// Prebid Server and plain OpenRTB.
const BID_PATH_MARKS: &[&str] = &["/openrtb2/auction", "/openrtb2/amp", "/openrtb/", "/openrtbb/"];

/// Hosts whose business is user identity: id modules and match tables.
const IDENTITY_HOSTS: &[&str] = &[
    "id5-sync.com",
    "rlcdn.com",
    "liveintent.com",
    "crwdcntrl.net",
    "intentiq.com",
    "uidapi.com",
    "permutive.com",
    "permutive.app",
    "adsrvr.org",
    "bidswitch.net",
];

/// Path fragments of cookie-sync and id endpoints on any ad host.
const SYNC_PATH_MARKS: &[&str] = &[
    "usersync", "user_sync", "user-sync", "cookiesync", "cookie-sync", "cksync", "/sync", "setuid",
    "getuid", "/match", "/pixel", "/ups/", "/usync", "/cm/", "idsync",
];

/// Google ad host paths that report rather than serve.
const GOOGLE_MEASUREMENT_PATHS: &[&str] = &[
    "/pcs/", "/pagead/gen_204", "/pagead/interaction", "/activeview", "/ddm/", "/pagead/1p-",
    "/pagead/viewthroughconversion", "/getconfig/", "/pagead/ppub_config",
];

/// File names of the ad libraries themselves, wherever they are served from.
const AD_LIBRARY_FILES: &[&str] = &["prebid", "pbjs", "gpt.js", "pubads_impl", "apstag", "hb-wrapper"];

pub fn category(facts: &RequestFacts) -> NetworkCategory {
    use NetworkCategory::*;
    if matches(BID_CALLS, facts) || facts.bidder.is_some() {
        return Bid;
    }
    if matches(AD_SERVER_CALLS, facts) {
        return AdServer;
    }
    if BID_PATH_MARKS.iter().any(|mark| facts.path.contains(mark)) && facts.method == "POST" {
        return Bid;
    }
    let file = facts.path.rsplit('/').next().unwrap_or_default().to_ascii_lowercase();
    let is_script = facts.kind == ResourceKind::Script;
    if is_script && AD_LIBRARY_FILES.iter().any(|name| file.contains(name)) {
        return Wrapper;
    }
    if is_suffix_of_any(IDENTITY_HOSTS, facts.host) {
        return Identity;
    }
    let known = known_host(facts.host).map(|(_, category)| category);
    let looks_like_sync = || {
        let path = facts.path.to_ascii_lowercase();
        SYNC_PATH_MARKS.iter().any(|mark| path.contains(mark))
    };
    // An image an ad host serves into the page itself is a tracking pixel;
    // inside a frame it is part of a creative.
    let renders = || {
        matches!(
            facts.kind,
            ResourceKind::Media | ResourceKind::Font | ResourceKind::Stylesheet
        ) || (facts.context == RequestContext::Iframe
            && matches!(
                facts.kind,
                ResourceKind::Image | ResourceKind::Document | ResourceKind::Script
            ))
    };
    match known {
        Some(ScriptCategory::Cmp) => Consent,
        Some(ScriptCategory::Analytics) => Measurement,
        Some(ScriptCategory::AdQuality) => Measurement,
        Some(ScriptCategory::Wrapper | ScriptCategory::Prebid) => Wrapper,
        Some(ScriptCategory::AdServer) => {
            if GOOGLE_MEASUREMENT_PATHS.iter().any(|mark| facts.path.contains(mark)) {
                Measurement
            } else if looks_like_sync() {
                Identity
            } else if is_script && facts.context != RequestContext::Iframe {
                Wrapper
            } else {
                Creative
            }
        }
        Some(ScriptCategory::Bidder) => {
            if looks_like_sync() {
                Identity
            } else if facts.method == "POST" && matches!(facts.kind, ResourceKind::Xhr | ResourceKind::Fetch) {
                // An auction endpoint the capture has no adapter entry for.
                Bid
            } else if is_script && facts.context != RequestContext::Iframe {
                Wrapper
            } else if renders() {
                Creative
            } else {
                Measurement
            }
        }
        Some(ScriptCategory::Application | ScriptCategory::Library | ScriptCategory::Unknown) | None => {
            if facts.first_party {
                Site
            } else if looks_like_sync() {
                Identity
            } else if facts.kind == ResourceKind::Ping {
                Measurement
            } else {
                OtherThirdParty
            }
        }
    }
}

/// Whether a request is a bid or ad-server call by what is known before its
/// answer is read: the engine reads those answers whether or not the scan
/// stores network data.
pub fn is_ad_call(host: &str, path: &str, bidder: Option<&str>) -> bool {
    let on = |table: &[(&str, &str)]| {
        table
            .iter()
            .any(|(suffix, prefix)| is_suffix(suffix, host) && path.starts_with(prefix))
    };
    bidder.is_some()
        || on(BID_CALLS)
        || on(AD_SERVER_CALLS)
        || BID_PATH_MARKS.iter().any(|mark| path.contains(mark))
}

/// Display name of the host's owner, when the knowledge base knows it.
pub fn vendor(host: &str) -> Option<&'static str> {
    known_host(host).map(|(vendor, _)| vendor)
}

/// Whether a bid request goes to a Prebid Server, which asks several bidders.
pub fn is_prebid_server(path: &str) -> bool {
    path.contains("/openrtb2/auction") || path.contains("/openrtb2/amp")
}

/// Whether an ad-server ad call carries Prebid's targeting. `query` is the
/// URL's query string; for a POST the request body does as well.
pub fn has_prebid_targeting(query: &str, post: Option<&str>) -> bool {
    // GAM puts slot targeting in `prev_scp` / `scp` / `cust_params`, each
    // percent-encoded once more; `hb_` survives every encoding.
    query.contains("hb_") || post.is_some_and(|post| post.contains("hb_"))
}

fn matches(table: &[(&str, &str)], facts: &RequestFacts) -> bool {
    table
        .iter()
        .any(|(host, path)| is_suffix(host, facts.host) && facts.path.starts_with(path))
}

fn is_suffix_of_any(suffixes: &[&str], host: &str) -> bool {
    suffixes.iter().any(|suffix| is_suffix(suffix, host))
}

fn is_suffix(suffix: &str, host: &str) -> bool {
    host == suffix
        || host
            .strip_suffix(suffix)
            .is_some_and(|prefix| prefix.ends_with('.'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use NetworkCategory::*;
    use RequestContext::{Iframe, Page};
    use ResourceKind::*;

    fn facts<'a>(
        url: &'a str,
        method: &'a str,
        kind: ResourceKind,
        context: RequestContext,
        bidder: Option<&'a str>,
    ) -> RequestFacts<'a> {
        let rest = url.strip_prefix("https://").unwrap();
        let (host, path) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
        RequestFacts {
            host,
            path,
            method,
            kind,
            context,
            bidder,
            first_party: host.ends_with("pub.example"),
        }
    }

    #[test]
    fn requests_are_classified_by_host_path_and_kind() {
        #[rustfmt::skip]
        let table: &[(&str, &str, ResourceKind, RequestContext, Option<&str>, NetworkCategory)] = &[
            // Bid endpoints the capture recognised, on any host.
            ("https://rtb.openx.net/openrtbb/prebidjs", "POST", Fetch, Page, Some("openx"), Bid),
            ("https://fastlane.rubiconproject.com/a/api/fastlane.json", "GET", Xhr, Page, Some("rubicon"), Bid),
            // One host, two jobs: Prebid's endpoint and the AST ad server.
            ("https://ib.adnxs.com/ut/v3/prebid", "POST", Xhr, Page, Some("appnexus"), Bid),
            ("https://ib.adnxs.com/ut/v3/prebid", "POST", Xhr, Page, None, Bid),
            ("https://ib.adnxs.com/ut/v3", "POST", Xhr, Page, None, AdServer),
            ("https://secure.adnxs.com/ttj", "GET", Script, Page, None, AdServer),
            ("https://ib.adnxs.com/getuid", "GET", Image, Page, None, Identity),
            // Equativ / Smart AdServer.
            ("https://prg.smartadserver.com/prebid/v1", "POST", Xhr, Page, None, Bid),
            ("https://www8.smartadserver.com/ac", "GET", Script, Page, None, AdServer),
            ("https://www8.smartadserver.com/h/nshow", "GET", Fetch, Page, None, AdServer),
            ("https://www8.smartadserver.com/genericpost", "POST", Xhr, Page, None, AdServer),
            // Google: ad call, library, creative, beacons, cookie match.
            ("https://securepubads.g.doubleclick.net/gampad/ads", "GET", Xhr, Page, None, AdServer),
            ("https://securepubads.g.doubleclick.net/tag/js/gpt.js", "GET", Script, Page, None, Wrapper),
            ("https://securepubads.g.doubleclick.net/pagead/managed/js/gpt/m1/pubads_impl.js", "GET", Script, Page, None, Wrapper),
            ("https://tpc.googlesyndication.com/simgad/123", "GET", Image, Iframe, None, Creative),
            ("https://s0.2mdn.net/ads/richmedia/studio/x.js", "GET", Script, Iframe, None, Creative),
            ("https://securepubads.g.doubleclick.net/pcs/view", "GET", Image, Page, None, Measurement),
            ("https://pagead2.googlesyndication.com/pagead/gen_204", "GET", Ping, Page, None, Measurement),
            ("https://cm.g.doubleclick.net/pixel", "GET", Image, Page, None, Identity),
            // Amazon TAM and Prebid Server.
            ("https://aax.amazon-adsystem.com/e/dtb/bid", "GET", Xhr, Page, None, Bid),
            ("https://pbs.pub-server.example/openrtb2/auction", "POST", Fetch, Page, None, Bid),
            // Bidder hosts doing something else.
            ("https://eus.rubiconproject.com/usync.html", "GET", Document, Iframe, None, Identity),
            ("https://ads.pubmatic.com/AdServer/js/user_sync.html", "GET", Document, Iframe, None, Identity),
            ("https://hbopenbid.pubmatic.com/translator", "POST", Xhr, Page, None, Bid),
            ("https://static.criteo.net/images/logo.png", "GET", Image, Iframe, None, Creative),
            ("https://c.amazon-adsystem.com/aax2/apstag.js", "GET", Script, Page, None, Wrapper),
            ("https://us-u.openx.net/w/1.0/pd", "GET", Image, Page, None, Measurement),
            // Consent, identity, measurement.
            ("https://cdn.cookielaw.org/scripttemplates/otSDKStub.js", "GET", Script, Page, None, Consent),
            ("https://id5-sync.com/g/v2/123.json", "POST", Xhr, Page, None, Identity),
            ("https://match.adsrvr.org/track/cmf/generic", "GET", Image, Page, None, Identity),
            ("https://www.google-analytics.com/g/collect", "POST", Ping, Page, None, Measurement),
            ("https://pixel.adsafeprotected.com/jload", "GET", Script, Iframe, None, Measurement),
            // The site itself, its own ad libraries, and everything else.
            ("https://www.pub.example/article", "GET", Document, Page, None, Site),
            ("https://cdn.pub.example/js/prebid9.12.min.js", "GET", Script, Page, None, Wrapper),
            ("https://cdn.pub.example/static/ads/hb-wrapper.min.js", "GET", Script, Page, None, Wrapper),
            ("https://cdn.pub.example/js/app.js", "GET", Script, Page, None, Site),
            ("https://cdn.jsdelivr.net/npm/x.js", "GET", Script, Page, None, OtherThirdParty),
            ("https://unknown.example.net/sync", "GET", Image, Page, None, Identity),
            ("https://unknown.example.net/beacon", "POST", Ping, Page, None, Measurement),
        ];
        for (url, method, kind, context, bidder, expected) in table {
            assert_eq!(
                category(&facts(url, method, *kind, *context, *bidder)),
                *expected,
                "{method} {url} ({kind:?}, {context:?})"
            );
        }
    }

    #[test]
    fn bid_and_ad_server_calls_are_known_before_their_answer_is_read() {
        for (host, path, bidder, expected) in [
            ("rtb.openx.net", "/openrtbb/prebidjs", Some("openx"), true),
            ("fastlane.rubiconproject.com", "/a/api/fastlane.json", Some("rubicon"), true),
            ("ib.adnxs.com", "/ut/v3", None, true),
            ("ib.adnxs.com", "/ut/v3/prebid", None, true),
            ("www8.smartadserver.com", "/ac", None, true),
            ("securepubads.g.doubleclick.net", "/gampad/ads", None, true),
            ("pbs.example.net", "/openrtb2/auction", None, true),
            ("securepubads.g.doubleclick.net", "/tag/js/gpt.js", None, false),
            ("www.pub.example", "/article", None, false),
            ("ib.adnxs.com", "/getuid", None, false),
        ] {
            assert_eq!(is_ad_call(host, path, bidder), expected, "{host}{path}");
        }
    }

    #[test]
    fn the_vendor_comes_from_the_host_table() {
        assert_eq!(vendor("ib.adnxs.com"), Some("Xandr"));
        assert_eq!(vendor("securepubads.g.doubleclick.net"), Some("Google Ad Manager"));
        assert_eq!(vendor("www.pub.example"), None);
    }

    #[test]
    fn prebid_targeting_is_found_in_the_query_however_it_is_encoded() {
        assert!(has_prebid_targeting("iu=/1/top&prev_scp=hb_pb%3D1.20%26hb_bidder%3Drubicon", None));
        assert!(has_prebid_targeting("iu=/1/top&cust_params=hb_adid%253Dabc", None));
        assert!(has_prebid_targeting("", Some(r#"{"keywords":[{"key":"hb_pb","value":["1.20"]}]}"#)));
        assert!(!has_prebid_targeting("iu=/1/top&prev_scp=pos%3Dtop&sz=728x90", None));
    }
}
