//! Built-in knowledge of the ad-tech ecosystem, used to classify scripts
//! deterministically so the model is only asked about ambiguous ones.

use adscan_model::ScriptCategory::{self, *};

#[derive(Debug, Clone, PartialEq)]
pub struct Classification {
    pub category: ScriptCategory,
    pub ad_related: bool,
    pub vendor: Option<&'static str>,
    pub confidence: f64,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Verdict {
    Decided(Classification),
    /// The script mentions ad-tech identifiers but nothing conclusive.
    Ambiguous {
        hints: Vec<String>,
    },
}

pub struct ScriptFacts<'a> {
    pub url: &'a str,
    pub hostname: &'a str,
    pub inline: bool,
    pub first_party: bool,
    pub body: &'a str,
}

pub fn is_ad_related(category: ScriptCategory) -> bool {
    matches!(
        category,
        Prebid | Wrapper | AdServer | Bidder | Cmp | AdQuality
    )
}

/// (hostname suffix, vendor, category)
const KNOWN_HOSTS: &[(&str, &str, ScriptCategory)] = &[
    // Ad servers
    (
        "securepubads.g.doubleclick.net",
        "Google Ad Manager",
        AdServer,
    ),
    ("pagead2.googlesyndication.com", "Google", AdServer),
    ("googletagservices.com", "Google Ad Manager", AdServer),
    ("googlesyndication.com", "Google", AdServer),
    ("doubleclick.net", "Google", AdServer),
    ("googleadservices.com", "Google", AdServer),
    ("2mdn.net", "Google", AdServer),
    ("imasdk.googleapis.com", "Google IMA", AdServer),
    ("adform.net", "Adform", Bidder),
    // Header bidding partners, SSPs and exchanges
    ("amazon-adsystem.com", "Amazon", Bidder),
    ("adnxs.com", "Xandr", Bidder),
    ("adnxs-simple.com", "Xandr", Bidder),
    ("rubiconproject.com", "Magnite", Bidder),
    ("pubmatic.com", "PubMatic", Bidder),
    ("casalemedia.com", "Index Exchange", Bidder),
    ("indexww.com", "Index Exchange", Bidder),
    ("openx.net", "OpenX", Bidder),
    ("criteo.com", "Criteo", Bidder),
    ("criteo.net", "Criteo", Bidder),
    ("smartadserver.com", "Equativ", Bidder),
    ("sascdn.com", "Equativ", Bidder),
    ("360yield.com", "Improve Digital", Bidder),
    ("360yield-basic.com", "Improve Digital", Bidder),
    ("lijit.com", "Sovrn", Bidder),
    ("sharethrough.com", "Sharethrough", Bidder),
    ("taboola.com", "Taboola", Bidder),
    ("outbrain.com", "Outbrain", Bidder),
    ("gumgum.com", "GumGum", Bidder),
    ("omnitagjs.com", "Adyoulike", Bidder),
    ("onetag-sys.com", "OneTag", Bidder),
    ("a-mo.net", "AMX", Bidder),
    ("creativecdn.com", "RTB House", Bidder),
    ("nexx360.io", "Nexx360", Bidder),
    ("seedtag.com", "Seedtag", Bidder),
    ("richaudience.com", "Rich Audience", Bidder),
    ("smilewanted.com", "Smile Wanted", Bidder),
    ("teads.tv", "Teads", Bidder),
    ("3lift.com", "TripleLift", Bidder),
    ("yieldmo.com", "Yieldmo", Bidder),
    ("sonobi.com", "Sonobi", Bidder),
    ("media.net", "Media.net", Bidder),
    ("adsrvr.org", "The Trade Desk", Bidder),
    ("bidswitch.net", "BidSwitch", Bidder),
    ("33across.com", "33Across", Bidder),
    ("smaato.net", "Smaato", Bidder),
    ("emxdgt.com", "EMX", Bidder),
    ("dotomi.com", "Epsilon", Bidder),
    ("unrulymedia.com", "Unruly", Bidder),
    ("justpremium.com", "JustPremium", Bidder),
    ("adagio.io", "Adagio", Bidder),
    ("kargo.com", "Kargo", Bidder),
    ("spotxchange.com", "SpotX", Bidder),
    ("advertising.com", "Yahoo", Bidder),
    // Managed wrappers and yield platforms
    ("adthrive.com", "Raptive", Wrapper),
    ("mediavine.com", "Mediavine", Wrapper),
    ("pub.network", "Freestar", Wrapper),
    ("ezoic.net", "Ezoic", Wrapper),
    ("snigelweb.com", "Snigel", Wrapper),
    ("htlbid.com", "HTL", Wrapper),
    ("pubwise.io", "PubWise", Wrapper),
    ("yieldlove.com", "Yieldlove", Wrapper),
    ("browsi.com", "Browsi", Wrapper),
    ("relevant-digital.com", "Relevant Digital", Wrapper),
    ("nitropay.com", "NitroPay", Wrapper),
    ("playwire.com", "Playwire", Wrapper),
    ("venatusmedia.com", "Venatus", Wrapper),
    // Consent management
    ("cookielaw.org", "OneTrust", Cmp),
    ("onetrust.com", "OneTrust", Cmp),
    ("cookiebot.com", "Cookiebot", Cmp),
    ("consentframework.com", "Sirdata", Cmp),
    ("privacy-center.org", "Didomi", Cmp),
    ("didomi.io", "Didomi", Cmp),
    ("privacy-mgmt.com", "Sourcepoint", Cmp),
    ("sp-prod.net", "Sourcepoint", Cmp),
    ("consensu.org", "IAB TCF CMP", Cmp),
    (
        "fundingchoicesmessages.google.com",
        "Google Funding Choices",
        Cmp,
    ),
    ("usercentrics.eu", "Usercentrics", Cmp),
    ("cookieyes.com", "CookieYes", Cmp),
    ("iubenda.com", "iubenda", Cmp),
    ("trustarc.com", "TrustArc", Cmp),
    ("consentmanager.net", "consentmanager", Cmp),
    ("osano.com", "Osano", Cmp),
    // Verification, brand safety, identity, audience
    ("moatads.com", "Oracle Moat", AdQuality),
    ("doubleverify.com", "DoubleVerify", AdQuality),
    ("dvtps.com", "DoubleVerify", AdQuality),
    ("adsafeprotected.com", "Integral Ad Science", AdQuality),
    ("confiant-integrations.net", "Confiant", AdQuality),
    ("geoedge.be", "GeoEdge", AdQuality),
    ("adlightning.com", "Ad Lightning", AdQuality),
    ("id5-sync.com", "ID5", AdQuality),
    ("rlcdn.com", "LiveRamp", AdQuality),
    ("liveintent.com", "LiveIntent", AdQuality),
    ("crwdcntrl.net", "Lotame", AdQuality),
    ("intentiq.com", "Intent IQ", AdQuality),
    ("uidapi.com", "Unified ID 2.0", AdQuality),
    ("permutive.com", "Permutive", AdQuality),
    ("permutive.app", "Permutive", AdQuality),
    // Analytics and tag management
    ("google-analytics.com", "Google Analytics", Analytics),
    ("googletagmanager.com", "Google Tag Manager", Analytics),
    ("segment.com", "Segment", Analytics),
    ("hotjar.com", "Hotjar", Analytics),
    ("chartbeat.com", "Chartbeat", Analytics),
    ("chartbeat.net", "Chartbeat", Analytics),
    ("scorecardresearch.com", "Comscore", Analytics),
    ("nr-data.net", "New Relic", Analytics),
    ("newrelic.com", "New Relic", Analytics),
    ("sentry-cdn.com", "Sentry", Analytics),
    ("clarity.ms", "Microsoft Clarity", Analytics),
    ("mixpanel.com", "Mixpanel", Analytics),
    ("amplitude.com", "Amplitude", Analytics),
    ("parsely.com", "Parse.ly", Analytics),
    ("facebook.net", "Meta", Analytics),
    ("quantserve.com", "Quantcast", Analytics),
    ("cloudflareinsights.com", "Cloudflare", Analytics),
    ("plausible.io", "Plausible", Analytics),
    // Generic libraries and CDNs
    ("ajax.googleapis.com", "Google Hosted Libraries", Library),
    ("cdnjs.cloudflare.com", "cdnjs", Library),
    ("cdn.jsdelivr.net", "jsDelivr", Library),
    ("unpkg.com", "unpkg", Library),
    ("code.jquery.com", "jQuery", Library),
    ("bootstrapcdn.com", "Bootstrap CDN", Library),
    ("gstatic.com", "Google", Library),
    ("recaptcha.net", "Google reCAPTCHA", Library),
    ("youtube.com", "YouTube", Library),
    ("platform.twitter.com", "X", Library),
];

