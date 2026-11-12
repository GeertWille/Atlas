//! The set of script bodies kept for one capture: dedupe and size budgets. No I/O.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};

use adscan_model::CapturedScript;
use base64::Engine;
use serde_json::Value;

/// Larger bodies are truncated: nothing useful to an analyst is past this point.
pub(crate) const MAX_SCRIPT_BYTES: usize = 5 * 1024 * 1024;
/// Bodies are held in memory for the whole scan.
pub(crate) const MAX_TOTAL_BYTES: usize = 60 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Admission {
    Stored,
    /// Same URL and same source as a script already held.
    Duplicate,
    Empty,
    /// The capture already holds `MAX_TOTAL_BYTES`.
    OverBudget,
}

struct Entry {
    url: String,
    inline: bool,
    body: String,
    loaded_at_epoch_ms: Option<f64>,
    /// When the same script was loaded again, on a later page view.
    loaded_again_at_epoch_ms: Vec<f64>,
    initiator: Option<String>,
}

#[derive(Default)]
pub(crate) struct ScriptLibrary {
    entries: Vec<Entry>,
    /// Position in `entries` by fingerprint.
    fingerprints: HashMap<u64, usize>,
    total_bytes: usize,
    inline_count: usize,
}

impl ScriptLibrary {
    /// The same file under two URLs is kept twice on purpose: a library loaded
    /// from two places is itself a finding.
    pub(crate) fn add_external(
        &mut self,
        url: String,
        body: String,
        loaded_at_epoch_ms: Option<f64>,
        initiator: Option<String>,
    ) -> Admission {
        let fingerprint = fingerprint(&url, &body);
        if let Some(rejection) = self.rejection(fingerprint, &body, loaded_at_epoch_ms) {
            return rejection;
        }
        self.store(
            fingerprint,
            Entry {
                url,
                inline: false,
                body,
                loaded_at_epoch_ms,
                loaded_again_at_epoch_ms: Vec::new(),
                initiator,
            },
        )
    }

    /// Inline blocks get the synthetic URL `inline:<n>`; `document_url` becomes
    /// their initiator.
    pub(crate) fn add_inline(
        &mut self,
        body: String,
        loaded_at_epoch_ms: Option<f64>,
        document_url: Option<String>,
    ) -> Admission {
        let fingerprint = fingerprint("inline:", &body);
        if let Some(rejection) = self.rejection(fingerprint, &body, loaded_at_epoch_ms) {
            return rejection;
        }
        self.inline_count += 1;
        let url = format!("inline:{}", self.inline_count);
        self.store(
            fingerprint,
            Entry {
                url,
                inline: true,
                body,
                loaded_at_epoch_ms,
                loaded_again_at_epoch_ms: Vec::new(),
                initiator: document_url,
            },
        )
    }

    /// A script held already keeps its one body; the repeated load is noted,
    /// because every page view that loaded the script lists it.
    fn rejection(
        &mut self,
        fingerprint: u64,
        body: &str,
        loaded_at_epoch_ms: Option<f64>,
    ) -> Option<Admission> {
        if body.trim().is_empty() {
            Some(Admission::Empty)
        } else if let Some(held) = self.fingerprints.get(&fingerprint) {
            self.entries[*held]
                .loaded_again_at_epoch_ms
                .extend(loaded_at_epoch_ms);
            Some(Admission::Duplicate)
        } else if self.total_bytes >= MAX_TOTAL_BYTES {
            Some(Admission::OverBudget)
        } else {
            None
        }
    }

