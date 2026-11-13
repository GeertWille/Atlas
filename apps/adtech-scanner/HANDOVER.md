# AdTech Scanner — handover

Status on 2026-11-13. Branch `ccr-a6f7da93-wz5uos` in the Atlas repo, everything under `apps/adtech-scanner`.
No PR, nothing merged.

## What this is

A Tauri 2 desktop app that finds revenue leaks caused by logic bugs in a publisher's ad-tech wrapper.
One scan runs these pipeline steps:

1. **Capture**: launches local Chrome over CDP, injects a tracker before any page script, loads the page
   (desktop or mobile emulation), optionally accepts the consent banner, and then behaves as the scan's
   visitor: watches only, scrolls, browses like a reader and follows same-site links, or records while a
   person browses (see "Visits" below). Records Prebid activity, GPT slot lifecycle and timings, Amazon
   TAM calls and every JavaScript body, per page view.
2. **Telemetry, bidder checks and heuristics**: derives page views, auctions, slots and metrics, verifies
   each bidder's setup (see "Bidder checks" below), then runs 33 deterministic rules. Works with no API
   key.
3. **Script triage (Phase 1)**: classifies the scripts the built-in knowledge base cannot decide.
   `claude-haiku-5-5` does it by default. With the optional "Classify scripts with Jev" setting on,
   TypeSafe's Jev System One (`jev-1.14.0`) does it, and Haiku is the fallback.
4. **Phase 2**: `claude-sonnet-5-5` reads the beautified ad scripts and telemetry in a tool loop and
   reports logic bugs that lose revenue. With Jev on it starts from a reading guide Jev built.
5. **Synthesize**: dedupes findings, scores ad-stack health, writes an overview.

Rules that came from you and must keep holding:

- Local only. A SQLite file in the app data dir, no server, no cloud storage. Outbound traffic is the
  scanned site (through Chrome) and that site's ads.txt (one GET from the app itself),
  api.anthropic.com, api.typesafe.ai only while Jev is on, and, unless "Check sellers.json at ad
  exchanges" is switched off (it is on by default since 2026-11-12, your decision), a GET of
  sellers.json at the ad-system domains that ads.txt names, at most twelve per scan and at most once a
  day per exchange (see "sellers.json cache"). A scan can send the first two kinds through one proxy
  the user configured (see "Proxies"); the sellers.json requests and the two model APIs never use it.
