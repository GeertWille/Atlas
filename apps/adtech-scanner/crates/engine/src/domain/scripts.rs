//! Script metadata derived from captured scripts. Bodies never leave memory.

use adscan_model::{Capture, CapturedScript, FindingSource, ScriptCategory, ScriptRecord};
use sha2::{Digest, Sha256};
use url::Url;

use super::knowledge::{self, ScriptFacts, Verdict};
use super::triage::ANALYSE_THRESHOLD;

/// Script categories that can contain the publisher's own bidding logic.
const ANALYZABLE: &[ScriptCategory] = &[
    ScriptCategory::Wrapper,
    ScriptCategory::Prebid,
    ScriptCategory::Unknown,
];

/// Script URLs are persisted and shown; this keeps them from growing without bound.
const MAX_STORED_URL_CHARS: usize = 2_000;

/// A script record plus what the knowledge base could not decide about it.
#[derive(Debug, Clone, PartialEq)]
pub struct DescribedScript {
    pub record: ScriptRecord,
    /// `Some` when the knowledge base left the decision to script triage.
    pub ambiguous_hints: Option<Vec<String>>,
    /// How many kinds of ad stack calls the body makes.
    pub ad_api_calls: usize,
    /// Whether the body configures or hooks into a consent platform.
    pub sets_up_consent: bool,
    /// The body starts auctions or defines slots itself. In a file that is
    /// otherwise a vendor library, that is the publisher's wrapper bundled
    /// into it.
    pub orchestrates: bool,
}

/// One record per captured script, in capture order, with ids `s1`, `s2`, ...
pub fn describe(capture: &Capture) -> Vec<DescribedScript> {
    let page_host = hostname_of(&capture.final_url);
    let page_domain = registrable_domain(&page_host);
    capture
        .scripts
        .iter()
        .enumerate()
        .map(|(index, script)| describe_one(index, script, &page_host, page_domain.as_deref()))
        .collect()
}

fn describe_one(
    index: usize,
    script: &CapturedScript,
    page_host: &str,
    page_domain: Option<&str>,
) -> DescribedScript {
    let hostname = if script.inline {
        page_host.to_string()
    } else {
        hostname_of(&script.url)
    };
    let first_party = script.inline
        || hostname == page_host
        || (page_domain.is_some() && registrable_domain(&hostname).as_deref() == page_domain);
    let verdict = knowledge::classify(&ScriptFacts {
        url: &script.url,
        hostname: &hostname,
        inline: script.inline,
        first_party,
        body: &script.body,
    });
    let mut record = ScriptRecord {
        id: format!("s{}", index + 1),
        url: stored_url(&script.url),
        hostname,
        inline: script.inline,
        size_bytes: script.body.len() as f64,
        hash: content_hash(&script.body),
        category: ScriptCategory::Unknown,
        ad_related: false,
        classification_reason: None,
        classification_confidence: None,
        classified_by: FindingSource::Heuristic,
        classifier_model: None,
        ad_probability: None,
        analyzed: false,
        vendor: None,
        loaded_at_ms: script.loaded_at_ms,
        first_party,
        pages: script.pages.clone(),
        initiator: None,
        request_id: None,
        sha256: None,
    };
    let ambiguous_hints = match verdict {
        Verdict::Decided(classification) => {
            record.category = classification.category;
            record.ad_related = classification.ad_related;
            record.vendor = classification.vendor.map(str::to_string);
            record.classification_confidence = Some(classification.confidence);
            record.classification_reason = Some(classification.reason);
            None
        }
        Verdict::Ambiguous { hints } => {
            record.classification_reason = Some(format!(
                "Mentions {} but nothing conclusive; not classified by AI",
                hints.join(", ")
            ));
            Some(hints)
        }
    };
    DescribedScript {
        record,
        ambiguous_hints,
        ad_api_calls: knowledge::ad_api_calls(&script.body),
        sets_up_consent: knowledge::sets_up_consent(&script.body),
        orchestrates: knowledge::orchestrates(&script.body),
    }
}

