//! Console lines of the scanned page, read from CDP event payloads. No I/O.

use adscan_model::ConsoleLine;
use serde_json::Value;

const MAX_LINES: usize = 2_000;
const MAX_LINE_CHARS: usize = 2_000;

struct Entry {
    epoch_ms: f64,
    level: &'static str,
    text: String,
}

#[derive(Default)]
pub(crate) struct ConsoleLog {
    entries: Vec<Entry>,
}

impl ConsoleLog {
    /// `Runtime.consoleAPICalled`
    pub(crate) fn record_api_call(&mut self, params: &Value) {
        let level = match params["type"].as_str() {
            Some("warning") => "warning",
            Some("error" | "assert") => "error",
            Some("info") => "info",
            Some("debug") => "debug",
            _ => "log",
        };
        let args = params["args"].as_array().map(Vec::as_slice).unwrap_or_default();
        let text = args.iter().map(remote_object_text).collect::<Vec<_>>().join(" ");
        self.push(params["timestamp"].as_f64(), level, text);
    }

    /// `Log.entryAdded` (network errors, CSP violations, deprecations, ...)
    pub(crate) fn record_log_entry(&mut self, params: &Value) {
        let entry = &params["entry"];
        let level = match entry["level"].as_str() {
            Some("warning") => "warning",
            Some("error") => "error",
            Some("verbose") => "debug",
            _ => "info",
        };
        let mut text = entry["text"].as_str().unwrap_or_default().to_string();
        if let Some(url) = entry["url"].as_str().filter(|url| !url.is_empty()) {
            text = format!("{text} ({url})");
        }
        self.push(entry["timestamp"].as_f64(), level, text);
    }

    fn push(&mut self, epoch_ms: Option<f64>, level: &'static str, text: String) {
        let Some(epoch_ms) = epoch_ms else { return };
        if self.entries.len() >= MAX_LINES || text.is_empty() {
            return;
        }
        let text = match text.char_indices().nth(MAX_LINE_CHARS) {
            Some((cut, _)) => format!("{}…", &text[..cut]),
            None => text,
        };
        self.entries.push(Entry { epoch_ms, level, text });
    }

    pub(crate) fn into_lines(self, navigation_start: f64) -> Vec<ConsoleLine> {
        self.entries
            .into_iter()
            .map(|entry| ConsoleLine {
                t: entry.epoch_ms - navigation_start,
                level: entry.level.to_string(),
                text: entry.text,
            })
            .collect()
    }
}

/// How DevTools would print one console argument (a CDP `RemoteObject`).
fn remote_object_text(object: &Value) -> String {
    match &object["value"] {
        Value::String(text) => text.clone(),
        Value::Null => object["description"]
            .as_str()
            .or(object["unserializableValue"].as_str())
            .or(object["subtype"].as_str())
            .or(object["type"].as_str())
            .unwrap_or_default()
            .to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn joins_console_arguments_like_devtools() {
        let mut log = ConsoleLog::default();
        log.record_api_call(&json!({
            "type": "warning",
            "timestamp": 1_250.0,
            "args": [
                {"type": "string", "value": "slow bidder"},
                {"type": "number", "value": 42},
                {"type": "boolean", "value": false},
                {"type": "object", "className": "Object", "description": "Object"},
                {"type": "undefined"},
                {"type": "object", "subtype": "null", "value": null},
                {"type": "number", "unserializableValue": "NaN", "description": "NaN"},
            ]
        }));
        let lines = log.into_lines(1_000.0);
        assert_eq!(lines[0].text, "slow bidder 42 false Object undefined null NaN");
        assert_eq!(lines[0].level, "warning");
        assert_eq!(lines[0].t, 250.0);
    }

    #[test]
    fn maps_levels_of_both_sources() {
        let mut log = ConsoleLog::default();
        for kind in ["log", "info", "debug", "error", "assert", "table"] {
            log.record_api_call(&json!({"type": kind, "timestamp": 1.0, "args": [{"type": "string", "value": "x"}]}));
        }
        for level in ["verbose", "info", "warning", "error"] {
            log.record_log_entry(&json!({"entry": {"level": level, "text": "x", "timestamp": 1.0}}));
        }
        let levels: Vec<String> = log.into_lines(0.0).into_iter().map(|line| line.level).collect();
        assert_eq!(
            levels,
            ["log", "info", "debug", "error", "error", "log", "debug", "info", "warning", "error"]
        );
    }

    #[test]
    fn log_entries_mention_the_resource() {
        let mut log = ConsoleLog::default();
        log.record_log_entry(&json!({"entry": {
            "level": "error",
            "text": "Failed to load resource: net::ERR_FAILED",
            "url": "https://a.test/x.js",
            "timestamp": 5.0
        }}));
        assert_eq!(
            log.into_lines(0.0)[0].text,
            "Failed to load resource: net::ERR_FAILED (https://a.test/x.js)"
        );
    }

    #[test]
    fn caps_line_length_and_line_count_and_skips_empty_lines() {
        let mut log = ConsoleLog::default();
        log.record_api_call(&json!({"type": "log", "timestamp": 1.0, "args": []}));
        let long = "ü".repeat(MAX_LINE_CHARS + 50);
        for _ in 0..(MAX_LINES + 10) {
            log.record_api_call(&json!({"type": "log", "timestamp": 1.0, "args": [{"type": "string", "value": long}]}));
        }
        let lines = log.into_lines(0.0);
        assert_eq!(lines.len(), MAX_LINES);
        assert_eq!(lines[0].text.chars().count(), MAX_LINE_CHARS + 1);
        assert!(lines[0].text.ends_with('…'));
    }
}
