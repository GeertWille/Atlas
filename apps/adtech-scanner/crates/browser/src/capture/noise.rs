//! What Browserbase itself does in a page it hosts, so that it stays out
//! of the scan's record: the service keeps its own DevTools client on
//! every page, logs to the page's console, runs scripts in worlds of its
//! own, and puts scripts of its own into the page's world before the
//! page's first script. Every pattern here was observed on the service
//! (see `docs/plans/browserbase-measurements.md`, and for the scripts a
//! session of 2026-11-13); nothing is guessed. A local browser has no such
//! noise and is never filtered.

use std::collections::HashSet;

use serde_json::Value;

const KEEPALIVE_TEXT: &str = "Browserbase keeping connection alive";
const KEEPALIVE_SCRIPT: &str = "./dist/scripts/keepalive.js";
const CAPTCHA_SOLVING_PREFIX: &str = "browserbase-solving-";
const CAPTCHA_EVENT_KEY: &str = r#""key":"browserbase-captcha-event""#;
const SERVICE_WORLD_PREFIXES: [&str; 2] = ["__playwright_utility_world_", "__bb_"];
const SERVICE_WORLD_NAME: &str = "Native Select Polyfill";
const SERVICE_EXTENSION_ORIGIN: &str = "chrome-extension://cfiapcmfjmppkelmogiebeahmidbpdbi";
const WEBGL_FALLBACK_NOTICE: &str = "Automatic fallback to software WebGL";
/// How the scripts start that the service runs in the page's own world of
/// every document, each wrapped in a function: five of 37 to 73 kB on a
/// page of 2026-11-13, for its captcha solving among other things.
const SERVICE_SCRIPT_MARKER: &str = "/* browserbase:stealth-init-script */";
/// The marker is the first thing inside the wrapper.
const SERVICE_SCRIPT_MARKER_WITHIN: usize = 64;

/// Whether the source of a script that ran in the page's world without an
/// address is one the service put there, not the page.
pub(crate) fn is_service_script(source: &str) -> bool {
    let head_ends = source
        .char_indices()
        .nth(SERVICE_SCRIPT_MARKER_WITHIN)
        .map_or(source.len(), |(at, _)| at);
    let head = &source[..head_ends];
    head.trim_start().starts_with("(() => {") && head.contains(SERVICE_SCRIPT_MARKER)
}

/// The first argument of a console call, when it is a string.
fn first_text(console_call: &Value) -> Option<&str> {
    console_call["args"][0]["value"].as_str()
}

/// Whether a `Runtime.consoleAPICalled` is the service's own line.
fn is_service_console_call(console_call: &Value) -> bool {
    let Some(text) = first_text(console_call) else {
        return false;
    };
    // The scanner's tracker wraps the console, and its frame, which has no
    // URL, is then on top of the one that logged.
    let logged_by = console_call["stackTrace"]["callFrames"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|frame| frame["url"].as_str())
        .find(|url| !url.is_empty())
        .unwrap_or_default();
    (text == KEEPALIVE_TEXT && logged_by == KEEPALIVE_SCRIPT)
        || text.starts_with(CAPTCHA_SOLVING_PREFIX)
        || text.contains(CAPTCHA_EVENT_KEY)
}

/// Whether the execution context of a `Runtime.executionContextCreated`
/// is a world the service made for itself.
fn is_service_world(context: &Value) -> bool {
    let name = context["name"].as_str().unwrap_or_default();
    let origin = context["origin"].as_str().unwrap_or_default();
    let is_isolated = context["auxData"]["isDefault"] == false;
    SERVICE_WORLD_PREFIXES
        .iter()
        .any(|prefix| name.starts_with(prefix))
        || name == SERVICE_WORLD_NAME
        || origin == SERVICE_EXTENSION_ORIGIN
        || origin.starts_with(&format!("{SERVICE_EXTENSION_ORIGIN}/"))
        || (is_isolated && is_w_hex(name))
}