    fn store(&mut self, fingerprint: u64, mut entry: Entry) -> Admission {
        truncate_at_char_boundary(&mut entry.body, MAX_SCRIPT_BYTES);
        entry.url = without_embedded_payload(entry.url);
        entry.initiator = entry.initiator.map(without_embedded_payload);
        self.total_bytes += entry.body.len();
        self.fingerprints.insert(fingerprint, self.entries.len());
        self.entries.push(entry);
        Admission::Stored
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    /// `place` turns an epoch time into the page view showing then and the
    /// time since that page view started.
    pub(crate) fn into_captured(self, place: impl Fn(f64) -> (u32, f64)) -> Vec<CapturedScript> {
        self.entries
            .into_iter()
            .map(|entry| {
                let first_load = entry.loaded_at_epoch_ms.map(&place);
                let mut pages: Vec<u32> = first_load
                    .iter()
                    .map(|(page, _)| *page)
                    .chain(entry.loaded_again_at_epoch_ms.iter().map(|at| place(*at).0))
                    .collect();
                pages.sort_unstable();
                pages.dedup();
                if pages.is_empty() {
                    pages.push(0);
                }
                CapturedScript {
                    url: entry.url,
                    inline: entry.inline,
                    body: entry.body,
                    loaded_at_ms: first_load.map(|(_, since_page_start)| since_page_start),
                    initiator: entry.initiator,
                    pages,
                    request_seq: None,
                }
            })
            .collect()
    }
}

fn fingerprint(url: &str, body: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    (url, body).hash(&mut hasher);
    hasher.finish()
}

fn truncate_at_char_boundary(text: &mut String, max_bytes: usize) {
    if text.len() > max_bytes {
        let mut end = max_bytes;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
}

/// A `data:` URL is its own content. URLs outlive the capture as script
/// metadata, which must stay small and free of raw payloads, so a long one is
/// reduced to its media type, its length and a fingerprint that keeps it
/// distinct.
fn without_embedded_payload(url: String) -> String {
    const MAX_DATA_URL_BYTES: usize = 120;
    const MAX_MEDIA_TYPE_BYTES: usize = 60;
    if !url.starts_with("data:") || url.len() <= MAX_DATA_URL_BYTES {
        return url;
    }
    let mut media_type = url.split(',').next().unwrap_or_default().to_string();
    truncate_at_char_boundary(&mut media_type, MAX_MEDIA_TYPE_BYTES);
    format!(
        "{media_type},[{} bytes, #{:08x}]",
        url.len(),
        fingerprint("data:", &url) as u32
    )
}

/// Scripts of the browser itself or of extensions say nothing about the page.
pub(crate) fn is_page_script_url(url: &str) -> bool {
    const INTERNAL_SCHEMES: &[&str] = &[
        "chrome-extension:",
        "chrome:",
        "devtools:",
        "about:",
        "extensions:",
    ];
    !url.is_empty()
        && !INTERNAL_SCHEMES
            .iter()
            .any(|scheme| url.starts_with(scheme))
}

/// URL of the script or document behind a CDP `Network.Initiator`.
pub(crate) fn initiator_url(initiator: &Value) -> Option<String> {
    let direct = initiator["url"].as_str().filter(|url| !url.is_empty());
    direct
        .or_else(|| first_stack_url(&initiator["stack"]))
        .map(str::to_string)
}

fn first_stack_url(stack: &Value) -> Option<&str> {
    let mut stack = stack;
    while stack.is_object() {
        let frames = stack["callFrames"].as_array().into_iter().flatten();
        if let Some(url) = frames
            .filter_map(|frame| frame["url"].as_str())
            .find(|url| !url.is_empty())
        {
            return Some(url);
        }
        stack = &stack["parent"];
    }
    None
}

/// Text of a `Network.getResponseBody` result.
pub(crate) fn response_text(result: &Value) -> Option<String> {
    let body = result["body"].as_str()?;
    if result["base64Encoded"].as_bool() != Some(true) {
        return Some(body.to_string());
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(body)
        .ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn external(library: &mut ScriptLibrary, url: &str, body: &str) -> Admission {
        library.add_external(url.into(), body.into(), None, None)
    }

    #[test]
    fn same_url_and_body_is_stored_once() {
        let mut library = ScriptLibrary::default();
        assert_eq!(
            external(&mut library, "https://a.test/x.js", "let a = 1"),
            Admission::Stored
        );
        assert_eq!(
            external(&mut library, "https://a.test/x.js", "let a = 1"),
            Admission::Duplicate
        );
        assert_eq!(library.len(), 1);
    }

    #[test]
    fn same_body_under_another_url_and_changed_body_are_both_kept() {
        let mut library = ScriptLibrary::default();
        external(&mut library, "https://a.test/gpt.js", "gpt()");
        assert_eq!(
            external(&mut library, "https://a.test/gpt.js?again", "gpt()"),
            Admission::Stored
        );
        assert_eq!(
            external(&mut library, "https://a.test/gpt.js", "gpt(2)"),
            Admission::Stored
        );
        assert_eq!(library.len(), 3);
    }

    #[test]
    fn inline_scripts_are_numbered_and_deduped_by_body() {
        let mut library = ScriptLibrary::default();
        let page = Some("https://a.test/".to_string());
        assert_eq!(
            library.add_inline("one()".into(), Some(1_500.0), page.clone()),
            Admission::Stored
        );
        assert_eq!(
            library.add_inline("  \n".into(), None, page.clone()),
            Admission::Empty
        );
        assert_eq!(
            library.add_inline("one()".into(), None, page.clone()),
            Admission::Duplicate
        );
        assert_eq!(
            library.add_inline("two()".into(), None, page),
            Admission::Stored
        );

        let scripts = library.into_captured(|at| (0, at - 1_000.0));
        let urls: Vec<&str> = scripts.iter().map(|script| script.url.as_str()).collect();
        assert_eq!(urls, ["inline:1", "inline:2"]);
        assert!(scripts.iter().all(|script| script.inline));
        assert_eq!(scripts[0].loaded_at_ms, Some(500.0));
        assert_eq!(scripts[0].initiator.as_deref(), Some("https://a.test/"));
    }

    #[test]
    fn a_script_lists_every_page_view_that_loaded_it() {
        let mut library = ScriptLibrary::default();
        let mut load = |url: &str, at: f64| {
            library.add_external(url.into(), "wrapper()".into(), Some(at), None)
        };
        assert_eq!(load("https://a.test/hb.js", 21_500.0), Admission::Stored);
        assert_eq!(load("https://a.test/hb.js", 41_200.0), Admission::Duplicate);
        assert_eq!(load("https://a.test/hb.js", 22_900.0), Admission::Duplicate);
        assert_eq!(
            load("https://a.test/only-once.js", 1_300.0),
            Admission::Stored
        );

        // Page views of 20 seconds each.
        let scripts = library.into_captured(|at| ((at / 20_000.0).floor() as u32, at % 20_000.0));

        assert_eq!(scripts[0].pages, [1, 2]);
        assert_eq!(
            scripts[0].loaded_at_ms,
            Some(1_500.0),
            "counted from page 1"
        );
        assert_eq!(scripts[1].pages, [0]);
        assert_eq!(scripts[1].loaded_at_ms, Some(1_300.0));
    }

    #[test]
    fn oversized_body_is_truncated_on_a_character_boundary() {
        let mut library = ScriptLibrary::default();
        let body = "é".repeat(MAX_SCRIPT_BYTES / 2 + 10);
        assert_eq!(
            external(&mut library, "https://a.test/big.js", &body),
            Admission::Stored
        );
        let scripts = library.into_captured(|at| (0, at));
        assert!(scripts[0].body.len() <= MAX_SCRIPT_BYTES);
        assert!(scripts[0].body.len() >= MAX_SCRIPT_BYTES - 1);
        assert!(scripts[0].body.chars().all(|c| c == 'é'));
    }

    #[test]
    fn stops_storing_once_the_total_budget_is_spent() {
        let mut library = ScriptLibrary::default();
        let body = "x".repeat(MAX_SCRIPT_BYTES);
        for index in 0..(MAX_TOTAL_BYTES / MAX_SCRIPT_BYTES) {
            assert_eq!(
                external(&mut library, &format!("https://a.test/{index}.js"), &body),
                Admission::Stored
            );
        }
        assert_eq!(
            external(&mut library, "https://a.test/late.js", "late()"),
            Admission::OverBudget
        );
        assert_eq!(
            external(&mut library, "https://a.test/0.js", &body),
            Admission::Duplicate
        );
    }

    #[test]
    fn long_data_urls_are_stored_without_their_payload() {
        let mut library = ScriptLibrary::default();
        let data_url =
            |code: &str| format!("data:text/javascript;charset=utf-8,{}", code.repeat(100));
        external(&mut library, &data_url("a()%3B"), "a();");
        external(&mut library, &data_url("b()%3B"), "b();");
        external(&mut library, "data:text/javascript,c()", "c()");
        library.add_external(
            "https://a.test/x.js".into(),
            "x()".into(),
            None,
            Some(data_url("a()%3B")),
        );

        let scripts = library.into_captured(|at| (0, at));
        assert_eq!(
            scripts[0].url,
            format!(
                "data:text/javascript;charset=utf-8,[635 bytes, #{:08x}]",
                fingerprint("data:", &data_url("a()%3B")) as u32
            )
        );
        assert_ne!(
            scripts[0].url, scripts[1].url,
            "different scripts stay distinct"
        );
        assert_eq!(scripts[2].url, "data:text/javascript,c()");
        assert_eq!(scripts[3].initiator.as_ref(), Some(&scripts[0].url));
    }

    #[test]
    fn internal_urls_are_not_page_scripts() {
        assert!(is_page_script_url("https://cdn.test/prebid.js"));
        assert!(is_page_script_url("blob:https://a.test/1234"));
        assert!(!is_page_script_url("chrome-extension://abc/content.js"));
        assert!(!is_page_script_url("devtools://devtools/bundled/x.js"));
        assert!(!is_page_script_url(""));
    }

    #[test]
    fn initiator_prefers_the_direct_url_then_the_nearest_stack_frame() {
        assert_eq!(
            initiator_url(&json!({"type": "parser", "url": "https://a.test/"})).as_deref(),
            Some("https://a.test/")
        );
        let from_script = json!({"type": "script", "stack": {
            "callFrames": [{"url": ""}, {"url": "https://a.test/wrapper.js"}]
        }});
        assert_eq!(
            initiator_url(&from_script).as_deref(),
            Some("https://a.test/wrapper.js")
        );
        let from_async_parent = json!({"type": "script", "stack": {
            "callFrames": [],
            "parent": {"callFrames": [{"url": "https://a.test/loader.js"}]}
        }});
        assert_eq!(
            initiator_url(&from_async_parent).as_deref(),
            Some("https://a.test/loader.js")
        );
        assert_eq!(initiator_url(&json!({"type": "other"})), None);
    }

    #[test]
    fn response_text_decodes_base64_bodies() {
        assert_eq!(
            response_text(&json!({"body": "let a", "base64Encoded": false})).as_deref(),
            Some("let a")
        );
        assert_eq!(
            response_text(&json!({"body": "bGV0IGE=", "base64Encoded": true})).as_deref(),
            Some("let a")
        );
        assert_eq!(
            response_text(&json!({"body": "***", "base64Encoded": true})),
            None
        );
        assert_eq!(response_text(&json!({})), None);
    }
}
