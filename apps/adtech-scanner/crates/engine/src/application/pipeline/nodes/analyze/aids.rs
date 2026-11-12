//! What the opening message gives the analysis model beyond the telemetry:
//! the reading guide, and source it would otherwise spend its first turns
//! fetching. Every exploratory turn re-reads the whole conversation, so a
//! small wrapper costs less inlined once than read through tool calls.

use super::toolbox::Toolbox;
use crate::domain::chunks::LineRange;
use crate::domain::guide::GuideEntry;

/// Publisher scripts are inlined whole while together they stay under this.
/// Uncalibrated.
const INLINE_MAX_LINES: usize = 400;
/// About 5,000 tokens of source in the opening message. Uncalibrated.
const INLINE_MAX_CHARS: usize = 15_000;

pub struct ReadingAids {
    /// Ranked line ranges, one per line. Empty without a guide.
    pub guide: String,
    /// Numbered source to start with. Empty when nothing was small enough.
    pub source: String,
}

pub fn reading_aids(toolbox: &Toolbox, guide: &[GuideEntry]) -> ReadingAids {
    let scripts = toolbox.scripts();
    ReadingAids {
        guide: guide
            .iter()
            .map(|entry| {
                let aspects: Vec<String> = entry
                    .aspects
                    .iter()
                    .map(|(label, probability)| format!("{label} {probability:.2}"))
                    .collect();
                format!(
                    "{} L{}-{}  {}",
                    scripts[entry.source].id,
                    entry.lines.start,
                    entry.lines.end,
                    aspects.join(", ")
                )
            })
            .collect::<Vec<_>>()
            .join("\n"),
        source: inlined_ranges(toolbox, guide)
            .into_iter()
            .map(|(script, range)| toolbox.excerpt(script, range))
            .collect::<Vec<_>>()
            .join("\n\n"),
    }
}

