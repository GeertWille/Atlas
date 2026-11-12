# Script source viewer: design and implementation plan

The key decision is retention: I recommend an in-memory session cache as the default plus an opt-in local store that is off by default. The viewer itself should be a dedicated full-height route built on CodeMirror 6, with text formatted by the existing oxc formatter in Rust.

## What the code says today

- **Evidence lines refer to beautified text only.** `Evidence.line` is "1-based line in the beautified source". `verify_citation` in `analyze/toolbox.rs` stores `lines[line-1..end].join("\n")`, so the stored body is the formatter's own text, not the model's copy.
- **No end line and no script id are stored.** Evidence carries `script_url` only, and the frontend deep-links by URL (`ScriptsTab` matches `script.url === focusedScriptUrl`). That is ambiguous when one URL loads twice, which the demo site does with GPT.
- **Excerpts are up to 600 characters, not 300.** `MAX_EVIDENCE_CHARS = 600` in `toolbox.rs`; the handover's 300 applies to bidder checks and triage excerpts. Tell the product owner.
- **Only AI findings carry code evidence.** Heuristic findings use metric, log and timeline evidence, so findings-in-code exist only for scripts handed to Phase 2.
- **Only Phase 2 scripts are beautified.** That is at most 12 scripts and 4 MB (`graph.rs`). `OxcFormatter` returns raw lines above 3 MB or after 20 s, and falls back to `Reflow` when parsing fails.
- **Bodies die with the scan.** They live in `ScanState.capture.scripts[i].body` and `ScanState.sources[*].lines`, dropped when `Pipeline::run` returns.
- **A whitespace-insensitive locator already exists.** `locate(lines, needle, cited)` in `toolbox.rs` finds the occurrence nearest a hint line. It is the right tool for mapping evidence to line ranges.
- **"What the AI read" is already persisted.** Each `ToolCall` log has `data: { tool, input }` with `script_id`, `start_line`, `end_line`. The Jev guide logs `{ scriptId, lines: [start, end], rank, answers }` per chunk in `analyze/guide.rs`. No new storage is needed for either.
- **The initiator is captured but not recorded.** `CapturedScript.initiator` exists; `ScriptRecord` has `pages` but no initiator.
- **The handover rules out disk storage explicitly.** "Not built, on purpose: any switch that writes script bodies to disk." The Settings "What is stored" text and the About text both state the memory-only rule.

## 1. Retention decision

| Option | Works for | Cost and risk | Verdict |
|---|---|---|---|
| (a) Keep in app memory until quit | Scans from this session | Nothing on disk. Roughly 5 MB raw plus 6 MB formatted per scan. The rule wording changes from "dropped after the scan" to "dropped when you quit". | v1 default |
| (b) Re-fetch from the URL | External scripts only | No inline scripts, and wrappers are often inline. Content is versioned, geo- or A/B-varied, so evidence lines would not match. The app's own client would not use the scan's Chrome proxy profile. Adds outbound requests to arbitrary third-party hosts, which breaks the host list in About. | Reject. Offer "Open URL in browser" instead. |
| (c) Opt-in local retention | All later sessions | Reverses an explicit owner decision. Inline scripts from a logged-in, manual or persistent-profile visit can hold personal data. | Only behind a setting, off by default |
| (d) Hybrid: (a) always, (c) as a setting | Both | Two store implementations behind one port | Recommended |

How (d) should work:

- **One port.** `ScriptSourceStore { put(scan_id, sources), get(scan_id, script_id), delete(scan_id), clear() }`, written once when a scan completes. Failed and cancelled scans have no Scripts tab and store nothing.
- **Memory store, always on.** Bodies are `Arc<str>` deduplicated by hash. Evict least-recently-used scans at about 150 MB or 10 scans.
- **Disk store, when the setting is on.** It stores all captured scripts as raw text and re-formats on demand. Storing ad-related scripts only saves little after compression and makes the viewer feel broken for the rest.
- **Anchors survive formatter upgrades.** Because anchors are always resolved with `locate` using the stored line as a hint, an oxc upgrade cannot misalign old scans.
- **Storage layout.** A separate SQLite file `script-sources.db` in the data dir, with `bodies(sha256 PRIMARY KEY, raw_len, zstd BLOB)` and `scan_scripts(scan_id, script_id, sha256)`. Use the full sha-256 as the key, not the 16-hex `ScriptRecord.hash`.
- **Why a separate file.** The main database stays small, and "Clear stored sources" becomes delete-and-recreate with no `VACUUM` of the main database. Blobs of 10 KB to 1 MB perform fine in SQLite and delete transactionally; loose files would add orphan handling.
- **Size estimate (not measured).** zstd level 3 on minified JS gives roughly 3.5 to 4x. A 3 to 6 MB page is about 1 to 1.6 MB per scan. Repeat scans of the same site mostly dedupe (GPT, Prebid and CMP bodies are identical), so they cost perhaps 10 to 30% of that. A 500 MB default cap evicting oldest scans is ample; add an optional "keep for N days".
- **No encryption at rest.** The content is what the site serves to every visitor, the findings database is unencrypted too, and FileVault covers the disk. A Keychain-held key would add failure modes for little gain. The honest risk is inline scripts, so say it in the setting text.
- **Deletion.** `delete_scan` and `delete_all_scans` call the store, then remove orphan bodies. Turning the setting off asks "Also delete stored sources?". A "Clear stored sources (212 MB)" button sits beside it.
- **Unavailable state.** The viewer shows what is still known (facts, evidence excerpts, AI-read ranges) and a "Scan again" button that reuses the scan's options. Say plainly that a new scan may capture different code. Line 12 of `ScanOptions` shows `ai_enabled` can be false, so offer "Scan again without AI (free)".

## 2. Editor engine

Versions verified with `npm view` today. Bundle sizes are from memory and were not measured; the implementer should measure the lazy chunk.

| | CodeMirror 6 | Monaco 0.57 | Shiki 4.6 + TanStack Virtual | prism (current) |
|---|---|---|---|---|
| CSP | No workers, no eval. Injects one `<style>`, allowed by `'unsafe-inline'`; `EditorView.cspNonce` exists if that tightens. | Needs worker files bundled same-origin and a custom worker setup | Oniguruma engine is WASM and needs `wasm-unsafe-eval`; the JS regex engine avoids it | Fine |
| Size | Roughly 120 KB gzip for the set below | Over 1 MB gzip; 100 MB unpacked package | Core plus one grammar and theme is moderate | Already in |
| Large files | Viewport rendering, incremental Lezer parse | Good | Must tokenise 20k lines up front, or build incremental tokenising yourself | Unusable without virtualisation |
| Search, folding, gutters, decorations, widgets | First-class extensions | Built in | All hand-built, including selection across virtual rows | None |
| WebKit and accessibility | Well supported; contenteditable with a documented screen-reader model | Historically weaker on WebKit; heavy | Selection, copy and find across virtualised rows are the hard part | n/a |
| Theming with CSS variables | Yes | Themes are JS colour tables, re-registered on theme change | Yes with a CSS-variables theme | Yes |

**Recommendation: CodeMirror 6, assembled by hand.** Do not use the `codemirror` meta package (its `basicSetup` pulls in autocomplete, lint, history and close-brackets) or `@uiw/react-codemirror`. Write a small `useCodeMirror` hook that holds the `EditorView` in a ref and reconfigures through `Compartment`s.

Packages:

- `@codemirror/state` 6.7.6
- `@codemirror/view` 6.43.15: `lineNumbers`, `gutter`, `highlightActiveLine`, `highlightActiveLineGutter`, `highlightSpecialChars`, `drawSelection`, `Decoration`, `EditorView.lineWrapping`
- `@codemirror/language` 6.13.1: `syntaxHighlighting`, `foldGutter`, `codeFolding`, `foldKeymap`, `bracketMatching`
- `@codemirror/search` 6.7.2: `search({ createPanel })`, `highlightSelectionMatches`, `gotoLine`
- `@codemirror/commands` 6.11.1: `standardKeymap` only
- `@codemirror/lang-javascript` 6.2.5
- `@lezer/highlight` 1.2.5: use `classHighlighter` and add `.tok-*` rules in `src/styles/app.css` mapped to the existing `--syn-*` tokens, so light and dark switch with no editor reconfigure

