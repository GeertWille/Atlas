# AdTech Scanner — handover

Status on 2026-11-12. Branch `ccr-a6f7da93-wz5uos` in the Atlas repo, everything under `apps/adtech-scanner`.
No PR, nothing merged.

## What this is

A Tauri 2 desktop app that finds revenue leaks caused by logic bugs in a publisher's ad-tech wrapper.
One scan runs these pipeline steps:

1. **Capture**: launches local Chrome over CDP, injects a tracker before any page script, loads the page
   (desktop or mobile emulation), optionally accepts the consent banner and scrolls. Records Prebid
   activity, GPT slot lifecycle and timings, Amazon TAM calls and every JavaScript body.
2. **Telemetry, bidder checks and heuristics**: derives auctions, slots and metrics, verifies each
   bidder's setup (see "Bidder checks" below), then runs 27 deterministic rules. Works with no API key.
3. **Script triage (Phase 1)**: classifies the scripts the built-in knowledge base cannot decide.
   `claude-haiku-5-5` does it by default. With the optional "Classify scripts with Jev" setting on,
   TypeSafe's Jev System One (`jev-1.14.0`) does it, and Haiku is the fallback.
4. **Phase 2**: `claude-sonnet-5-5` reads the beautified ad scripts and telemetry in a tool loop and
   reports logic bugs that lose revenue. With Jev on it starts from a reading guide Jev built.
5. **Synthesize**: dedupes findings, scores ad-stack health, writes an overview.

Rules that came from you and must keep holding:

- Local only. A SQLite file in the app data dir, no server, no cloud storage. Outbound traffic is the
  scanned site (through Chrome) and that site's ads.txt (one GET from the app itself),
  api.anthropic.com, api.typesafe.ai only while Jev is on, and, only while "Check sellers.json at ad
  exchanges" is on, a GET of sellers.json at the ad-system domains that ads.txt names.
- No raw payloads stored. Script bodies, bid responses, ads.txt and sellers.json stay in memory; only
  findings, derived telemetry, the bidder check results (with excerpts of at most 300 characters) and
  the audit trail (steps plus AI debug logs, full-text indexed) are persisted.
- API keys live in the macOS Keychain (service `com.webclew.adtech-scanner`, accounts
  `anthropic-api-key` and `typesafe-api-key`), never in SQLite, logs or the frontend.
- Per-scan LLM budget cap, default $0.50.
- `clew-app` is read-only inspiration.
- Run a simplify pass and a review pass after every change, fix, repeat.

## State of each part

| Part | Path | State |
|---|---|---|
| Data contract | `crates/model` | Done. Generates `src/bindings/*.ts` (never hand-edit). |
| Browser capture and demo site | `crates/browser` | Done, reviewed, fixes applied. 52 unit and 29 integration tests pass. |
| Engine (domain, pipeline, SQLite, Anthropic and TypeSafe clients) | `crates/engine` | Done, reviewed, fixes applied. Tests pass, all against mock servers. |
| Tauri shell (16 commands, composition root) | `src-tauri` | Done, reviewed. Command tests pass; 2 real-browser pipeline tests pass on Linux. |
| React frontend | `src/` | **In progress.** Feature slices, shell and fake backend exist. Not yet confirmed to typecheck, no e2e tests yet, never reviewed. |
| Integration of frontend with the real backend | `e2e-tauri/` | Done on Linux. The built app (production CSP) is driven through WebKitGTK by `pnpm test:tauri`: onboarding, demo-site scan, every report tab, history, log search, settings, export and the Chrome picker through the native dialogs, delete, an AI scan against a mock of the Anthropic API, a Jev scan and its fallback against a mock of the TypeSafe API, and every command's contract. Not run on macOS. |

The checkpoint commit that adds this file contains the frontend exactly as the build agent left it
mid-work. Expect `pnpm typecheck` and `pnpm lint` to fail until the frontend is finished.

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

`pnpm test:tauri` needs `tauri-driver` (`cargo install tauri-driver --locked`), `WebKitWebDriver`
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

The built-in demo publisher page is the fastest way to see a full scan. It is served on a random
loopback port and has seven planted bugs, documented in `crates/browser/demo-site/README.md`:

- Scroll gate uses `min-width` where `max-width` was meant, so desktop never auctions before scroll.
- The anchor slot is requested before the auction ends, without `hb_*` keys.
- Bidder timeout is 250 ms instead of 1200 ms, so two bidders always time out.
- A ghost ad unit with no container, and GPT loaded twice.
- `ix` is called but has no line in the site's `/ads.txt`, and never bids.
- `triplelift` has no `inventoryCode` on two of its three ad units, so Prebid drops those bids.
- The `openx` bid request is answered by the demo exchange with OpenRTB no-bid reason 7.

## What is left, in order

1. **Finish the frontend.** Get `pnpm typecheck`, `pnpm lint`, `pnpm test` green; write the Playwright
   e2e suite against the fake backend (`src/lib/fake-backend`); `e2e/` is empty.
2. **Integrate.** Done on Linux (`pnpm test:tauri`). On the Mac, still check by hand: the save and
   open dialogs, external links opening in the browser, dragging and double-click zoom of the title
   bar, and that a scan's Chrome is gone after quitting mid-scan.
3. **Icons.** Done. The source is `src-tauri/icons/icon.svg`; after changing it, render it to a
   1024 px PNG and run `pnpm tauri icon <png>`.
4. **Simplify and review** the frontend, then the integrated app. Fix and repeat until clean.
5. **Visual polish** of every screen in light and dark, at 1024x680 and 1280x820.
6. **First real API run.** Enter a key in Settings and scan the demo site, then a real publisher page.
7. **README** with the run instructions above.

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
- Whether a second Keychain item makes macOS prompt a second time.