/// Whole small publisher scripts in relevance order, then the best-ranked
/// ranges of the guide, as far as the size limits allow.
fn inlined_ranges(toolbox: &Toolbox, guide: &[GuideEntry]) -> Vec<(usize, LineRange)> {
    let mut chosen: Vec<(usize, LineRange)> = Vec::new();
    let mut chars_left = INLINE_MAX_CHARS;
    let mut lines_left = INLINE_MAX_LINES;
    let mut take = |chosen: &mut Vec<(usize, LineRange)>, script: usize, range: LineRange| {
        let chars = toolbox.excerpt(script, range).len();
        let fits = chars <= chars_left;
        if fits {
            chars_left -= chars;
            chosen.push((script, range));
        }
        fits
    };
    for (index, script) in toolbox.scripts().iter().enumerate() {
        let lines = script.lines.len();
        if script.is_vendor_library() || lines == 0 || lines > lines_left {
            continue;
        }
        let whole = LineRange {
            start: 1,
            end: lines,
        };
        if take(&mut chosen, index, whole) {
            lines_left -= lines;
        }
    }
    for entry in guide {
        let already_shown = chosen.iter().any(|(script, range)| {
            *script == entry.source
                && range.start <= entry.lines.end
                && entry.lines.start <= range.end
        });
        if !already_shown {
            take(&mut chosen, entry.source, entry.lines);
        }
    }
    chosen
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::pipeline::nodes::analyze::toolbox::ScriptSource;
    use crate::domain::telemetry::Telemetry;
    use adscan_model::{Capture, DeviceProfile, ScanMetrics, ScriptCategory, Viewport};

    fn source(id: &str, lines: usize, vendor_library: bool) -> ScriptSource {
        ScriptSource {
            id: id.into(),
            url: format!("https://pub.example/{id}.js"),
            category: if vendor_library {
                ScriptCategory::Prebid
            } else {
                ScriptCategory::Wrapper
            },
            size_bytes: 1000.0,
            lines: (1..=lines)
                .map(|n| format!("{id}_statement{n}();"))
                .collect(),
            default_search: vendor_library.then(Vec::new),
        }
    }

    fn toolbox(scripts: Vec<ScriptSource>) -> Toolbox {
        let capture = Capture {
            environment: adscan_model::CaptureEnvironment::default(),
            pages: vec![adscan_model::CapturedPage::entry("https://pub.example/")],
            final_url: "https://pub.example/".into(),
            page_title: None,
            device: DeviceProfile::Desktop,
            viewport: Viewport {
                width: 1440,
                height: 900,
                device_scale_factor: 1.0,
                mobile: false,
            },
            user_agent: "test".into(),
            navigation_start: 0.0,
            page_load_ms: None,
            scripts: vec![],
            events: vec![],
            console: vec![],
            max_scroll_depth: 0.0,
            consent_accepted: false,
            bid_responses: vec![],
        };
        let telemetry = Telemetry {
            auctions: vec![],
            slots: vec![],
            metrics: ScanMetrics::default(),
        };
        Toolbox::new(scripts, &capture, &telemetry)
    }

    fn entry(source: usize, start: usize, end: usize) -> GuideEntry {
        GuideEntry {
            source,
            lines: LineRange { start, end },
            rank: 0.8,
            aspects: vec![("auction gating", 0.94), ("device detection", 0.88)],
        }
    }

    fn ranges(toolbox: &Toolbox, guide: &[GuideEntry]) -> Vec<(usize, usize, usize)> {
        inlined_ranges(toolbox, guide)
            .into_iter()
            .map(|(script, range)| (script, range.start, range.end))
            .collect()
    }

    #[test]
    fn small_publisher_scripts_are_inlined_whole_and_vendor_libraries_never() {
        let toolbox = toolbox(vec![
            source("s3", 302, false),
            source("s9", 40, true),
            source("s5", 60, false),
        ]);
        assert_eq!(ranges(&toolbox, &[]), [(0, 1, 302), (2, 1, 60)]);
        let aids = reading_aids(&toolbox, &[]);
        assert!(aids.guide.is_empty());
        assert!(aids.source.starts_with(
            "s3 lines 1-302 of 302 (https://pub.example/s3.js)\n    1| s3_statement1();"
        ));
        assert!(aids.source.contains("\n\ns5 lines 1-60 of 60 ("));
        assert!(!aids.source.contains("s9"));
    }

    #[test]
    fn scripts_that_would_exceed_the_line_or_size_limit_are_left_to_the_tools() {
        // The second script does not fit in what the first left of the 400 lines.
        let by_lines = toolbox(vec![
            source("s3", 302, false),
            source("s4", 120, false),
            source("s5", 60, false),
        ]);
        assert_eq!(ranges(&by_lines, &[]), [(0, 1, 302), (2, 1, 60)]);

        let mut wide = source("s3", 100, false);
        wide.lines = vec!["x".repeat(280); 100];
        let by_size = toolbox(vec![wide, source("s5", 60, false)]);
        assert_eq!(ranges(&by_size, &[]), [(1, 1, 60)]);

        let nothing = toolbox(vec![source("s3", 5_000, false)]);
        assert!(reading_aids(&nothing, &[]).source.is_empty());
    }

    #[test]
    fn the_guide_adds_its_best_ranges_of_larger_scripts_within_the_size_limit() {
        let toolbox = toolbox(vec![source("s3", 5_000, false), source("s5", 60, false)]);
        let guide = [
            entry(0, 118, 262),
            // Part of a script that is already shown whole.
            entry(1, 1, 60),
            entry(0, 900, 1_060),
            // Would overrun what is left of the size limit.
            entry(0, 2_000, 2_199),
            entry(0, 3_000, 3_010),
        ];
        assert_eq!(
            ranges(&toolbox, &guide),
            [
                (1, 1, 60),
                (0, 118, 262),
                (0, 900, 1_060),
                (0, 3_000, 3_010)
            ]
        );
        let aids = reading_aids(&toolbox, &guide);
        assert_eq!(
            aids.guide.lines().next(),
            Some("s3 L118-262  auction gating 0.94, device detection 0.88")
        );
        assert_eq!(aids.guide.lines().count(), 5);
        assert!(aids.source.len() <= INLINE_MAX_CHARS + 4 * 2);
        assert!(aids.source.contains("\n  118| s3_statement118();"));
    }
}
