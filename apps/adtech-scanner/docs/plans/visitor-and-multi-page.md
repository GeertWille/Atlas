# Implementation plan: visitor behaviour and multi-page browsing (adtech-scanner)

All paths are under `/home/user/Atlas/apps/adtech-scanner`. Nothing was modified and no browser was run, so Chromium behaviour below is from memory or from reading the repo's tests, and is marked where unsure.

## 0. Findings that shape the design

- **Launch is already pipe-based.** `crates/browser/src/cdp/chrome.rs` uses `--remote-debugging-pipe` and a locked throwaway profile in the temp dir. `Drop`, `shutdown()` and the watchdog all delete the profile directory, so a persistent profile needs changes in all three.
- **The app already suppresses `navigator.webdriver`.** `launch_arguments` passes `--disable-blink-features=AutomationControlled` (chrome.rs:311-312, with the comment "Without this navigator.webdriver is true and ad stacks see a bot"), and `crates/browser/tests/emulation.rs:83,147` assert it reads `false`. This conflicts with policy D (see 7.1).
- **Headless desktop already rewrites identity.** `device.rs::desktop` strips `HeadlessChrome/` and sets client hints, and chrome.rs adds `--blink-settings=primaryHoverType=…`. Headed desktop sends no UA override, but `Page::prepare` always applies `Emulation.setDeviceMetricsOverride` (1440x900, DSF 1.0), which policy D forbids for headed desktop.
- **What determines `navigator.webdriver`.** It returns the Blink runtime feature `AutomationControlled`. I am sure `--enable-automation` and `--headless` enable it (MDN documents both, plus `--remote-debugging-port=0`). I believe `--remote-debugging-pipe` enables it too (`content/child/runtime_features.cc`), but could not confirm; the first browser task must measure it headed and headless with the disable flag removed.
- **CDP attachment itself is detectable.** `Runtime.enable` and `Debugger.enable` can be noticed by page scripts regardless of flags. The design reports this and does not hide it.
- **One document per capture is hard-wired.** `TrackerLog::ingest` clears all events when a batch arrives with a new `performance.timeOrigin` (tracker_log.rs). `Page` is bound to one CDP session and drops events from any other session (page.rs `handle`).
- **Script bodies are already deduped** by (url, body) in `ScriptLibrary`, so multi-page only needs a page list on each entry.
- **Storage is mostly schema-free.** Auctions, slots, scripts, findings and options are JSON rows, so `#[serde(default)]` fields load old scans. Settings need a `json_set` migration (every field is required on load), and `scans` needs two new columns.
- **The tracker never reports mouse movement.** `firstInteraction` listens to pointerdown, touchstart, mousedown, keydown and wheel only, so a "bids on first mouse move" gate is invisible today.
- **JS dialogs are always accepted.** `Page.javascriptDialogOpening` answers `accept: true`, including `confirm()`. That is unsafe once the app clicks things.

## 1. Contract changes (`crates/model`)

### Time base (the central decision)
Every `t` and `*_at_ms` becomes "ms since the start of its page view", and every record carries a `page` index. `PageView.started_at_ms` is relative to `Capture.navigation_start` (entry page start). A single-page scan has page 0 starting at 0, so every stored number keeps its meaning. Existing rules, waterfalls and slot timelines work unchanged on a page-filtered view. Absolute time is `pages[page].started_at_ms + t`.

### `lib.rs` (ts-rs exported, persisted)

```rust
#[derive(Copy, Eq, Hash)]
pub enum VisitorBehavior { None, Scroll, Browse, Manual }

pub struct ScanOptions {
    // existing fields unchanged; auto_scroll stays for old callers and stored scans
    /// Absent: derived from auto_scroll (true -> scroll, false -> none).
    #[ts(optional)] #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visitor: Option<VisitorBehavior>,
    /// Further pages to visit by following links, 0-5. Browse only.
    #[ts(optional)] #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra_pages: Option<u32>,
    /// Seed of the scripted visit; filled in by the backend when absent.
    #[ts(optional)] #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u32>,          // u32: ts-rs maps u64 to bigint
    /// Absent: Settings.headless for none/scroll, false for browse. Manual is always headed.
    #[ts(optional)] #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headless: Option<bool>,
    /// Absent: Settings.persistent_profile.
    #[ts(optional)] #[serde(default, skip_serializing_if = "Option::is_none")]
    pub persistent_profile: Option<bool>,
}

// Settings and SettingsPatch gain:
pub default_visitor: VisitorBehavior,   // default Scroll (today's behaviour)
pub default_extra_pages: u32,           // default 2
pub persistent_profile: bool,           // default false

#[derive(Copy, Eq, Hash)]
pub enum PageReach { Entry, LinkClick, RouteChange, UserNavigation, Redirect, Reload }

pub struct FollowedLink {
    pub text: String,    // at most 80 chars
    pub href: String,    // origin + path, query values removed
    pub reason: String,  // "content link in <article>, article-like path; best of 5"
}

#[derive(Default)]
pub struct PageMetrics {
    pub auction_count: u32, pub bid_request_count: u32, pub bid_response_count: u32,
    pub slot_count: u32, pub empty_slot_count: u32, pub fill_rate: f64,
    pub time_to_first_auction_ms: Option<f64>, pub time_to_first_ad_ms: Option<f64>,
}

pub struct PageView {
    pub index: u32,
    pub url: String,
    pub title: Option<String>,
    pub started_at_ms: f64,        // since the scan's navigation start
    pub dwell_ms: f64,
    pub reached_by: PageReach,
    pub link: Option<FollowedLink>,
    pub load_ms: Option<f64>,      // None for route changes
    pub max_scroll_depth: f64,
    pub metrics: PageMetrics,
}

#[derive(Copy, Eq, Hash)]
pub enum DetectorKind { InvalidTrafficMeasurement, BotProtection }
pub struct AutomationDetector { pub vendor: String, pub kind: DetectorKind, pub host: String }

pub struct ScanEnvironment {
    pub visitor: VisitorBehavior,
    pub seed: u32,
    pub headless: bool,
    pub persistent_profile: bool,
    /// navigator.webdriver as the page saw it; None when it could not be read.
    pub webdriver: Option<bool>,
    pub user_agent_overridden: bool,
    pub viewport_overridden: bool,
    pub emulated_mobile: bool,
    pub detectors: Vec<AutomationDetector>,
}

pub struct ScanProfileInfo { pub exists: bool, pub size_bytes: f64, pub last_used_at: Option<f64>, pub in_use: bool }
```

