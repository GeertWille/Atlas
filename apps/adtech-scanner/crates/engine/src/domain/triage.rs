//! Script triage policy for the decision model: which parts of an undecided
//! script are worth asking about, and how the probabilities that come back
//! route the script.
//!
//! Only scripts the knowledge base left undecided get here, so a script with
//! a strong ad signature can never be dropped by a probability.
//!
//! Every number is a recall-biased starting value that has not been
//! calibrated: wrongly keeping a script costs a little analysis time, wrongly
//! dropping a publisher's wrapper loses the findings the scan exists for.

use adscan_model::ScriptCategory;

use super::knowledge::ad_identifiers;
use super::questions::{AD_ORCHESTRATION, PUBLISHER_SPECIFIC};

/// At or above this the script is analysed.
pub const ANALYSE_THRESHOLD: f64 = 0.50;
/// Below this on every screened part the script is dropped.
pub const DROP_THRESHOLD: f64 = 0.15;
/// At or above this an ad-related script counts as the publisher's own code.
pub const PUBLISHER_SPECIFIC_THRESHOLD: f64 = 0.50;
/// Keeps one identifier-dense bundle from using up the calls of a scan.
pub const MAX_SCREENED_PARTS: usize = 24;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    Analyse,
    /// Neither confirmed nor ruled out: kept for analysis, ranked last.
    Unsure,
    Drop,
}

impl Route {
    /// The rule name the audit trail and the stored reason carry.
    pub fn rule(self) -> &'static str {
        match self {
            Self::Analyse => "jev-yes",
            Self::Unsure => "jev-unsure",
            Self::Drop => "jev-no",
        }
    }
}

/// One part of a script chosen for screening.
#[derive(Debug, Clone, PartialEq)]
pub struct ScreenedPart {
    /// Position among the script's parts.
    pub index: usize,
    /// Why it was chosen; empty for the head of the script.
    pub identifiers: Vec<&'static str>,
}

/// The parts worth asking about: the head, which shows what kind of file this
/// is, and every part that names an ad-tech identifier. When there are more
/// than the cap, the parts naming the most identifiers are kept.
pub fn parts_to_screen(parts: &[&str]) -> Vec<ScreenedPart> {
    let mut chosen: Vec<ScreenedPart> = parts
        .iter()
        .enumerate()
        .map(|(index, part)| ScreenedPart {
            index,
            identifiers: ad_identifiers(part),
        })
        .filter(|part| part.index == 0 || !part.identifiers.is_empty())
        .collect();
    if chosen.len() > MAX_SCREENED_PARTS {
        chosen.sort_by_key(|part| {
            (
                part.index != 0,
                std::cmp::Reverse(part.identifiers.len()),
                part.index,
            )
        });
        chosen.truncate(MAX_SCREENED_PARTS);
        chosen.sort_by_key(|part| part.index);
    }
    chosen
}

/// The answers for one screened part, in question order.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PartScore {
    pub ad_orchestration: f64,
    pub publisher_specific: f64,
}

impl PartScore {
    /// `probabilities` are the answers to the triage questions, in order.
    pub fn from_answers(probabilities: &[f64]) -> Option<Self> {
        Some(Self {
            ad_orchestration: *probabilities.get(AD_ORCHESTRATION)?,
            publisher_specific: *probabilities.get(PUBLISHER_SPECIFIC)?,
        })
    }