/// The world the service makes on a blank page with everything switched
/// off: `w` and a run of hex digits.
fn is_w_hex(name: &str) -> bool {
    name.strip_prefix('w')
        .is_some_and(|rest| !rest.is_empty() && rest.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

/// Whether a `Log.entryAdded` is the browser's note that the service's
/// machine renders WebGL in software.
fn is_service_log_entry(log_entry: &Value) -> bool {
    let entry = &log_entry["entry"];
    entry["source"] == "rendering"
        && entry["text"]
            .as_str()
            .is_some_and(|text| text.contains(WEBGL_FALLBACK_NOTICE))
}

/// Tells the service's activity in a page from the page's own.
#[derive(Debug, Default)]
pub(crate) struct ServiceNoise {
    /// The service's worlds that exist, by session and context id.
    worlds: HashSet<(String, u64)>,
}

impl ServiceNoise {
    /// Takes every event of a tab's session and says whether it is the
    /// service's doing and must not be recorded.
    pub(crate) fn is_noise(&mut self, session_id: &str, method: &str, params: &Value) -> bool {
        match method {
            "Runtime.executionContextCreated" => {
                let context = &params["context"];
                let is_service = is_service_world(context);
                if let (true, Some(id)) = (is_service, context["id"].as_u64()) {
                    self.worlds.insert((session_id.to_string(), id));
                }
                is_service
            }
            "Runtime.executionContextDestroyed" => {
                let id = params["executionContextId"].as_u64().unwrap_or_default();
                self.worlds.remove(&(session_id.to_string(), id))
            }
            "Runtime.executionContextsCleared" => {
                self.worlds.retain(|(session, _)| session != session_id);
                false
            }
            "Runtime.consoleAPICalled" => {
                let from_service_world = params["executionContextId"]
                    .as_u64()
                    .is_some_and(|id| self.worlds.contains(&(session_id.to_string(), id)));
                from_service_world || is_service_console_call(params)
            }
            "Log.entryAdded" => is_service_log_entry(params),
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn console(kind: &str, text: &str, stack_url: &str, context: u64) -> Value {
        json!({
            "type": kind,
            "args": [{ "type": "string", "value": text }],
            "executionContextId": context,
            "timestamp": 1_763_024_000_000.0_f64,
            "stackTrace": { "callFrames": [{ "functionName": "", "url": stack_url }] },
        })
    }

    fn world(id: u64, name: &str, origin: &str, is_default: bool) -> Value {
        json!({ "context": {
            "id": id,
            "name": name,
            "origin": origin,
            "auxData": { "isDefault": is_default, "frameId": "F" },
        } })
    }

    #[test]
    fn the_services_console_lines_are_noise_exactly_as_they_were_measured() {
        let mut noise = ServiceNoise::default();
        let mut is_noise = |call: Value| noise.is_noise("tab", "Runtime.consoleAPICalled", &call);

        assert!(is_noise(console(
            "info",
            "Browserbase keeping connection alive",
            "./dist/scripts/keepalive.js",
            1
        )));
        // With the scanner's tracker on the page, its console wrapper is the
        // top frame (observed on Chromium 153).
        assert!(is_noise(json!({
            "type": "info",
            "args": [{ "type": "string", "value": "Browserbase keeping connection alive" }],
            "executionContextId": 3,
            "stackTrace": { "callFrames": [
                { "functionName": "info", "lineNumber": 699, "scriptId": "13", "url": "" },
                { "functionName": "", "lineNumber": 0, "url": "./dist/scripts/keepalive.js" },
            ] },
        })));
        assert!(is_noise(console(
            "log",
            "browserbase-solving-started",
            "",
            1
        )));
        assert!(is_noise(console(
            "log",
            "browserbase-solving-finished",
            "",
            1
        )));
        assert!(is_noise(console(
            "log",
            r#"{"key":"browserbase-captcha-event","value":{"type":"solved"}}"#,
            "",
            1
        )));
    }

    #[test]
    fn what_a_page_logs_itself_is_kept_even_when_it_looks_alike() {
        let mut noise = ServiceNoise::default();
        let mut is_noise = |call: Value| noise.is_noise("tab", "Runtime.consoleAPICalled", &call);

        // The same words from the page's own script.
        assert!(!is_noise(console(
            "info",
            "Browserbase keeping connection alive",
            "https://publisher.example/app.js",
            1
        )));
        assert!(!is_noise(json!({
            "type": "info",
            "args": [{ "type": "string", "value": "Browserbase keeping connection alive" }],
            "stackTrace": { "callFrames": [
                { "functionName": "info", "url": "" },
                { "functionName": "log", "url": "https://publisher.example/app.js" },
                { "functionName": "", "url": "./dist/scripts/keepalive.js" },
            ] },
        })));
        assert!(!is_noise(console(
            "info",
            "Browserbase keeping connection alive",
            "",
            1
        )));
        assert!(!is_noise(console("log", "Prebid: auction ended", "", 1)));
        assert!(!is_noise(console(
            "log",
            "solving browserbase-solving-",
            "",
            1
        )));
        assert!(!is_noise(console(
            "log",
            r#"{"key":"captcha-event"}"#,
            "",
            1
        )));
        // An argument that is not a string at all.
        assert!(!is_noise(
            json!({ "type": "log", "args": [{ "type": "number", "value": 3 }] })
        ));
        assert!(!is_noise(json!({ "type": "log", "args": [] })));
    }

    #[test]
    fn the_services_worlds_are_noise_and_so_is_what_they_log() {
        let extension = "chrome-extension://cfiapcmfjmppkelmogiebeahmidbpdbi";
        let service_worlds = [
            world(
                11,
                "__playwright_utility_world_page@5f0c6a1e",
                "https://example.com",
                false,
            ),
            world(12, "__bb_frame_reads", "https://example.com", false),
            world(13, "Native Select Polyfill", extension, false),
            world(14, "some other name", extension, false),
            world(15, "w3fa9c01b", "://", false),
        ];
        let mut noise = ServiceNoise::default();
        for created in &service_worlds {
            assert!(
                noise.is_noise("tab", "Runtime.executionContextCreated", created),
                "{created}"
            );
        }

        let from_world = |id| console("log", "select polyfilled", "", id);
        assert!(noise.is_noise("tab", "Runtime.consoleAPICalled", &from_world(13)));
        // Context ids are per session.
        assert!(!noise.is_noise("other tab", "Runtime.consoleAPICalled", &from_world(13)));

        // A destroyed world's id may be given to a world of the page.
        let destroyed = json!({ "executionContextId": 13 });
        assert!(noise.is_noise("tab", "Runtime.executionContextDestroyed", &destroyed));
        assert!(!noise.is_noise("tab", "Runtime.consoleAPICalled", &from_world(13)));
        assert!(noise.is_noise("tab", "Runtime.consoleAPICalled", &from_world(12)));
        noise.is_noise("tab", "Runtime.executionContextsCleared", &json!({}));
        assert!(!noise.is_noise("tab", "Runtime.consoleAPICalled", &from_world(12)));
    }

    #[test]
    fn the_pages_own_worlds_and_the_scanners_are_not_noise() {
        let mut noise = ServiceNoise::default();
        let own_worlds = [
            world(1, "", "https://example.com", true),
            world(2, "adscan", "https://example.com", false),
            world(3, "__playwright", "https://example.com", false),
            // A page may name its main world's frame anything.
            world(4, "wabc", "https://example.com", true),
            world(5, "w", "https://example.com", false),
            world(6, "widget", "https://example.com", false),
            world(
                7,
                "",
                "chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                false,
            ),
        ];
        for created in &own_worlds {
            assert!(
                !noise.is_noise("tab", "Runtime.executionContextCreated", created),
                "{created}"
            );
        }
        assert!(!noise.is_noise(
            "tab",
            "Runtime.consoleAPICalled",
            &console("log", "hello", "", 2)
        ));
    }

    #[test]
    fn the_software_webgl_notice_is_noise_and_other_log_entries_are_not() {
        let mut noise = ServiceNoise::default();
        let entry = |source: &str, text: &str| json!({ "entry": { "source": source, "level": "warning", "text": text } });
        let notice = "Automatic fallback to software WebGL has been deprecated. Please use the \
                      --enable-unsafe-swiftshader flag to opt in to lower security guarantees \
                      for trusted content.";

        assert!(noise.is_noise("tab", "Log.entryAdded", &entry("rendering", notice)));
        assert!(!noise.is_noise("tab", "Log.entryAdded", &entry("javascript", notice)));
        assert!(!noise.is_noise(
            "tab",
            "Log.entryAdded",
            &entry("rendering", "A WebGL context was lost")
        ));
        assert!(!noise.is_noise(
            "tab",
            "Log.entryAdded",
            &entry("network", "Failed to load resource: net::ERR_FAILED")
        ));
        assert!(!noise.is_noise("tab", "Page.loadEventFired", &json!({})));
    }

    #[test]
    fn the_services_own_scripts_in_the_pages_world_are_told_by_how_they_start() {
        // The starts of the five that were on https://example.com/.
        for measured in [
            "(() => {\n      /* browserbase:stealth-init-script */\n\"use strict\";\n\n// ../../node_modules/.pnpm/uuid@11.1.1/node_modules/uuid/dist/esm-browser/stringify.js\nvar byteToHex = [];",
            "(() => {\n      /* browserbase:stealth-init-script */\n\"use strict\";\n\n// src/captchas/solve-markers.ts\nvar SOLVE_STARTED = \"browserbase-solving-started\";",
        ] {
            assert!(is_service_script(measured), "{measured}");
        }
        for the_pages_own in [
            "",
            "(() => { window.pbjs = window.pbjs || {}; })();",
            "var note = '/* browserbase:stealth-init-script */';",
            "/* browserbase:stealth-init-script */ var a = 1;",
            // The marker somewhere inside a script of the page.
            &format!(
                "(() => {{ {} /* browserbase:stealth-init-script */ }})()",
                "x();".repeat(40)
            ),
            "(() => {\n  // ünïcödé before any marker: ééééééééééééééééééééééééééééééééééééééééééééééééééééééééééé",
        ] {
            assert!(!is_service_script(the_pages_own), "{the_pages_own}");
        }
    }
}