Engine decisions:

- **Read-only setup.** `EditorState.readOnly.of(true)` with the editor left focusable, so there is a caret, keyboard selection and Cmd+F focus. Add `aria-readonly` and an `aria-label` through `contentAttributes`. Do not bind Tab, so focus is never trapped.
- **Overview ruler.** Build a 12 px positioned strip yourself. `@replit/codemirror-minimap` (0.5.2) is pre-1.0, and a pixel minimap of minified code carries no information.
- **Left out.** Sticky scroll, breadcrumbs and a generic symbol outline. In minified code the scopes and names are `t`, `e`, `n`; they add clutter without meaning.
- **Excerpts.** Keep prism for short excerpts in v1. Later, Lezer's `highlightCode` could replace it and remove a dependency.

## 3. Reading experience

**Layout.** Use a dedicated full-height route, `/scans/$scanId/source/$scriptId?line=118&to=124&view=original&finding=<id>`. The report page is a scrolling stack (header, overview, tabs), so a viewer inside the Scripts tab would have nested scrolling and about 400 px of height. The Scripts tab stays the inventory. No multi-file tabs: the file list, Back/Forward and Cmd+P cover it.

Entry points:

- Scripts tab: Enter or double-click on a row, or "View source" in `ScriptDetails`.
- Findings: the evidence chip in `EvidenceBlock`, which deep-links to the line range.

Main state at 1280x820 (content area is roughly 1060 px after the app sidebar):

```
+--------------------------------------------------------------------------------------------+
| < Report  pub.example > wrapper.js                       [Formatted|Original] [Wrap] [...] |
| Wrapper - first party - 41 KB - pages 1,3 - Jev 0.91 - analysed by Phase 2                 |
+----------------------+------------------------------------------------+--+-----------------+
| Filter scripts       |  112   function startAuction(cfg) {            |  | Findings (2)    |
| v Analysed (5)       |  113     var mq = window.matchMedia(           |::| ! High  L118    |
|  * wrapper.js     2! |! 118 |   if (mq("(min-width: 768px)")) {       |##| Scroll gate uses|
|    inline:4          |  119 |     waitForScroll(run);                 |::| min-width...    |
|    prebid.js      1! |  120 |   }                                     |  | ! Med   L402    |
| v Ad-related (14)    |      +-- High - Scroll gate uses min-width --+ |  |-----------------|
|    gpt.js   Google   |      |  ...  [Open finding] [Next F8]        | |  | Ad API calls(9) |
|    apstag.js Amazon  |      +---------------------------------------+ |--| requestBids L131|
| > Other (62)         |  121     pbjs.requestBids({ ...                |  | addAdUnits  L88 |
|                      |  122       timeout: 250,                       |  | setConfig   L61 |
|                      |                                                |  | defineSlot L140 |
|                      |                                                |  |-----------------|
|                      |                                                |  | AI read 61% [on]|
+----------------------+------------------------------------------------+--+-----------------+
| Formatted by the app (parser) - 1,204 lines - Ln 118, Col 5 - 2 findings - Find: 3 of 41   |
+--------------------------------------------------------------------------------------------+
   240 px list              code (flexible, at least 520 px)             ruler   260 px panel
```

