I could not save the plan to `/tmp/claude-0/-home-user/ee93eb55-ef69-5991-bc2c-57c8a322dcdf/scratchpad/browserbase/01-plan.md`: this session is read-only and forbids creating files anywhere, including the scratchpad. No docs were downloaded to disk either (they were read from stdout). The full plan is below; please save it from here.

Nothing was built or run. The Browserbase facts come from docs.browserbase.com fetched today (2026-11-12); anything from memory or inferred is marked.

# Browserbase as a browser backend: research, design and plan

## 0. The three findings that shape everything

1. **Paid plans have an undocumented "Basic" identity level with no API switch.** The plans page lists "Identity & bot detection: how well your sessions avoid bot detection" as Free = No, Developer = Basic, Startup = Basic, Scale = Verified. The create-session guide still lists "Automatic fingerprinting (devices, locales, operating systems)". The current API schema has no `fingerprint` field. Nowhere do the docs say what `navigator.webdriver` or the user agent report. So I cannot confirm from documentation that a paid-plan session is honest. It must be measured, and the integration must fail closed.
2. **`solveCaptchas` defaults to true** and recording and logging default to on. Logging stores CDP events, including network requests and responses with `rawBody`. All three must be sent as `false` on every session.
3. **Without `keepAlive`, a session ends when the WebSocket closes.** That is the cloud equivalent of today's "pipe closes, Chrome exits", so app death releases the session. The price is no reconnection in v1.

## 1. Research: API

Sources are listed at the end. Base URL is `https://api.browserbase.com`, authenticated with the header `X-BB-API-Key`.

**Create a session: `POST /v1/sessions`**

| Field | Documented behaviour |
|---|---|
| `projectId` | Optional; inferred from the key when absent. |
| `region` | `us-west-2` (default, Oregon), `us-east-1` (Virginia), `eu-central-1` (Frankfurt), `ap-southeast-1` (Singapore). |
| `timeout` | Seconds, 60 to 21600. Defaults to the project's `defaultTimeout`. |
| `keepAlive` | Paid plans only. Session survives disconnects until timeout or release. |
| `userMetadata` | JSON object, at most 512 characters, no arrays, string values for querying. |
| `browserSettings.viewport` | `{width, height}`. Ignored for Verified sessions. |
| `browserSettings.context` | `{id, persist}`; `persist` defaults to false. |
| `browserSettings.blockAds` | Default false. |
| `browserSettings.solveCaptchas` | Default **true**. |
| `browserSettings.recordSession`, `logSession` | Default **true**. Can be set false on every plan. |
| `browserSettings.verified` | Default false; Scale plan only. `advancedStealth` is its deprecated name. |
| `browserSettings.os` | Verified only; an error otherwise. Non-Verified sessions "always use linux". |
| `proxies` | `true`, or an array of `browserbase` (`geolocation {country, state, city}` or `proxyId`), `external` (`server`, `username`, `password`) and `none`, each with an optional `domainPattern`. Default is no proxy. |
| Others | `labels`, `extensionId`, `allowedDomains`, `ignoreCertificateErrors`, `riskPolicyId`, `proxySettings.caCertificates`, `captchaImageSelector`, `captchaInputSelector`. |

The 201 response is the session plus `connectUrl`, `seleniumRemoteUrl` and `signingKey`. The session carries `id`, `status` (`PENDING`, `RUNNING`, `ERROR`, `TIMED_OUT`, `COMPLETED`), `startedAt`, `endedAt`, `expiresAt`, `proxyBytes`, `region`, `contextId`, `userMetadata`.

**Connecting and ending**
- Connect a CDP client to `connectUrl` (`wss://connect.browserbase.com/...`). Integration examples also show `wss://connect.browserbase.com?apiKey=…&sessionId=…`. Either way the URL is a credential.
- You have 5 minutes to connect after creation.
- The connection can close after 10 minutes with no data in either direction; the docs say to send a command every 5 minutes.
- Release with `POST /v1/sessions/{id}` and body `{"status":"REQUEST_RELEASE"}`. `Browser.close` or a disconnect (without `keepAlive`) also ends it, as `COMPLETED`.
- `GET /v1/sessions?status=RUNNING&q=user_metadata['k']:'v'` lists sessions; `GET /v1/sessions/{id}` reads one.