/// Indexes (into the script list) of the scripts worth handing to Phase 2,
/// most relevant first: ad-related code that can hold publisher logic,
/// bounded in count and total size. Of scripts with identical bodies only the
/// first is taken.
pub fn select_for_analysis(
    scripts: &[DescribedScript],
    max_scripts: usize,
    max_total_bytes: f64,
) -> Vec<usize> {
    let mut candidates: Vec<usize> = scripts
        .iter()
        .enumerate()
        .filter(|(_, script)| {
            let record = &script.record;
            // A consent script that is no known vendor's library is the
            // site's own consent setup, which decides when bidding may start.
            let own_consent_setup =
                record.category == ScriptCategory::Cmp && record.vendor.is_none();
            record.ad_related && (ANALYZABLE.contains(&record.category) || own_consent_setup)
        })
        .map(|(index, _)| index)
        .collect();
    candidates.sort_by(|&a, &b| {
        analysis_rank(&scripts[a])
            .partial_cmp(&analysis_rank(&scripts[b]))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.cmp(&b))
    });
    let mut total = 0.0;
    let mut selected: Vec<usize> = Vec::new();
    for index in candidates {
        let record = &scripts[index].record;
        let duplicate = selected
            .iter()
            .any(|&taken| scripts[taken].record.hash == record.hash);
        if !duplicate
            && selected.len() < max_scripts
            && total + record.size_bytes <= max_total_bytes
        {
            total += record.size_bytes;
            selected.push(index);
        }
    }
    selected
}

/// Sort key that puts the publisher's own orchestration code first and stock
/// vendor libraries last. A Prebid build with the publisher's wrapper bundled
/// into it is orchestration code: where the limits bite it must not be the
/// script that is left out. Smaller sorts earlier.
fn analysis_rank(script: &DescribedScript) -> (u8, bool, std::cmp::Reverse<usize>, f64) {
    let record = &script.record;
    let unsure = record
        .ad_probability
        .is_some_and(|probability| probability < ANALYSE_THRESHOLD);
    let tier = match record.category {
        ScriptCategory::Wrapper => 0,
        ScriptCategory::Prebid if script.orchestrates => 0,
        ScriptCategory::Prebid => 3,
        ScriptCategory::Cmp => 1,
        _ if unsure => 2,
        _ => 1,
    };
    (
        tier,
        !record.first_party,
        // A real wrapper makes several kinds of ad calls; a one-line inline
        // snippet makes one.
        std::cmp::Reverse(script.ad_api_calls),
        -record.ad_probability.unwrap_or(0.0),
    )
}

/// The URL as it may be persisted. A `data:` URL is the script body itself,
/// which must never be stored, so only its media type is kept.
fn stored_url(url: &str) -> String {
    if let Some(rest) = url.strip_prefix("data:") {
        let media_type: String = rest
            .chars()
            .take_while(|c| !matches!(c, ',' | ';'))
            .take(100)
            .collect();
        return format!("data:{media_type},[{} bytes not stored]", rest.len());
    }
    match url.char_indices().nth(MAX_STORED_URL_CHARS) {
        Some((cut, _)) => format!("{}…", &url[..cut]),
        None => url.to_string(),
    }
}