- **File list.** Grouped Analysed, Ad-related, Other. Each row shows file name, hostname or vendor, a category dot, and finding count with the worst severity. It is filterable and virtualised with the existing TanStack Virtual. "Other" is collapsed by default.
- **Header.** Row one has breadcrumb, view toggle, wrap toggle, and an overflow menu with Copy URL, Open URL in browser, Copy all and Show in Scripts tab. Row two has the facts: category, vendor, first or third party, size, hash, who classified it with probability or confidence, pages, and initiator.
- **In the code.**
  - Gutter marker by severity, with an icon shape as well as colour.
  - Tinted line range using `--tone-*-soft`.
  - Clicking the marker toggles a one-line lens under the range: severity, title, "Open finding". The full card stays on the Findings tab.
  - F8 and Shift+F8 move between findings.
- **Overview ruler.** Two lanes. The left lane shows AI-read ranges and Jev guide chunks as a violet tint with opacity by rank. The right lane shows finding ticks in severity colour and search hits in amber. Click to jump.
- **Right panel.** Three stacked sections, each a jump list: Findings, Ad API calls, and "Read by the AI".
  - Ad API calls covers `requestBids`, `addAdUnits`, `setConfig`, `defineSlot`, `refresh`, `apstag.fetchBids`, `googletag.cmd`, `__tcfapi`.
  - "Read by the AI" has a toggle to dim unread lines. This shows directly which parts of the file the model never read.
- **Search.** Cmd+F opens a custom panel through `createPanel`, with match count, case, regex and whole-word. Cmd+G is next match and Ctrl+G is go to line. Cmd+Shift+F, search across all scripts with a results list, is v1.1.
- **Polish.**
  - JetBrains Mono at 12.5 px with 1.55 line height.
  - Jumps centre the target line and play a 600 ms flash, disabled under `prefers-reduced-motion`.
  - Scroll position and selection are remembered per scan, script and view for the session.
  - Selecting lines updates `line` and `to` in the URL with `replace`, so reload and Back work.
  - Cmd+P opens quick-open with the existing `cmdk`. `[` and `]` toggle the side panels.
- **Minified code.** Formatted is the default. The status bar says how it was made:
  - "Formatted by the app (parser)"
  - "Formatted approximately; the file did not parse"
  - "Shown as captured; too large to format"
- **Original view.** Forces wrap on and hides finding anchors, with a note that line numbers in findings refer to the formatted view. Source maps are out of scope.

States:

```
Loading:      list and header render at once from ScanDetail; code area shows skeleton lines
Unavailable:  +------------------------------------------------------------+
              |  This script's source is no longer held                    |
              |  Sources are kept until you quit the app. [Scan again]     |
              |  [Keep sources for future scans...]                        |
              |  -- Evidence the report kept (2) --  excerpt cards, L118.. |
              +------------------------------------------------------------+
Too large:    text over 8 MB is not sent; facts are shown with a note
No AI scan:   Findings section says "Code findings need an AI scan"; Ad API calls still work
```

**At 1024x680** (about 800 px of content): the right panel becomes an overlay drawer, closed by default, with a findings-count badge on its button. The file list shrinks to 200 px and collapses to a 40 px rail below 520 px of code width, with Cmd+P as the way in. The facts row truncates to one line with the rest in a tooltip. The ruler and status bar stay.

**Later, not v1:** diff between two scans or desktop versus mobile, sticky scroll, source maps, download to file, replacing prism.

## 4. Data flow and contract

Formatting stays in Rust (`OxcFormatter`), so viewer text is the analysed text. No JavaScript formatter ships.

Additive contract changes in `crates/model`:

```rust
// Evidence (additive, serde default)
pub script_id: Option<String>,
pub end_line: Option<u32>,
// ScriptRecord (additive)
pub initiator: Option<String>,   // through stored_url()
// Settings / SettingsPatch
pub keep_script_sources: bool,   // default false
// ErrorCode: no new code; unavailability is data, not an error

pub enum SourceView { Formatted, Original }
pub enum SourceAvailability { Available, NotRetained, TooLarge }
pub enum SourceFormat { Parser, Reflow, Unformatted }
pub struct ScriptSource {
    pub scan_id: String, pub script_id: String, pub view: SourceView,
    pub availability: SourceAvailability,
    pub text: Option<String>, pub line_count: u32, pub format: SourceFormat,
    pub anchors: Vec<SourceAnchor>,   // Formatted only
    pub regions: Vec<SourceRegion>,
    pub ad_calls: Vec<AdCall>,
}
pub struct SourceAnchor { pub finding_id: String, pub evidence_index: u32,
                          pub start_line: u32, pub end_line: u32, pub located: bool }
pub struct SourceRegion { pub kind: RegionKind /* Read | Guide */, pub start_line: u32,
                          pub end_line: u32, pub rank: Option<f64> }
pub struct AdCall { pub name: String, pub line: u32 }
pub struct SourceStorageInfo { pub scans: u32, pub bytes: f64 }
```