/// Vendor libraries recognised by file name when served from another host
/// (self-hosted copies, first-party proxies).
const KNOWN_LIBRARY_FILES: &[(&str, &str, ScriptCategory)] = &[
    ("gpt.js", "Google Ad Manager", AdServer),
    ("pubads_impl.js", "Google Ad Manager", AdServer),
    ("apstag.js", "Amazon", Bidder),
];

/// Calls that only publisher ad orchestration code makes.
const STRONG_SIGNATURES: &[&str] = &[
    ".defineSlot(",
    ".defineOutOfPageSlot(",
    ".requestBids(",
    ".addAdUnits(",
    ".setTargetingForGPTAsync(",
    ".fetchBids(",
    "googletag.cmd",
    "googletag.pubads",
    "pbjs.que",
];

const WEAK_IDENTIFIERS: &[&str] = &[
    "googletag",
    "pbjs",
    "apstag",
    "prebid",
    "adUnit",
    "adunit",
    "bidder",
    "hb_",
    "doubleclick",
    "adslot",
    "adSlot",
    "__tcfapi",
    "cpm",
];

/// Three letters that turn up by accident in any large minified bundle: good
/// enough to call a script undecided, too weak to point at a place in it.
const ACCIDENTAL_IDENTIFIER: &str = "cpm";