Additive `#[serde(default)]` fields on existing types:

| Type | New field |
|---|---|
| `ScanDetail` | `pages: Vec<PageView>` (empty means one implicit page), `environment: Option<ScanEnvironment>` |
| `Auction`, `AdSlot` | `page: u32` |
| `AdSlot` | `requests: Vec<SlotRequest>` where `SlotRequest { at_ms: f64, had_prebid_targeting: bool, prebid_ad_id: Option<String> }` |
| `ScriptRecord` | `pages: Vec<u32>` (`loaded_at_ms` is relative to the first of them) |
| `Finding` | `pages: Vec<u32>` (empty means not page-specific) |
| `DiagnosticCheck` | `pages: Vec<u32>` |
| `LogKind` | new variant `Visitor` |

`ScanMetrics` doc changes only: counts are sums over page views; `page_load_ms`, `time_to_first_auction_ms` and `time_to_first_ad_ms` are the entry page's.

### `capture.rs` (in memory only)

```rust
pub struct CaptureRequest {
    pub url: String, pub device: DeviceProfile, pub observe_seconds: u32,
    pub visitor: VisitorBehavior,          // replaces auto_scroll
    pub extra_pages: u32, pub seed: u32,
    pub accept_consent: bool, pub headless: bool, pub persistent_profile: bool,
    pub chrome_path: Option<String>,
}
pub struct CapturedPage {
    pub url: String, pub title: Option<String>, pub started_at_ms: f64, pub ended_at_ms: f64,
    pub reached_by: PageReach, pub link: Option<FollowedLink>,
    pub load_ms: Option<f64>, pub max_scroll_depth: f64,
}
pub struct CaptureEnvironment {
    pub headless: bool, pub persistent_profile: bool, pub webdriver: Option<bool>,
    pub user_agent_overridden: bool, pub viewport_overridden: bool,
    /// Third-party hostnames the pages requested (names only, at most 500).
    pub request_hosts: Vec<String>,
}
pub struct CaptureActivity {            // replaces the String progress line
    pub kind: ActivityKind,            // Progress | PageView | Action | LinkChoice | Blocked | Environment
    pub message: String, pub page: Option<u32>, pub data: Option<serde_json::Value>,
}
```

- `Capture` gains `pages: Vec<CapturedPage>` and `environment: CaptureEnvironment`.
- `TrackerEvent`, `ConsoleLine` and `ObservedBidResponse` gain `#[serde(default)] page: u32`.
- `CapturedScript` gains `pages: Vec<u32>`.
- New `TrackerEventKind` variants:
  - `RouteChange { url, navigation_type }`
  - `SlotsDestroyed { element_ids }`
  - `SlotDefinitionRejected { element_id }`
- `SlotRequested` gains `#[serde(default)] prebid_ad_id: Option<String>`.
- `PageLifecycle.phase` gains `"firstPointerMove"`.

There are 17 `Capture { … }` and 6 `ScanOptions { … }` literals in tests. Add `Capture::single_page(…)` and `ScanOptions::for_url(…)` test builders in the contract task so they compile once.

## 2. Browser crate design

### 2.1 Module layout (`crates/browser/src/capture/`)

| File | Role |
|---|---|
| `mod.rs` | `run`: launch, connect, hand over to the driver, collect. `collect` must also work on a dead connection. |
| `tabs.rs` (new) | Browser-level target tracking, the active tab, adoption and closing of new tabs. |
| `page.rs` | Session plumbing and evaluate. `prepare()` becomes re-runnable per tab. Input code moves out. |
| `input.rs` (new) | The only place that calls the `Input` domain: `move_pointer`, `wheel`, `swipe`, `click`, `tap`. No keyboard function exists. |
| `pages.rs` (new, pure) | `PageLog`: page views, binding of time origins to pages, replace-or-open rule, `index_at(epoch_ms)`. |
| `tracker_log.rs` | Stops clearing; assigns `page` and rebases `t`. |
| `visitor/{mod,rng,motion,plan,links,site}.rs` (new, pure) | Seeded planner, motion curves, link policy, same-site test. |
| `driver.rs` (new) | State machine for `none`, `scroll`, `browse`. `scenario.rs` stays as the `scroll` planner, untouched. |
| `manual.rs` (new) | Manual-session loop. |
| `environment.rs` (new) | Reads automation indicators; collects request hostnames. |
| `injected/links.js`, `injected/hit.js` (new) | Isolated-world link facts and pre-click hit test. |

`cdp/profile.rs` gains `PersistentProfile`. `cdp/chrome.rs` gets `LaunchOptions { profile: ProfileChoice, headless, window_size }`. Add `psl = "2"` to `crates/browser/Cargo.toml`.

Public API (`lib.rs`):

```rust
pub struct CaptureHooks {
    pub cancel: CancellationToken,
    pub finish: CancellationToken,                        // manual: stop and keep the data
    pub on_activity: Arc<dyn Fn(CaptureActivity) + Send + Sync>,
    pub scan_profile_dir: Option<PathBuf>,                // required when persistent_profile is set
}
pub fn scan_profile_info(dir: &Path) -> ScanProfileInfo;
pub fn clear_scan_profile(dir: &Path) -> Result<(), BrowserError>;   // new variant BrowserError::ProfileBusy
```

### 2.2 Page views in the recorder

`PageLog` rules:

