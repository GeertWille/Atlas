//! Formats minified JavaScript with the oxc parser and printer, off the async
//! runtime, falling back to a token-level reflow when the source does not parse.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use oxc::allocator::Allocator;
use oxc::codegen::Codegen;
use oxc::parser::Parser;
use oxc::span::SourceType;

use crate::application::ports::{FormatMethod, FormattedSource, SourceFormatter};

/// Larger scripts are bundles nobody will read line by line; formatting them
/// would dominate the scan time.
const MAX_FORMAT_BYTES: usize = 3_000_000;
const FORMAT_TIMEOUT: Duration = Duration::from_secs(20);
const INDENT: &str = "  ";

pub struct OxcFormatter {
    max_bytes: usize,
    timeout: Duration,
}

impl Default for OxcFormatter {
    fn default() -> Self {
        Self {
            max_bytes: MAX_FORMAT_BYTES,
            timeout: FORMAT_TIMEOUT,
        }
    }
}

impl OxcFormatter {
    pub fn with_limits(max_bytes: usize, timeout: Duration) -> Self {
        Self { max_bytes, timeout }
    }
}

#[async_trait]
impl SourceFormatter for OxcFormatter {
    async fn format(&self, source: String) -> FormattedSource {
        if source.len() > self.max_bytes {
            return unformatted(&source);
        }
        format_within(source.into(), self.timeout, format_blocking).await
    }
}

/// Runs `work` on a blocking thread and gives up waiting after `timeout`.
async fn format_within(
    source: Arc<str>,
    timeout: Duration,
    work: fn(&str) -> FormattedSource,
) -> FormattedSource {
    let worker_source = Arc::clone(&source);
    let worker = tokio::task::spawn_blocking(move || work(&worker_source));
    match tokio::time::timeout(timeout, worker).await {
        Ok(Ok(formatted)) => formatted,
        // Timed out or the formatter panicked: the raw source is still usable.
        Ok(Err(_)) | Err(_) => unformatted(&source),
    }
}

fn format_blocking(source: &str) -> FormattedSource {
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, source, SourceType::unambiguous()).parse();
    if parsed.fatal_error {
        return FormattedSource {
            lines: reflow(source),
            method: FormatMethod::Reflow,
        };
    }
    let printed = Codegen::new().build(&parsed.program).code;
    FormattedSource {
        lines: printed
            .lines()
            .map(|line| line.replace('\t', INDENT))
            .collect(),
        method: FormatMethod::Parser,
    }
}

fn unformatted(source: &str) -> FormattedSource {
    FormattedSource {
        lines: source.lines().map(str::to_string).collect(),
        method: FormatMethod::Unformatted,
    }
}

/// Breaks lines after `;`, `{` and `}` outside string and template literals
/// and indents by brace depth. It does not understand regex literals or
/// comments, which is acceptable for a fallback whose job is to make broken
/// input addressable by line.
fn reflow(source: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut depth: usize = 0;
    let mut paren_depth: usize = 0;
    let mut quote: Option<char> = None;
    let mut escaped = false;

    let mut flush = |current: &mut String, depth: usize| {
        let text = current.trim();
        if !text.is_empty() {
            lines.push(format!("{}{text}", INDENT.repeat(depth)));
        }
        current.clear();
    };

    for c in source.chars() {
        if let Some(open) = quote {
            current.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == open {
                quote = None;
            }
            continue;
        }
        match c {
            '"' | '\'' | '`' => {
                quote = Some(c);
                current.push(c);
            }
            '(' => {
                paren_depth += 1;
                current.push(c);
            }
            ')' => {
                paren_depth = paren_depth.saturating_sub(1);
                current.push(c);
            }
            '{' => {
                current.push(c);
                flush(&mut current, depth);
                depth += 1;
            }
            '}' => {
                flush(&mut current, depth);
                depth = depth.saturating_sub(1);
                current.push(c);
                flush(&mut current, depth);
            }
            // `for (a; b; c)` headers stay on one line.
            ';' if paren_depth == 0 => {
                current.push(c);
                flush(&mut current, depth);
            }
            '\n' | '\r' => {
                if !current.ends_with(' ') {
                    current.push(' ');
                }
            }
            _ => current.push(c),
        }
    }
    flush(&mut current, depth);
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIFIED: &str = "function isMobile(){return window.innerWidth>768}if(isMobile()){window.addEventListener(\"scroll\",start,{once:true})}else{start()}";

    #[tokio::test]
    async fn minified_code_is_printed_one_statement_per_line() {
        let formatted = OxcFormatter::default().format(MINIFIED.to_string()).await;
        assert_eq!(formatted.method, FormatMethod::Parser);
        assert!(formatted.lines.len() >= 7, "{:#?}", formatted.lines);
        assert_eq!(formatted.lines[0], "function isMobile() {");
        assert!(
            formatted
                .lines
                .iter()
                .any(|line| line == "  return window.innerWidth > 768;")
        );
        assert!(formatted.lines.iter().all(|line| !line.contains('\t')));
    }

    #[tokio::test]
    async fn unparseable_input_falls_back_to_reflow() {
        let broken = "var a = {b: 1};function x({ if (a) { call(\"}; {\"); } for(i=0;i<2;i++){y()}";
        let formatted = OxcFormatter::default().format(broken.to_string()).await;
        assert_eq!(formatted.method, FormatMethod::Reflow);
        assert!(formatted.lines.len() > 3, "{:#?}", formatted.lines);
        assert!(
            formatted
                .lines
                .iter()
                .any(|line| line.contains("call(\"}; {\");"))
        );
        assert!(
            formatted
                .lines
                .iter()
                .any(|line| line.trim_start().starts_with("for(i=0;i<2;i++){"))
        );
    }

    #[tokio::test]
    async fn oversized_input_is_left_unformatted() {
        let formatter = OxcFormatter::with_limits(16, FORMAT_TIMEOUT);
        let formatted = formatter
            .format("var a=1;\nvar b=2;var c=3;".to_string())
            .await;
        assert_eq!(formatted.method, FormatMethod::Unformatted);
        assert_eq!(formatted.lines, ["var a=1;", "var b=2;var c=3;"]);
    }

    #[tokio::test]
    async fn slow_or_panicking_formatting_falls_back_to_the_raw_source() {
        fn slow(source: &str) -> FormattedSource {
            std::thread::sleep(Duration::from_millis(300));
            format_blocking(source)
        }
        fn panicking(_: &str) -> FormattedSource {
            panic!("formatter bug");
        }
        for work in [slow as fn(&str) -> FormattedSource, panicking] {
            let formatted = format_within(MINIFIED.into(), Duration::from_millis(20), work).await;
            assert_eq!(formatted.method, FormatMethod::Unformatted);
            assert_eq!(formatted.lines, [MINIFIED]);
        }
    }
}