/// Where a publisher hands its own configuration to the ad libraries.
const CALL_SITES: &[&str] = &["pbjs.que", ".addAdUnits(", ".requestBids(", "googletag.cmd"];

/// Browser APIs that viewport, device and lazy-load decisions are made with.
const PAGE_STATE_APIS: &[&str] = &[
    "IntersectionObserver",
    "matchMedia",
    "innerWidth",
    "userAgent",
    "scroll",
];

const PREBID_LIBRARY_MARKERS: &[&str] = &["registerBidAdapter", "auctionManager", "bidderSettings"];

pub fn classify(facts: &ScriptFacts) -> Verdict {
    let strong = present(facts.body, STRONG_SIGNATURES);
    let known_host = (!facts.inline)
        .then(|| known_host(facts.hostname))
        .flatten();

    if let Some((vendor, category)) = known_host {
        // Tag managers, analytics hosts and public CDNs can carry publisher ad code.
        let may_host_publisher_code = matches!(category, Analytics | Library);
        if !may_host_publisher_code {
            return decided(category, Some(vendor), 0.95, format!("Known {vendor} host"));
        }
        if is_prebid_library(facts) {
            return decided(
                Prebid,
                None,
                0.9,
                format!("Prebid.js build served from {vendor}"),
            );
        }
        if strong.is_empty() {
            return decided(category, Some(vendor), 0.9, format!("Known {vendor} host"));
        }
        return decided(
            Wrapper,
            Some(vendor),
            0.8,
            format!("Served by {vendor} but calls {}", strong.join(", ")),
        );
    }

    if is_prebid_library(facts) {
        return decided(Prebid, None, 0.9, "Prebid.js library signature".into());
    }
    let name = file_name(facts.url);
    if let Some((_, vendor, category)) = KNOWN_LIBRARY_FILES
        .iter()
        .find(|(file, _, _)| !facts.inline && *file == name)
    {
        return decided(
            *category,
            Some(vendor),
            0.8,
            format!("{vendor} library by file name ({name})"),
        );
    }
    if !strong.is_empty() {
        return decided(
            Wrapper,
            None,
            0.85,
            format!("Calls ad stack APIs: {}", strong.join(", ")),
        );
    }

    let weak = present(facts.body, WEAK_IDENTIFIERS);
    if weak.is_empty() {
        let (category, what) = if facts.first_party || facts.inline {
            (Application, "first-party code")
        } else {
            (Library, "third-party code")
        };
        return decided(
            category,
            None,
            0.7,
            format!("No ad-tech identifiers in {what}"),
        );
    }
    Verdict::Ambiguous {
        hints: weak.into_iter().map(str::to_string).collect(),
    }
}

