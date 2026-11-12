//! The reading guide for the logic analysis: which stretches of the selected
//! scripts hold the publisher's ad logic, ranked, so the analysis model can
//! start there instead of searching for it.
//!
//! The weights, thresholds and caps are first guesses that have not been
//! calibrated against real answers.

use std::collections::BTreeSet;

use super::chunks::LineRange;
use super::knowledge::{ad_api_calls, has_call_site, logic_anchors};
use super::questions::{
    AD_ORCHESTRATION, AUCTION_GATING, CONSENT_GATING, DEVICE_DETECTION, LAZY_LOAD,
    PUBLISHER_SPECIFIC, SLOT_DEFINITION, TIMING_CONFIG,
};
use super::telemetry::Telemetry;

/// Ranges ranked below this are left out of the guide.
pub const MIN_RANK: f64 = 0.30;
pub const MAX_ENTRIES: usize = 20;
/// Chunks one scan may ask the decision model about, across all scripts.
pub const MAX_SCREENED_CHUNKS: usize = 60;
/// Both must hold for a stretch of a vendor library to count as the
/// publisher's own configuration.
const PROMOTION_THRESHOLD: f64 = 0.50;
/// An aspect is named next to a range when its probability reaches this.
const ASPECT_SHOWN_FROM: f64 = 0.50;
const MAX_ASPECTS_SHOWN: usize = 3;
/// Shorter names (`top`, `side`) occur in any library by accident.
const MIN_LITERAL_CHARS: usize = 5;
/// A publisher that bundles its configuration with a library puts it before
/// or after the library, so these chunks of a vendor library are screened.
const VENDOR_EDGE_CHUNKS: usize = 2;

/// (question, label, weight). The weights follow how often each kind of
/// logic is where a revenue bug sits.
const ASPECTS: [(usize, &str, f64); 6] = [
    (AUCTION_GATING, "auction gating", 0.35),
    (DEVICE_DETECTION, "device detection", 0.20),
    (TIMING_CONFIG, "timing", 0.15),
    (LAZY_LOAD, "lazy load", 0.15),
    (SLOT_DEFINITION, "slot definition", 0.10),
    (CONSENT_GATING, "consent gating", 0.05),
];

/// Names the page itself uses for its ad placements: ad unit codes, slot
/// element ids and ad unit paths seen in the telemetry. Stock library code
/// never contains them, so they mark publisher code without asking a model.
pub fn telemetry_literals(telemetry: &Telemetry) -> Vec<String> {
    let codes = telemetry
        .auctions
        .iter()
        .flat_map(|auction| &auction.ad_units)
        .map(|unit| unit.code.as_str());
    let slots = telemetry.slots.iter().flat_map(|slot| {
        std::iter::once(slot.element_id.as_str()).chain(slot.ad_unit_path.as_deref())
    });
    codes
        .chain(slots)
        .filter(|name| name.chars().count() >= MIN_LITERAL_CHARS)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(str::to_string)
        .collect()
}

/// Whether `text` holds one of the names as a quoted string.
pub fn names_a_placement(text: &str, literals: &[String]) -> bool {
    literals.iter().any(|literal| {
        text.match_indices(literal.as_str()).any(|(at, matched)| {
            let before = text[..at].chars().next_back();
            let after = text[at + matched.len()..].chars().next();
            matches!(before, Some('"' | '\'' | '`')) && before == after
        })
    })
}

/// The line ranges of a vendor library that can hold the publisher's own
/// code: they name one of the page's placements or call the ad stack the way
/// only orchestration code does. A wrapper bundled with the library (before
/// it, after it, or under a renamed global) stays searchable through these
/// calls even when the page never got as far as an auction, while the rest of
/// the library is only searched on request.
pub fn publisher_ranges(
    lines: &[String],
    chunks: &[LineRange],
    literals: &[String],
) -> Vec<LineRange> {
    chunks
        .iter()
        .copied()
        .filter(|chunk| {
            let text = chunk.of(lines).join("\n");
            names_a_placement(&text, literals) || ad_api_calls(&text) > 0
        })
        .collect()
}

/// Which chunks of one script to ask about. Publisher scripts: every chunk
/// with an ad-tech identifier, a page-state API or a placement name. Vendor
/// libraries: only where publisher configuration could have been bundled in.
pub fn chunks_to_screen(
    lines: &[String],
    chunks: &[LineRange],
    vendor_library: bool,
    literals: &[String],
) -> Vec<usize> {
    chunks
        .iter()
        .enumerate()
        .filter(|(index, chunk)| {
            let text = chunk.of(lines).join("\n");
            let placement = names_a_placement(&text, literals);
            if vendor_library {
                let at_an_edge =
                    *index < VENDOR_EDGE_CHUNKS || index + VENDOR_EDGE_CHUNKS >= chunks.len();
                placement || at_an_edge || has_call_site(&text)
            } else {
                placement || !logic_anchors(&text).is_empty()
            }
        })
        .map(|(index, _)| index)
        .collect()
}