    /// Whether this part alone already gives the script its best possible
    /// verdict, so its remaining parts need not be asked about.
    pub fn settles_the_script(&self) -> bool {
        self.ad_orchestration >= ANALYSE_THRESHOLD
            && self.publisher_specific >= PUBLISHER_SPECIFIC_THRESHOLD
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TriageDecision {
    pub route: Route,
    pub ad_related: bool,
    pub category: ScriptCategory,
    /// The script's score: the highest ad-orchestration probability of any
    /// screened part. Not the mean, which would bury one wrapper function in a
    /// large bundle, and not a noisy-or, which a hundred weak parts would push
    /// to certainty.
    pub ad_probability: f64,
    /// How site-specific the script's ad code is: the highest such probability
    /// among the parts that count as ad code, or of the best-scoring part when
    /// none does.
    pub publisher_specific: f64,
    /// Probability that `ad_related` is right.
    pub confidence: f64,
    /// Built from the numbers and the rule only; never contains script text.
    pub reason: String,
}

/// Routes a script from the scores of its screened parts. `None` when no part
/// was scored.
pub fn decide(
    scores: &[PartScore],
    total_parts: usize,
    first_party: bool,
) -> Option<TriageDecision> {
    let best = scores
        .iter()
        .max_by(|a, b| a.ad_orchestration.total_cmp(&b.ad_orchestration))?;
    let p = best.ad_orchestration;
    let screened = scores.len();
    let publisher_specific = scores
        .iter()
        .filter(|score| score.ad_orchestration >= ANALYSE_THRESHOLD)
        .map(|score| score.publisher_specific)
        .reduce(f64::max)
        .unwrap_or(best.publisher_specific);
    let decision = if p >= ANALYSE_THRESHOLD {
        // The publisher's own code is what the analysis looks for first.
        let category = if publisher_specific >= PUBLISHER_SPECIFIC_THRESHOLD {
            ScriptCategory::Wrapper
        } else {
            ScriptCategory::Unknown
        };
        TriageDecision {
            route: Route::Analyse,
            ad_related: true,
            category,
            ad_probability: p,
            publisher_specific,
            confidence: p,
            reason: format!(
                "Jev: ad orchestration {p:.2} in its best-scoring part, publisher-specific {publisher_specific:.2}; rule {} (>= {ANALYSE_THRESHOLD:.2})",
                Route::Analyse.rule()
            ),
        }
    } else if p >= DROP_THRESHOLD {
        TriageDecision {
            route: Route::Unsure,
            ad_related: true,
            category: ScriptCategory::Unknown,
            ad_probability: p,
            publisher_specific,
            confidence: p,
            reason: format!(
                "Jev: ad orchestration {p:.2} at most over {screened} screened parts of {total_parts}; rule {} ({DROP_THRESHOLD:.2} to {ANALYSE_THRESHOLD:.2}), kept for analysis and ranked last",
                Route::Unsure.rule()
            ),
        }
    } else {
        TriageDecision {
            route: Route::Drop,
            ad_related: false,
            category: if first_party {
                ScriptCategory::Application
            } else {
                ScriptCategory::Library
            },
            ad_probability: p,
            publisher_specific,
            confidence: 1.0 - p,
            reason: format!(
                "Jev: ad orchestration {p:.2} at most over {screened} screened parts of {total_parts}; rule {} (< {DROP_THRESHOLD:.2})",
                Route::Drop.rule()
            ),
        }
    };
    Some(decision)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn score(ad_orchestration: f64, publisher_specific: f64) -> PartScore {
        PartScore {
            ad_orchestration,
            publisher_specific,
        }
    }

    #[test]
    fn routing_follows_the_thresholds_at_their_boundaries() {
        let route = |p: f64| decide(&[score(p, 0.0)], 1, true).unwrap().route;
        assert_eq!(route(0.50), Route::Analyse);
        assert_eq!(route(0.4999), Route::Unsure);
        assert_eq!(route(0.15), Route::Unsure);
        assert_eq!(route(0.1499), Route::Drop);
        assert_eq!(route(0.0), Route::Drop);
        assert_eq!(route(1.0), Route::Analyse);
        assert_eq!(decide(&[], 3, true), None);
    }

    #[test]
    fn the_script_score_is_the_highest_part_not_the_average() {
        let mut scores = vec![score(0.02, 0.0); 99];
        scores.push(score(0.91, 0.2));
        let decision = decide(&scores, 131, true).unwrap();
        assert_eq!(decision.route, Route::Analyse);
        assert_eq!(decision.ad_probability, 0.91);
        assert_eq!(decision.confidence, 0.91);
        assert!(decision.ad_related);
    }

    #[test]
    fn publisher_specific_ad_code_is_a_wrapper_and_other_ad_code_stays_unknown() {
        let wrapper = decide(&[score(0.9, 0.5)], 1, true).unwrap();
        assert_eq!(wrapper.category, ScriptCategory::Wrapper);
        let generic = decide(&[score(0.9, 0.49)], 1, true).unwrap();
        assert_eq!(generic.category, ScriptCategory::Unknown);
        // A site-specific part that is not ad code does not make a wrapper.
        let elsewhere = decide(&[score(0.9, 0.1), score(0.1, 0.95)], 2, true).unwrap();
        assert_eq!(elsewhere.category, ScriptCategory::Unknown);
        assert_eq!(elsewhere.publisher_specific, 0.1);
    }

    #[test]
    fn the_unsure_band_is_kept_with_low_confidence() {
        let unsure = decide(&[score(0.3, 0.9)], 4, false).unwrap();
        assert!(unsure.ad_related);
        assert_eq!(unsure.category, ScriptCategory::Unknown);
        assert_eq!(unsure.confidence, 0.3);
        assert!(unsure.reason.contains("jev-unsure"));
    }

    #[test]
    fn a_dropped_script_is_application_or_library_code_by_origin() {
        let own = decide(&[score(0.05, 0.9)], 1, true).unwrap();
        assert!(!own.ad_related);
        assert_eq!(own.category, ScriptCategory::Application);
        assert!((own.confidence - 0.95).abs() < 1e-12);
        let third_party = decide(&[score(0.05, 0.0)], 1, false).unwrap();
        assert_eq!(third_party.category, ScriptCategory::Library);
        assert_eq!(
            third_party.reason,
            "Jev: ad orchestration 0.05 at most over 1 screened parts of 1; rule jev-no (< 0.15)"
        );
    }

    #[test]
    fn only_a_part_that_is_both_ad_code_and_site_specific_settles_a_script() {
        assert!(score(0.5, 0.5).settles_the_script());
        assert!(!score(0.9, 0.4).settles_the_script());
        assert!(!score(0.4, 0.9).settles_the_script());
    }

    #[test]
    fn the_head_and_parts_naming_ad_identifiers_are_screened() {
        let parts = ["boot();", "render();", "x.adUnits.push(u);", "var cpm=1;"];
        let chosen = parts_to_screen(&parts);
        assert_eq!(
            chosen,
            [
                ScreenedPart {
                    index: 0,
                    identifiers: vec![]
                },
                ScreenedPart {
                    index: 2,
                    identifiers: vec!["adUnit"]
                },
            ]
        );
    }

    #[test]
    fn beyond_the_cap_the_parts_naming_most_identifiers_are_kept_with_the_head() {
        let mut parts = vec!["boot();"];
        parts.extend(std::iter::repeat_n("a.bidder;", 30));
        parts.push("pbjs.adUnits.bidder;");
        let chosen = parts_to_screen(&parts);
        assert_eq!(chosen.len(), MAX_SCREENED_PARTS);
        assert_eq!(chosen[0].index, 0);
        assert_eq!(chosen.last().unwrap().index, 31);
        assert!(chosen.windows(2).all(|pair| pair[0].index < pair[1].index));
    }
}
