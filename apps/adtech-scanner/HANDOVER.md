# AdTech Scanner — handover

Status on 2026-11-12. Branch `ccr-a6f7da93-wz5uos` in the Atlas repo, everything under `apps/adtech-scanner`.
No PR, nothing merged.

## What this is

A Tauri 2 desktop app that finds revenue leaks caused by logic bugs in a publisher's ad-tech wrapper.
One scan runs these pipeline steps:

1. **Capture**: launches local Chrome over CDP, injects a tracker before any page script, loads the page
   (desktop or mobile emulation), optionally accepts the consent banner and scrolls. Records Prebid
   activity, GPT slot lifecycle and timings, Amazon TAM calls and every JavaScript body.
2. **Telemetry and heuristics**: derives auctions, slots and metrics, then runs 18 deterministic rules.
   Works with no API key.
3. **Phase 1 ("Jeff")**: `claude-haiku-5-5` classifies which scripts are ad-related.
4. **Phase 2**: `claude-sonnet-5-5` reads the beautified ad scripts and telemetry in a tool loop and
   reports logic bugs that lose revenue.
5. **Synthesize**: dedupes findings, scores ad-stack health, writes an overview.

Rules that came from you and must keep holding:

- Local only. A SQLite file in the app data dir, no server, no cloud storage.
- No raw payloads stored. Script bodies stay in memory; only findings, derived telemetry and the audit
  trail (steps plus AI debug logs, full-text indexed) are persisted.
- The API key lives in the macOS Keychain, never in SQLite, logs or the frontend.
- Per-scan LLM budget cap, default $0.50.
- `clew-app` is read-only inspiration.
- Run a simplify pass and a review pass after every change, fix, repeat.

## State of each part

| Part | Path | State |
|---|---|---|
| Data contract | `crates/model` | Done. Generates `src/bindings/*.ts` (never hand-edit). |
| Browser capture and demo site | `crates/browser` | Done, reviewed, fixes applied. 52 unit and 29 integration tests pass. |
| Engine (domain, pipeline, SQLite, Anthropic client) | `crates/engine` | Done, reviewed, fixes applied. 133+ tests pass. |
| Tauri shell (16 commands, composition root) | `src-tauri` | Done, reviewed. Command tests pass; 2 real-browser pipeline tests pass on Linux. |
| React frontend | `src/` | **In progress.** Feature slices, shell and fake backend exist. Not yet confirmed to typecheck, no e2e tests yet, never reviewed. |
| Integration of frontend with the real backend | – | **Not started.** The real app window has never been opened. |

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
```

Environment overrides for tests and local runs:

- `ADSCAN_DATA_DIR`: directory for the SQLite file.
- `ADSCAN_ANTHROPIC_BASE_URL`: replaces `https://api.anthropic.com` (must be https unless loopback).
- `ADSCAN_API_KEY`: use this key and skip the Keychain.
- `ADSCAN_CHROME_PATH`: browser executable.

The built-in demo publisher page is the fastest way to see a full scan. It is served on a random
loopback port and has four planted bugs, documented in `crates/browser/demo-site/README.md`:

- Scroll gate uses `min-width` where `max-width` was meant, so desktop never auctions before scroll.
- The anchor slot is requested before the auction ends, without `hb_*` keys.
- Bidder timeout is 250 ms instead of 1200 ms, so two bidders always time out.
- A ghost ad unit with no container, and GPT loaded twice.

## What is left, in order

1. **Finish the frontend.** Get `pnpm typecheck`, `pnpm lint`, `pnpm test` green; write the Playwright
   e2e suite against the fake backend (`src/lib/fake-backend`); `e2e/` is empty.
2. **Integrate.** Open the real app and run a demo-site scan end to end. Check every command and the
   `scan-event` stream against the real backend, the export flow (the path must come from the save
   dialog, `.json`, `.md` or `.markdown` only), the Chrome file picker, external links, the drag region
   and the production CSP with the built `dist/`.
3. **Replace the placeholder icons**: `pnpm tauri icon <1024px png>`.
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
- The real cost per scan against the $0.50 target.

## Known open issues

- Turning on Prebid debug logging (`setConfig({debug:true})`) has minor side effects on real sites.
- A cancelled step shows as Failed with summary "Cancelled"; there is no cancelled step status.
- `delete_all_scans` can race with a scan started at the same moment.
- Keychain read errors are reported as "no API key configured".
- Very small scripts can fit entirely inside one stored excerpt.
- The Prebid-age rule flags major version 7 and below; that threshold was not checked against current
  Prebid releases.

## Architecture in one screen

```
crates/model     shared types, serde camelCase, ts-rs export
crates/browser   detect/, cdp/, capture/, injected/tracker.js, injected/consent.js, demo-site/
crates/engine    domain/          pure: scan, pricing, telemetry, heuristics (Rule trait), synthesis
                 application/     use cases, ports, pipeline graph and nodes
                 infrastructure/  sqlite, anthropic (reqwest), keychain, oxc beautifier
src-tauri        composition root: state.rs, adapters.rs, commands.rs
src/             app/ (shell, routes), features/*, shared/, lib/api.ts, lib/fake-backend/
```

The engine depends on ports only (`ScanRepository`, `LogRepository`, `SettingsRepository`,
`SecretStore`, `LlmClient`, `CaptureSource`, `SourceFormatter`, `EventSink`, `Clock`, `IdGenerator`),
so it has no dependency on the browser crate or on Tauri. The frontend runs in a plain browser against
the fake backend, which is how Vitest and Playwright test it.

## Tauri commands

Defined in `src-tauri/src/commands.rs`. Arguments are camelCase JSON; all return `Result<T, CommandError>`.

| Command | Args | Returns |
|---|---|---|
| `get_settings` | – | `Settings` |
| `update_settings` | `{ patch }` | `Settings` |
| `set_api_key` | `{ key }` | `ApiKeyStatus` (stored only if the API accepts it) |
| `clear_api_key` | – | `null` |
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

The backend emits the Tauri event `scan-event` with a `ScanEvent` payload for every scan, step, log
and finding change.