/// How many of the calls that only ad orchestration code makes occur in `body`.
pub fn ad_api_calls(body: &str) -> usize {
    present(body, STRONG_SIGNATURES).len()
}

/// The ad-tech identifiers in `text` that are specific enough to mark it as a
/// place worth a closer look.
pub fn ad_identifiers(text: &str) -> Vec<&'static str> {
    STRONG_SIGNATURES
        .iter()
        .chain(WEAK_IDENTIFIERS)
        .copied()
        .filter(|needle| *needle != ACCIDENTAL_IDENTIFIER && occurrences(text, needle) > 0)
        .collect()
}

/// [`ad_identifiers`] plus the browser APIs ad timing logic branches on.
pub fn logic_anchors(text: &str) -> Vec<&'static str> {
    let mut anchors = ad_identifiers(text);
    anchors.extend(present(text, PAGE_STATE_APIS));
    anchors
}

pub fn has_call_site(text: &str) -> bool {
    !present(text, CALL_SITES).is_empty()
}

/// How often each ad-tech identifier occurs, for the script triage prompt.
pub fn identifier_counts(body: &str) -> Vec<(&'static str, usize)> {
    STRONG_SIGNATURES
        .iter()
        .chain(WEAK_IDENTIFIERS)
        .map(|needle| (*needle, occurrences(body, needle)))
        .filter(|(_, count)| *count > 0)
        .collect()
}

fn decided(
    category: ScriptCategory,
    vendor: Option<&'static str>,
    confidence: f64,
    reason: String,
) -> Verdict {
    Verdict::Decided(Classification {
        category,
        ad_related: is_ad_related(category),
        vendor,
        confidence,
        reason,
    })
}

fn known_host(hostname: &str) -> Option<(&'static str, ScriptCategory)> {
    KNOWN_HOSTS
        .iter()
        .find(|(suffix, _, _)| {
            hostname == *suffix
                || hostname
                    .strip_suffix(suffix)
                    .is_some_and(|prefix| prefix.ends_with('.'))
        })
        .map(|(_, vendor, category)| (*vendor, *category))
}

fn file_name(url: &str) -> String {
    url.split(['?', '#'])
        .next()
        .and_then(|path| path.rsplit('/').next())
        .unwrap_or_default()
        .to_ascii_lowercase()
}

fn is_prebid_library(facts: &ScriptFacts) -> bool {
    let file_name = file_name(facts.url);
    let named_prebid =
        !facts.inline && file_name.starts_with("prebid") && file_name.ends_with(".js");
    let has_library_internals = PREBID_LIBRARY_MARKERS
        .iter()
        .filter(|marker| facts.body.contains(*marker))
        .count()
        >= 2;
    named_prebid || has_library_internals
}

fn present<'a>(body: &str, needles: &[&'a str]) -> Vec<&'a str> {
    needles
        .iter()
        .copied()
        .filter(|needle| occurrences(body, needle) > 0)
        .collect()
}