**Limits and pricing (plans page)**

| | Free | Developer $20 | Startup $99 | Scale |
|---|---|---|---|---|
| Concurrent browsers | 3 | 25 | 100 | 250+ |
| Sessions created per minute | 5 | 25 | 50 | 150+ |
| Session duration | 15 min | 6 h | 6 h | 6+ h |
| Browser hours included / overage | 1 h / none | 100 h / $0.12 per h | 500 h / $0.10 per h | custom |
| Proxy included / overage | 0 GB / none | 1 GB / $12 per GB | 5 GB / $10 per GB | usage-based |
| Captcha solving | No | Auto | Auto | Auto |
| Identity | No | Basic | Basic | Verified |
| Data retention | 7 days | 7 days | 30 days | 30+ days |

- Billing is per minute and per MB, with a one-minute and one-MB minimum per session.
- Exceeding either limit returns 429 with `x-ratelimit-limit`, `x-ratelimit-remaining`, `x-ratelimit-reset` and `retry-after`.
- Concurrency is per organization, split across projects.

**Other endpoints**
- **Projects:** `GET /v1/projects`, `GET /v1/projects/{id}` (name, `defaultTimeout`, `concurrency`), `GET /v1/projects/{id}/usage` (`browserMinutes`, `proxyBytes`). The API does not expose the plan name or remaining allowance.
- **Live view:** `GET /v1/sessions/{id}/debug?expiresIn=` returns `debuggerFullscreenUrl`, `debuggerUrl`, `pages[]` (one URL per tab) and `wsUrl`. It is interactive (click, type, scroll) and meant to be embedded in an iframe or opened in a browser.
- **Contexts:** `POST /v1/contexts {name?}`, `GET` and `DELETE /v1/contexts/{id}`.
  - A context stores the Chromium user data directory (cookies, localStorage, IndexedDB, service workers, preferences) but not the HTTP cache.
  - Contexts live indefinitely until deleted.
  - Avoid two sessions on one context at once; wait a few seconds after a `persist: true` session ends before reusing it.

**What Browserbase stores**
- A video of every session by default, kept up to 30 days.
- Session logs by default: CDP events, console, and network requests and responses.
- Both are off with `recordSession: false` and `logSession: false` on every plan; live view still works.
- Downloads are synced to Browserbase cloud storage.
- Account-wide zero data retention and bring-your-own storage are Enterprise features.
- Session metadata (id, times, region, `userMetadata`, `proxyBytes`) is kept regardless; the docs do not enumerate it or give its retention beyond the plan table.

## 2. Research: CDP over their WebSocket

**Documented**
- Playwright `connectOverCDP` and Puppeteer `connect` are the supported clients.
- "Always use the default context and page when possible".
- Sessions bound to a navigation policy reject `Target.createBrowserContext`.
- The session viewport is set at creation.
- Each interaction costs round trips; picking a near region "may produce an 8-9x gain".
- With a Browserbase proxy, the browser timezone is automatically aligned to the proxy's location.

**Inferred, not documented**
- The scanner's methods are the same ones Playwright and Puppeteer rely on, so they should work: `Target.setAutoAttach` with flatten, `Fetch`, `Network` on child sessions, `Runtime.addBinding`, `Page.addScriptToEvaluateOnNewDocument`, `Input`, `Emulation`, `Debugger`.
- The scanner never creates contexts or targets, so the default-page rule costs nothing.

**Not in the docs; a spike with a real account must answer these**
- WebSocket message and frame size limits, compression, and whether WebSocket pings count as "data".
- Whether the browser runs `--headless`.
- Whether Browserbase adds its own page targets, injected scripts or console lines (the docs mention `browserbase-solving-*` and "Starting recording" console messages).
- Whether `Browser.setDownloadBehavior: deny` is honoured.
- Memory available for the 256 MiB `Network.enable` buffer.
- Error body shapes and status codes other than 429 and one 400.