- Raw scan data is stored locally and only locally (this replaced "no raw payloads stored"): one
  compressed archive per scan with every request, its headers, and the bodies of scripts, documents,
  JSON, text, CSS and POST requests, bid requests and responses included. Never images, video, audio
  or fonts. Credentials are replaced by their length before anything is written: cookie and
  authorization values and every header named like a key or token (unless the user turns on "Store
  full headers"), and, whatever that setting says, the app's own proxy password, credential
  parameters in URLs, and password and token fields in request bodies and JSON answers. The table
  under "Scan archive" is the exact list. It is deleted with the scan, bounded by a size limit (5 GB
  by default, oldest scans first), and can be switched off ("Keep network data"). ads.txt is still
  read in memory and not kept; the sellers.json files of ad exchanges are kept, apart from the scans
  (public files, the same for every site: see "sellers.json cache"); the audit trail still holds
  bounded excerpts only, never whole scripts.
- API keys and proxy passwords live in the macOS Keychain, all in one item (service
  `com.webclew.adtech-scanner`, account `vault`; see "Keychain prompts"), never in SQLite, logs or the
  frontend. SQLite only records which of them exist (table `secret_presence`, names without values).
- A scripted visit never hides that it is automated: no stealth flags, no spoofed fingerprint. What the
  page could see is recorded with the scan and reported (rule `scan-identifiable-as-automated`).
- Per-scan LLM budget cap, default $0.50.
- `clew-app` is read-only inspiration.
- Run a simplify pass and a review pass after every change, fix, repeat.

## State of each part

| Part | Path | State |
|---|---|---|
| Data contract | `crates/model` | Done. Generates `src/bindings/*.ts` (never hand-edit). |
| Browser capture and demo site | `crates/browser` | Done, reviewed, fixes applied. 226 unit tests; the integration tests drive real Chromium. |
| Engine (domain, pipeline, SQLite, Anthropic and TypeSafe clients) | `crates/engine` | Done, reviewed, fixes applied. Tests pass, all against mock servers. |
| Tauri shell (35 commands, composition root) | `src-tauri` | Done, reviewed. Command tests pass; 4 real-browser pipeline tests on Linux. `cargo test --workspace`: 1081 tests, 5 ignored (2026-11-13). |
| React frontend | `src/` | Done, reviewed. `pnpm typecheck` and `pnpm lint` are clean; 788 Vitest tests and 233 Playwright tests against the fake backend pass (2026-11-13). |
| Integration of frontend with the real backend | `e2e-tauri/` | Done on Linux. The built app (production CSP) is driven through WebKitGTK by `pnpm test:tauri`: onboarding, demo-site scan, every report tab, history, log search, settings, export and the Chrome picker through the native dialogs, delete, an AI scan against a mock of the Anthropic API, a Jev scan and its fallback against a mock of the TypeSafe API, the scan archive (network record, decoded bid exchange, bid inspector, script viewer with the finding's line marked, storage off), and every command's contract: 41 tests (2 more only run with `TAURI_E2E_SCREENSHOTS`). Not run on macOS. |

## Run it on a Mac

Prerequisites: Xcode Command Line Tools, Rust (stable, 1.97 was used), Node 22, pnpm 10, Google Chrome.

```sh
cd apps/adtech-scanner
pnpm install
pnpm tauri dev          # dev window, Vite on http://localhost:1420
pnpm tauri build        # .app and .dmg under target/release/bundle
```

Checks:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo test -p adtech-scanner --test real_browser -- --ignored   # drives real Chrome against the demo site
cargo test -p adscan-model                                      # regenerates src/bindings
pnpm typecheck && pnpm lint && pnpm test && pnpm test:e2e && pnpm build
pnpm test:tauri         # Linux only: builds the app and drives the real window (see below)
```

`pnpm test:tauri` runs the app from `target/e2e/adtech-scanner`, a copy it makes right after
`tauri build`: any other `cargo build`/`cargo test` rewrites `target/debug/adtech-scanner` without the
frontend embedded, which used to break a test run that was under way. It needs `tauri-driver` (`cargo install tauri-driver --locked`), `WebKitWebDriver`
(package `webkit2gtk-driver`) and `xvfb-run`; with `xdotool` and `xclip` installed it also drives the
native save and open dialogs, otherwise those two tests are skipped. tauri-driver has no macOS backend,
so the suite cannot run on a Mac. `TAURI_E2E_SCREENSHOTS=<dir> pnpm test:tauri screens` saves a picture
of every screen in light and dark.

Environment overrides for tests and local runs:

- `ADSCAN_DATA_DIR`: directory for the SQLite file.
- `ADSCAN_ANTHROPIC_BASE_URL`: replaces `https://api.anthropic.com` (must be https unless loopback).
- `ADSCAN_API_KEY`: use this Anthropic key and skip the Keychain.
- `ADSCAN_TYPESAFE_BASE_URL`: replaces `https://api.typesafe.ai` (must be https unless loopback).
- `ADSCAN_TYPESAFE_API_KEY`: use this TypeSafe key and skip the Keychain. When either key variable is
  set, both keys live in memory only and the Keychain is never touched.
- `ADSCAN_CHROME_PATH`: browser executable.
- `ADSCAN_PROXY_PASSWORD`: stands in for the password of every proxy profile that has none of its own.
  Like the key variables it moves every secret to memory and leaves the Keychain alone.

The built-in demo publisher page is the fastest way to see a full scan. It is served on a random
loopback port and has seven planted bugs, documented in `crates/browser/demo-site/README.md`:

- Scroll gate uses `min-width` where `max-width` was meant, so desktop never auctions before scroll.
- The anchor slot is requested before the auction ends, without `hb_*` keys.
- Bidder timeout is 250 ms instead of 1200 ms, so two bidders always time out.
- A ghost ad unit with no container, and GPT loaded twice.
- `ix` is called but has no line in the site's `/ads.txt`, and never bids.
- `triplelift` has no `inventoryCode` on two of its three ad units, so Prebid never sends those bids.
- The `openx` bid request is answered by the demo exchange with OpenRTB no-bid reason 7.

Three more only show when the visit goes beyond one passive page load (visitor browse or manual):
a sidebar slot refreshed on engagement with the bid it already used, a session-wide "double-fire
guard" that makes every later page view skip bidding, and a route change that defines the slots again
without destroying the old ones.

## What is left, in order

1. **Finish the frontend.** Done: `pnpm typecheck`, `pnpm lint`, `pnpm test` and the Playwright suite
   against the fake backend (`src/lib/fake-backend`, specs in `e2e/`) pass.
2. **Integrate.** Done on Linux (`pnpm test:tauri`). On the Mac, still check by hand: the save and
   open dialogs, external links opening in the browser, dragging and double-click zoom of the title
   bar, and that a scan's Chrome is gone after quitting mid-scan.
3. **Icons.** Done. The source is `src-tauri/icons/icon.svg`; after changing it, render it to a
   1024 px PNG and run `pnpm tauri icon <png>`.
4. **Simplify and review** the frontend, then the integrated app. Fix and repeat until clean.
5. **Visual polish** of every screen in light and dark, at 1024x680 and 1280x820.
6. **First real API run.** Enter a key in Settings and scan the demo site, then a real publisher page.
7. **README.** Done: `README.md`. The checklist for the first run on a Mac is `docs/MAC-TESTING.md`.

## Never verified (no Mac and no API key were available)

- Anything on macOS: Keychain read and write, overlay title bar and traffic-light clearance, the
  bundle, Chrome discovery in `/Applications`, picking `Google Chrome.app`, headed mode.
- The real Anthropic API. All LLM code was tested against a mock HTTP server only, including the
  request shape, model ids, pricing table and `anthropic-version` header.
- Real Prebid, GPT, Amazon TAM and consent platforms. The tracker has only run against local stand-ins.
  That includes the bidder checks: params, supply chain and consent capture have not seen a real
  publisher page or a real CMP, and bid-endpoint answers have not been observed from a real exchange.
  The fetcher and both parsers were tried once by hand against real ads.txt and sellers.json files.
- The real cost per scan against the $0.50 target.

- The real TypeSafe API. The client was written from the published API reference and tested against
  a mock only: request and response shape, the `jev-1.14.0` model id, the $0.042 per million input
  tokens price, 401/403 handling, `GET /v1/models` as the key check, rate-limit behaviour.
- Whether Jev judges minified JavaScript well at all. Every threshold, chunk size, question wording,
  ranking weight and token estimate on the Jev path is a starting value. They are named constants in
  `crates/engine/src/domain/{triage,guide,chunks,questions}.rs`, each marked as uncalibrated.
- Everything in "Keychain prompts" that happens on a Mac: the number of prompts, the move of the old
  items into the vault, what macOS returns when the prompt is denied, and the signing steps. The
  status codes `infrastructure/keychain.rs` maps were checked against Apple's published values
  (`errSecUserCanceled` -128, `errSecAuthFailed` -25293, `errSecDuplicateItem` -25299,
  `errSecInteractionNotAllowed` -25308) and against what `apple-native-keyring-store` 1.0.2 passes
  through as a platform failure; which of them a denied prompt really returns was never observed.
- The scan archive on macOS: nothing of it ran there. On real publisher pages: only the demo site
  and fixtures were recorded, so the request assembly across sessions was measured on Chromium 1243
  for Linux only.
- Proxies of any kind: no scan was run through a real proxy. The browser side (flag mapping, answering
  the proxy's password challenge) and the fetcher side were tested against local stand-ins only, and
  SOCKS5 not at all beyond building the client.
- Headed browse and manual visits (a visible Chrome window, a person at the keyboard), and what a
  persistent browser profile does to a real consent platform across scans.
- The thresholds of the six visit rules (5 s minimum dwell, 2 slots, the 2 s consent delay) and the
  merge of page findings by rule and bidders are starting values that only the demo site has exercised.
- The detection-vendor table against real pages.
- The sellers.json cache against real exchanges: whether they send `ETag` or `Last-Modified`, answer
  304, and how large their files are. And the scan time now that the lookup is on by default.
- The focus-ring fix on macOS. On WebKitGTK a select that was opened by a click and closed on the
  option it already had showed a ring on its trigger, and no longer does while WebKit itself still
  matches `:focus-visible` there (`pnpm test:tauri`). Dialogs and menus did not show one on
  WebKitGTK to begin with, so their part of the fix was only checked not to take the ring away
  from keyboard users. WebKit's own rule also applies: a scripted focus is ringed only if the
  control focused before it was, so a ring that a mouse click took away comes back with Tab, not
  with the next key press inside the same control.

## Known open issues

- **Two ways the unconfined tracker can still reach another site.** If the entry page runs into the
  load timeout while a redirect of it is still on its way, that navigation can commit after the page
  was taken for opened and before the tracker is confined. And a tab the page opens before the site is
  known is set up with the unconfined tracker and keeps it. Nothing of another site is recorded
  either way; the tracker runs there, which it should not. That the tracker is confined in time at
  all rests on Chrome running kept-back commands before the new document's first script: observed
  on Chromium 1243 (40 of 40 runs of a probe), not documented by Chrome, not checked on a Mac.
- **A navigation that does not end keeps the scanner's questions to the page waiting.** Chrome
  holds every command that the page itself must answer (reading the title, the scroll position,
  `navigator.webdriver`, the tracker flush, the consent search) while a navigation of the tab is
  under way. The one case that was the scanner's own doing is fixed (see "The tracker stays on the
  scanned site"). A page that starts a navigation to a server that takes long to answer would have
  the same effect for as long as that takes: each such command waits, up to the 60 s
  `CALL_TIMEOUT` of `cdp/client.rs`, and is then taken for unanswered. Recording goes on
  meanwhile. Inferred from the trace of the fixed case, not measured with a slow server.
- **Hidden empty files in the temp dir on Linux.** A Chrome that is killed in the first fraction
  of a second of its start can leave a `.org.chromium.Chromium.XXXXXX` file of 0 bytes in `/tmp`
  (3 of 30 launches killed after 150 ms, 0 of 60 killed later; about 3 per run of the browser test
  suite, from the tests that end a browser early). It is the file Chrome makes for shared memory
  under `--disable-dev-shm-usage` and removes a moment later; without the flag the same file is
  left in `/dev/shm`. Nothing ties such a file to a launch, so nothing removes it. The flag is
  only passed on Linux.
- **Ad servers other than Google Publisher Tag are not read.** Slots, ad requests, `hb_*` targeting on
  the request, fill and render timing all come from hooks on `googletag`. A page that serves through
  Xandr AST (`apntag`, `ib.adnxs.com/ut/v3`), Equativ/Smart AdServer (`sas`), Adform or a direct ad
  server has Prebid telemetry only: no slots, no "requested without bids", no fill rate, and the slot
  rules (`slots-never-requested`, `ad-request-before-auction-end`, `slots-outside-header-bidding`,
  refresh rules) cannot fire. `ScanMetrics.hasGpt` is the only ad-server flag. The script host table
  (`domain/knowledge.rs`) lists `adnxs.com` and `smartadserver.com` as bidders, which is right for
  their Prebid endpoints (`/ut/v3/prebid`). The network view does tell their ad-server calls apart
  by path (`domain/network/classify.rs`: `/ut/v3`, `/ttj`, Smart `/ac`, `/call/`, `/h/`), and the
  engine reads the answers of those calls in memory for every scan (`application/archive/analysis.rs`),
  but nothing is derived from them yet. What is missing: a tracker hook per ad server (AST
  `apntag.defineTag`/`loadTags`/`onEvent`, Smart `sas.call`/`sas.render`) or a decoder of the calls
  in that analysis step, mapped onto the same slot events, and `hasAdServer` next to `hasGpt`.

- Turning on Prebid debug logging (`setConfig({debug:true})`) has minor side effects on real sites.
- Scans stored before 2026-11-12 keep what was recorded then: the step a cancel interrupted reads
  Failed with the summary "Cancelled" (new scans store the step status `cancelled`), a CORS preflight
  can have a duration of a millisecond or two below zero (shown as 0) and can be filed under "other
  third party" instead of with its request, and a request made in the first tenth of a second after an
  in-page route change can sit on the page view before it.
- **Twin ad units.** A page may register several Prebid ad units under one code (seen on knack.be:
  one per bidder, each with its own sizes). The tracker reports them as registered; the engine makes
  one ad unit per code of them in every auction (`domain/telemetry.rs::merge_ad_units_by_code`: the
  sizes of all in the order first named, the bids of all), so the waterfall, the bid exchanges view,
  the rules, the exports and Phase 2 see one. Scans stored before 2026-11-12 hold one entry per
  registered ad unit; they are merged when read (`ScanService::get`), the stored rows are not
  rewritten. The bidder params are still checked per registered bid, and a params finding counts
  those ("missing on 1 of 2 ad units" can mean two registrations of one code). The bid exchanges
  table used to key its header columns by ad unit code, which with twins left columns of the auction
  shown before in the header; header and rows are now built from one column list.
- **Currencies are never converted.** Bids are shown in the currency Prebid recorded. Where an
  auction has bids in more than one, every price is written with its code ("2.05 USD", "10.01 EUR")
  and the bid exchanges view says so; the average winning CPM (report and `ScanMetrics`), a bidder's
  average CPM and a request's best price are not computed across currencies, and rows are sorted by
  price within a currency only. Which bid won is Prebid's own decision (`bidWon`) and is shown as
  recorded: a page without Prebid's currency module compares raw numbers across currencies, which
  loses money, and the scanner does not report that as a finding yet.
- A keychain vault that is not valid JSON is never overwritten: every read and every change of a
  secret fails with "the stored secrets are not in the expected format" until the item `vault` is
  removed in Keychain Access. There is no button for that.
- A legacy keychain item whose deletion failed after its secret moved into the vault stays in the
  keychain; it is never read again while the vault exists, and clearing the key in the app does not
  remove it.
- **The document of a cross-site iframe that is removed as it finishes loading has no body**
  (`unavailable`, "the frame or worker that loaded it is gone"): about one per three visits of a
  heavy news page. What frames and workers fetch themselves is covered since 2026-11-13; see
  "Bodies of frames and workers that go away" under Scan archive for what was tried.
- Very small scripts can fit entirely inside one stored excerpt.
- The Prebid-age rule compares with a constant, not with the web: `CURRENT_PREBID_MAJOR` (11, from
  `npm view prebid.js version` = 11.49.0 on 2026-11-12) and `MAX_PREBID_MAJORS_BEHIND` (3) in
  `domain/heuristics/setup.rs`. It reports builds more than three majors behind (7.x and older today)
  and the finding names both versions. Raise the first constant when Prebid 12 is released.
- Model prices are a table in `domain/pricing.rs`. The frontend's cost estimate reads
  `src/shared/domain/model-prices.json`, which that file's own test writes from the table: after a
  price change `cargo test -p adscan-engine` fails once, rewrites the file, and the file is committed.
- Log search matches an identifier with underscores (`report_finding`) as one term and marks it as
  one. The index itself still splits words at underscores (tokenizer `unicode61`, unchanged, so no
  migration): each word of an identifier is also found alone.

## Scan archive

What a scan loaded is kept next to its report, so the script viewer, the network view and the bid
inspector have something to show after the scan. The contract between the three crates is written
down in the session scratchpad (`archive/02-contract.md`); the Rust sources are the truth.

**Where.** `<data dir>/archive/scans/<scan id>.pack.zst`, one file per scan, plus three SQLite tables
(migration 5): `scan_archives` (one row per scan: state `writing`, `sealed`, `evicted` or `disabled`),
`scan_bodies` (sha-256 and byte range of every kept body) and `requests` with a `hosts` dictionary
(one row per request: host, path without query, status, sizes, timing, category, links). A request
row never holds a query string, a header or a body; those are in the pack.

**Pack format (version 1).** A sequence of independent zstd frames (level 3, checksum on), in the
location syntax of the Webclew data lake (`scans/<id>.pack.zst#o=<offset>&l=<length>`): one frame per
distinct body (raw bytes, deduplicated by sha-256 within the scan), record frames of up to 128
request documents as JSON lines (the first seven fields are the data lake's: `_v`, `id`, `fullUrl`,
`headers`, `postData`, `responseHeaders`, `responseContent`), a manifest frame, and a trailer that is
a zstd skippable frame naming the manifest. The whole file is a valid `.zst`. The local adapter is
`infrastructure/pack_store.rs` behind the `PackStore` port; an S3 adapter was not built.

**What is kept.** Every request's metadata and headers, from every frame and worker the browser
crate reports. Bodies of scripts, documents, stylesheets, JSON, XML and text, and POST bodies; every
script source of the capture, inline ones included. Images, video, audio, fonts and other binary
content: size and type only. Caps: 5 MiB per body and 1 MiB per POST body (cut and flagged), 256 MiB
of kept bodies per scan (after that metadata only, each request says so), 64 MiB of bodies in flight
between browser and disk. Every way a body can be missing has its own `BodyState`. The in-flight
budget counts a body with its full size, because the browser hands it over whole before it is cut
to the cap: answers of more than 64 MiB, or more of them at once than fit, are `dropped`. Chrome
itself keeps at most 32 MiB per response for the scanner (`maxResourceBufferSize`). "First party"
is the registrable domain of the page view a request belongs to (the page the tab showed after
redirects), not of the address the scan was started with.

**Bodies of frames and workers that go away.** A body is read with `Network.getResponseBody` when
its response has finished, on the session that reported it. A cross-site iframe or a worker that is
removed in the task that read its last answer takes its session with it; no ordering on the
scanner's side helps (measured: the command sent in the handler of `loadingFinished` is refused 85
to 100% of the time when the target is removed at once). So every request a frame or worker
*announces* that may carry a kept body (document, script, stylesheet, XHR, fetch, other; or a
recognised bid request) is also asked to send its body along while it loads
(`Network.streamResourceContent`, Chrome 122 and later; `capture/network/assembly.rs` holds the
state, `capture/recorder.rs` asks). The copy is used only when the target refuses the normal read
and the copy is whole (its length is the decoded length the browser reported); it then goes to the
sink, the bid-response reader and the record like any body, cut to what the sink admitted and not
at all when the sink skipped it. Copies of open requests are held up to 32 MiB together
(`MAX_COPIED_BYTES`); one that does not fit is given up. A Chrome without the method says so once
per capture and is read as before. Nothing is paused or delayed for this. Measured on the test
Chromium (153), frames and workers removed in the task that read a fetch, 20 rounds per case:
before, 41 of 60 bodies lost in cross-site frames and 60 of 60 in workers; after, 0 of 120 when
the answer takes 30 ms, 2 of 120 at 5 ms, 7 of 120 when it comes at once from the same machine.
Frames in the page's own process never lost a body. Cost on a heavy real page (theguardian.com,
about 1,000 requests, half of them in frames and workers, 25 s): app CPU 0.94 to 1.08 s before,
0.95 to 1.29 s after (the page varies more than that between runs), peak memory 54 MB both, no
change in load or first-auction time.

What this does **not** cover, and what was tried:

- **The document of a cross-site iframe itself** (announced by its parent, finished in the
  frame). This is the loss actually seen on real pages: about one in three visits of
  theguardian.com loses one, a 7 kB SafeFrame container that is removed as it finishes loading.
  The frame's target does not exist before the response is committed, so there is nothing to ask
  beforehand; asked at attach or at the response, Chrome answers "Request with the provided ID does
  not exists"; asked at the frame's first data event the copy was whole 5 times in 15.
- **An answer that is complete within a few ms of the request** (disk cache, a server on the same
  machine) in a target that then goes away at once: the question arrives too late and the body is
  `unavailable`, as before.
- `Network.configureDurableMessages` (Chrome 145 and later; keeps bodies in the network service)
  does make all of these readable through the tab's session after the target is gone, the iframe
  document included, but **must not be turned on**: once a session has it, Chrome 153 answers that
  session's reads only from that store. Bodies of 4 MB and more that arrive at local speed come
  back cut short without an error or not at all, answers served by a service worker or from the
  memory cache are missing, and so are the fetches of a worker started by a same-process iframe.
  Three existing tests fail with it. Worth another look when Chrome fixes it: with the store only
  as a second source this gap would close.
- Asking for the copy when the response headers arrive (the first moment the sink's admission
  could be known) is too late 90% of the time; reading through the parent's or the browser's
  session does not find the body; `Network.loadNetworkResource` makes a new request; nothing keeps
  a session alive past its frame.

**What is stored, replaced, and never stored.** The rules are `domain/archive/redact.rs`; they run
before a byte reaches the pack, and the request rows in SQLite never hold a query string, a header or
a body at all. "Credential name" means a name whose words include password, pass, passwd, pwd,
passphrase, secret, token, jwt, apikey, auth, authorization, signature, csrf, xsrf, otp, cvv or cvc,
or "key" after api, secret, private, access or subscription (`X-Api-Key`, `access_token`,
`csrfToken`, `user[password]`), unless the next word says the field only describes one
(`token_type`, `auth_method`). `:authority`, `Sec-WebSocket-Key`, `gdpr_consent`, `session_id`, `sig`
and `key` alone are not credential names.

| What | Default | With "Store full headers" |
|---|---|---|
| `Cookie`, `Set-Cookie` values | replaced: `_ga=[27 bytes]`; cookie names and attributes stay | stored |
| `Authorization` value | replaced: `Bearer [212 bytes]`; the scheme stays when it is a known one, otherwise the whole value goes | stored |
| Values of other headers with a credential name (`X-Api-Key`, `X-Auth-Token`, `X-CSRF-Token`, `X-Amz-Security-Token`, ...) | replaced: `[40 bytes]` | stored |
| `Proxy-Authorization` (the app's own proxy password) | replaced | **replaced** |
| URL parameters with a credential name, in the request URL, a fragment, `Referer`, `Location`, `Content-Location`, initiator and stack-frame URLs; a password before the host | replaced: `access_token=[redacted-16-bytes]` | **replaced** |
| Fields with a credential name in request bodies: form fields, JSON string and number values (also in JSON nested in a string), multipart parts, `name=value` pairs anywhere in the text | replaced: `pwd=[7+bytes]`, `"password":"[7 bytes]"`; everything else in the body stays byte for byte | **replaced** |
| Fields with a credential name in JSON responses (`access_token`, `refresh_token`, ...) | replaced | **replaced** |
| All other headers, the rest of every URL (ad-server calls with their targeting, `gdpr_consent`, cookie-sync ids), bid requests and responses, HTML, scripts, CSS, text | stored as captured | stored as captured |
| Bodies of images, video, audio, fonts, other binary content, WebSocket and EventSource traffic | never stored (size and type only) | never stored |
| sellers.json of ad exchanges (public files, the same for every site) | kept as received in `<data dir>/cache/sellers-json/`, apart from the scan archive; see "sellers.json cache" | the same |
| The scanned site's ads.txt | never stored (read in memory, only the checks' results are kept) | never stored |
| API keys (Anthropic, TypeSafe), proxy passwords, the keychain vault | never stored: the app's own HTTP calls do not go through the scanned browser, and the one place a proxy password can surface in it is the header above | never stored |

Why this line and not another: the archive exists for ad debugging, which needs the URLs, the
targeting, the consent strings and the bid payloads exactly as sent, so those stay. What a request
carries to prove who the visitor is has no such use and is replaced wherever it is recognisable by
name, on every host: a rule tied to "the site's login path" would miss GraphQL endpoints, localised
paths and third-party identity providers, and promise more than it keeps. A login therefore keeps
its user name and loses its password. Known gaps, on purpose or by nature: a secret in a URL *path*
(`/reset/<token>`), in a field with an unusual name, in a response that is not JSON, or printed
into a page (a signed-in page's HTML is stored as received, personal data included) is stored. A
manual visit or a kept browser profile can make that signed-in content; the Settings copy must say
so. `ScanDetail` data outside the archive (page-view URLs, script URLs in the script list) is stored
as before and does not pass these rules. No app-level encryption (FileVault). The archive directory
is created with mode 0700 and packs with 0600.

**While the capture runs.** The engine hands the browser crate a `NetworkSink` for every scan
(`application/archive/recording.rs`). Reading a body and storing it are separate decisions: the sink
admits a body when it will be stored, or when it is a bid or ad-server call, which the engine always
reads in memory. Every record passes `RecordAnalysis::observe` (`application/archive/analysis.rs`)
with its bodies still attached: the one place to plug in what is read out of the traffic (today the
bid exchanges and the list of ad-server calls; decoding ad-server calls belongs there). With "Keep
network data" off that analysis still runs, and nothing is written. Rows are added to SQLite only
after the bytes they point at are synced.

**After the capture.** Telemetry ties bids to requests (`domain/network/exchange.rs`) and gives each
bid a verdict: in time, at the deadline (within 10 ms, `VERDICT_TOLERANCE_MS`, not calibrated), late,
in time but timed out in Prebid, never answered, failed, not sent. These read as statements about a
bidder, so the matching says nothing where it cannot be sure:

- A request that contains one of Prebid's ids for the bidder request is that request ("exact").
- Without an id, a request is matched by timing only when it is the one request of that bidder that
  started within -30/+300 ms of the announcement and no other id-less announcement could claim it.
  A bidder that sends one request per ad unit without ids, or two auctions in the same moment, stay
  unmatched. A timing match that Prebid's own record contradicts (Prebid has the answer, the
  request is late or unanswered) is dropped.
- Several exact matches without bid ids in them (the auction id in every per-unit request) are not
  split over the ad units by guessing.
- "Not sent" is said in two cases only: the bidder's request is there, was read whole and holds
  the bid ids of the other ad units but not this one; or the capture knows the bidder's endpoint
  and no bid request that could be the missing one (the bidder's own, a Prebid Server call, an
  endpoint without a known adapter) started on that page view since the announcement. A request body
  that was cut off, could not be read, or is not text (compressed by the adapter) is never grounds
  for it.
- A bidder running under an alias is not matched to the endpoint of its adapter's main code, so it
  gets no verdict.

The deadline is the auction's start plus its timeout, which is when Prebid's timer is due; a busy
main thread fires it later, so an answer that the network says came late can still have been
accepted, and one that came in time can still have been timed out (that is the "in time but timed
out" verdict). Network times come from each request's own wall-clock stamp, tracker times from the
page's `performance.timeOrigin + now()`; they agree to about a millisecond as long as the system
clock is not stepped during the visit. Slot requests are tied to the nearest ad-server call. Bid
exchanges are decoded as OpenRTB 2.x with Prebid Server's `ext` when a request is opened
(`domain/network/openrtb.rs`); other protocols (Rubicon fastlane, AppNexus `ut/v3`) are shown raw.
No-bid reason names are the published list OpenRTB 2.6 refers to (OpenRTB 3.0 "List: No-Bid Reason
Codes", checked against the IAB repository), one table in `crates/model` for the bidder checks and
the decoder; codes from 500 on are left as numbers.

**Limit and lifetime.** After each scan the oldest sealed archives lose their pack until the total
is under `Settings.archiveLimitBytes`; their request rows stay, so the request list still reads, with
URLs cut to host and path. Deleting a scan deletes its rows, then its pack, the host names only it
used, and the formatted sources held in memory. SQLite runs with `secure_delete` and the write-ahead
log is folded in and emptied after every deletion, so deleted rows are not left readable in the
database files (the file system may still hold freed blocks). A cancelled scan and a failed capture
leave no archive. "Clear stored scan data" removes packs and rows of every finished scan. At
start-up (`Archive::sweep`) an archive that was still marked as being written is sealed if its pack
ends with the trailer and the manifest of exactly the rows the index has (the app stopped between
the two), and removed otherwise; packs without rows are removed and rows without packs marked
evicted. Only the first app on a data directory does that, and marks interrupted scans failed: it
holds a lock on `<data dir>/instance.lock`, and a second instance leaves running scans and their
packs alone. Exports (Markdown, JSON) never contain bodies.

**Hostile or damaged files.** Pack names from the index must be plain relative `.pack.zst` names
under the archive directory. A byte range is checked against the file's length before memory is
reserved for it, a frame may not be larger than its kind can be, and unpacking stops at the size
the kind allows (a body at twice the 5 MiB cap, a record frame and a manifest at 64 MiB), so a wrong
index row or a zstd bomb is an error, not an allocation. Every frame carries a checksum.

**Script viewer.** `get_script_source` reads the source from the archive by the script's sha-256,
formats it on demand with the same oxc formatter the analysis uses (cached in memory, never on
disk), finds each finding's code evidence again by its text (`domain/source.rs::locate`, shared with
the citation check, which now also stores `Evidence.scriptId` and `endLine`), and reads the "read by
the AI" ranges from the stored audit trail. Phase 2 itself still reads the in-memory sources.

**When a fetch fails.** The report, the network view and the script viewer follow one rule, with one
component (`src/shared/components/retry-notice.tsx`, `RetryNotice`): what is on screen stays when
fetching it again fails, under a warning with the backend's message and "Try again"; only a first
load that fails takes the place of the content (`QueryError`). Each query is judged by itself:

- The scan (`get_scan`), on all three routes: the report, the workbench or the editor stay, with
  their state (filter, selection, scroll position). A scan that was deleted is still "This scan no
  longer exists".
- The request list (`use-network-rows.ts`, `failure`): a first chunk that fails is an error in the
  list's place. A later chunk that fails keeps the rows before it; the notice says how many of how
  many are shown, the footer stops saying "loading more", the chunk is not asked for again until
  "Try again", and the retry goes on from that chunk. A refresh that fails keeps the list as it was.
- The details of a request: a newly selected request whose record cannot be fetched shows the
  error inside the panel (on the Summary tab above the facts the list holds), the list stays. A
  record that was loaded stays when refreshing it fails.
- A body: a first stretch that fails is an error in the body's place; a later one ("Load more")
  keeps the text read so far.
- The network summary on the report's Network tab and the source text of a script: loaded content
  stays. Another script, or the other view of the same script, is a first load.

The states that are data and not failures are untouched: not recorded, storage off, evicted, list
only, not retained. Tested at route level in `src/app/routes/{Scan,Network,Source}Route.test.tsx`
and `src/features/network/detail/BodyView.test.tsx`. Not in the Playwright suite: the fake backend
has no way to make a read fail, and none was added.

Known limits: the host and path tables of `domain/network/classify.rs`, the timing windows of the
exchange matching and the 10 ms verdict tolerance have only seen the demo site and test fixtures;
bidder aliases get no network verdict; a script source longer than 5 MiB is not in the archive whole
and reads as not retained; the columns of initiator positions are UTF-16 units in Chrome and are
used as characters; nothing here ran on macOS (file modes, the instance lock and `fsync` behave as
on Linux there in principle, unverified).

## Keychain prompts

macOS shows "AdTech Scanner wants to use your confidential information stored in
"com.webclew.adtech-scanner" in your keychain" when an app reads a Keychain item that does not list
that app as trusted. The item remembers the app by its code signature. `pnpm tauri dev` and an
unsigned `pnpm tauri build` produce a binary whose signature is unique to that build (no signature, or
the ad-hoc one the linker adds on Apple Silicon, which is a hash of the binary). So after every
rebuild the app is a stranger to its own item, and "Always Allow" is forgotten. macOS asks once per
item and per read, which is where the four or five prompts came from: the app kept one item per
secret (Anthropic key, TypeSafe key, one per proxy password) and read every one of them on every
settings fetch and again when a scan started.

What the app does now:

- **One item.** Every secret is in one generic-password item, account `vault`, holding a small JSON
  map (`anthropic`, `typesafe`, `proxy:<profile id>`). One item means one prompt at most.
- **Read once, on first need.** Nothing reads the Keychain at start, for Settings, or while browsing
  scans and reports. `hasApiKey`, `hasTypesafeKey` and `hasPassword` come from the SQLite table
  `secret_presence`. The first action that needs a secret reads the vault once and keeps it in memory
  until the app quits: starting a scan that uses a stored key or a proxy password, or saving or
  removing a key or password. A scan with AI off and no proxy password never reads it.
- **A refusal is reported as one.** Denying the prompt fails the action with "Keychain access was
  denied" (a broken or locked Keychain: "The keychain is unavailable: ..."). The scan is not started
  without AI, the record of what exists is not changed, and the next need asks again.
- **Old items are moved once.** The first read that finds no vault looks for the items of earlier
  versions (`anthropic-api-key`, `typesafe-api-key`, `proxy-password:<id>` for the profiles in the
  settings), writes them into the vault and then deletes them. If the vault cannot be written, the old
  items stay. Expect one last round of prompts, one per old item, on the first scan after updating.
  Until then the flags in Settings are a guess made by the database migration (an Anthropic key if any
  scan used AI, a TypeSafe key if Jev was on, a password for every proxy profile); that first read
  corrects them.
- `ADSCAN_API_KEY`, `ADSCAN_TYPESAFE_API_KEY` and `ADSCAN_PROXY_PASSWORD` still bypass the Keychain
  completely. For day-to-day development that is the simplest way to see no prompt at all.

The code: `infrastructure/keychain.rs` (the OS calls, behind the `Keychain` trait),
`infrastructure/secrets.rs` (`VaultSecretStore`: vault, cache, migration),
`infrastructure/sqlite/secret_presence.rs` (the record and its migration), and
`crates/engine/tests/keychain_prompts.rs`, which counts Keychain accesses per user action against a
counting stand-in for the OS layer.

What you see per launch, as long as the build is not signed with a stable identity: one prompt, at the
first scan that uses a key or at the first change of a key. "Allow" covers that one read; a later
change of a key in the same launch can ask once more, because the `keyring` crate looks the item up
again before it overwrites it. "Always Allow" covers the rest of that build's life.

### Making "Always Allow" stick

Sign every build with the same identity. Then the item recognises the app after a rebuild and macOS
stops asking. The signing configuration was left alone because the identity is yours to choose. In
`src-tauri/tauri.conf.json`:

```json
"bundle": {
  "macOS": {
    "minimumSystemVersion": "11.0",
    "signingIdentity": "Developer ID Application: Your Name (TEAMID)"
  }
}
```

The environment variable `APPLE_SIGNING_IDENTITY` does the same without touching the file.

(a) With an Apple Developer ID:

1. Create a "Developer ID Application" certificate in the Apple Developer account and install it
   (Xcode > Settings > Accounts > Manage Certificates, or download and double-click it).
2. `security find-identity -v -p codesigning` prints its exact name. Put that name in
   `signingIdentity`.
3. `pnpm tauri build`.

(b) Free, with a self-signed certificate:

1. Open Keychain Access. Menu Keychain Access > Certificate Assistant > Create a Certificate...
2. Name: `AdTech Scanner Dev`. Identity Type: Self Signed Root. Certificate Type: Code Signing. Create.
3. `security find-identity -p codesigning` should list it (it may be marked as not trusted; that is
   fine for signing your own build).
4. Set `"signingIdentity": "AdTech Scanner Dev"` and run `pnpm tauri build`. The first signing asks
   for permission to use the new key; choose Always Allow there too.

A self-signed build is for this Mac only: Gatekeeper on another Mac will not accept it. That needs (a)
plus notarization, which has nothing to do with the Keychain prompt.

Verify either one:

```sh
codesign -dv --verbose=4 "target/release/bundle/macos/AdTech Scanner.app"
codesign -d -r- "target/release/bundle/macos/AdTech Scanner.app"
```

The first must show `Authority=` with your certificate, not `Signature=adhoc`. The second prints the
designated requirement, which is what the Keychain item stores: it must name the identifier and the
certificate, not `cdhash`. A requirement by `cdhash` changes with every build.

The first launch of the first signed build still asks once, because the vault was created by a build
with another signature. Choose "Always Allow". Later builds with the same identity should not ask.

`pnpm tauri dev` does not sign: it runs the debug binary straight from `target/debug`, and
`signingIdentity` only applies to `tauri build`. Use the environment variables above in development,
or expect one prompt per rebuild.

### The `keyring` crate

Version 4.2 with its default feature `v1`, which on macOS uses `apple-native-keyring-store` with the
`keychain` feature: the file-based login keychain through the `SecKeychain` calls.

- A read is one call (`SecKeychainFindGenericPassword`). There is no separate existence check, so the
  crate adds no prompt of its own on reads.
- A write is a lookup of the item and then a change of it (`set_generic_password` in
  `security-framework`). The lookup reads the secret, so a write to an item that does not trust the
  app can prompt like a read does. The app therefore writes only when a secret changes, and always
  after its one read.
- The crate's other macOS store (`protected`, the data protection keychain) never shows this prompt,
  but it only works for an app signed with a provisioning profile and a keychain access group. An
  unsigned or self-signed build gets an error from it. It was not chosen for that reason; it becomes
  an option once the app ships with a Developer ID and a provisioning profile, and would need the
  vault moved once more.

Not verified, because no Mac was available: every statement above about what macOS shows and when.
The macOS half of `infrastructure/keychain.rs` was type-checked and linted for `aarch64-apple-darwin`
on Linux (in a scratch crate, because the engine's C dependencies do not cross-compile here) and has
never run. The mapping of a denied prompt assumes macOS answers with `errSecAuthFailed` (-25293) or
`errSecUserCanceled` (-128), and with `errSecDuplicateItem` (-25299) when the denied read is the lookup
inside a write. If a denial shows up as "The keychain operation failed: ..." instead of "Keychain
access was denied", the status code in that message is the one to add in `platform_failure`.

## Visits: visitor behaviour, page views, the browser profile

A scan is a visit of one or more page views. The options of a scan choose its visitor
(`ScanOptions.visitor`; older callers' `autoScroll` still works and means scroll or none):

- **none**: load and watch, no input.
- **scroll**: scroll through the page (what every scan did before).
- **browse**: move the pointer, scroll, and follow up to `extraPages` (0-5, default 2) same-site links.
  Shown in a window unless `headless` is set; planned from `seed`, so a repeated scan with the same
  seed plans the same visit.
- **manual**: Chrome opens, a person browses, the app records until `finish_capture` (or the last
  window closes, or 15 minutes). Never headless, never clicks the consent banner.

What the backend guarantees about it:

- The options a scan is stored with are fully settled (`visitor`, `extraPages`, `seed`, `headless`,
  `persistentProfile`, `proxy`, `budgetUsd` are always present), so "repeat this visit" is sending them
  back. Choices left open follow the settings at the start of the scan.
- **Time base.** Every `t` and `*AtMs` of an auction, slot or script counts from the start of its page
  view; `pages[page].startedAtMs` is relative to the scan's navigation start. A single-page scan reads
  exactly as before. `ScanMetrics` counts are sums over page views; load time and the two
  time-to-first figures are the entry page's. `ConsentReport.resolvedAtMs` is scan-absolute, because a
  consent choice holds across page views.
- The same container on two page views is two `AdSlot`s (`page`, `elementId`). After a route change
  (single-page app) a request opens a slot of the new page view with what is known carried over, and a
  late answer goes to the slot it answers. Every ad request of a slot is kept in `AdSlot.requests`.
- Rules are scoped (`Rule::scope`). Page rules run once per page view and equal findings (same rule,
  same bidders, different page views) merge into one that names its `pages`; scan rules see the whole
  visit. Six rules need more than one page load, a refreshed slot or the environment
  (`domain/heuristics/visit.rs`): `later-page-view-without-bids`, `spa-slots-not-reset`,
  `refresh-with-stale-bids`, `refresh-without-auction`, `consent-asked-again`,
  `scan-identifiable-as-automated`.
- **Environment.** `ScanDetail.environment` holds what the pages could see (`navigator.webdriver`,
  headless, overrides) and which detection vendors were on the page. The vendor table
  (`domain/environment.rs`) only lists hosts whose ownership was confirmed in the DuckDuckGo Tracker
  Radar entity data: DoubleVerify, Integral Ad Science, Pixalate, CHEQ, HUMAN, DataDome, hCaptcha,
  Arkose Labs, reCAPTCHA (`recaptcha.net` only). Left out because they could not be confirmed from
  here: Cloudflare's challenge host, Anura, Imperva.
- **Audit trail.** Everything the visitor did is a log entry of kind `visitor` on the Capture step,
  with `data = { activity, page, detail }`: page views, actions (one per gesture), each link choice with
  what it passed over and why, what the scanner refused (off-site navigation, downloads), and the
  environment.
- **What a browse visit can and cannot do.** It moves the pointer, turns the wheel (or swipes) and
  clicks links; there is no code path that presses a key or enters text (`capture/input.rs`, with a test
  that searches the sources). Which link is followed is decided in Rust (`visitor/links.rs`) from facts
  the page supplies: never a link in or under an ad, off the site, to a file, marked
  `nofollow`/`sponsored`/`ugc`, named like an account or basket action (text, `aria-label`, `title`,
  path, query), inside a form, or with an attribute that makes a click do more than navigate
  (`onclick`, `data-method`, `hx-post`, `ping`, ...). Right before the click the point is tested again
  (`injected/hit.js`): something over the link, or a frame, form control or custom element inside it,
  and it is not clicked.
  From the moment the entry page has loaded until the visit ends, every main-frame document request is
  checked in Rust before it is sent (`capture/interception.rs`, `Fetch` domain, documents only): another
  site (public-suffix aware, and http from an https site), a non-GET request (a form submission), a
  file, or an address that names an account or basket action is failed, and the page that was showing
  stays. That covers a link rewritten as the button goes down, redirect chains hop by hop, tabs the
  click opens, and pages that navigate by themselves. Each refusal is a `blocked` entry (`off-site`,
  `form`, `sensitive`, `download`); a refused click ends the link following of that visit. `none` and
  `scroll` send no clicks except the consent banner's and are not guarded; `manual` is the person's.
- **The tracker stays on the scanned site.** It is installed without a site, because redirects of the
  entry page decide which site the visit is on, and confined to that site once the entry page has
  settled (`Page::confine_to_site`). So that a page which leaves its site in between cannot take the
  unconfined tracker along, every scan pauses document requests until the tracker is confined, not
  only a scan with a proxy login or a visitor that follows links. A main-frame navigation that
  starts after the entry page has settled confines the tracker as it is paused, without waiting for
  the answers: Chrome keeps commands back while a tab moves to another site's process and runs them
  there before the new document's first script, so waiting would never end. A navigation that
  started before the entry page settled is still a redirect, and the load wait waits for it to
  commit or fail. With the confinement the pausing ends, unless the proxy login or the navigation
  guard needs it. It ends with `Fetch.enable` and no patterns, not `Fetch.disable`: Chromium 153
  never loads a document whose request it was about to pause when the disable arrived, and holds
  back every later command on the tab (about 1 visit in 90 of the test below, each then waiting out
  two 60 s command timeouts; an iframe whose document is caught the same way stays empty, without
  any sign). With no pattern, such a request is still announced and continued. That an empty
  pattern list pauses nothing was observed on Chromium 153 only (0 of the requests of a demo visit
  paused after it); the protocol description reads as if a missing list means "everything". On a
  Chrome that took an empty list that way the visit would still work, with every request waiting
  one round trip. The order of events that hangs, from a trace: `Page.frameStartedNavigating` and
  `Network.requestWillBeSent` for the new document, then `Fetch.disable` sent and answered, and
  never a `Fetch.requestPaused`, a response or a `Page.frameNavigated`; Chrome holds commands for
  the page while a navigation of its tab is under way, so `Page.createIsolatedWorld` went
  unanswered twice. Second test: `no_document_request_is_lost_when_the_scan_stops_pausing_them`
  (a page that adds iframes all through that moment; with `Fetch.disable` it failed 20 of 20 runs).
  Until then each document of the entry page, iframes included, waits one round trip, which
  overlaps the first auction. Test:
  `a_page_that_leaves_its_site_while_the_scan_settles_on_it_is_still_scanned` in `tests/network.rs`,
  whose other site reports whether the tracker touched it.
- **What the navigation guard does not cover.** The guard checks navigations, nothing else. A link
  that passes every check above is clicked with a real click, and whatever the page itself attached
  to that click runs: a handler added with `addEventListener` (on the link or on any ancestor, which
  is how most analytics and single-page routers listen) is invisible to the link checks, which only
  see attributes. Such a handler can send requests, open an overlay, change the page or write to
  storage before or instead of navigating. The scanner does not prevent that and cannot know it
  happened beyond what the capture records (requests, route changes, console lines). Only the
  navigation that follows is checked.
- **Which page view a request is on.** A request belongs to the page view that is showing when it
  starts (`capture/pages.rs::place_at`), the same rule as for auctions and tracker events. The tracker
  reports an in-page route change the moment it happens, ahead of its batch, so that what the page
  requests in the same task as the route change is already placed on the new page view.
- **Consent dialogs.** With "accept consent" the scanner looks for the dialog in every frame of the tab
  (`capture/consent.rs`, `injected/consent.js`): the page, its same-process iframes of other origins (a
  consent platform on a subdomain of the publisher, as Sourcepoint on bild.de) and out-of-process
  iframes, each in the scanner's isolated world. What to press comes from one table of platforms
  (`PLATFORMS` in consent.js; a row there, a row in `tests/consent.rs` and one in
  `fixtures/consent-platforms.html` adds a platform) and from labels that plainly mean "accept all" in
  en/nl/de/fr/es/it/pt, read from text, `aria-label` and `title`. A control whose name mentions a
  purchase (PUR, Abo, subscribe, pay, a price) is never pressed, so on a consent-or-pay wall only the
  consent option is. The control is pressed with a real click at its place on the screen once
  `DOM.getNodeForLocation` confirms that its frame is what is there; otherwise from script. It is looked
  for twice a second for ten seconds, then every two seconds for as long as the page view is watched.
  `Capture.consentNotes` records what the scanner did (`accepted`) or could not do
  (`dialogWithoutAccept`, also a `blocked` entry with reason `consent`).
- **Consent the scan did not give.** The engine combines those notes with what the consent platform
  reports (`TcfState`): `domain/consent_wait.rs` says per page view whether the scanner gave consent,
  someone else did (a person, in a visible browser) and when, or nobody. Where bidding on a page view
  waited for that, the info finding `bidding-waited-for-consent` says so, the same sentence is a note in
  the overview and a warning in the audit trail of the Heuristics step, and it is stated as fact in the
  Phase 2 prompt's visit block. The gate rule does not report "no auction ever started" unless Prebid
  had at least 5 s after it loaded and consent was given; late-auction and slow-GPT are counted from the
  consent choice.
- **What the page asks of Prebid.** The tracker reports every `requestBids` call (`bidsRequested`: the
  ad unit codes named, how many ad units Prebid had to auction, the timeout) through Prebid's own
  `requestBids.before` hook, and `addAdUnits` calls (`adUnitsAdded`); calls made before it subscribed
  are replayed from `pbjs.getEvents()` without those details. The rule
  `bids-requested-without-ad-units` (high) fires when bids were asked for with zero ad units and no
  auction ran on that page view: the wrapper dropped every ad unit, usually because it filtered out
  every bidder for this visitor. The gate rule stands down for it. Both events are in the Phase 2
  page timeline.
- **Back/forward cache.** When a person presses Back and Chrome restores the earlier document instead
  of loading it, that is a new page view (`userNavigation`, no `loadMs`) of the old document: what the
  document reports from then on is on the new page view, timed from the restore. Console lines and bid
  responses of the first few milliseconds after a restore can still land on the page view before.
- **Browser profile.** With "keep a browser profile" (`Settings.persistentProfile`, or per scan) Chrome runs
  in `<data dir>/scan-profile/{desktop,mobile}` and cookies, consent and cache survive between scans.
  One scan at a time uses it; a second waits at its Capture step. `get_scan_profile` /
  `clear_scan_profile` report and remove it (`profileBusy` while a scan uses it). Throwaway profiles
  that a crash left in the temp dir are swept when the app starts.
- **Chrome's socket directory.** Chrome keeps the socket that makes it the one browser on a profile
  in a directory of its own in the temp dir (`org.chromium.Chromium.XXXXXX` on the test Chromium)
  and links to it from the profile (`SingletonSocket`). It removes the directory when it exits; the
  scanner kills the browser of every scan on a throwaway profile, so each scan left one behind, in
  the app as in the tests. The directory the profile links to is now removed with the browser
  (`cdp/profile.rs::remove_singleton_directory`: the two files Chrome puts there, by name, then the
  directory if that emptied it): at shutdown, when a browser is dropped, by the watchdog, with an
  abandoned profile at the next start, and when a kept profile is next used. Measured on Linux:
  32 directories left per run of `tests/network.rs` before, 0 after, 0 over the whole browser
  suite. Not checked on a Mac, where the directory has another name; it is found through the link,
  not by name.
- Phase 2 knows about page views: the opening message has a visit block and the timelines of the entry
  page plus up to two page views with findings; `get_telemetry` takes a `page`, `report_finding` takes
  `pages` (numbered from 1 towards the model).
- Exports: Markdown gains a Visit block, a page-view table, "Seen on" per finding and per-page tables
  when the visit has more than one page view. A single-page scan stored before this exports as before.

## Proxies

Settings hold named proxy profiles (`Settings.proxies`: name, scheme http/https/socks5, host, port,
optional user name). A scan runs through none, the system's proxy (the default, as before) or one
profile: `Settings.defaultProxy`, or `ScanOptions.proxy` per scan.

- The password of a profile is kept in the app's one Keychain item and is only ever reported as
  `hasPassword`. It is
  handed to the browser outside the capture request (answered to the proxy's challenge over CDP, never
  on the command line) and to the app's own file fetcher.
- Chrome cannot sign in to a SOCKS5 proxy, so a SOCKS5 profile with a user name is refused.
- The site's ads.txt is read through the same proxy as the scan's pages. The sellers.json of ad
  exchanges are not: they are requested like the app's other traffic (the system's proxy, if any),
  whatever proxy, or "no proxy", the scan chose. The file does not depend on where it is fetched from,
  one copy serves scans with different proxies, and a metered proxy is not billed for megabytes it
  adds nothing to. A page on this machine (the demo site) is always reached directly.
- Stored scans keep the profile's id plus its name and host as they were (`options.proxy`,
  `summary.proxy`, `environment.proxy`), so they still read after the profile is renamed or deleted.
- A profile that a saved scan configuration uses cannot be deleted (the error names the configuration).
  If it is the settings default, deleting it puts the default back to the system's proxy.
- **Behind a proxy the app cannot fully verify where a request lands.** The fetcher's private-address
  check covers IP addresses and what this machine's resolver answers; the proxy resolves names itself,
  and the connection cannot be pinned to the checked addresses. This is stated in
  `infrastructure/public_files.rs` and the Settings copy must say it too.
- Not built: proxy pools, rotation, per-request proxies.

## Scan configurations

A saved setup to scan one site again and again (`ScanConfiguration`: name, the scan options including
URL, visitor, pages, headless, profile, consent, observation time, AI on/off, budget and proxy; never
a seed). SQLite table `scan_configurations`, use cases in `application/configurations.rs`.

- Names are unique without regard to case. Options are validated like a scan start when saved.
- `start_scan { configurationId }` runs the saved options settled against the settings of that moment,
  records `ScanSummary.configuration` (id and name as they were) and the configuration's last run.
  `list_scans` filters by `configurationId`; exports name the configuration.
- Deleting a configuration never deletes scans. Deleting a scan only clears the configuration's link
  to it.

## Bidder checks

A pipeline step of its own (`BidderDiagnostics`, between Telemetry and Heuristics) that answers "why
does this bidder not bid?" as far as the app can check by itself. It is skipped when the page has no
Prebid bidders. Its result is stored with the scan (`ScanDetail.bidderDiagnostics`, `null` for older
scans), shown under "By bidder" on the Auctions tab, available to Phase 2 as the `bidder_checks`
section of `get_telemetry`, and used by the rules. The "bidder never bids" finding names the most
likely cause per bidder, most certain first, and keeps the generic advice only for bidders where
every check passed or could not be made.

1. **Adapter params.** The tracker captures each ad unit's `bids[].params` from `auctionInit`. They
   are validated against a table of 24 adapters (`domain/diagnostics/adapters.rs`) whose required
   params and types were read from `isBidRequestValid` in Prebid.js 11.49.0: missing param, wrong
   type, the adapter documentation's test id, and one placement id shared by three or more
   differently sized ad units. Prebid announces such a bid in `bidRequested`, sends nothing and
   reports `noBid`, so a rejected bid looks like a bidder that declined. A bidder code that is not
   a built-in code or alias is not checked against the table, and neither is a bidder that
   `s2sConfig.bidders` hands to Prebid Server, whose param rules differ; for those, and for rules the
   table does not model, Prebid's own "Invalid bid sent to bidder" warning is used. A wrongly typed
   param is not held against a bid that another complete alternative of the adapter makes valid.
2. **Bid responses.** The browser crate recognises bid requests by endpoint host and path (same
   Prebid release) and keeps status, failure and a reason of at most 200 characters: a top-level
   `error` or `message`, AppNexus per-tag `error`, or the OpenRTB `nbr` code; an answer that carries
   bids (`seatbid`) has no reason. That reason is read in memory; the exchange itself is in the
   scan's archive when the scan keeps one. A path alone identifies an
   endpoint only at the own origin of a page served from this machine, which is how the demo site
   stands in for an exchange; on every real host the adapter's own host must match. Prebid's own
   `bidderError` events are used too.
3. **ads.txt.** One GET of `https://<registrable domain>/ads.txt` (public suffix list), parsed per
   the IAB ads.txt 1.1 specification. Each bidder's ad system domain must be listed; where the params
   carry the account id (appnexus `member`, rubicon `accountId`, pubmatic `publisherId`, medianet
   `cid`) or the request carries a supply chain, the id must be listed too. Only 404 and 410 count as
   "no ads.txt"; any other failure, a web page, binary data or an answer without a single seller
   record (an empty file, a soft 404) is "could not be read" and raises nothing. A repeated record
   counts once. A `SUBDOMAIN=` declaration for the scanned host is followed to that host's own file.
   A page served from this machine never gets the "site has no ads.txt" finding.
4. **sellers.json.** On by default (Settings, "Check sellers.json at ad exchanges"; the default
   changed on 2026-11-12, and migration 7 switched it on once for existing installs, because a
   stored "off" could not be told from the old default). The app
   reads `https://<ad system domain>/sellers.json` for the ad systems of the page's bidders that
   ads.txt lists, and looks up the account in use, or else the DIRECT records (at most ten per
   exchange): unknown seller id, confidential seller, seller type against DIRECT/RESELLER, seller
   domain against the site, OWNERDOMAIN and MANAGERDOMAIN. Only ad system domains of the adapter
   table are ever asked, never a name that is merely written in ads.txt, each must be a host name
   under a public suffix, and one scan asks at most twelve. It is never done for a page served from
   this machine, so the demo site cannot send the app to a real exchange. A file is kept between
   scans: see "sellers.json cache" below.
5. **Consent.** The tracker listens to `__tcfapi` and records, per state change, whether GDPR
   applies, whether there is a TC string, and the consented purposes and vendor ids. The tracker
   never reports the string itself; where it travels in a URL or a bid request, it is in the scan's
   archive like the rest of that request. From each `bidRequested` it records `gdprApplies` and whether a consent
   string was passed. Findings: auctions that started before the choice was known, bid requests
   without a consent string, and a bidder whose IAB vendor id (adapter `gvlid`, cross-checked against
   GVL v184) lacks consent.

The fetcher (`infrastructure/public_files.rs`, port `PublicFileFetcher`) reads https on port 443
only, refuses loopback (by address or by name), private, link-local, shared and IPv4-in-IPv6
addresses, credentials in the address and non-http schemes, pins the connection to the addresses
it checked on every hop (a name that did not resolve is never resolved a second time), follows
redirects by hand (any number within the root domain, one outside it, five in total), sends no
cookies, and stops at 1 MiB and 10 s for ads.txt and 24 MiB and 20 s for sellers.json, counted on
the decompressed bytes. The one exception is the scanned page's own origin when that
is on loopback, which is how the demo site's `/ads.txt` is read, over http and never through a proxy.
The site's ads.txt leaves through the scan's proxy choice, a sellers.json through the system's (see
"Proxies"). A request can name the version of the file it already has (`If-None-Match`,
`If-Modified-Since`); the answer is then 304 when the file is unchanged.

### sellers.json cache

An exchange's sellers.json is the same for every site and several megabytes large, so it is kept
between scans, one copy per exchange for all scans and sites. Built on 2026-11-12 together with the
new default; tested against test doubles and a mock server only, never against a real exchange.

- **Where.** `<data dir>/cache/sellers-json/`, directory mode 0700, files 0600. Per exchange two
  files named by the first 32 hex characters of the sha-256 of its ad system domain (never by the
  domain itself): `<key>.json.zst`, the file as received, zstd level 3, and `<key>.meta.json` with
  the domain, when it was fetched, the address that answered after redirects, `ETag` and
  `Last-Modified`, the sizes and the sha-256 of the content, or instead of a file the reason why
  there was none ("the exchange answered HTTP 404"). An entry exists once its meta file does; both
  files are written under a temporary name, synced and renamed, so a download that was cut short is
  never an entry. The start-up sweep removes temporary files and files without meta.
- **How long.** Constants in `domain/diagnostics/sellers_cache.rs`: a file is used without asking for
  24 hours; a failed lookup is remembered for 1 hour. After that the exchange is asked with the
  kept version; on 304 only the date moves. When the exchange cannot be reached (network error,
  timeout, HTTP 429 or 5xx) a file of at most 7 days is used, the audit trail says so and each
  check read from it ends with "(from a copy fetched <date>: the exchange could not be reached during
  this scan)". Nothing older than 7 days is ever used. Any other answer (404, 403) is believed and
  replaces the kept file.