- **Hard navigation.** A main-frame `Page.frameNavigated` with a new loader id creates a page view, with a provisional start at the document request's wall time. The first tracker batch whose `o` (timeOrigin) is unknown binds to it and refines the start.
- **Replace instead of open.** The new document replaces the current page view (its events are dropped, as today) when the driver is still in `Opening` for that page, or within 3 s of the scanner's own consent click (CMP reload). Everything else opens a new page view, with the reach the driver announced beforehand (`LinkClick`), else `Redirect` or `Reload`, or `UserNavigation` in manual mode.
- **Route change.** A `routeChange` tracker event opens a page view at that event's document time, but only for push or traverse with a different path or query; hash-only changes and same-URL replaces are ignored. The tracker emits it from the Navigation API (`navigation.addEventListener('currententrychange')`), so `history` is not patched. `Page.navigatedWithinDocument` remains the Rust-side URL source.
- **Page of an event.** Decided by timestamp, not arrival: the page view of that document whose interval contains the event's `t`. The tracker back-dates some events (`slotDefined`), and this keeps them on the right page.
- **Network-derived records.** Scripts, console lines and bid responses get `page = index_at(epoch_ms)` at collect time. A duplicate script admission appends the page to the existing entry.
- **Off-site documents.** While the main-frame URL is not same-site, tracker batches, scripts and console lines are dropped. The tracker gets a one-line prelude with the site's registrable domain and returns immediately elsewhere; Rust remains the authority.
- **Caps.** 8 page views in automated modes, 25 in manual. 50,000 events per capture and 20,000 per page. The existing 60 MiB script-body budget is unchanged. Past the page cap, events stay on the last page and one `Blocked` activity is logged.

Titles are read at load and at route change, not at the end, so a closed window loses nothing.

### 2.3 Tabs

`tabs.rs` enables `Target.setDiscoverTargets` and browser-level `Target.setAutoAttach { autoAttach, waitForDebuggerOnStart: true, flatten: true }`. A new page target is paused, `prepare()` runs on it (domains, emulation, tracker), then it is resumed. `Page` gets an active session that can switch; in-flight body fetches keep their own session.

- **Browse.** A tab opened within 3 s of our click, with a same-site URL, becomes active and the old tab is closed. Any other popup is closed and logged.
- **Manual.** Recording follows the newest same-site tab; when it closes, recording returns to the most recent open tab; when none is left, the session finishes.

Unverified: that browser-level auto-attach pauses `_blank` and `window.open` popups before their first script. A fixture test (2.9) settles it. The fallback is to close the popup and `Page.navigate` the main tab with a `referrer`.

### 2.4 Scenario engine (driver state machine)

```
Launch -> Open(entry) -> Settle -> [Consent] -> Engage -> PickLink -> Approach -> Verify -> Click
                           ^                       |          |                               |
                           |                       |          +-- no candidate ---------------+--> Finish
                           +---- AwaitNavigation <-+------------------------------------------+
Finish -> flush -> collect
```

- **`none`.** Open, Settle, observe for `observe_seconds`, Finish. No input.
- **`scroll`.** Today's `observe()` loop and `scroll_plan`, unchanged.
- **`browse`.** The full machine, per page:
  - *Settle.* Wait for load or `LOAD_TIMEOUT`, then no input for max(2 s, 15% of dwell). The existing gate rule needs to see the page idle first.
  - *Consent.* Today's attempt schedule on every page view, so re-appearing banners are handled and logged. A second banner after an accepted one is itself rule R6.
  - *Engage.* Ask `plan::next_action` until the planned time reaches the dwell budget. Dwell per page is min(`observe_seconds`, 300 s / (1 + extra_pages)).
  - *PickLink.* Only while pages followed < `extra_pages` and the deadline allows.
  - *AwaitNavigation.* Up to 4 s for the first signal, then Settle. Outcomes:
    - new main-frame document: next page;
    - route change: next page, settle 500 ms;
    - new tab: adopt it;
    - main-frame document request or redirect hop to another site: `Page.stopLoading`, log `Blocked`, stop following and keep observing the current page;
    - `Page.downloadWillBegin`, or nothing: mark the link dead and try the next. At most 3 attempts per page.
- **Deadline.** 8 minutes wall clock for automated modes. Every wait goes through `page.record_until*` or `page.call`, inside the existing `select!` on `hooks.cancel`, so cancellation is observed at every await.

Planner (pure): `next_action(&mut Rng, &PageState) -> Action`.

- `Action` is one of `Idle{ms}`, `MovePointer{to, ms}`, `Hover{ms}`, `Wheel{ticks: Vec<(dy, gap_ms)>}`, `ScrollBack{dy}`, `Swipe{distance, ms, drift_x}`.
- `PageState` is scroll position, viewport, pointer position and planned elapsed time.
- `Rng` is a small SplitMix64 in `visitor/rng.rs`, with no new dependency.
- The stop criterion is planned time, not wall-clock time. The action sequence is therefore a function of seed, page geometry and link set. CDP latency shifts real timing but not the sequence.

### 2.5 Input synthesis (CDP `Input` domain only)

- **Pointer.** `Input.dispatchMouseEvent { type: "mouseMoved", x, y, button: "none", buttons: 0 }` every 16 ms ± 4 ms.
  - Path: cubic Bezier with two control points offset up to 20% of the distance, perpendicular to it.
  - Speed: minimum-jerk profile, s = 10τ³ − 15τ⁴ + 6τ⁵.
  - Duration: 250 + 120·log2(1 + D/W) ms ± 20%.
  - Extras: ±1 px jitter; for D > 400 px, a 20% chance of an 8-20 px overshoot and correction.
  - Targets: paragraphs, headings and images in view (rects from one isolated-world evaluate), never inside an ad rect. Hover 150-900 ms.
- **Wheel.** `mouseWheel` at the pointer position. One device profile per scan, chosen from the seed:
  - wheel mouse: 3-9 ticks of 100 px, 30-90 ms apart;
  - trackpad: 12-40 events of 4-60 px, decaying, 16 ms apart.
  - Pauses between gestures are log-normal (median 1.2 s, cap 6 s); 12% of gestures scroll back 0.2-0.6 viewports.