Commands:

- `get_script_source { scanId, scriptId, view } -> ScriptSource`
- `get_source_storage -> SourceStorageInfo`
- `clear_script_sources -> SourceStorageInfo`
- v1.1: `search_scan_scripts { scanId, query, regex, caseSensitive, limit } -> ScriptSearchHit[]`, reusing the toolbox's regex size limits

Backend behaviour:

- **Anchors.** Resolve the script by `script_id`, falling back to a URL match for old scans. Then run `locate(lines, squash(evidence.body), evidence.line)`, moved from `toolbox.rs` into `domain` so the citation check and the viewer share it. When not found, fall back to the stored line with `located: false`, drawn with a dashed marker.
- **Regions.** Read from stored logs: `ToolCall` entries where `tool == "read_script"` (clamped to the 250-line cap), and the Jev chunk log.
- **Ad calls.** A line scan of the formatted lines with the existing signature lists in `domain/knowledge.rs`. It also works on `Reflow` output; no AST pass.
- **Transport.** One command reply per script, never the `scan-event` stream. 2 to 6 MB of JSON should be fine over Tauri 2 IPC but is not measured. Start unchunked; only if the e2e budget fails, switch to `tauri::ipc::Response` raw bytes. Cap at 8 MB.
- **Caching.** Formatted text for non-analysed scripts is produced on first request and cached in the memory store. On the frontend, a TanStack Query entry keyed `["scriptSource", scanId, scriptId, view]` with infinite `staleTime` and a short `gcTime` (about 5 minutes), so only a few files stay in JS memory.

Fake backend, export and security:

- **Fake backend.** It gets a small source catalogue: the demo wrapper with the planted `min-width` bug at the line its fake finding cites, one generated 20k-line file, and one `notRetained` scan.
- **Export.** Unchanged; exports never contain bodies.
- **Untrusted text.** CodeMirror writes text nodes only. Do not add link detection or any HTML or markdown rendering of script text. Lens and panel content comes from findings, rendered as plain text.
- **Hidden characters.** `highlightSpecialChars` shows control, zero-width and bidi-override characters as visible placeholders. Force `direction: ltr`.
- **Never evaluated.** Text is never passed to `eval`, `Function`, a worker or a `blob:` URL. The CSP stays as it is.

## 5. Privacy wording

- **Settings, Data section, new switch "Keep script sources with reports" (off).** "Lets you open a script's code after you quit the app. The JavaScript captured during a scan is stored compressed on this Mac, next to the report, and is deleted with it. Inline scripts can contain details of the account you were signed in to on the scanned site. Off: sources are held in memory and discarded when you quit."
- **Below the switch.** "Stored sources: 212 MB in 34 scans" and a "Clear stored sources" button.
- **"What is stored" text.** Replace the sentence about script bodies with: "Script sources are held in memory until you quit, unless you turn on Keep script sources. Raw network payloads, ads.txt and sellers.json files are discarded after each scan."
- **About.** The host list is unchanged because nothing is re-fetched. Update the `SettingsScreen.test.tsx` expectations.
- **Handover.** Amend the "No raw payloads stored" rule and the "Not built, on purpose" line.

## 6. Implementation plan