- **Only a sellers.json is kept as one.** A 2xx answer is kept as a file only when the lookup's own
  parser can read it (`domain/diagnostics/sellers_json.rs::check_readable`, which is `find_sellers`
  asked for no id). A web page, a challenge page, a soft 404, JSON without a `sellers` array or a
  file that was cut short is handled like an exchange that could not be reached
  (`application/sellers_json.rs`): a kept file of at most 7 days is used and left exactly as it was,
  with the same note on its checks, and the exchange is asked again by the next scan; without such a
  file the answer is remembered as a failed lookup for 1 hour and the check reads "The sellers.json
  of <exchange> could not be read: the file is not valid sellers.json (...)", never "seller id not
  listed". A kept file is checked the same way every time it is read: one that is not a sellers.json
  (kept by a build from before 2026-11-13) counts as no entry, its version is not asked about, it
  is never used in place of an unreachable exchange, and the exchange's answer replaces it. A 304
  only renews a kept file: the cache refuses to move the date of a failed lookup
  (`LocalSellersJsonCache::confirm`), and a 304 to a request that named no version is a failed
  lookup ("the exchange answered HTTP 304").
- **Sharing.** Two scans that need the same exchange at the same time make one download
  (`application/sellers_json.rs`, one lookup per exchange at a time); the second reads the kept
  copy. A cancelled scan keeps nothing and the next one in line downloads the file itself.