**Consequences for this app**
- **Body volume.** 10 to 18 MB per page view crosses the WAN, base64-inflated for binary bodies. The 3 s `BODY_PATIENCE` in `capture/mod.rs` and the 60 s `CALL_TIMEOUT` in `cdp/client.rs` need remote values.
- **Timing perturbation.** Every iframe or worker that starts paused (`waitForDebuggerOnStart`) waits at least one round trip before it runs. So does every paused document request on browse visits. Local scans pay near zero. Remote scans will show slightly later iframe and render timings; in-page auction timers are unaffected. Record the measured round-trip time with the scan and say so in the report.
- **Region is two things at once:** latency to the user and, without a proxy, where the visit exits. The UI must say that.
- **Loopback.** A cloud browser cannot reach `127.0.0.1`, `localhost`, private ranges or `.local` names.
  - The engine refuses such a URL for Browserbase at validation.
  - The composer disables the Browserbase choice for it, with the line "This address is on this Mac; a cloud browser cannot reach it."
  - "Try the demo" always runs locally, whatever the default. No tunnel is offered.

## 3. Research: automation and stealth, feature by feature

| Feature | Default | Can it be turned off? | Verdict |
|---|---|---|---|
| `verified` / `advancedStealth` (purpose-built Chromium with real fingerprints recognised by "bot protection partners") | Off | Yes: do not send it. Scale plan only. | Never send. |
| `solveCaptchas` | **On** | Yes, `false`. Plans table says "No" on Free. | Always send `false`. |
| `blockAds` | Off | Yes | Always send `false` explicitly. |
| `fingerprint` (randomised fingerprints) | Not in the current API | No field exists | From memory: older docs had this field and a "Basic Stealth Mode" that was on by default. Current docs only leave the traces described in section 0. |
| "Basic" identity on Developer and Startup | Appears always on | **No documented switch** | Unknown content. Must be measured. |
| Built-in proxies | Off | Yes. Not on Free (0 GB). | They are residential exits from third-party providers. Geolocation is "best effort"; `proxies: true` may leave through Canada instead of the US. The docs do not say the exit IP stays fixed for a session. A fixed IP exists only as static proxies on Scale. One page says "Rotate residential proxies". |
| Timezone alignment with a Browserbase proxy | Automatic | Not documented | An environment alteration; record it. |
| Web Bot Auth (signed requests that identify the agent) | Beta, by request | n/a | The honest direction, but paired with Verified. Not for v1. |

`navigator.webdriver` and the default user agent are not documented anywhere. From memory, community reports say Browserbase sessions show `webdriver` as false; I could not verify this.

**Recommended session body** (every scripted scan, every plan):

```json
{
  "projectId": "<from vault>",
  "region": "<chosen>",
  "timeout": "<computed cap, seconds>",
  "keepAlive": false,
  "browserSettings": {
    "blockAds": false,
    "solveCaptchas": false,
    "recordSession": false,
    "logSession": false,
    "viewport": { "width": 0, "height": 0 }
  },
  "userMetadata": { "app": "adscan", "install": "<uuid>", "scan": "<uuid>" }
}
```

- `viewport` carries the device window size.
- Never send `verified`, `advancedStealth`, `os`, `extensionId`, or the captcha selectors.
- `proxies` is absent unless the scan uses a proxy (section 4a).
- `context` is present only for scan-profile scans.
- The metadata holds random ids only, never the scanned URL or host.

**Residential exits are themselves a disguise.** Invalid-traffic detection leans on datacenter IP lists, so a residential exit makes an automated visit look like a household. With no proxy, the visit leaves from Browserbase's cloud region, which is honest and gives four locations.

**Fail closed.** Before navigating, the capture reads `navigator.webdriver` and `navigator.userAgent` on the session's blank page, in the main world and in the scanner's isolated world. It compares the user agent with `Browser.getVersion`.
- The existing probe in `capture/environment.rs` reads only the isolated world, which would miss a provider patch injected into the main world.
- If a scripted visit does not report `webdriver === true`, the scan stops before any page loads and the session is released. The error says Browserbase hides the automation flag on this account.
- Cost of a refused scan: one billed minute.

## 4. Design

### a. Browser crate

**Transport seam.** `Cdp::start` in `cdp/client.rs` takes byte streams and frames them with `pipe.rs`.
- Move the seam to message level: a reader that yields one message and a writer that sends one.
- `pipe.rs` implements both with NUL framing, as today. A new `cdp/websocket.rs` implements them with text frames.
- `route`, `send` and the tests stay unchanged.