/// How much a chunk deserves to be read first, 0 to 1: what kind of ad logic
/// it holds, weighted down when it looks like generic library code.
pub fn rank(probabilities: &[f64]) -> f64 {
    let p = |question: usize| probabilities.get(question).copied().unwrap_or(0.0);
    let logic: f64 = ASPECTS
        .iter()
        .map(|(question, _, weight)| weight * p(*question))
        .sum();
    (0.3 + 0.7 * p(PUBLISHER_SPECIFIC)) * logic
}

/// Whether the answers say this chunk is the publisher's own ad code.
pub fn is_publisher_ad_code(probabilities: &[f64]) -> bool {
    let p = |question: usize| probabilities.get(question).copied().unwrap_or(0.0);
    p(AD_ORCHESTRATION) >= PROMOTION_THRESHOLD && p(PUBLISHER_SPECIFIC) >= PROMOTION_THRESHOLD
}

/// One scored chunk of one script.
#[derive(Debug, Clone, PartialEq)]
pub struct ScoredChunk {
    /// Position of the script in the analysis selection.
    pub source: usize,
    pub lines: LineRange,
    /// Answers in question order.
    pub probabilities: Vec<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GuideEntry {
    pub source: usize,
    pub lines: LineRange,
    pub rank: f64,
    /// The kinds of logic found here, strongest first.
    pub aspects: Vec<(&'static str, f64)>,
}

/// The best-ranked chunks, highest first.
pub fn build(scored: &[ScoredChunk]) -> Vec<GuideEntry> {
    let mut entries: Vec<GuideEntry> = scored
        .iter()
        .map(|chunk| {
            let mut aspects: Vec<(&'static str, f64)> = ASPECTS
                .iter()
                .filter_map(|(question, label, _)| {
                    let probability = *chunk.probabilities.get(*question)?;
                    (probability >= ASPECT_SHOWN_FROM).then_some((*label, probability))
                })
                .collect();
            aspects.sort_by(|a, b| b.1.total_cmp(&a.1));
            aspects.truncate(MAX_ASPECTS_SHOWN);
            GuideEntry {
                source: chunk.source,
                lines: chunk.lines,
                rank: rank(&chunk.probabilities),
                aspects,
            }
        })
        .filter(|entry| entry.rank >= MIN_RANK)
        .collect();
    entries.sort_by(|a, b| {
        b.rank
            .total_cmp(&a.rank)
            .then(a.source.cmp(&b.source))
            .then(a.lines.start.cmp(&b.lines.start))
    });
    entries.truncate(MAX_ENTRIES);
    entries
}

#[cfg(test)]
mod tests {
    use super::*;
    use adscan_model::{AdSlot, Auction, AuctionAdUnit, ScanMetrics, SlotOutcome};

    fn answers(publisher: f64, gating: f64, device: f64) -> Vec<f64> {
        let mut probabilities = vec![0.0; 8];
        probabilities[AD_ORCHESTRATION] = 0.9;
        probabilities[PUBLISHER_SPECIFIC] = publisher;
        probabilities[AUCTION_GATING] = gating;
        probabilities[DEVICE_DETECTION] = device;
        probabilities
    }

    fn chunk(source: usize, start: usize, probabilities: Vec<f64>) -> ScoredChunk {
        ScoredChunk {
            source,
            lines: LineRange {
                start,
                end: start + 99,
            },
            probabilities,
        }
    }

    fn lines(text: &[&str]) -> Vec<String> {
        text.iter().map(|line| line.to_string()).collect()
    }

    #[test]
    fn rank_weights_the_logic_by_how_site_specific_the_code_is() {
        assert!((rank(&[1.0; 8]) - 1.0).abs() < 1e-12);
        assert_eq!(rank(&[0.0; 8]), 0.0);
        // Gating 1.0 and device 1.0 give 0.55 of logic; generic code keeps 30% of it.
        assert!((rank(&answers(0.0, 1.0, 1.0)) - 0.3 * 0.55).abs() < 1e-12);
        assert!((rank(&answers(1.0, 1.0, 1.0)) - 0.55).abs() < 1e-12);
        assert_eq!(rank(&[]), 0.0);
    }

    #[test]
    fn the_guide_lists_the_best_ranges_first_and_leaves_out_weak_ones() {
        let guide = build(&[
            chunk(1, 1, answers(0.2, 0.9, 0.0)),
            chunk(0, 101, answers(1.0, 0.95, 0.9)),
            chunk(0, 1, answers(1.0, 0.9, 0.2)),
        ]);
        assert_eq!(
            guide
                .iter()
                .map(|entry| (entry.source, entry.lines.start))
                .collect::<Vec<_>>(),
            [(0, 101), (0, 1)],
            "the generic chunk ranks below the minimum"
        );
        assert_eq!(
            guide[0].aspects,
            [("auction gating", 0.95), ("device detection", 0.9)]
        );
        assert_eq!(guide[1].aspects, [("auction gating", 0.9)]);
    }

    #[test]
    fn the_guide_is_capped() {
        let scored: Vec<ScoredChunk> = (0..50)
            .map(|n| chunk(0, n * 100 + 1, answers(1.0, 1.0, 1.0)))
            .collect();
        assert_eq!(build(&scored).len(), MAX_ENTRIES);
    }

    #[test]
    fn placement_names_come_from_the_telemetry_and_short_ones_are_ignored() {
        let telemetry = Telemetry {
            auctions: vec![Auction {
                id: "a".into(),
                started_at_ms: 0.0,
                ended_at_ms: None,
                duration_ms: None,
                timeout_ms: None,
                ad_units: ["top", "div-gpt-ad-leaderboard"]
                    .into_iter()
                    .map(|code| AuctionAdUnit {
                        code: code.into(),
                        sizes: vec![],
                        bids: vec![],
                        winning_bidder: None,
                        winning_cpm: None,
                    })
                    .collect(),
            }],
            slots: vec![AdSlot {
                element_id: "div-gpt-ad-leaderboard".into(),
                ad_unit_path: Some("/123/site/top".into()),
                sizes: vec![],
                outcome: SlotOutcome::Filled,
                rendered_size: None,
                defined_at_ms: None,
                requested_at_ms: None,
                response_at_ms: None,
                rendered_at_ms: None,
                viewable_at_ms: None,
                in_initial_viewport: None,
                had_prebid_targeting: None,
                requested_before_auction_end: None,
                advertiser_id: None,
                line_item_id: None,
            }],
            metrics: ScanMetrics::default(),
        };
        assert_eq!(
            telemetry_literals(&telemetry),
            ["/123/site/top", "div-gpt-ad-leaderboard"]
        );
    }

    #[test]
    fn a_placement_name_counts_only_as_a_quoted_string() {
        let literals = vec!["ad-leaderboard".to_string()];
        assert!(names_a_placement(r#"{code:"ad-leaderboard"}"#, &literals));
        assert!(names_a_placement(
            "defineSlot(p,s,'ad-leaderboard')",
            &literals
        ));
        assert!(!names_a_placement("var x_ad-leaderboard_y", &literals));
        assert!(!names_a_placement(r#""ad-leaderboard-2""#, &literals));
        assert!(!names_a_placement("anything", &[]));
    }

    #[test]
    fn publisher_scripts_are_screened_where_ad_logic_can_be() {
        let source = lines(&[
            "function helper(){}",
            "if(window.innerWidth>768){start()}",
            "render();",
            r#"units.push({code:"ad-leaderboard"});"#,
        ]);
        let chunks: Vec<LineRange> = (1..=4).map(|n| LineRange { start: n, end: n }).collect();
        let literals = vec!["ad-leaderboard".to_string()];
        assert_eq!(chunks_to_screen(&source, &chunks, false, &literals), [1, 3]);
    }

    #[test]
    fn vendor_libraries_are_screened_at_the_edges_and_where_publishers_call_in() {
        let mut source = vec!["internal();".to_string(); 10];
        source[5] = "pbjs.que.push(setup);".into();
        source[7] = r#"var u={code:"ad-leaderboard"};"#.into();
        let chunks: Vec<LineRange> = (1..=10).map(|n| LineRange { start: n, end: n }).collect();
        let literals = vec!["ad-leaderboard".to_string()];
        assert_eq!(
            chunks_to_screen(&source, &chunks, true, &literals),
            [0, 1, 5, 7, 8, 9]
        );
        assert_eq!(
            publisher_ranges(&source, &chunks, &literals),
            [
                LineRange { start: 6, end: 6 },
                LineRange { start: 8, end: 8 }
            ]
        );
    }

    #[test]
    fn a_wrapper_bundled_with_the_library_stays_searchable_without_any_telemetry() {
        // The auction never started, so the telemetry names no placement, and
        // the publisher renamed the Prebid global.
        let mut source = vec!["function adapterInternals(){return bidderSettings}".to_string(); 6];
        source[0] = "function isMobile(){return window.innerWidth>768}".into();
        source[1] = "if(isMobile()){addEventListener('scroll',start)}".into();
        source[2] = "function start(){acmeHb.requestBids({timeout:800})}".into();
        source[5] = "window.googletag.cmd.push(function(){googletag.defineSlot(p,s,i)})".into();
        let chunks = [
            LineRange { start: 1, end: 3 },
            LineRange { start: 4, end: 5 },
            LineRange { start: 6, end: 6 },
        ];
        assert_eq!(
            publisher_ranges(&source, &chunks, &[]),
            [
                LineRange { start: 1, end: 3 },
                LineRange { start: 6, end: 6 }
            ]
        );
    }

    #[test]
    fn only_code_that_is_both_ad_code_and_site_specific_is_promoted() {
        assert!(is_publisher_ad_code(&answers(0.5, 0.0, 0.0)));
        assert!(!is_publisher_ad_code(&answers(0.49, 1.0, 1.0)));
        let mut not_ad_code = answers(0.9, 0.0, 0.0);
        not_ad_code[AD_ORCHESTRATION] = 0.4;
        assert!(!is_publisher_ad_code(&not_ad_code));
    }
}