- **Limits.** The 24 MiB cap per file applies to the download and again when a kept file is
  unpacked. All files together stay under 500 MiB; the entries read longest ago go first.
- **Trust.** A kept file is parsed by the same parser as a download, per scan: once to see that it
  is a sellers.json, once for the seller ids that scan needs. A file whose size or hash differs from its meta, or whose meta names another
  domain, is deleted and downloaded again. Every download and every revalidation goes through the
  fetcher above with all its address checks.
- **In the app.** Settings, "Bidder checks": size and number of exchanges, and "Clear kept files"
  (commands `get_sellers_json_cache`, `clear_sellers_json_cache`). "Delete all scans" and "Clear
  stored scan data" leave these files alone: they are not scan data. The audit trail of the Bidder
  checks step says per exchange whether the file was downloaded, confirmed unchanged, or a kept copy
  and how old.
- **Measured (test double, `tests/bidder_checks_pipeline.rs`):** a first scan of a page with two
  exchanges makes 3 requests (ads.txt and two sellers.json), a second scan 1. No real exchange was
  timed; note the scan time with the check on (see `docs/MAC-TESTING.md`).

Known limits:

- Left out of the tables on purpose: the ads.txt domain of `ttd`, and every account-id param that
  could not be confirmed (ix, openx, criteo, triplelift, seedtag, onetag and the rest). Those bidders
  get the domain check only.