**WebSocket client.**
- Do the HTTP/1.1 upgrade with `reqwest` (0.13, same features as the engine: rustls, system-proxy), then wrap the upgraded stream with `tokio-tungstenite`'s `WebSocketStream::from_raw_socket`.
- This gives rustls with the platform verifier and the system proxy for the app's own connection, with no second TLS or proxy stack.
- `reqwest-websocket` packages the same idea; I did not check whether it supports reqwest 0.13.
- Raise tungstenite's defaults (64 MiB message, 16 MiB frame) to 128 MiB for both. A 32 MiB body is about 43 MiB in base64.
- No compression: tokio-tungstenite has none (from memory), and Browserbase documents none.
- The reader task never blocks on consumers (unbounded channel, as today), so pings are always answered.
- Send a ping every 30 s and treat 90 s of silence as `Closed`. Send `Browser.getVersion` when 4 minutes pass without traffic.
- `connectUrl` must be `wss://` on a `browserbase.com` host unless overridden for tests. It is never logged and never placed in an error.

**Backend.** A two-variant enum in `cdp/`, not a trait object: `Local(Chrome)` and `Browserbase(RemoteSession)`. It offers:
- `connect()` returning `(Cdp, events)`.
- `close(keep_state)`.
- Facts: whether it has its own window, whether it keeps a profile, the clock offset, the browser version.

`capture::run` in `capture/mod.rs` branches once, where it calls `Chrome::spawn` today. `Visit::scan`, `Page`, the recorder, the visitor driver, consent and interception are untouched, except:
- no `Page.bringToFront` on remote;
- the viewport is always overridden on remote;
- remote patience values;
- `detect_browser` is skipped on remote.

**REST client.** A new `cdp/browserbase.rs` in the browser crate holds create, get, release, list, debug, contexts, project and usage. It handles 429 by honouring `retry-after` for up to 60 s. Session create and release are this backend's spawn and kill, so they belong next to `chrome.rs`.

**Lifecycle.**
1. Create with the body above. Write a row to a local ledger first (below).
2. Connect within the 5-minute window, run the pre-navigation probe, then scan as today.
3. End: send `Browser.close`, then `REQUEST_RELEASE` with three retries. This runs in the capture's own task, which already survives the caller dropping the future.
4. Read the session once more for `startedAt`, `endedAt` and `proxyBytes`.

Three safety nets:
- No `keepAlive`: app death or a dropped socket ends the session server-side.
- The server-side `timeout`.
- A startup sweep.

The sweep uses a small SQLite table of created and released session ids (not secrets). At startup, only if unreleased rows exist, the app reads the key and releases those sessions.
- A clean start never touches the keychain.
- Another machine on the same account is never touched; `install` in the metadata is a second guard.

**Timeout value.**
- Scripted: startup allowance plus pages times (load allowance plus observe seconds), plus margin, clamped to 60 to 900 s.
- Manual, if built: 15 minutes plus margin, which exceeds the Free plan's 15-minute session limit.
- The brief says a per-scan time cap exists; I did not locate it in the code. The timeout should derive from it.

**Mapping of local-only features**

| Local feature | On Browserbase |
|---|---|
| Kept scan profile per device | One Context per device, created on first use, `persist: true`. One scan at a time per context; hold the turn 5 s after release. Clear means `DELETE` both and forget the ids. |
| Proxy: system | Meaningless in the cloud. Resolves to "no proxy" and the summary says so. |
| Proxy: named profile | Sent as an `external` proxy. The password leaves the machine to Browserbase; disclose. Refuse a proxy host on loopback or a private range. Keep refusing SOCKS5 with a user name. |
| Proxy: Browserbase location | New choice: country, optional state and city. Residential; see decision 3. |
| Headless setting | Hidden. Recorded as observed from the browser's user agent. |
| Device emulation, consent click, visitor driver, link following, download refusal | Unchanged. |
| Manual visit | Possible through the live view. The backend fetches the URL and opens it with the opener plugin from Rust, so the URL never reaches the frontend or the logs. Each tab has its own URL. See decision 5. |
| ads.txt and sellers.json fetch | Through the named proxy when one is used; otherwise from this machine. It cannot go through Browserbase's network. State this in the UI. |