/// An identifier followed by this is a different name: `googletagmanager` is
/// Google Tag Manager, an analytics loader, and not the `googletag` of GPT.
/// (`googletagservices`, the host GPT is loaded from, still counts.)
const LONGER_NAMES: &[(&str, &str)] = &[("googletag", "manager")];

/// How often `needle` occurs in `text` as the identifier it stands for.
fn occurrences(text: &str, needle: &str) -> usize {
    let other_name = LONGER_NAMES
        .iter()
        .find(|(identifier, _)| *identifier == needle)
        .map(|(_, rest)| *rest);
    text.match_indices(needle)
        .filter(|(at, _)| {
            other_name.is_none_or(|rest| !text[at + needle.len()..].starts_with(rest))
        })
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts<'a>(
        url: &'a str,
        hostname: &'a str,
        first_party: bool,
        body: &'a str,
    ) -> ScriptFacts<'a> {
        ScriptFacts {
            url,
            hostname,
            inline: false,
            first_party,
            body,
        }
    }

    fn decided_as(verdict: Verdict) -> Classification {
        match verdict {
            Verdict::Decided(classification) => classification,
            Verdict::Ambiguous { hints } => panic!("expected a decision, got hints {hints:?}"),
        }
    }

    #[test]
    fn recognises_vendor_hosts_including_subdomains() {
        let gpt = decided_as(classify(&facts(
            "https://securepubads.g.doubleclick.net/tag/js/gpt.js",
            "securepubads.g.doubleclick.net",
            false,
            "",
        )));
        assert_eq!(
            (gpt.category, gpt.vendor, gpt.ad_related),
            (AdServer, Some("Google Ad Manager"), true)
        );

        let bidder = decided_as(classify(&facts(
            "https://fastlane.rubiconproject.com/a/api/fastlane.json",
            "fastlane.rubiconproject.com",
            false,
            "",
        )));
        assert_eq!((bidder.category, bidder.vendor), (Bidder, Some("Magnite")));

        let cmp = decided_as(classify(&facts(
            "https://sdk.privacy-center.org/loader.js",
            "sdk.privacy-center.org",
            false,
            "",
        )));
        assert_eq!((cmp.category, cmp.vendor), (Cmp, Some("Didomi")));
    }

    #[test]
    fn host_suffix_must_match_on_a_label_boundary() {
        let verdict = classify(&facts(
            "https://notcriteo.com/x.js",
            "notcriteo.com",
            false,
            "var a=1",
        ));
        assert_eq!(decided_as(verdict).category, Library);
    }

    #[test]
    fn first_party_wrapper_is_ad_related_by_content() {
        let body = "window.pbjs=window.pbjs||{};pbjs.que.push(function(){pbjs.requestBids({timeout:800})});googletag.defineSlot('/1/a',[300,250],'a')";
        let wrapper = decided_as(classify(&facts(
            "https://pub.example/js/ads.js",
            "pub.example",
            true,
            body,
        )));
        assert_eq!(wrapper.category, Wrapper);
        assert!(wrapper.ad_related);
        assert!(wrapper.reason.contains(".requestBids("));
    }

    #[test]
    fn prebid_library_is_detected_by_name_or_internals() {
        let by_name = decided_as(classify(&facts(
            "https://cdn.jsdelivr.net/npm/prebid.js@9/dist/prebid.js?v=2",
            "cdn.jsdelivr.net",
            false,
            "",
        )));
        assert_eq!(by_name.category, Prebid);
        let by_body = decided_as(classify(&facts(
            "https://pub.example/bundle.min.js",
            "pub.example",
            true,
            "function registerBidAdapter(){} var auctionManager={}",
        )));
        assert_eq!(by_body.category, Prebid);
    }

    #[test]
    fn self_hosted_vendor_libraries_are_recognised_by_file_name() {
        let gpt = decided_as(classify(&facts(
            "https://pub.example/tag/js/gpt.js?v=2",
            "pub.example",
            true,
            "googletag.cmd=googletag.cmd||[];function defineSlot(){}",
        )));
        assert_eq!(
            (gpt.category, gpt.vendor),
            (AdServer, Some("Google Ad Manager"))
        );
        let aps = decided_as(classify(&facts(
            "https://pub.example/ads/apstag.js",
            "pub.example",
            true,
            "apstag",
        )));
        assert_eq!((aps.category, aps.vendor), (Bidder, Some("Amazon")));
    }

    #[test]
    fn tag_manager_container_with_ad_code_becomes_wrapper() {
        let container = decided_as(classify(&facts(
            "https://www.googletagmanager.com/gtm.js?id=GTM-1",
            "www.googletagmanager.com",
            false,
            "googletag.cmd.push(function(){x.defineSlot('/1/a',[1,1],'a')})",
        )));
        assert_eq!(container.category, Wrapper);
        let plain = decided_as(classify(&facts(
            "https://www.googletagmanager.com/gtm.js?id=GTM-1",
            "www.googletagmanager.com",
            false,
            "dataLayer.push({})",
        )));
        assert_eq!((plain.category, plain.ad_related), (Analytics, false));
    }

    #[test]
    fn code_without_identifiers_is_not_sent_to_the_model() {
        let app = decided_as(classify(&facts(
            "https://pub.example/app.js",
            "pub.example",
            true,
            "render(App)",
        )));
        assert_eq!((app.category, app.ad_related), (Application, false));
    }

    #[test]
    fn weak_identifiers_alone_are_ambiguous() {
        let verdict = classify(&facts(
            "https://pub.example/lazy.js",
            "pub.example",
            true,
            "if(isMobile){loadAds(window.adUnits)}; var cpm=0;",
        ));
        assert_eq!(
            verdict,
            Verdict::Ambiguous {
                hints: vec!["adUnit".into(), "cpm".into()]
            }
        );
    }

    #[test]
    fn anchors_ignore_the_accidental_identifier_and_add_page_state_apis() {
        assert!(ad_identifiers("var cpm=0;").is_empty());
        assert_eq!(
            ad_identifiers("x.adUnits;pbjs.que.push(f)"),
            ["pbjs.que", "pbjs", "adUnit"]
        );
        assert_eq!(
            logic_anchors("if(window.innerWidth>768){googletag.display(id)}"),
            ["googletag", "innerWidth"]
        );
        assert!(logic_anchors("document.title='x'").is_empty());
        assert_eq!(
            ad_api_calls("pbjs.que.push(function(){pbjs.requestBids({})})"),
            2
        );
        assert!(has_call_site("a.requestBids({})") && !has_call_site("requestBids"));
    }

    #[test]
    fn google_tag_manager_is_not_taken_for_the_gpt_global() {
        let loader = "(function(w,d,s,l,i){j.src='https://www.googletagmanager.com/gtm.js?id='+i})(window,document,'script','dataLayer','GTM-X')";
        assert!(ad_identifiers(loader).is_empty());
        assert!(identifier_counts(loader).is_empty());
        let verdict = classify(&ScriptFacts {
            url: "inline:3",
            hostname: "pub.example",
            first_party: true,
            inline: true,
            body: loader,
        });
        assert_eq!(
            decided_as(verdict).category,
            Application,
            "a tag manager loader is no triage candidate"
        );
        // GPT itself, next to the tag manager and by its loading host, still counts.
        assert_eq!(
            identifier_counts("googletagmanager.com;window.googletag=window.googletag||{}"),
            [("googletag", 2)]
        );
        assert_eq!(
            ad_identifiers("s.src='https://www.googletagservices.com/tag/js/gpt.js'"),
            ["googletag"]
        );
    }

    #[test]
    fn identifier_counts_report_occurrences() {
        let counts = identifier_counts("pbjs.que.push(a);pbjs.que.push(b);googletag");
        assert!(counts.contains(&("pbjs.que", 2)));
        assert!(counts.contains(&("pbjs", 2)));
        assert!(counts.contains(&("googletag", 1)));
    }
}