- **Touch (mobile).** `Input.dispatchTouchEvent` start, move, end as today, with a length of 0.25-0.6 viewport, duration 180-450 ms ease-out and x drift ±12 px. The finger still rests before lifting, so no fling of unknown length. A link tap is touchStart then touchEnd after 60-120 ms.
- **Click.** Move to the point, `mousePressed { button: "left", buttons: 1, clickCount: 1 }`, wait 50-130 ms, `mouseReleased`.
- **Focus.** `Emulation.setFocusEmulationEnabled` stays; headed runs call `Page.bringToFront` once.
- **Dialogs.** `alert` and `beforeunload` are accepted; `confirm` and `prompt` are dismissed.

CDP input events are trusted (`isTrusted === true`); the demo instrumentation asserts it.

### 2.6 Link discovery and policy

Facts come from the page; decisions are made in Rust.

**`injected/links.js`** runs in the existing isolated world "adscan". Rust passes in the slot element ids and ad unit codes it knows from tracker events. It scans up to 1,500 `a[href]` of the top document only, so iframe content is never a candidate, and returns up to 60 entries: resolved `href`, raw attribute, text (80 chars), `rel`, `target`, `download`, rect, `visible`, `region`, `inAd`, `inForm`, `hasImage`.

- `visible`: `checkVisibility()`, size at least 8x8, opacity above 0.1, pointer-events not none.
- `region`: `content` (main, article, role=main), `nav`, `aside`, `footer` or `other`.
- `inAd` is true when any of these holds:
  - an ancestor is a known slot container;
  - an ancestor matches an ad selector (`[id^="google_ads_iframe"]`, `[id^="div-gpt-ad"]`, `[data-google-query-id]`, `ins.adsbygoogle`, `[data-ad-slot]`, `[data-ad-unit]`, taboola and outbrain containers);
  - an ancestor has an id or class token that is exactly `ad`, `ads`, `advert`, `advertisement`, `adslot`, `sponsored`, `promoted` or `native-ad`, after splitting on `-`, `_` and spaces, so "header" and "shadow" do not match;
  - the anchor contains an iframe, or its rect overlaps an iframe or slot rect by 30% or more.

**`visitor/links.rs::classify(candidate, current_url, site, visited) -> Verdict`** applies the first matching skip, in this order (table-tested):

| # | Skip reason | Condition |
|---|---|---|
| 1 | `hidden` | not visible |
| 2 | `inside-ad` | `inAd` |
| 3 | `scheme` | not http(s): mailto, tel, javascript, data, blob |
| 4 | `anchor` | same URL ignoring the fragment |
| 5 | `off-site` | `visitor/site.rs`: different `psl::domain_str`; for IPs, localhost and suffix-less hosts, different host or port; or an https to http downgrade |
| 6 | `file` | `download` attribute, or an extension such as pdf, zip, dmg, exe, apk, docx, xlsx, csv, mp3, mp4, jpg, png, svg, xml, rss, json, ics |
| 7 | `rel` | nofollow, sponsored or ugc |
| 8 | `sensitive` | a path segment, query key or link-text token from the sensitive list: login, signin, signup, register, account, profile, cart, basket, checkout, payment, buy, order, subscribe, unsubscribe, logout, delete, remove, cancel, admin, auth, password, donate, download, print, share, vote, report, plus de, fr, es, it and nl equivalents; or query keys token, key, session, sid, nonce, action |
| 9 | `control` | inside a form, or `role=button` |
| 10 | `visited` | already visited in this scan |

Followable links are scored:

- region: content +3, nav +2, aside +1, footer −2;
- article-like path (depth of 2 or more, a slug of 3 or more words, or a year segment) +2;
- text of 25-120 chars +1;
- contains an image +1;
- within two viewports of the current scroll +1;
- `target=_blank` −1;
- has a query string −1;
- boilerplate word (privacy, terms, imprint, contact, about, advertise, careers) −5.

The choice is a seeded weighted pick among the top 5. One `LinkChoice` activity carries the chosen link, its score and reasons, the other four with scores, and skip counts per reason. URLs are logged as origin + path only.

**Before the click (Verify).** `hit.js` checks that `document.elementFromPoint(x, y)` is inside the chosen anchor and returns its current `href`. A point over an iframe or overlay fails. Rust runs `classify` again on that href.

**After the click.** Navigation-level enforcement as in 2.4. It is the real guarantee, because sites can rewrite `href` on mousedown.

### 2.7 Manual mode

- Always headed; no synthetic input and no automatic consent click.
- Loop: `select!` over cancel (discard), `finish` (collect), all page targets gone or `CdpError::Closed` (collect from memory, skipping flush and settle), the 15-minute limit or the page cap (collect, with a caveat activity), and a 1 s tick.
- Every committed same-site main-frame document or route change is a page view (`UserNavigation`, `Reload`, `RouteChange`). Off-site pages are not recorded and are shown as "Left the scanned site (host): not recorded".
- `observe_seconds` does not apply; the composer hides it. After Finish there is the existing 3 s body-settle.
- On macOS, closing the last window leaves Chrome running, so "all page targets destroyed" is the signal, not process exit.

### 2.8 Persistent scan profile

- **Location.** `<data_dir>/scan-profile/{desktop,mobile}/`. Separate directories per device, so a phone UA and a desktop UA never share one cookie jar. The path is built by src-tauri and never taken from the user or the frontend.
- **Lock.** `PersistentProfile::open(dir)` takes the same advisory lock file as `Profile` (`adscan-owner.lock`) with `try_lock`; failure is `LaunchError::ProfileBusy`. The engine also serialises persistent-profile scans with a one-permit semaphore, so the second one queues.
- **Before launch.** Kill stragglers with `kill_helpers(dir)`, then delete Chrome's `SingletonLock`, `SingletonSocket` and `SingletonCookie`. Otherwise a crashed run makes the next Chrome hand off to a dead process and exit.
- **Shutdown.** Send `Browser.close` and wait up to 5 s so cookies are flushed, then use the existing kill path. The directory is never removed: `ProfileChoice::Persistent` skips `remove_dir_all` in `shutdown`, in `Drop` and in the watchdog (`Watchdog::guard` takes an `Option<&Path>` to delete).
- **Extra flags.** `--disk-cache-size=104857600`. `--use-mock-keychain` stays, so macOS never prompts.
- **Clear.** `clear_scan_profile` takes the lock; if busy it returns `ProfileBusy`, otherwise it removes the directory.