**Environment record.** `CaptureEnvironment` and `ScanEnvironment` gain a `browser` field: `local`, or `browserbase` with:
- region, session id, browser version;
- proxy kind and requested country;
- the settings sent: captcha solving off, ad blocking off, recording, logging;
- round-trip time, billed minutes and proxy bytes;
- observed facts: `webdriver` in both worlds, user agent against the version's user agent, timezone.

A new rule fires when a scripted remote visit did not report itself as automated.

**Time base.** Today the timeline mixes this machine's clock (`epoch_ms_now()` in `capture/recorder.rs`: page commits, script parse times) with the browser's (`wallTime`, tracker `timeOrigin`). With a remote browser they differ by skew plus latency.
- At connect, estimate the offset from a few `Date.now()` round trips and keep the lowest round-trip sample.
- Route every `epoch_ms_now()` in the capture through a capture clock that adds the offset. It is zero for local.
- Arrival-stamped events keep an error of about half a round trip. Record the round-trip time.

### b. Settings and scan options

**Contract (additive)**
- `ScanOptions.browser?`: `{kind:"local"}` or `{kind:"browserbase", region?}`. Absent means the settings default. Stored settled.
- `ProxyChoice` gains `browserbaseLocation {country, state?, city?}`, valid only with Browserbase.
- `CaptureRequest.browser`, defaulting to local.
- `Settings.defaultBrowser`.
- `Settings.browserbase`:
  - `region`;
  - `recordSession` and `logSession`, both false;
  - `defaultProxy`;
  - read-only `connected`, `projectName`, `concurrency`.

**Secrets**
- Add two `SecretName` variants: API key and project id, both in the existing vault item, as asked. The frontend only sees `connected` and the project's name.
- The key reaches the capture the way `proxy_password` does, through `CaptureSession` and `CaptureHooks`, never in the request.

**Commands**
- `set_browserbase_credentials {apiKey, projectId?}`: validated with `GET /v1/projects` (free); adopts the only project when none is given; stored only if valid.
- `clear_browserbase_credentials`.
- `test_browserbase`: reads project, usage and running sessions. Creates no session and costs nothing.
- `probe_browserbase {region}`: creates one honest session, reads what a page would see on the blank page, releases. Costs one browser minute and says so on the button.
- `get_browserbase_profile` and `clear_browserbase_profile`.
- `open_live_view {scanId}`, only if manual is in scope.

**Environment overrides**
- `ADSCAN_BROWSERBASE_API_KEY` and `ADSCAN_BROWSERBASE_PROJECT_ID`: memory store, no keychain, like the other key variables.
- `ADSCAN_BROWSERBASE_BASE_URL`: https unless loopback.
- `ADSCAN_BROWSERBASE_CONNECT_URL`: replaces the returned connect URL.

**Disclosure text for Settings, shown before credentials can be saved**

> With Browserbase, the browser runs in Browserbase's cloud, not on this Mac. The pages you scan, their cookies, and everything the scanner reads from them (scripts, requests, bid data) travel through Browserbase, a third party. Results are still stored only on this Mac. The scanner turns off Browserbase's recording, logging, captcha solving and ad blocking for every scan, and stops a scan if Browserbase hides that the visit is automated. Browserbase bills browser time per minute, and proxy traffic per megabyte.

### c. Cost and limits

- Keep the per-scan budget LLM-only. Browser time is not controllable mid-scan the way model calls are.
- Show Browserbase usage separately: estimated minutes in the composer, actual billed minutes and proxy MB on the report. Take the actuals from the session after release, in a new optional `ScanSummary.remote` field.
- Browser time is negligible: a 2-minute scan is about $0.004 at Developer overage.
- Proxy traffic is not: at $12 per GB, one 18 MB page view is about $0.22 and a three-page browse visit about $0.65. That is more than the LLM target. The composer must show this when a Browserbase location is chosen.
- The API does not expose the plan, so show units always and dollars as "at Developer overage rates".

### d. Failure modes and messages

