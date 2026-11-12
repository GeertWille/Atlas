//! The yes/no questions put to the decision model about a piece of script
//! source. Written the way its documentation asks: literal, one judgment per
//! question, with the yes case and the no case each described as a concrete
//! situation.
//!
//! The wording has not been measured against labelled scripts. Thresholds in
//! `triage` and `guide` are tied to this exact text; change the version when
//! the text changes so audit entries say which set produced a number.

pub const QUESTION_SET_VERSION: &str = "jev-questions-v1";

/// What a request costs before any state or question is counted. Inferred
/// from the usage of the documentation's smallest examples; uncalibrated.
pub const REQUEST_OVERHEAD_TOKENS: u64 = 300;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct YesNoQuestion {
    /// Unique within a request. Never shown to the model.
    pub id: &'static str,
    pub instructions: &'static str,
    /// The concrete situation in which the answer is yes.
    pub yes: &'static str,
    /// The concrete situation in which the answer is no.
    pub no: &'static str,
}

pub const AD_ORCHESTRATION: usize = 0;
pub const PUBLISHER_SPECIFIC: usize = 1;
pub const AUCTION_GATING: usize = 2;
pub const DEVICE_DETECTION: usize = 3;
pub const TIMING_CONFIG: usize = 4;
pub const LAZY_LOAD: usize = 5;
pub const SLOT_DEFINITION: usize = 6;
pub const CONSENT_GATING: usize = 7;

/// Indexed by the constants above. Script triage asks the first two; the
/// reading guide asks all of them in one request, which bills the code once.
pub const QUESTIONS: [YesNoQuestion; 8] = [
    YesNoQuestion {
        id: "ad_orchestration",
        instructions: "Does the JavaScript in `code` call or configure an advertising stack for a web page?",
        yes: "The code itself calls functions such as requestBids, addAdUnits, setConfig, defineSlot, display, refresh, setTargetingForGPTAsync or fetchBids, or defines the ad units, bidders or slot sizes of a site.",
        no: "The code is unrelated to advertising, or only mentions an advertising word inside a string, comment, URL or analytics event without calling or configuring an ad stack.",
    },
    YesNoQuestion {
        id: "publisher_specific",
        instructions: "Is the JavaScript in `code` written or configured for one specific website, rather than being a generic library that is identical on every site that uses it?",
        yes: "It contains site-specific values: ad unit codes or paths, element ids of ad containers, bidder account or placement ids, page-specific conditions, or glue code that wires libraries together for one site.",
        no: "Generic reusable library internals: adapter implementations, utilities, polyfills or framework code with no site-specific identifiers.",
    },
    YesNoQuestion {
        id: "auction_gating",
        instructions: "Does the code in `code` decide whether or when an ad auction or ad request starts?",
        yes: "A condition, event listener, timer, promise or callback controls when requestBids, fetchBids, refresh or display is called.",
        no: "Ad functions are called unconditionally, or no ad auction or ad request is started in this code.",
    },
    YesNoQuestion {
        id: "device_detection",
        instructions: "Does the code in `code` determine the visitor's device type, screen size or viewport?",
        yes: "It reads userAgent, innerWidth, screen size, matchMedia, touch support or a mobile/desktop flag and branches on the result.",
        no: "It does not inspect the device, screen or viewport.",
    },
    YesNoQuestion {
        id: "timing_config",
        instructions: "Does the code in `code` set a timeout or delay for ad auctions or ad loading?",
        yes: "It sets bidderTimeout, an auction timeout or a failsafe timer, or passes a number of milliseconds to requestBids, setTimeout or a similar call in ad-loading code.",
        no: "No timeout or delay for ads is set here.",
    },
    YesNoQuestion {
        id: "lazy_load",
        instructions: "Does the code in `code` wait until an ad slot is near the visible area, or until the user scrolls, before requesting ads?",
        yes: "It uses IntersectionObserver, a scroll listener, getBoundingClientRect or enableLazyLoad to trigger ad requests.",
        no: "Ads are not tied to visibility or scrolling in this code.",
    },
    YesNoQuestion {
        id: "slot_definition",
        instructions: "Does the code in `code` define ad slots or ad units?",
        yes: "It lists ad unit codes, slot element ids, ad unit paths, sizes or the bidders for each unit, or calls defineSlot or addAdUnits.",
        no: "No ad slot or ad unit is defined here.",
    },
    YesNoQuestion {
        id: "consent_gating",
        instructions: "Does the code in `code` wait for or check user consent before running ad code?",
        yes: "It calls __tcfapi or a consent platform callback, or reads a consent cookie or flag, and ad code depends on the result.",
        no: "No consent check controls ad code here.",
    },
];

pub fn triage_questions() -> &'static [YesNoQuestion] {
    &QUESTIONS[..=PUBLISHER_SPECIFIC]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn question_ids_are_unique_and_the_index_constants_match() {
        let ids: std::collections::BTreeSet<&str> =
            QUESTIONS.iter().map(|question| question.id).collect();
        assert_eq!(ids.len(), QUESTIONS.len());
        assert_eq!(QUESTIONS[AD_ORCHESTRATION].id, "ad_orchestration");
        assert_eq!(QUESTIONS[PUBLISHER_SPECIFIC].id, "publisher_specific");
        assert_eq!(QUESTIONS[AUCTION_GATING].id, "auction_gating");
        assert_eq!(QUESTIONS[DEVICE_DETECTION].id, "device_detection");
        assert_eq!(QUESTIONS[TIMING_CONFIG].id, "timing_config");
        assert_eq!(QUESTIONS[LAZY_LOAD].id, "lazy_load");
        assert_eq!(QUESTIONS[SLOT_DEFINITION].id, "slot_definition");
        assert_eq!(QUESTIONS[CONSENT_GATING].id, "consent_gating");
        assert_eq!(
            triage_questions()
                .iter()
                .map(|question| question.id)
                .collect::<Vec<_>>(),
            ["ad_orchestration", "publisher_specific"]
        );
    }
}