fn content_hash(body: &str) -> String {
    let digest = Sha256::digest(body.as_bytes());
    digest
        .iter()
        .take(8)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn hostname_of(url: &str) -> String {
    Url::parse(url)
        .ok()
        .and_then(|parsed| parsed.host_str().map(str::to_string))
        .unwrap_or_default()
}

/// eTLD+1 according to the public suffix list (`news.example.co.uk` -> `example.co.uk`).
fn registrable_domain(hostname: &str) -> Option<String> {
    psl::domain_str(hostname).map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn capture_with(scripts: Vec<CapturedScript>) -> Capture {
        Capture {
            scripts,
            ..Capture::single_page("https://www.news.example.co.uk/article", Vec::new())
        }
    }

    fn script(url: &str, inline: bool, body: &str) -> CapturedScript {
        CapturedScript {
            pages: vec![0],
            url: url.into(),
            inline,
            body: body.into(),
            loaded_at_ms: Some(120.0),
            initiator: None,
            request_seq: None,
        }
    }

    #[test]
    fn first_party_is_decided_by_registrable_domain() {
        let described = describe(&capture_with(vec![
            script("https://static.example.co.uk/app.js", false, "boot()"),
            script("https://other.co.uk/lib.js", false, "boot()"),
            script("inline:1", true, "boot()"),
        ]));
        let first_party: Vec<bool> = described.iter().map(|d| d.record.first_party).collect();
        assert_eq!(first_party, [true, false, true]);
        assert_eq!(described[0].record.hostname, "static.example.co.uk");
        assert_eq!(described[2].record.hostname, "www.news.example.co.uk");
        assert_eq!(described[0].record.id, "s1");
        assert_eq!(described[2].record.id, "s3");
    }

    #[test]
    fn data_urls_and_oversized_urls_are_not_stored_verbatim() {
        let payload = "pbjs.requestBids({timeout:1500});".repeat(50);
        let long_query = format!("https://cdn.example/ads.js?cb={}", "9".repeat(5_000));
        let described = describe(&capture_with(vec![
            script(
                &format!("data:text/javascript;charset=utf-8,{payload}"),
                false,
                &payload,
            ),
            script(&long_query, false, "boot()"),
        ]));
        let inline_url = &described[0].record.url;
        assert!(
            inline_url.starts_with("data:text/javascript,["),
            "{inline_url}"
        );
        assert!(inline_url.ends_with(" bytes not stored]"));
        assert!(!inline_url.contains("requestBids"));
        let long_url = &described[1].record.url;
        assert_eq!(long_url.chars().count(), MAX_STORED_URL_CHARS + 1);
        assert!(long_url.starts_with("https://cdn.example/ads.js?cb=999"));
        assert_eq!(described[1].record.hostname, "cdn.example");
    }

    #[test]
    fn hash_is_a_stable_sha256_prefix() {
        assert_eq!(content_hash("abc"), "ba7816bf8f01cfea");
        assert_eq!(content_hash("abc").len(), 16);
    }

    #[test]
    fn ambiguous_scripts_keep_their_hints() {
        let described = describe(&capture_with(vec![script(
            "https://static.example.co.uk/lazy.js",
            false,
            "loadAds(window.adUnits)",
        )]));
        assert_eq!(
            described[0].ambiguous_hints,
            Some(vec!["adUnit".to_string()])
        );
        assert!(!described[0].record.ad_related);
        assert_eq!(described[0].record.category, ScriptCategory::Unknown);
    }

    #[test]
    fn analysis_selection_prefers_wrappers_and_respects_limits() {
        let described = describe(&capture_with(vec![
            script(
                "https://cdn.jsdelivr.net/npm/prebid.js",
                false,
                &"x".repeat(900),
            ),
            script(
                "https://securepubads.g.doubleclick.net/tag/js/gpt.js",
                false,
                "gpt",
            ),
            script(
                "https://static.example.co.uk/ads.js",
                false,
                "pbjs.que.push(f)",
            ),
            script("https://static.example.co.uk/app.js", false, "boot()"),
        ]));
        assert_eq!(select_for_analysis(&described, 10, 1e6), vec![2, 0]);
        assert_eq!(select_for_analysis(&described, 1, 1e6), vec![2]);
        assert_eq!(select_for_analysis(&described, 10, 500.0), vec![2]);
    }

    #[test]
    fn analysis_selection_ranks_publisher_orchestration_first_and_vendor_libraries_last() {
        let wrapper = "pbjs.que.push(function(){pbjs.addAdUnits(u);pbjs.requestBids({})});googletag.cmd.push(function(){googletag.defineSlot(p,s,i)})";
        let mut described = describe(&capture_with(vec![
            script("https://cdn.jsdelivr.net/npm/prebid.js", false, "lib()"),
            script(
                "inline:1",
                true,
                "googletag.cmd.push(function(){googletag.display('a')})",
            ),
            script("https://static.example.co.uk/unsure.js", false, "x.adUnits"),
            script("https://static.example.co.uk/likely.js", false, "y.adUnits"),
            script("https://static.example.co.uk/ads.js", false, wrapper),
            script(
                "https://other.example/third-party-wrapper.js",
                false,
                &format!("{wrapper};"),
            ),
        ]));
        for (index, probability) in [(2, 0.2), (3, 0.8)] {
            described[index].record.ad_related = true;
            described[index].record.ad_probability = Some(probability);
        }
        assert_eq!(
            select_for_analysis(&described, 10, 1e6),
            vec![4, 1, 5, 3, 2, 0],
            "first-party wrapper with most ad calls, inline snippet, third-party wrapper, likely, unsure, Prebid"
        );
    }

    /// The publisher ships its wrapper inside the Prebid file, under its own
    /// namespace and with the Prebid global aliased. Where not everything
    /// fits, that file is analysed before any stock library.
    #[test]
    fn a_prebid_build_with_the_wrapper_bundled_into_it_is_not_ranked_as_a_stock_library() {
        let bundle = "var auctionManager={};function registerBidAdapter(){}\n\
                      ASCDP.adS={init:function(){var x=window[ns];x.que.push(function(){x.requestBids({timeout:900})})}};";
        let described = describe(&capture_with(vec![
            script(
                "https://cdn.jsdelivr.net/npm/prebid.js",
                false,
                "var auctionManager={};function registerBidAdapter(){}",
            ),
            script(
                "inline:1",
                true,
                "googletag.cmd.push(function(){googletag.display('a')})",
            ),
            script(
                "https://static.example.co.uk/hb/prebid-bundle.js",
                false,
                bundle,
            ),
        ]));
        assert_eq!(described[2].record.category, ScriptCategory::Prebid);
        assert!(described[2].orchestrates && !described[0].orchestrates);
        assert_eq!(select_for_analysis(&described, 10, 1e6), vec![1, 2, 0]);
        assert_eq!(
            select_for_analysis(&described, 2, 1e6),
            vec![1, 2],
            "the stock library is what is left out, not the bundle"
        );
    }

    #[test]
    fn the_sites_own_consent_setup_is_analysed_and_a_vendors_consent_library_is_not() {
        let mut described = describe(&capture_with(vec![
            script("inline:1", true, "window.didomiConfig={app:{apiKey:'0'}}"),
            script("https://sdk.privacy-center.org/loader.js", false, "lib()"),
        ]));
        assert!(described[0].sets_up_consent && !described[1].sets_up_consent);
        // As script triage leaves the inline configuration.
        described[0].record.category = ScriptCategory::Cmp;
        described[0].record.ad_related = true;
        described[0].record.ad_probability = Some(0.25);
        assert_eq!(described[1].record.category, ScriptCategory::Cmp);
        assert_eq!(select_for_analysis(&described, 10, 1e6), vec![0]);
    }

    #[test]
    fn analysis_selection_takes_identical_bodies_once() {
        let described = describe(&capture_with(vec![
            script(
                "https://static.example.co.uk/ads.js",
                false,
                "pbjs.que.push(f)",
            ),
            script(
                "https://static.example.co.uk/ads.js?again",
                false,
                "pbjs.que.push(f)",
            ),
        ]));
        assert_eq!(select_for_analysis(&described, 10, 1e6), vec![0]);
    }
}