| Situation | Message |
|---|---|
| Key rejected | "Browserbase did not accept the API key. Check it in Settings." |
| Project unknown | "This Browserbase project was not found for this key." |
| 429 after waiting | "Browserbase is at its limit of N browsers at once (or sessions per minute). Try again in S seconds." |
| Region refused | "Browserbase cannot start a browser in <region> right now." |
| Proxy refused at creation | "Browserbase could not connect to the proxy <name>. It must be reachable from the internet." |
| Location on Free plan | "Browserbase locations need a paid Browserbase plan." |
| Session timed out mid-scan | "Browserbase ended the browser at its time limit. Nothing was kept." |
| WebSocket dropped | "The connection to the Browserbase browser was lost. The browser has ended. Nothing was kept." |
| Automation flag hidden | "Browserbase hides that this visit is automated on this account. The scan was stopped before the page loaded." |
| Loopback or private URL | "This address is on this Mac or your network; a cloud browser cannot reach it. Scan it with this Mac's Chrome." |
| No key but Browserbase chosen | "Browserbase is not set up. Add the key in Settings." |

Status codes other than 429 are not documented. Map by status class and append the server's message with any URL query stripped.

### e. Changes, tests, order

**Migration**
- Settings and stored options are JSON; new fields take serde defaults, plus one `json_set` step in the existing style.
- One new table for the session ledger.
- New `ErrorCode` values for remote auth, limit and failure.
- Two HANDOVER rules need rewording:
  - the outbound traffic list gains `api.browserbase.com` and `connect.browserbase.com`;
  - "local only" becomes "stored locally only".

**Frontend, file by file**
- `src/features/settings/`:
  - new `BrowserbaseSection.tsx`;
  - `SettingsScreen.tsx`, `settings-schema.ts`;
  - `ScanDefaultsFields.tsx`: default for where the browser runs; headless hidden for Browserbase;
  - `ScanProfileField.tsx`: cloud profile status and clear.
- `src/features/scan-composer/`:
  - `OriginOptions.tsx`: where the browser runs, region, exit;
  - `VisitOptions.tsx`: headless hidden, manual disabled;
  - `scan-form.ts`, `scan-url.ts` (loopback detection), `scan-summary.ts`, `ComposerSummary.tsx`;
  - `cost-estimate.ts`: minutes and proxy MB.
- `src/features/live-scan/`: `LiveScanView.tsx`, `VisitPanel.tsx` (region and elapsed minutes).
- `src/features/report/`: `EnvironmentNotice.tsx`, `environment-notice.ts`, `ReportHeader.tsx`.
- `src/features/onboarding/`: no local Chrome is no longer a dead end.
- `src/lib/api/`: `port.ts`, `tauri-adapter.ts`.
- `src/lib/fake-backend/`: `index.ts`, `resolve.ts`, `runtime.ts`, `config.ts`, `seed.ts`. They mirror the validation rules, the credential flow, and a seeded scan whose environment says Browserbase.

**Tests**
- **Unit:** WebSocket framing and large messages; keepalive and silence detection; session body builder (the honest settings are asserted field by field); timeout computation; validation matrix of browser by proxy by URL; clock offset; error mapping; vault names.
- **Adapter, against a mock:** an axum REST mock plus a local Chromium started with `--remote-debugging-port=0` as the cloud browser.
  - A full demo-shaped capture over WebSocket.
  - Release is called on success, on cancel, on failure and when the future is dropped.
  - 429 with `retry-after`.
  - A context is created and reused.
  - A browser that hides the flag (`--disable-blink-features=AutomationControlled`) is refused before navigation.
  - No credential appears in any activity line or error.
- **Pipeline:** startup sweep from the ledger; usage recorded; the rule for a disguised visit.
- **End to end:** fake-backend Playwright specs for settings, composer and report; one `e2e-tauri` spec against the mock.
- **Live, ignored by default:** one real-account test that runs the probe.

**Order**
1. **Spike with a real account** (half a day). Read `webdriver`, user agent, timezone, targets and console noise on Free and Developer with the body above. Try a 20 MB body. Check the 15-minute Free limit against the timeout. This decides decisions 1 and 2 below.
2. **Model:** contract types and bindings.
3. **Browser crate:**
   1. message-level transport seam;
   2. WebSocket transport;
   3. REST client;
   4. backend enum, capture branch, clock, pre-navigation probe, environment;
   5. public functions for test, probe, sweep and context;
   6. mock and tests.
