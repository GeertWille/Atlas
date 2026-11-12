//! Cuts script source into pieces small enough for the decision model to
//! judge well. Every size here is a starting value that has not been
//! calibrated against real answers.

/// Raw (not beautified) source per triage request.
pub const TEXT_CHUNK_CHARS: usize = 6_000;

const LINE_CHUNK_MIN_LINES: usize = 120;
const LINE_CHUNK_TARGET_LINES: usize = 160;
/// Below the 250 lines one `read_script` call returns, so a chunk the reading
/// guide points at can always be read in one call.
pub const LINE_CHUNK_MAX_LINES: usize = 200;
pub const LINE_CHUNK_MAX_CHARS: usize = 8_000;

/// Lines of a beautified script, 1-based and inclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineRange {
    pub start: usize,
    pub end: usize,
}

impl LineRange {
    pub fn contains(&self, line: usize) -> bool {
        (self.start..=self.end).contains(&line)
    }

    pub fn of<'a>(&self, lines: &'a [String]) -> &'a [String] {
        &lines[self.start - 1..self.end]
    }
}

/// Splits source as captured, without formatting it first, into consecutive
/// pieces of at most [`TEXT_CHUNK_CHARS`] characters. A piece ends after a
/// statement or block when one closes in its last quarter.
pub fn split_text(body: &str) -> Vec<&str> {
    let mut chunks = Vec::new();
    let mut rest = body;
    while !rest.is_empty() {
        let Some((limit, _)) = rest.char_indices().nth(TEXT_CHUNK_CHARS) else {
            chunks.push(rest);
            break;
        };
        let window = &rest[..limit];
        let mut floor = window.len() * 3 / 4;
        while !window.is_char_boundary(floor) {
            floor += 1;
        }
        let cut = window[floor..]
            .rfind([';', '}', '\n'])
            .map_or(limit, |at| floor + at + 1);
        chunks.push(&rest[..cut]);
        rest = &rest[cut..];
    }
    chunks
}

/// Splits beautified source into consecutive line ranges of about
/// [`LINE_CHUNK_TARGET_LINES`] lines. A range ends before the least indented
/// line near the target, which is where a top-level statement starts.
pub fn split_lines(lines: &[String]) -> Vec<LineRange> {
    let mut ranges = Vec::new();
    let mut start = 0;
    while start < lines.len() {
        let limit = furthest_end(lines, start);
        let end = if limit == lines.len() || limit - start <= LINE_CHUNK_MIN_LINES {
            limit
        } else {
            (start + LINE_CHUNK_MIN_LINES..=limit)
                .min_by_key(|&cut| {
                    (
                        indentation(&lines[cut]),
                        cut.abs_diff(start + LINE_CHUNK_TARGET_LINES),
                    )
                })
                .unwrap_or(limit)
        };
        ranges.push(LineRange {
            start: start + 1,
            end,
        });
        start = end;
    }
    ranges
}

/// One past the last line a chunk starting at `start` may hold: the line and
/// character caps both apply, and a chunk always holds at least one line.
fn furthest_end(lines: &[String], start: usize) -> usize {
    let mut chars = 0;
    let mut end = start;
    while end < lines.len() && end - start < LINE_CHUNK_MAX_LINES {
        chars += lines[end].len() + 1;
        if chars > LINE_CHUNK_MAX_CHARS && end > start {
            break;
        }
        end += 1;
    }
    end
}

fn indentation(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines_of(count: usize, line: impl Fn(usize) -> String) -> Vec<String> {
        (0..count).map(line).collect()
    }

    fn assert_covers_every_line_once(ranges: &[LineRange], total: usize) {
        let mut next = 1;
        for range in ranges {
            assert_eq!(range.start, next, "{ranges:?}");
            assert!(range.end >= range.start, "{ranges:?}");
            next = range.end + 1;
        }
        assert_eq!(next, total + 1, "{ranges:?}");
    }

    #[test]
    fn line_ranges_cover_every_line_once_and_fit_one_read() {
        for total in [0, 1, 119, 120, 121, 200, 201, 302, 1_000, 15_622] {
            let lines = lines_of(total, |n| format!("{}statement{n}();", "  ".repeat(n % 5)));
            let ranges = split_lines(&lines);
            assert_covers_every_line_once(&ranges, total);
            assert!(
                ranges
                    .iter()
                    .all(|range| range.end - range.start < LINE_CHUNK_MAX_LINES),
                "{total}"
            );
        }
        assert!(split_lines(&[]).is_empty());
    }

    #[test]
    fn a_range_ends_where_a_top_level_statement_starts() {
        // Top-level statements start on lines 1, 151 and 301 (1-based).
        let lines = lines_of(400, |n| {
            if n % 150 == 0 {
                "function next() {".to_string()
            } else {
                "  body();".to_string()
            }
        });
        let ranges = split_lines(&lines);
        assert_eq!(
            ranges
                .iter()
                .map(|range| (range.start, range.end))
                .collect::<Vec<_>>(),
            [(1, 150), (151, 300), (301, 400)]
        );
    }

    #[test]
    fn long_lines_end_a_range_at_the_character_cap() {
        let lines = lines_of(50, |_| "x".repeat(1_000));
        let ranges = split_lines(&lines);
        assert_covers_every_line_once(&ranges, 50);
        assert!(ranges.iter().all(|range| range.end - range.start < 8));
        // One enormous line (an unformatted bundle) is a range of its own.
        let single = split_lines(&["y".repeat(50_000), "z();".to_string()]);
        assert_eq!(single[0], LineRange { start: 1, end: 1 });
    }

    #[test]
    fn text_is_split_after_statements_and_nothing_is_lost() {
        let body = "window.a=1;".repeat(2_000);
        let chunks = split_text(&body);
        assert_eq!(chunks.concat(), body);
        assert!(chunks.len() > 3);
        for chunk in &chunks {
            assert!(chunk.chars().count() <= TEXT_CHUNK_CHARS);
            assert!(chunk.ends_with(';'));
        }
        assert_eq!(split_text("var a=1;"), ["var a=1;"]);
        assert!(split_text("").is_empty());
    }

    #[test]
    fn text_without_statement_ends_and_with_multibyte_characters_is_cut_at_the_cap() {
        let body = "é".repeat(TEXT_CHUNK_CHARS * 2 + 10);
        let chunks = split_text(&body);
        assert_eq!(chunks.concat(), body);
        assert_eq!(
            chunks
                .iter()
                .map(|chunk| chunk.chars().count())
                .collect::<Vec<_>>(),
            [TEXT_CHUNK_CHARS, TEXT_CHUNK_CHARS, 10]
        );
    }
}