### 2.9 Fidelity and detection

- **Headed desktop in `browse` and `manual`.** Skip `Emulation.setDeviceMetricsOverride`, send no UA override (already the case), and read the real `innerWidth`, `innerHeight` and `devicePixelRatio` into `Capture.viewport`. `none` and `scroll` keep today's behaviour. Mobile stays emulated and is reported as such.
- **Probe.** `environment.rs` evaluates `navigator.webdriver` in the isolated world after the first load (same host object, not tamperable by the page). It fills `CaptureEnvironment` and emits one `Environment` activity, for example "This visit is identifiable as automated: navigator.webdriver is true, headless".
- **Nothing is injected to change any indicator.** No fingerprint, proxy or stealth code is added.
- **Flags.** No other flag in `launch_arguments` is page-visible. `--enable-automation` is not passed, which is correct.

## 3. Engine changes, file by file

- **`domain/scan.rs`**
  - Resolve `visitor` (from `auto_scroll` when absent) and write the resolved values back into the stored options, including `auto_scroll = visitor != None`, the seed (from the scan id's first 4 bytes when absent) and headless.
  - `extra_pages` must be 0-5 and is forced to 0 unless browse. Manual forces `headless = false` and `accept_consent = false`.
  - Build the new `CaptureRequest`.
- **`domain/telemetry.rs`**
  - One pass over all events with absolute time. Auctions take the page of their `auctionInit`; an SPA auction may end on the next page.
  - Slots are keyed by `(page, element_id)`. Every `SlotRequested` appends to `requests`.
  - Convert to page-relative, then build `ScanTelemetry { pages: Vec<PageView>, by_page: Vec<Telemetry>, auctions, slots, metrics }`. `PageMetrics` comes from `by_page[i]`; the aggregate metrics are sums plus the entry-page timings.
- **`domain/environment.rs` (new).** Table mapping hosts to (vendor, kind) and `detect(capture) -> ScanEnvironment`. Starting entries from memory, each to be checked before shipping: HUMAN/PerimeterX, DoubleVerify, IAS (`adsafeprotected.com`), Pixalate (`adrta.com`), DataDome, Cloudflare challenge, reCAPTCHA, hCaptcha, Arkose, Anura, CHEQ, Imperva.
- **`domain/heuristics/mod.rs`**
  - `Rule::scope() -> RuleScope { Page, Scan }`, defaulting to `Scan`.
  - `RuleContext` gains `events: &[TrackerEvent]` (the page's), `page: Option<&PageView>`, `scan: &ScanTelemetry` and `environment`. Rules read `ctx.events`, not `ctx.capture.events` (mechanical change).
  - `evaluate` runs page rules once per page view and merges drafts by `(rule id, sorted affected_bidders)`. The body comes from the most severe page (ties: earliest). Slots are unioned, `pages` is filled, and a "Per page view" timeline evidence line is added when there is more than one.
  - `FindingDraft` gains `pages`.
- **Rule scopes**
  - Page scope: `lazy`, `timing`, and `delivery` except `BiddersNeverBid`.
  - Scan scope: `BiddersNeverBid`, `bidders::*`, `setup::*`.
  - `AuctionGatedOnInteraction`: skips `RouteChange` pages; suppresses `NeverStarted` on later pages where R1 applies; treats `firstPointerMove` as an interaction.
  - `AdLibraryLoadedTwice`: compares only scripts that share a page, so per-page cache-busters do not count.
- **`domain/heuristics/visit.rs` (new).** Rules R1-R6 (section 6).
- **`domain/diagnostics/*`** (another agent is editing these).
  - Consent timing must use absolute time through a new `Capture::absolute_ms(&TrackerEvent)`.
  - Counts sum over pages; fill `DiagnosticCheck.pages`.
- **`domain/synthesis.rs`.** `absorb` unions `pages`. The template overview adds one sentence on pages visited and one environment caveat when R6 fired.
- **`application/ports.rs`**
  - `ProgressFn = Arc<dyn Fn(CaptureActivity)>`.
  - `CaptureSource::capture(request, control: CaptureControl { cancel, finish }, on_activity)`, plus `scan_profile_info()` and `clear_scan_profile()`.
  - `ScanRepository::save_telemetry` gains `pages`; new `save_environment`.
- **`application/pipeline/nodes/capture.rs`.** Maps activities to `run.log` (`LogKind::Visitor` for PageView, Action, LinkChoice, Blocked and Environment; `Message` for the rest), passing `data` through. The start line names visitor, pages, seed, headed or headless, and profile.
- **`nodes/telemetry.rs`, `nodes/heuristics.rs`, `pipeline/state.rs`.** Carry `ScanTelemetry` and the environment.
- **`application/scans.rs`.** `ActiveScan.finish`; `finish_capture(scan_id)`, which is a no-op unless the scan is in Capture; the persistent-profile semaphore.
- **`nodes/analyze/{prompt,render,toolbox}.rs`**
  - The system prompt says "one visit of one or more page views" (a one-time cache-prefix change).
  - The opening message gains a visit block (visitor, environment signals, page table capped at 8 rows) and timelines for the entry page plus at most two pages that have findings.
  - `get_telemetry` gains a `pages` section and an optional `page` argument. `report_finding` gains optional `pages`.
- **`application/export.rs`.** Markdown gains a "Visit" block and a "Page views" table; with more than one page, findings carry "Seen on" and Auctions and Slots get per-page subheadings. With one page the body is unchanged. JSON follows the types.
- **`infrastructure/sqlite/mod.rs`.** Migration 4:
  ```sql
  ALTER TABLE scans ADD COLUMN pages TEXT;
  ALTER TABLE scans ADD COLUMN environment TEXT;
  UPDATE settings SET data = json_set(data, '$.defaultVisitor', 'scroll',
      '$.defaultExtraPages', 2, '$.persistentProfile', json('false'));
  ```
  `sqlite/scans.rs` reads and writes both columns; NULL loads as `[]` and `None`.
- **`src-tauri`**
  - `adapters.rs`: `ChromeCaptureSource { scan_profile_dir }`.
  - `lib.rs` and `state.rs`: build it from `config.data_dir`.
  - `commands.rs` and the handler list: `finish_capture { scanId }`, `get_scan_profile`, `clear_scan_profile`.
  - Tests: `tests/commands.rs`, `tests/real_browser.rs`, `e2e-tauri/contract.spec.ts`.

**Budget and cost.**

- Script cost does not grow with pages that share bundles (dedupe by url + hash; per-page inline config blocks are small).
- Telemetry in the opening message is capped as above. Estimate about 1,500 input tokens per extra page; this is uncalibrated and belongs in a named constant.
- The per-scan cap is unchanged and still enforced by `Budget`.

## 4. Frontend changes, file by file

- **`src/shared/domain/scan-options.ts`.** `VISITOR_META` (label and description per behaviour), `EXTRA_PAGES { min: 0, max: 5 }`; `defaultScanOptions` uses the settings defaults; `repeatVisitOptions(detail)` keeps the seed.
- **`src/features/scan-composer/scan-form.ts`, `ScanComposer.tsx`**
  - A four-way "Visitor" toggle replaces the Auto-scroll switch: Observe only, Scroll, Browse like a visitor, I'll browse myself.
  - Browse shows a "Pages to visit" stepper (0-5), a "Show the browser" switch (on by default, with the hint "Headless Chrome is easier for ad partners to recognise as automated") and the persistent-profile state with a link to Settings.
  - Manual hides Observe and Accept-consent and shows "Chrome opens; browse the site, then press Finish capture (up to 15 minutes)".
- **`cost-estimate.ts`.** Adds the per-page token increment.
- **`src/features/live-scan/`**
  - `visit.ts` (new, pure): derives `{ pages, currentUrl, actions }` from log entries of kind `visitor`.
  - `VisitPanel.tsx` (new), in the left column during Capture: current page, pages visited with reach icons, the last few actions ("Scrolled to 60%", "Followed link: /sport/…", "Skipped 14 links: 9 off-site, 3 inside ads, 2 sign-in").
  - `ManualSessionBar.tsx` (new): elapsed and remaining time, page count, **Finish capture** (primary) and Cancel (existing dialog).
  - `LiveScanView.tsx` composes them.
- **`src/lib/api/port.ts`, `tauri-adapter.ts`, `src/lib/query/hooks.ts`.** `finishCapture`, `getScanProfile`, `clearScanProfile`.
- **`src/shared/domain/log-entry.ts`, `shared/components/log-entry-row.tsx`, `features/audit-trail/log-filters.ts`.** The `visitor` kind: icon, filter, and a link-choice detail showing candidates and reasons.
- **Report (`src/features/report/`)**
  - `pages/PageSwitcher.tsx` (new): a segmented control "All pages | 1 /article | 2 /sport/…", rendered only when `detail.pages.length > 1`. The selection lives in the route search (`page`), in `src/app/router.tsx`.
  - `ScanRoute.tsx` filters auctions and slots by page before passing them to the tabs. With one page nothing renders differently.
  - `auctions/AuctionsTab.tsx`, `slots/SlotsTab.tsx`: under "All pages", one group header per page view (title, reach, start offset).
  - `slots/slot-timeline.ts`: extra request marks from `slot.requests`.
  - `findings/FindingCard.tsx`: page chips when the scan has more than one page; `finding-filters.ts`: page filter; `finding-markdown.ts`: "Seen on".
  - `scripts/ScriptsTable.tsx`: a "Pages" column when there is more than one page.
  - `ReportOverview.tsx`: a visit line (behaviour, pages, headed or headless, profile).
  - `EnvironmentNotice.tsx` (new): an amber notice, shown only when R6 fired or `webdriver === true`.
  - `key-metrics.ts`: hints read "entry page" when there are several pages.
  - `ReportHeader.tsx`: a "Repeat this visit" menu item.
- **`src/features/settings/SettingsScreen.tsx`, `settings-schema.ts`.** "Scan defaults" gains visitor and pages. "Browser" gains "Keep a scan profile between scans" with an explanation, the profile size and a "Clear scan profile" button (disabled while in use).
- **Fake backend (`src/lib/fake-backend/`)**
  - `scenario.ts`, `site.ts`: multi-page results for browse and manual, visitor log beats.
  - `runtime.ts`, `index.ts`: `finishCapture` ends a manual capture beat; profile info and clear.
  - `seed.ts`: one seeded multi-page scan and one legacy scan without `pages`.
  - `export.ts`.
- **`src/test/fixtures.ts`.** Multi-page and legacy fixtures.

## 5. Demo site (`crates/browser/demo-site`, `src/demo.rs`)

**Pages**

- `/climate/village-substation` and `/climate/kelp-carbon`: full article pages, linked from "Most read" (plain anchors, hard navigation).
- The four story cards become `a[data-route]`: `app.9b3e7f.js` fetches `/fragments/<slug>.html`, swaps `<main>` and calls `pushState`. The same URLs also load directly.

**Trap links** (must never be requested)

- `/account/login`, `/logout`, `/files/media-kit.pdf`;
- `mailto:` and `https://partner.example/`;
- a `rel="sponsored"` link;
- a native card `div.ad-slot--native a[href="/sponsored/offer"]`;
- a link inside the GPT stand-in creative iframe;
- a `target=_blank` same-site link, which is allowed.

**Server and instrumentation**

- `demo.rs` counts hits per path and serves `GET /__demo/hits`.
- `analytics.js` logs `ENGAGEMENT {pointerMoves, wheels, touches, trusted}` to the console after 3 s without input and on pagehide.

**Stand-ins**

- `gpt.js`: `destroySlots`; a duplicate div is rejected (returns null and logs the "already associated with another slot" console error); refresh re-emits slot events; `impressionViewable` after 1 s in view.
- `prebid.js`: a bid can be used once, and a second render ends in `adRenderFailed`; `removeAdUnit`.

**Wrapper (`hb-wrapper.src.js`, regenerate the min file)**

- *Engagement-only, correct:* `ad-recirc` is defined and auctioned only from the second page view of a session (sessionStorage counter).
- *(h) Engagement refresh with stale bids.* After `ad-sidebar-1` has been viewable for 5 s with pointer or touch activity in the last 5 s, the wrapper calls `pubads().refresh([slot])` without `requestBids`. Ground truth: a second `slotRequested` with the same `hb_adid`, no `auctionInit` in between, an empty render and `adRenderFailed`. Visible only with `browse` or `manual`.
- *(i) Later page view never bids.* A "double-fire guard" stores the last auction time in sessionStorage and skips `requestBids` for 120 s, measured across navigations. Ground truth on a hard-navigated second page: the five standard slots are requested with no `hb_*` keys and render empty; only the `ad-recirc` auction runs.
- *(j) SPA navigation does not reset slots.* On route change the wrapper calls `addAdUnits` and `defineSlot` again without `destroySlots` or `removeAdUnit`. Ground truth: five `slotDefinitionRejected`, an auction whose bids are never used, and no `slotRequested` on that page view.

`README.md` gets the new pages, the trap list and ground truth for (h), (i), (j) per visitor mode.

## 6. New deterministic rules

| Id | Trigger | Severity, confidence | False-positive guard |
|---|---|---|---|
| R1 `later-page-view-without-bids` | On a hard-navigated page k ≥ 1, at least 2 slots are requested with no covering auction on that page and no `hb_*` keys, while slots with the same element ids were covered by an auction on an earlier page. | High, 0.8 | Same element ids required; skipped while a consent UI is undecided on page k; needs dwell of at least 5 s. |
| R2 `spa-slots-not-reset` | A `RouteChange` page view has at least one `SlotDefinitionRejected`, or none of the document's live slots is requested, destroyed or redefined within the page's dwell (5 s or more) while an auction ran on it. | High, 0.85 (rejected) or Medium, 0.6 | The route must change path; the second form needs at least 2 live slots. |
| R3 `refresh-with-stale-bids` | Two consecutive `requests` of one slot on one page carry the same non-empty `prebid_ad_id`, with no covering auction ending between them. | High, 0.9 | Exact id equality. |
| R4 `refresh-without-auction` | A second or later request of a slot has no `hb_*` keys and no covering auction since the previous request, while its first request had Prebid targeting. | Medium, 0.7 | Excludes slots whose first request lacked targeting, and requests within 2 s of a route change. |
| R5 `consent-asked-again` | `TcfState { cmpuishown }` on page k ≥ 1 after `useractioncomplete` (or the scanner's accepted banner) on an earlier page. | Medium, 0.75; High when an auction on page k started more than 2 s late or not at all | Session-internal ordering only; independent of the profile. |
| R6 `scan-identifiable-as-automated` | The environment shows `webdriver == Some(true)` or headless, and either a detector vendor is present or at least one bidder was asked 2 or more times and never bid. | Info, 0.5, category Configuration | Informational; also appends a caveat evidence line to `BiddersNeverBid`. |

R6 text: "Bidders may be filtering this scan as automated traffic: navigator.webdriver was true, the browser ran headless, DoubleVerify is on the page. Repeat as 'I'll browse myself' or with the browser shown before concluding a bidder is broken."

Considered and left out: a "refresh faster than 30 s" rule. It is deterministic, but it is a policy judgement and would also fire on the demo's correct contrast case.

## 7. Risks and open questions

1. **Policy conflict already in the code (needs the product owner).** `--disable-blink-features=AutomationControlled`, the headless UA and client-hint rewrite, and the headless `--blink-settings` hover override exist to hide automation. The first is the canonical stealth flag. Options:
   - (a) remove all three in every mode and rely on R6;
   - (b) remove them for `none`, `scroll` and `browse`, and keep only the blink-feature flag for `manual`, where a person really drives;
   - (c) keep the status quo.

   I recommend (a) or (b). In every case the probe reports the true value, and `tests/emulation.rs` must be updated to match. Until this is decided the browser implementer leaves the flags alone.
2. **`navigator.webdriver` under the pipe transport is unconfirmed.** Measure first. If the pipe sets it, even headed manual sessions are flagged, which makes decision 1 matter more. Choosing `--remote-debugging-port` to avoid the indicator would itself be evasion and opens a local port; not proposed.
3. **CDP is detectable** (`Runtime.enable`, `Debugger.enable`) whatever the flags. Manual mode is a human visit with a detectable debugger; the UI must not promise more.
4. **Browser-level auto-attach pausing popups** is unverified (2.3); fallback given.
5. **Navigation API `currententrychange`** exists in current Chrome; an old Chromium picked through "choose browser" might lack it. The fallback is `Page.navigatedWithinDocument` with receipt-time attribution.
6. **bfcache.** A restored document keeps its old time origin; its events go to the original page view. Documented, not solved.
7. **Per-page evaluation changes existing findings** only by adding `pages: [0]`. The merge key (rule, bidders) is a judgement call; check it against rules that emit several drafts.
8. **Diagnostics and chrome.rs are being edited by other agents.** Keep changes there narrow (`LaunchOptions`, `ProfileChoice`, `absolute_ms` call sites) and rebase last.
9. **Persistent profile.** A SIGKILL can lose recent cookies (hence `Browser.close`). Profile growth is bounded only by the cache flag. Consent stored in the profile means later scans never see the banner, which is intended but must be said in Settings.
10. **Timing-dependent demo truth.** Bug (i) needs the second page within 120 s; tests keep `observe_seconds` small.
11. **The link policy is English-centric** with small de, fr, es, it and nl lists. It errs towards skipping.
12. **Headed scans steal focus** on the user's desktop; `Page.bringToFront` is called once only.
13. **Open:** should `none` and `scroll` also stop overriding the viewport when headed? The plan says no, to keep them identical to today.

## 8. Ordered tasks for three implementers

**T0, contract (implementer B, alone, first).** `crates/model/src/{lib,capture}.rs` as in section 1, the test builders, regenerate `src/bindings`, fix literals just enough to compile (`visitor: None`, `page: 0`, `pages: vec![]`). Publish the type list. Everything below starts after T0.

**Implementer A: browser crate and demo site** (`crates/browser/**`)
1. Measure `navigator.webdriver` (item 7.2); report.
2. `visitor/{rng,motion,plan,links,site}.rs` with unit tests (pure).
3. `pages.rs`, `tracker_log.rs`, recorder page assignment, script page lists; `tracker.js` additions (routeChange, slot destroy and reject, `prebidAdId`, firstPointerMove, site prelude).
4. `input.rs`, `driver.rs` (none, scroll, browse), dialog fix, off-site enforcement.
5. `tabs.rs`; `manual.rs`; `finish` hook; collect on a dead connection.
6. `PersistentProfile`, `ProfileChoice`, watchdog option, graceful close, `scan_profile_info`, `clear_scan_profile`.
7. `environment.rs`; headed-desktop no-override path.
8. Demo site pages, traps, stand-in changes, wrapper bugs, README.
9. Integration tests (below).

**Implementer B: model, engine, src-tauri** (`crates/model`, `crates/engine/**`, `src-tauri/**`, `e2e-tauri/**`)
1. T0.
2. `domain/scan.rs`, migration 4, repository columns, ports, capture node, `finish_capture`, profile semaphore; src-tauri commands and adapter.
3. `telemetry.rs` multi-page derivation; `environment.rs`.
4. Heuristics scope and merge; `visit.rs` R1-R6; adjustments to existing rules.
5. Diagnostics absolute-time fix (coordinate with the diagnostics agent).
6. Analyze prompt, render, toolbox; export; synthesis.
7. Engine and src-tauri tests; `e2e-tauri` contract and demo-scan specs.

**Implementer C: frontend and fake backend** (`src/**`, `e2e/**`). After T0, works against bindings and the fake backend only.
1. Options and vocabulary; composer; settings.
2. API port methods; fake-backend multi-page scenario, manual finish, profile info.
3. Live view: `visit.ts`, `VisitPanel`, `ManualSessionBar`.
4. Report: `PageSwitcher`, per-page grouping, finding chips, `EnvironmentNotice`, repeat visit.
5. Vitest and Playwright.

**File overlap.** None between A and C. A and B meet only at the model (frozen after T0) and `src-tauri/src/adapters.rs` (B owns it and calls A's three public functions). `tracker.js` belongs to A.

### Test plan

- **Unit, browser**
  - `links.rs`: one table row per skip reason plus scoring order.
  - `site.rs`: `co.uk`, subdomains, IP and port, loopback, https to http.
  - `plan.rs`: the same seed and geometry give an identical action list; different seeds differ; the planned-time budget is respected.
  - `motion.rs`: path endpoints, bounds, monotonic time.
  - `pages.rs`: origin binding, replace versus open, route change, timestamp attribution of back-dated events, caps.
  - A source check that `Input.dispatchKeyEvent` and `Input.insertText` appear nowhere in `src/`.
- **Unit, engine**
  - Multi-page telemetry: hard navigation, route change, an auction spanning a route change, slot keying, `requests`.
  - Legacy capture equals today's output.
  - Finding merge across pages.
  - R1-R6, each with firing and near-miss cases.
  - Environment detection.
  - `ValidatedScan` resolution.
- **Browser integration** (headless Chromium; fixtures with a forced choice)
  - Fixtures `browse-hard.html`, `browse-spa.html`, `browse-newtab.html`, `browse-offsite-redirect.html`, `browse-traps.html`.
  - Demo site: browse with a fixed seed gives 2 or more page views, `/__demo/hits` shows no trap path, the `ENGAGEMENT` console line has `pointerMoves > 0`, `wheels > 0` (touches on mobile) and `trusted: true`. Bugs (h), (i), (j) appear on the page kinds reached.
  - Manual: finish with a token; finish by closing the target.
  - Persistent profile: a cookie survives two captures; busy lock; clear; the directory is never deleted by drop or the watchdog (extend `cleanup.rs` and `abrupt_exit.rs`).
  - Cancellation during each driver state.
  - The environment probe reports what the page reports.
- **Pipeline** (`crates/engine/tests/pipeline.rs`). A multi-page fake capture through all nodes; `finish_capture` wiring; activities stored as `visitor` logs.
- **Migration** (`tests/storage.rs`). A version-3 database with a scan and settings opens; the scan loads with `pages == []`, `environment == None` and options resolving to scroll; settings gain the defaults.
- **Frontend Vitest**
  - `visit.ts`, scan form per visitor, cost estimate, page filtering, finding page chips.
  - Legacy fixture snapshot-equal to today's report.
  - `EnvironmentNotice`.
- **Frontend Playwright**
  - Composer flows.
  - Live browse view.
  - Manual finish.
  - Page switcher.
  - Settings profile clear.
- **e2e-tauri**
  - A demo browse scan, headless via options, asserting page views and one multi-page finding.
  - Contract checks for the three new commands.

### Critical Files for Implementation
- /home/user/Atlas/apps/adtech-scanner/crates/model/src/lib.rs
- /home/user/Atlas/apps/adtech-scanner/crates/model/src/capture.rs
- /home/user/Atlas/apps/adtech-scanner/crates/browser/src/capture/mod.rs
- /home/user/Atlas/apps/adtech-scanner/crates/browser/src/capture/page.rs
- /home/user/Atlas/apps/adtech-scanner/crates/engine/src/domain/telemetry.rs