4. **Engine and shell:** secrets; settings and validation; ports and capture session; ledger and sweep; launch step ("Checking Browserbase" instead of locating Chrome); usage; rule; commands and environment variables; export.
5. **Frontend:** API port and fake backend, Settings, composer, live scan, report, onboarding, e2e.

**Out of scope for v1**
- Reconnecting after a drop, and `keepAlive`.
- Verified, Web Bot Auth, static proxies, extensions, proxy routing rules.
- Session replay or logs in the app; bring-your-own storage.
- ads.txt fetched from the cloud network.
- Tunnels to loopback.
- A general multi-provider abstraction.

## 5. Decisions for the owner

1. **What if Browserbase hides the automation flag on paid plans?** I recommend failing closed with no override: the scan stops before the page loads. The alternative is to allow it with a disclosure in the UI and the environment record. That contradicts the standing decision in substance, so I would not offer it.
2. **Ship before the spike answers finding 1?** No. If "Basic" cannot be turned off and hides the flag, Browserbase is usable only on the Free plan (3 browsers, 15 minutes, no proxies), or not at all. Ask Browserbase support in writing what "Basic" changes and whether it can be disabled per session.
3. **Browserbase residential locations.** I recommend leaving them out of v1. The honest geo choices are the four cloud regions, or the user's own proxy.
   - If you want them, the UI must label them: residential exit, country best effort, IP not guaranteed fixed, timezone changed by Browserbase.
   - They would also be the dominant cost.
4. **Recording and logging.** Default both off. I recommend not offering a switch in v1; turning logging on would store response bodies at Browserbase for 7 to 30 days.
5. **Manual visits on Browserbase.** I recommend not in v1.
   - A manual visit exists to be a real person on their own machine and network.
   - Through the live view it is a person steering a cloud Linux browser, with input injected by Browserbase and an unknown automation flag.
   - The design above makes it buildable later.
6. **Kept scan profile in the cloud.** Cookies and consent state would sit at Browserbase indefinitely until cleared. I recommend allowing it, off by default for Browserbase even when the local setting is on, with that sentence in the UI.
7. **Project id in the keychain.** Done as asked. It is an identifier, not a secret; keeping it in settings would be simpler. Say if you prefer that.
8. **Dollar estimates.** I recommend units always, plus dollars at Developer overage rates labelled as an estimate. The alternative is units only, which needs no price table to maintain.
9. **Disclosure wording** in section 4b: confirm or edit.

## Sources

All under `https://docs.browserbase.com/`, fetched 2026-11-12:
- `llms.txt`, `llms-full.txt`
- `reference/api/`: `create-a-session`, `list-sessions`, `update-a-session`, `session-live-urls`, `session-logs`, `get-a-project`, `list-projects`, `get-project-usage`, `create-a-context`, `get-a-context`, `delete-a-context`, `overview`
- `platform/browser/getting-started/`: `create-browser-session`, `using-browser-session`, `manage-browser-session`, `what-is-headless-browser`, `remote-browser-versus-local-browser`
- `platform/browser/long-sessions/`: `overview`, `keep-alive`, `timeouts`
- `platform/browser/core-features/`: `contexts`, `viewports`, `session-metadata`
- `platform/browser/observability/`: `observability`, `session-live-view`, `session-replay`
- `platform/browser/files/downloads`, `platform/browser/techniques/timezones`, `platform/browser/security/navigation-policies`
- `platform/identity/`: `overview`, `verified-customization`, `proxies`, `captcha-solving`, `web-bot-auth`
- `optimizations/`: `concurrency/overview`, `latency/multi-region`, `latency/speed-optimization`, `cost/measuring-usage`
- `account/billing/plans`, `account/enterprise/zero-data-retention`
- The old `features/stealth-mode` URL now redirects to `platform/identity/overview`.

### Critical Files for Implementation
- /home/user/Atlas/apps/adtech-scanner/crates/browser/src/cdp/client.rs
- /home/user/Atlas/apps/adtech-scanner/crates/browser/src/capture/mod.rs
- /home/user/Atlas/apps/adtech-scanner/crates/browser/src/capture/environment.rs
- /home/user/Atlas/apps/adtech-scanner/crates/model/src/lib.rs
- /home/user/Atlas/apps/adtech-scanner/crates/engine/src/domain/scan.rs