**Step 0, contract (one person, first).** Add the model types, settings field and bindings (`cargo test -p adscan-model`). Add the `ApiPort` methods `getScriptSource`, `getSourceStorage`, `clearScriptSources`, with stubs in both adapters. Add the route and its search schema.

**Backend track (model, engine, src-tauri):**

1. Add the `ScriptSourceStore` port and the memory store. Hand sources over in `Pipeline::run` before the state drops. Set `Evidence.script_id` and `end_line` in `verify_citation`.
2. Move `locate` and `squash` to `domain`. Add the `get_script_source` use case: formatting on demand, anchors, regions from logs, ad calls.
3. Add the commands in `src-tauri`, and wire the store into `delete_scan` and `delete_all_scans`.
4. Add the zstd SQLite store (`zstd` 0.14.3), the setting, cap eviction, `clear`, and storage info.
5. v1.1: `search_scan_scripts`.

**Frontend track (against the fake backend):**

1. Fake sources, `useScriptSource`, and the route shell with header, file list and states.
2. `useCodeMirror`, theme and `.tok-*` CSS, the view toggle, wrap, status bar.
3. Anchor decorations, gutter, lens, ruler, right panel, F8 navigation.
4. Search panel, go to line, URL sync, scroll memory, Cmd+P.
5. Change `EvidenceBlock` and `ScriptDetails` deep links to script id and line. Add the settings switch and wording.
6. v1.1: the cross-script search panel.

**Tests:**

- **Rust unit.**
  - `locate` after the move, and anchor fallback.
  - Regions parsed from the log fixtures.
  - Store round trip, dedupe, cap eviction, delete cascade.
  - No disk write when the setting is off.
  - A pinned formatter fixture, so an oxc upgrade shows up as a diff.
- **Vitest.** The anchors-to-decorations mapping, ruler positions, URL-to-selection parsing, file-list grouping, and the unavailable state.
- **Playwright (fake backend).**
  - Evidence chip opens the right line with the flash.
  - F8 cycles findings.
  - Cmd+F shows a count.
  - Reload restores line and view.
  - axe passes on every state.
  - Keyboard-only path: Tab does not trap.
  - A script containing `<img onerror>` and bidi characters renders as inert text.
  - Layout at 1024x680.
  - Performance budget: a 20k-line file is interactive in under 500 ms after data arrives, a jump takes under 100 ms, and a 2 MB single-line Original view does not freeze. If it does, turn highlighting off above a line-length threshold.
- **e2e-tauri (WebKitGTK, production CSP).** A demo scan, then open the wrapper and see the planted `min-width` line marked. No CSP violations in the console. A restart shows "not retained" with the setting off and the source with it on. Deleting a scan shrinks the store.

## Open questions for the product owner

1. **Retention.** Do you accept "sources stay in memory until quit" as the default, plus an opt-in "Keep script sources" switch that is off by default? This reverses the handover's "no switch that writes bodies to disk". My recommendation is yes to both. The fallback is memory-only, with the viewer honest about it.
2. If retained, should the store hold all scripts (recommended) or ad-related only? Is a 500 MB cap right, and do you want a retention period?
3. Evidence excerpts are stored at up to 600 characters, while the handover says 300. Which is intended?
4. Is "Scan again without AI" acceptable as the recovery path, given that a new capture may differ from the one the findings cite?
5. Should cross-script search (Cmd+Shift+F) be in v1 or v1.1? I suggest v1.1.
6. Is "Copy all" enough for v1, or is "Download source" needed? Download is a user-initiated disk write of a body.

### Critical Files for Implementation
- /home/user/Atlas/apps/adtech-scanner/crates/model/src/lib.rs
- /home/user/Atlas/apps/adtech-scanner/crates/engine/src/application/pipeline/nodes/analyze/toolbox.rs
- /home/user/Atlas/apps/adtech-scanner/crates/engine/src/application/pipeline/mod.rs
- /home/user/Atlas/apps/adtech-scanner/src/app/router.tsx
- /home/user/Atlas/apps/adtech-scanner/src/features/report/findings/EvidenceBlock.tsx