- A site that answers unknown clients with 403 or a challenge page gets "ads.txt could not be read".
- Endpoint reasons other than the top-level `error` of AppNexus (confirmed in the adapter source)
  rest on the OpenRTB specification and Xandr's documentation, not on observed live answers.
- Behind a proxy (the system's or a profile) the proxy resolves names itself: the private-address
  check falls to that proxy and the connection is not pinned to the checked addresses.
- Aliases a publisher defines with `pbjs.aliasBidder`, Prebid Server bidders and non-TCF consent
  (GPP, US Privacy) are not checked.

## Jev System One (optional second provider)

One switch in Settings, off by default and only available while a valid TypeSafe key is stored. Saving
a key does not turn it on; removing the key turns it off. A scan decides at its start whether it uses
Jev. The switch enables two things:

- **Script triage.** Each undecided script is cut into parts of about 6,000 characters of raw source.
  The first part and every part naming an ad-tech identifier (at most 24 per script) go to Jev, one
  part per request, with two yes/no questions. The script's score is the highest "ad orchestration"
  probability: 0.50 or more is analysed (`jev-yes`), below 0.15 is dropped (`jev-no`). In between the
  second question decides: the publisher's own code ("publisher-specific" 0.50 or more) is kept and
  ranked last (`jev-unsure`), or kept as consent code when it configures a consent platform
  (`jev-consent`, category CMP; a consent script that is no known vendor's library goes to Phase 2);
  generic library code is dropped (`jev-generic`). Scripts the knowledge base decided are never
  sent, so a strong ad signature cannot be dropped by a probability. Identical bodies are asked
  about once. The band rule and the token estimate (2 characters per token, 300 per request) come
  from one real run (2026-11-12, 15 requests, 33,847 tokens, $0.0014, 0.65 s) and are provisional.
  Requests stay one part each: at that cost and latency batching scripts would gain nothing.
- **Reading guide.** After Beautify, chunks of the scripts selected for Phase 2 (about 160 lines each,
  at most 60 per scan) get eight yes/no questions. The ranked line ranges, and the best ones as
  numbered source, go into the Phase 2 opening message. Kept in memory only.

Fallback: the first Jev failure ends Jev for that scan. Scripts without a verdict go to the filter
model when an Anthropic key exists and stay with the knowledge base otherwise; the step logs a warning
and the overview gets a note. Scripts the budget could not pay for are never sent to the dearer model.
A failed reading guide only shortens the guide. With a TypeSafe key and no Anthropic key, scripts are
classified and Phase 2 is skipped.

Jev returns numbers and no reasons. The audit trail therefore records the question set once per step
and one `classification` entry per script: the probabilities, the thresholds, the rule that routed it
(`jev-yes`, `jev-unsure`, `jev-consent`, `jev-generic`, `jev-no`) and at most 300 characters of the best-scoring part. All script
triage excerpts of one scan together stay under 24,000 characters; entries after that keep their
numbers and no source.

Three Phase 2 cost levers apply to every AI scan, with or without Jev:

- The scripts handed to Phase 2 are ordered by relevance (publisher wrappers first, stock Prebid
  last), not by load order. That order decides which scripts make the cap of 12 and whose matches fill
  a search result.
- `search_scripts` without a script id skips the internals of stock Prebid builds and says how many
  matches it hid. Stretches of such a file that name one of the page's ad placements or call the ad
  stack (`requestBids`, `addAdUnits`, `defineSlot`, `googletag.cmd`, ...) stay searchable, so a
  wrapper bundled with Prebid is still found; the whole file stays readable by id.
- Publisher scripts that together stay under 400 beautified lines and 15,000 characters are inlined
  in the opening message, so a small wrapper costs no tool turns.

Not built, on purpose: Jev answers inside the deterministic rules, and a Jev-versus-Haiku compare
mode.

## Architecture in one screen

```
crates/model     shared types, serde camelCase, ts-rs export
crates/browser   detect/, cdp/, capture/, injected/tracker.js, injected/consent.js, demo-site/
crates/engine    domain/          pure: scan, pricing, telemetry, diagnostics (adapter table, ads.txt and
                                  sellers.json parsers, consent), heuristics (Rule trait), synthesis,
                                  archive (pack format, request document, redaction, kept kinds),
                                  network (classification, exchange matching, OpenRTB), source
                 application/     use cases, ports, pipeline graph and nodes, archive (recording,
                                  analysis, network view, script sources, storage), sellers.json
                                  source (kept copy or download)
                 infrastructure/  sqlite, anthropic and typesafe (reqwest, shared http retry),
                                  public file fetcher, sellers.json cache, keychain, oxc beautifier,
                                  pack store
src-tauri        composition root: state.rs, adapters.rs, commands.rs
src/             app/ (shell, routes), features/*, shared/, lib/api.ts, lib/fake-backend/
```

The engine depends on ports only (`ScanRepository`, `LogRepository`, `SettingsRepository`,
`ScanConfigurationRepository`, `SecretStore`, `SecretPresenceRepository`, `LlmClient`, `JudgeClient`, `CaptureSource`,
`PublicFileFetcher`, `SellersJsonCache`, `SourceFormatter`, `PackStore`, `ArchiveRepository`,
`EventSink`, `Clock`, `IdGenerator`), so it has no dependency on the browser crate or on Tauri. The frontend runs in a plain browser against
the fake backend, which is how Vitest and Playwright test it.

## Tauri commands

Defined in `src-tauri/src/commands.rs`. Arguments are camelCase JSON; all return `Result<T, CommandError>`.

| Command | Args | Returns |
|---|---|---|
| `get_settings` | – | `Settings` |
| `update_settings` | `{ patch }` | `Settings` |
| `set_api_key` | `{ key, provider? }` | `ApiKeyStatus` (stored only if that provider's API accepts it) |
| `clear_api_key` | `{ provider? }` | `null` (clearing the TypeSafe key also turns Jev off) |
| `detect_browser` | – | `BrowserInfo \| null` |
| `get_demo_url` | – | `string` |
| `start_scan` | `{ options }` or `{ configurationId }` | `ScanSummary` (work continues in the background) |
| `finish_capture` | `{ scanId }` | `null` (ends a manual visit and keeps what it recorded) |
| `cancel_scan` | `{ scanId }` | `null` |
| `list_scans` | `{ query }` | `ScanList` (`query.configurationId` narrows to one configuration) |
| `get_scan` | `{ scanId }` | `ScanDetail` |
| `delete_scan` | `{ scanId }` | `null` |
| `delete_all_scans` | – | `null` (stops every running scan first; a scan started meanwhile begins after the deletion and is whole) |
| `get_logs` | `{ scanId, stepId? }` | `LogEntry[]` |
| `search_logs` | `{ query, limit? }` | `LogSearchHit[]` |
| `get_dashboard_stats` | – | `DashboardStats` |
| `export_scan` | `{ scanId, format, path? }` | `string` |
| `get_scan_profile` | – | `ScanProfileInfo` |
| `clear_scan_profile` | – | `ScanProfileInfo` (`profileBusy` while a scan uses the profile) |
| `save_proxy_profile` | `{ profile }` | `Settings` (create, or update with `profile.id`) |
| `delete_proxy_profile` | `{ id }` | `Settings` |
| `set_proxy_password` | `{ id, password }` | `Settings` (stored as given, not checked against the proxy) |
| `clear_proxy_password` | `{ id }` | `Settings` |
| `list_scan_configurations` | – | `ScanConfiguration[]` |
| `save_scan_configuration` | `{ configuration }` | `ScanConfiguration` (create, or update with `configuration.id`) |
| `delete_scan_configuration` | `{ id }` | `null` |
| `list_network_requests` | `{ scanId, pages?, cursor?, limit? }` | `NetworkRowChunk` (time-ordered, 2,000 rows by default; no filtering) |
| `get_network_summary` | `{ scanId }` | `NetworkSummary` |
| `get_network_request` | `{ scanId, requestId }` | `NetworkRequestDetail` |
| `get_network_body` | `{ scanId, requestId, side, offset?, maxBytes? }` | `NetworkBodyChunk` |
| `get_script_source` | `{ scanId, scriptId, view, originalLine?, originalColumn? }` | `ScriptSource` |
| `get_archive_storage` | – | `ArchiveStorage` |
| `clear_archive` | – | `ArchiveStorage` (network data of all finished scans; reports stay) |
| `get_sellers_json_cache` | – | `SellersJsonCacheInfo` (`entries`, `storedBytes`) |
| `clear_sellers_json_cache` | – | `SellersJsonCacheInfo` (the kept sellers.json files; scans are not touched) |

`provider` is `"anthropic"` (the default when omitted) or `"typesafe"`.

The backend emits the Tauri event `scan-event` with a `ScanEvent` payload for every scan, step, log
and finding change.