## Known open issues

- Turning on Prebid debug logging (`setConfig({debug:true})`) has minor side effects on real sites.
- A cancelled step shows as Failed with summary "Cancelled"; there is no cancelled step status.
- `delete_all_scans` can race with a scan started at the same moment.
- Keychain read errors are reported as "no API key configured".
- Very small scripts can fit entirely inside one stored excerpt.
- The Prebid-age rule flags major version 7 and below; that threshold was not checked against current
  Prebid releases.

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
   differently sized ad units. A bidder code that is not a built-in code or alias is not checked.
2. **Bid responses.** The browser crate recognises bid requests by endpoint host and path (same
   Prebid release) and keeps status, failure and a reason of at most 200 characters: a top-level
   `error` or `message`, AppNexus per-tag `error`, or the OpenRTB `nbr` code. Bodies are read in
   memory and dropped. Prebid's own `bidderError` events are used too.
3. **ads.txt.** One GET of `https://<registrable domain>/ads.txt` (public suffix list), parsed per
   the IAB ads.txt 1.1 specification. Each bidder's ad system domain must be listed; where the params
   carry the account id (appnexus `member`, rubicon `accountId`, pubmatic `publisherId`, medianet
   `cid`) or the request carries a supply chain, the id must be listed too. Only 404 and 410 count as
   "no ads.txt"; any other failure, a web page or binary data is "could not be read" and raises
   nothing. A `SUBDOMAIN=` declaration for the scanned host is followed to that host's own file.
4. **sellers.json.** Off by default (Settings, "Check sellers.json at ad exchanges"). When on, the app
   requests `https://<ad system domain>/sellers.json` for the ad systems of the page's bidders that
   ads.txt lists, and looks up the account in use, or else the DIRECT records (at most ten per
   exchange): unknown seller id, confidential seller, seller type against DIRECT/RESELLER, seller
   domain against the site, OWNERDOMAIN and MANAGERDOMAIN. It is never done for a page served from
   this machine, so the demo site cannot send the app to a real exchange.
5. **Consent.** The tracker listens to `__tcfapi` and records, per state change, whether GDPR
   applies, whether there is a TC string, and the consented purposes and vendor ids. The string
   itself is never captured. From each `bidRequested` it records `gdprApplies` and whether a consent
   string was passed. Findings: auctions that started before the choice was known, bid requests
   without a consent string, and a bidder whose IAB vendor id (adapter `gvlid`, cross-checked against
   GVL v184) lacks consent.

The fetcher (`infrastructure/public_files.rs`, port `PublicFileFetcher`) reads https only, refuses
loopback, private and link-local addresses, credentials in the address and non-http schemes, pins
the connection to the addresses it checked, follows redirects by hand (any number within the root
domain, one outside it, five in total), sends no cookies, and stops at 1 MiB and 10 s for ads.txt
and 24 MiB and 20 s for sellers.json. The one exception is the scanned page's own origin when that
is on loopback, which is how the demo site's `/ads.txt` is read, over http and never through a proxy.

Known limits:

- Left out of the tables on purpose: the ads.txt domain of `ttd`, and every account-id param that
  could not be confirmed (ix, openx, criteo, triplelift, seedtag, onetag and the rest). Those bidders
  get the domain check only.
- A site that answers unknown clients with 403 or a challenge page gets "ads.txt could not be read".
- Endpoint reasons other than the top-level `error` of AppNexus (confirmed in the adapter source)
  rest on the OpenRTB specification and Xandr's documentation, not on observed live answers.
- Behind a proxy that resolves names itself, the private-address check falls to that proxy.
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

Not built, on purpose: any switch that writes script bodies to disk, Jev answers inside the
deterministic rules, and a Jev-versus-Haiku compare mode.

## Architecture in one screen

```
crates/model     shared types, serde camelCase, ts-rs export
crates/browser   detect/, cdp/, capture/, injected/tracker.js, injected/consent.js, demo-site/
crates/engine    domain/          pure: scan, pricing, telemetry, diagnostics (adapter table, ads.txt and
                                  sellers.json parsers, consent), heuristics (Rule trait), synthesis
                 application/     use cases, ports, pipeline graph and nodes
                 infrastructure/  sqlite, anthropic and typesafe (reqwest, shared http retry),
                                  public file fetcher, keychain, oxc beautifier
src-tauri        composition root: state.rs, adapters.rs, commands.rs
src/             app/ (shell, routes), features/*, shared/, lib/api.ts, lib/fake-backend/
```

The engine depends on ports only (`ScanRepository`, `LogRepository`, `SettingsRepository`,
`SecretStore`, `LlmClient`, `JudgeClient`, `CaptureSource`, `PublicFileFetcher`, `SourceFormatter`,
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
| `start_scan` | `{ options }` | `ScanSummary` (work continues in the background) |
| `cancel_scan` | `{ scanId }` | `null` |
| `list_scans` | `{ query }` | `ScanList` |
| `get_scan` | `{ scanId }` | `ScanDetail` |
| `delete_scan` | `{ scanId }` | `null` |
| `delete_all_scans` | – | `null` |
| `get_logs` | `{ scanId, stepId? }` | `LogEntry[]` |
| `search_logs` | `{ query, limit? }` | `LogSearchHit[]` |
| `get_dashboard_stats` | – | `DashboardStats` |
| `export_scan` | `{ scanId, format, path? }` | `string` |

`provider` is `"anthropic"` (the default when omitted) or `"typesafe"`.

The backend emits the Tauri event `scan-event` with a `ScanEvent` payload for every scan, step, log
and finding change.
