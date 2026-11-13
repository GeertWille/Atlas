# Browserbase spike: measured behaviour for the ad-tech scanner

Run on 2026-11-13, 08:26–09:07 UTC, against the real account behind the saved key.
Everything below was observed unless it is marked **(inference)**.

## How this was measured, and what limits the evidence

- Client: a raw CDP client on the `ws` package (`scripts/lib.js`), no Playwright. REST through the session's HTTPS proxy with the placeholder key.
- **Every WebSocket went through this sandbox's HTTPS proxy**, which re-terminates TLS. The WebSocket worked, but two limits found below (message size, idle close) could in principle belong to that proxy and not to Browserbase. Where I could, I say which way the evidence points.
- This machine exits in the US (Ohio). All latency numbers are US-to-region.
- Local comparison: headless Chromium 153 (`--headless=new`) driven by the same scripts.
- **Sessions created: 16. Browser time: 20.1 minutes of wall clock, about 31 billed minutes** if each session rounds up to a minute. Proxy traffic: about 233 KB. All 16 are `COMPLETED`; `GET /v1/sessions?status=RUNNING` returned `[]` at 09:06:51.
- That is above the "under 15" target. Three went to first-contact debugging, two were created by error-shape requests I expected to be refused (`browserSettings.fingerprint`, a 10×10 viewport) and were released within 0.3 s, and one extra was needed after the first idle test ended its own session.
- **The project is shared with another application.** Sessions with `userMetadata` keys `scanId`, `auditId`, `organizationId` ran in eu-central-1 before and during the spike. One of them held the third slot and caused a 429 for one of my sessions. I did not touch those sessions.
- Scripts are in `scripts/`, outputs in `out/` (JSON per session, `sessions.jsonl` ledger). No connect URL, signing key or API key is stored; a pattern scan of `out/` and `scripts/` found none.

| Label | Region | Settings | Purpose | Seconds |
|---|---|---|---|---|
| smoke ×3 | us-west-2 | default | first contact | 0.3–1.6 |
| A | us-west-2 | default | CDP surface, ended by the 60 MB body | 32 |
| A2 | us-west-2 | default | CDP surface, captcha page, live view, two clients | 105 |
| E | ap-southeast-1 | default | silent idle socket | 302 |
| B | eu-central-1 | captcha, recording, logging, ad block all off | fingerprint, noise, live view, socket kill | 101 |
| D | us-east-1 | default, `keepAlive` | reconnect, 48 MB body, `Browser.close` | 102 |
| bisect | us-east-1 | default, `keepAlive` | largest body | 22 |
| E2 | ap-southeast-1 | default, `keepAlive` | idle with pings, latency | 383 |
| C | eu-central-1 | default, Browserbase proxy BE | exit IP, timezone, `Browser.close` | 100 |
| 2 accidental | us-west-2 | — | created by error-shape requests | 0.2 each |
| F | eu-central-1 | default, context `persist` | Guardian page view | 39 |
| H | eu-central-1 | context, `proxies: true` | persistence check | 10 |
| I | eu-central-1 | context, DE and NL proxies by domain | persistence check | 8 |

---

## 1. Account

| Check | Result |
|---|---|
| `GET /v1/projects` | one project, "Local", `defaultTimeout` 1800, `concurrency` 3 |
| `GET /v1/projects/{id}` | `id, createdAt, updatedAt, name, ownerId, defaultTimeout, concurrency`. No plan name. |
| `GET /v1/projects/{id}/usage` | `{"browserMinutes":444,"proxyBytes":1909206806}` at 08:26; `{"browserMinutes":483,"proxyBytes":1998349077}` at 09:05. Running totals only, no allowance or period. The increase includes the other application's sessions. |
| Session-create rate limit | response header `ratelimit-limit: 50` (per 60 s). Plain GETs report 6000. |
| Concurrency | really 3, enforced, shared with the other application |
| Regions | us-west-2 (default), us-east-1, eu-central-1, ap-southeast-1 all created |
| `keepAlive: true` | accepted (201, `keepAlive: true` in the session) |
| `proxies: true` | accepted; exit was a US residential line (AS7922 Comcast, Illinois) |
| `proxies: [{type:"browserbase", geolocation:{country:"BE"}}]` | accepted; exit AS6848 Telenet, Antwerp |
| DE and NL | accepted, as two entries with `domainPattern`: DE gave AS6805 Telefonica Germany (Hamburg), NL gave F2X Operator B.V. (Rotterdam). Hosts matching neither pattern left from AWS Frankfurt. |
| `browserSettings.verified: true` | 403 `{"statusCode":403,"error":"Forbidden","message":"Verified mode is only available on the Enterprise plan"}` |

**Which plan (inference):** the key behaves like a paid plan at Startup level: `keepAlive` and proxies are accepted, captcha solving works, and the create limit is 50 per minute, which is the Startup figure in the plans table (Developer is 25, Free is 5). The concurrency of 3 does not match Startup's 100. It reads as a per-project setting: the project's `updatedAt` is four minutes after its `createdAt`. The API never names the plan. Confidence: medium.

## 2. What a page sees

Identical in all six measured sessions (A, A2, B, C, D, E2) except where the table says otherwise.

| Property | Browserbase | Local headless Chromium |
|---|---|---|
| `navigator.webdriver` | **`false`** (boolean, native getter) | `true` |
| `navigator.userAgent` | `Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/157.0.0.0 Safari/537.36` | `…HeadlessChrome/153.0.0.0…` |
| `userAgentData.brands` | `Not-A.Brand 99, Google Chrome 157, Chromium 157` | `Chromium 153, Not_A Brand 8` |
| `userAgentData` platform / mobile | `Linux` / `false`; full version 157.0.8103.79, x86, 64 | same shape |
| `navigator.platform` | `Linux x86_64` | same |
| `navigator.languages` | `["en-US","en"]` in every region and with a BE proxy | `["en-US"]` |
| Timezone | `America/Los_Angeles` in **every** region without a proxy | `UTC` |
| Screen | 2560×1440, avail 2560×1440, depth 24 | 800×600 |
| Window | inner 2560×1440, outer 2568×1571, DPR 1, at 10,10 | inner 1280×657 |
| `hardwareConcurrency` / `deviceMemory` | 2 / 8 | 4 / 16 |
| `plugins.length` / `mimeTypes.length` | 5 / 2 | 5 / 2 |
| WebGL unmasked | `Google Inc. (Intel)` / `ANGLE (Intel, Mesa Intel(R) UHD Graphics 630 (CFL GT2), OpenGL 4.6)` | SwiftShader |
| `window.chrome` | object (`loadTimes, csi, app`) | same |
| Notification permission | `denied` in A, A2, D; `prompt` in B, C, E2 | `prompt` |
| `Browser.getVersion` | product `Chrome/157.0.8103.79`, same user agent as the page, jsVersion `15.7.112.43` | `HeadlessChrome/153…` |
| `Browser.getBrowserCommandLine` | `/usr/bin/chromium-browser --disable-blink-features=AutomationControlled --disable-background-networking --disable-dev-shm-usage --disable-sync --homepage=about:blank --start-maximized --headless=new` | — |
| `SystemInfo.getInfo` | modelName `Browserbase Sandbox`, modelVersion `42.0` | empty |

**Main world against isolated world.** I evaluated the same probe in both on `about:blank` and on `https://example.com/`. The only differences were `chrome.app` (main world only) and the page's own globals, and the local browser shows exactly the same differences. So Browserbase does **not** patch the main world with script. The values come from the browser itself: the launch flag turns the automation flag off, and the user agent and brands are set at browser level (the worker and a `srcdoc` iframe report the same).

**Headless:** yes, `--headless=new` is on the command line. The user agent does not say so. The WebGL string names Intel hardware while the browser logs "Automatic fallback to software WebGL", so the GPU string is not the real renderer **(inference)**.

**Does `navigator.webdriver` vary?** No. It was `false` with default settings, with everything turned off, in all four regions, with a Browserbase proxy, with `keepAlive`, and with a context. `Emulation.setAutomationOverride({enabled:true})` returned ok and left it `false`. In a worker it is `undefined`, as in any Chrome.

**Exit IP and timezone**

| Session | Exit as seen by ipinfo.io | Timezone in page |
|---|---|---|
| us-west-2, no proxy | AS16509 Amazon, Boardman, Oregon | America/Los_Angeles |
| us-east-1, no proxy | AS49915 Megaport, Ashburn, Virginia | America/Los_Angeles |
| eu-central-1, no proxy | AS16509 Amazon, Frankfurt, DE | America/Los_Angeles |
| ap-southeast-1, no proxy | AS16509 Amazon, Singapore | America/Los_Angeles |
| eu-central-1, proxy BE | AS6848 Telenet BV, Antwerp, BE (188.188.176.x) | **Europe/Brussels** |
| eu-central-1, `proxies: true` | AS7922 Comcast, Illinois, US | **America/Chicago** |
| eu-central-1, DE and NL by domain | DE Telefonica, NL F2X, others AWS Frankfurt | **Europe/Berlin** (the first entry) |

- The timezone follows a Browserbase proxy's location. Without a proxy it does not follow the region.
- **Without a proxy the exit IP is not fixed within a session.** Three services asked within one second saw three different AWS addresses in us-west-2 and ap-southeast-1, and two in eu-central-1 and us-east-1.
- **With the BE proxy the exit IP stayed the same.** I loaded four sites (ipinfo.io, api.ipify.org, cloudflare.com/cdn-cgi/trace, ifconfig.co) three times over 56 seconds: 12 loads, one address. That is one session and one minute; it does not show the address holds for a long scan.
- With a proxy the browser's network record shows `remoteIPAddress` `10.255.255.1:8091` for every request, so the real server address is lost from the scanner's network record.
- `navigator.connection` reported `3g`, rtt 350 through the BE proxy against `4g`, rtt 100 without.

## 3. CDP surface

**Targets on connect:** one `page` (`about:blank`, already `attached: true`, so Browserbase has its own client on it) and its `tab`. No service worker, extension or other page target is visible, with any filter.

| Method | Result |
|---|---|
| `Target.setAutoAttach` {autoAttach, waitForDebuggerOnStart, flatten} on the browser session | works; the existing page attaches, popups attach paused and resume |
| Same on a page session | works; workers attach with `waitingForDebugger: true` and stay paused until `Runtime.runIfWaitingForDebugger` (I held one 1.5 s; its first message arrived 1.76 s after creation) |
| **Out-of-process iframes** | **there are none.** A cross-site iframe (example.org inside example.com) created no target; its requests, its `Fetch.requestPaused` and the tracker's binding call all arrived on the page session. On the Guardian page: 0 iframe sessions against 50 locally. |
| `Page.addScriptToEvaluateOnNewDocument` | works with `runImmediately` and with `worldName`; runs before page script (`readyState: loading`, no body) in the top frame and in the cross-site iframe |
| `Page.removeScriptToEvaluateOnNewDocument` | works |
| `Runtime.addBinding` and `Runtime.bindingCalled` | work, from the top frame and from the iframe |
| `Network.enable` with 256 MiB `maxTotalBufferSize` and `maxResourceBufferSize` | accepted on page and worker sessions. Bodies up to 39 MB were held and read back. |
| `Network.getResponseBody` | works; see section 4 for the size limit |
| `Network.getRequestPostData` | works (200 KB POST body returned) |
| `Network.streamResourceContent` | exists and works: on a 40 MB download it returned 1.33 MB of buffered data, then 111 `dataReceived` events carried the rest |
| `Fetch.enable` on Document requests | works; `continueRequest` and `failRequest` work (`net::ERR_BLOCKED_BY_CLIENT`) |
| `Fetch` auth handling | `Fetch.authRequired` fired for a basic-auth site and `continueWithAuth` logged in |
| `Input.dispatchMouseEvent`, `mouseWheel`, `dispatchTouchEvent`, `dispatchKeyEvent`, `synthesizeScrollGesture` | all work; every resulting event had `isTrusted: true` (mousemove, mousedown, mouseup, click, wheel, scroll, touchstart, touchend, keydown) |
| `Emulation.setDeviceMetricsOverride` | works (390×844, DPR 3); clearing it returns to 2560×1440 |
| `Emulation.setUserAgentOverride` with client hints | works in the page and on the wire (`User-Agent`, `sec-ch-ua`, `sec-ch-ua-mobile`, `sec-ch-ua-platform`, `Accept-Language`) |
| `setTouchEmulationEnabled`, `setEmitTouchEventsForMouse`, `setFocusEmulationEnabled`, `setTimezoneOverride`, `setLocaleOverride`, `setGeolocationOverride`, `setEmulatedMedia`, `setIdleOverride`, `setCPUThrottlingRate`, `setScrollbarsHidden` | all returned ok; timezone, locale and touch were confirmed in the page |
| `Browser.setDownloadBehavior` deny | honoured: a blob download and an attachment navigation both ended `canceled`, the navigation with `net::ERR_ABORTED` |
| `Page.setInterceptFileChooserDialog` | works (`Page.fileChooserOpened`) |
| `Target.createBrowserContext`, `Target.createTarget` | **not restricted**: both succeeded, and dispose and close worked |
| `Browser.close` without `keepAlive` | reply ok, socket closed after 173 ms, session `COMPLETED` within 0.4 s |
| `Browser.close` with `keepAlive` | reply ok, socket closed, **session stayed `RUNNING`** (still running 77 s later) and accepted a new connection |
| Also ok | `Debugger.enable`, `Page.captureScreenshot`, `Storage.getCookies`, `Page.bringToFront`, `Browser.getWindowForTarget`, `SystemInfo.getInfo`, `Log.enable` |
| Refused | `chrome://version` gives `net::ERR_BLOCKED_BY_ADMINISTRATOR` |

**What Browserbase adds to a page**

| Noise | Default settings | Everything off | How to recognise it |
|---|---|---|---|
| Console line `Browserbase keeping connection alive` | yes | **yes** | `Runtime.consoleAPICalled`, type `info`, in the page's main world, first stack frame URL `./dist/scripts/keepalive.js`. Seen once per session at 30–81 s, twice on the Guardian page. |
| Console lines `browserbase-solving-started`, `browserbase-solving-finished`, and a JSON string with `"key":"browserbase-captcha-event"` | yes, when a captcha is on the page | no | type `log`, main world, empty stack URL |
| Isolated world `__playwright_utility_world_page@<hash>` | yes, in every frame | no | `Runtime.executionContextCreated` name |
| Isolated world named `w` plus hex | no | yes, on the blank page | name |
| Isolated world `Native Select Polyfill`, origin `chrome-extension://cfiapcmfjmppkelmogiebeahmidbpdbi` | yes, in every document | **yes** | origin |
| Isolated world `__bb_frame_reads` | on the captcha page | no | name |
| Log entry "Automatic fallback to software WebGL has been deprecated…" | yes | yes | `Log.entryAdded`, source `rendering`; appears when a page creates a WebGL context |

- With default settings the reCAPTCHA demo page was **solved without any action from me**: started 0.4 s after load, finished 6 s later, token present. With `solveCaptchas: false` nothing happened and no line was logged.
- No DOM additions: example.com had the same 22 elements as locally, no shadow roots, no extra scripts or stylesheets.
- No extra main-world globals and no extra bindings.
- No requests from Browserbase appeared in the page's network record.
- The extension named "Native Select Polyfill" has no visible target. Its name suggests it changes how `<select>` elements behave **(inference; not tested)**.

## 4. Volume and latency

**Message size.** Bodies were zero-filled binary files fetched in the page, so each reply is base64 and 4/3 the body size.

| Body | Reply message | Result |
|---|---|---|
| 1 MB | 1.33 MB | ok, 0.19 s |
| 10 MB | 13.3 MB | ok, 1.2 s |
| 30 MB | 40.0 MB | ok, 3.2–4.2 s (77–99 Mbit/s) |
| 39 MB | 52,000,097 bytes | ok, 3.4 s |
| 39.56 MB | about 52.75 MB | socket closed, code 1006 |
| 40.13, 41.25, 43.5, 48, 60 MB | 53.5–80 MB | socket closed, code 1006 |

- The limit lies between 52.00 and 52.75 million bytes. That brackets 50 MiB (52,428,800) **(inference: a 50 MiB maximum message)**.
- On failure nothing of the reply arrives (0 bytes read during the call), and the socket closes about 0.7 s after the command. There is no error reply and no close frame.
- Without `keepAlive` the session ended at the same moment. With `keepAlive` the session stayed `RUNNING`, the page was intact, and a new connection worked.
- Local Chromium over a direct socket returned an 80 MB message, so the limit is not Chrome's. This sandbox's proxy logged no relay failure for the Browserbase host at those moments. The evidence points to Browserbase's gateway **(inference, medium confidence)**.
- Plain-text bodies were not measured at the limit; my two attempts did not produce a request to read.
- No compression: the client offered `permessage-deflate` and the server did not accept it.

**Round trip per command** (50 × `Runtime.evaluate("1")`, US to region)

| Region | Median | p95 | WebSocket ping median |
|---|---|---|---|
| us-east-1 | 43.8 ms | 58.9 ms | 53 ms |
| us-west-2 | 83.1 ms | 85.0 ms | 81 ms |
| eu-central-1 | 118.6 ms | 125.7 ms | 115 ms |
| eu-central-1 with BE proxy | 119.1 ms | 134.7 ms | 116 ms |
| ap-southeast-1 | 249.6 ms | 257.4 ms | 248 ms |
| local | 1.3 ms | 3.1 ms | 0 ms |

**Start and end**

| Step | Measured |
|---|---|
| `POST /v1/sessions` | 235–677 ms; the session is `RUNNING` in the response |
| WebSocket connect | 310–570 ms without a proxy in the US and EU; 1.0–1.2 s in ap-southeast-1 and with a context; 1.6–1.7 s with a Browserbase proxy |
| Socket killed, no `keepAlive` | session `COMPLETED` 10.3 s later in one run, about 2 s in another |
| `REQUEST_RELEASE` | returns `COMPLETED` with `endedAt` in the response, 0.2–0.4 s |
| Reconnect with `keepAlive` after a drop | 275 ms; the page was still on its URL |

**Idle socket**

- With nothing sent in either direction, the socket was closed with code 1006 at **300 s**, and the session (no `keepAlive`) ended 0.4 s later. The docs say 10 minutes.
- With only a WebSocket ping every 60 s, the socket was still open at 345 s and the next command worked. Pings are answered, and they do keep the socket alive.
- The server never sent a ping of its own.
- I cannot say whether the 300 s belongs to Browserbase or to this sandbox's proxy. No relay failure was logged.

**Clock offset** (remote `Date.now()` minus this machine, best of 15 samples)

| Session | Offset | Later in the same session |
|---|---|---|
| us-west-2 | −51 ms | −51 ms after 88 s |
| eu-central-1 | −53 ms | −53 ms after 72 s |
| eu-central-1, BE proxy | −43.5 ms | −43.5 ms after 74 s |
| us-east-1 | −34 ms | −37 ms after 7 s |
| ap-southeast-1 | −18.5 ms | −18.5 ms after 4 s |

Stable to about 3 ms within a session. Each estimate is good to half a round trip at best, so the differences between regions may be path asymmetry and not clock differences **(inference)**.

## 5. A realistic page

One load of `https://www.theguardian.com/international`, 30 s, Network on every session, bodies read for scripts and JSON. Browserbase: eu-central-1, default settings, with a context. Local: headless Chromium from this machine.

**The two visits are not like for like:** Browserbase left from Germany and got the EU consent dialog; the local browser left from the US and got the US privacy notice.

| | Browserbase eu-central-1 | Local headless |
|---|---|---|
| Requests | 723 | 765 |
| Hosts | 152 | 125 |
| **Share in child sessions** | **1.8 %** (13 in one worker) | **56.7 %** (421 in 50 iframes, 13 in a worker) |
| Child sessions | 1 worker | 50 iframes, 1 worker |
| Bodies attempted / read | 245 / 245 | 229 / 229 |
| Body bytes | 9.1 MB | 7.9 MB |
| Unavailable bodies | 0 | 0 |
| Largest body | 1.5 MB | 1.5 MB |
| `getResponseBody` time | median 132 ms, p95 335 ms, max 843 ms | median 12 ms, p95 70 ms |
| CDP traffic received | 18.5 MB in 5,396 messages, largest 1.65 MB | not compared |
| Load event | 1.3 s | 1.1 s |
| Wall time | 31.0 s; body queue empty at the end | 30.6 s |
| Consent | EU dialog found in a frame on the page session, "Accept all" clicked at 8.1 s, TCF `useractioncomplete`, 11 purposes | US notice shown; nothing to accept |
| Prebid (v10.23.0) | 4 auctions, 36 bid requests, 37 no-bids, **0 bids** | 4 auctions, 36 bid requests, 37 no-bids, **0 bids** |
| Google ad slots | 11 defined, 1 with a response; a banner was visible in the screenshot | 9 defined, 0 with a response |
| Failed requests | 9 | 36 |
| Bot wall | none: status 200, normal title, 19.5 KB of text | none |
| Browserbase console lines | 2 (`keeping connection alive`) of 11 | 0 of 5 |

- Prebid ran and reached the bidders in both; no bidder returned a bid in either. One load each, so this says nothing about fill rates.
- The consent click was a DOM `click()` from an isolated world, not a mouse click as in `probe2.js`.

## 6. Contexts and live view

**Contexts**

- `POST /v1/contexts` returns 201 with `id, uploadUrl, publicKey, cipherAlgorithm (AES-256-CBC), initializationVectorSize`. `GET` returns `id, createdAt, updatedAt, projectId`.
- Session F wrote on example.com: a persistent cookie, a session cookie, an httpOnly cookie (through `Network.setCookie`), a localStorage key and an IndexedDB record. It ended with `REQUEST_RELEASE`.
- Session H started **30 s later** and found all five, plus the 20 Guardian cookies including `consentUUID`. The session cookie survived too.
- Session I started **1.7 s after** H was released and found H's new localStorage key and cookie as well. So no wait was needed in these two trials; that is two trials, not a guarantee.
- `DELETE` returns 204. After it, `GET` returns 404 and creating a session on it returns 400 `{"statusCode":400,"error":"Bad Request","message":"Context ID not found"}`.

**Live view** (`GET /v1/sessions/{id}/debug`)

- Keys: `debuggerFullscreenUrl`, `debuggerUrl`, `wsUrl`, `pages[]`. Each page has `id, url, title, faviconUrl, debuggerUrl, debuggerFullscreenUrl`, so per-tab URLs are present.
- Shapes: `https://www.browserbase.com/devtools/inspector.html?wss=…&debug=…` and a fullscreen variant; `wsUrl` is `wss://connect.<region>.browserbase.com/debug/<session>/devtools/browser/<id>?<token>`.
- Fetching the fullscreen URL once: 308, then 200 `text/html`, 605 bytes, title "DevTools". No `X-Frame-Options` or CSP header. No API key was sent; the token in the query string is the only credential. The same socket without its query string returned 403.
- It works with recording and logging off.
- On an ended session: 410 `{"statusCode":410,"error":"Gone","message":"Session stopped"}`.

**Two clients on one session**

- I connected a second CDP client to the page socket the live view uses (the `wss` parameter), while the first stayed connected and recording.
- A mouse press and release from the second client produced trusted `mousedown` and `click` events in the page. A `Page.navigate` from it appeared on the first client as `Page.frameNavigated` and `Network.requestWillBeSent`. A screencast started.
- Closing the second client left the first open and the session `RUNNING`.
- So the scanner can keep recording while someone steers through the live view. I drove the socket, not the live view's own user interface.
- **A second client on the main `connectUrl` is different:** it connects, but when it closed, the first client was closed too and the session ended (`COMPLETED`).

## 7. Session bookkeeping

**After release** `GET /v1/sessions/{id}` returns: `id, createdAt, updatedAt, projectId, status: "COMPLETED", proxyBytes, keepAlive, region, startedAt, endedAt, expiresAt, userMetadata`, and `contextId` when one was used. There is no billed-minutes field. Every way of ending gave `COMPLETED`: release, `Browser.close`, socket drop, idle close.

- `proxyBytes`: correct 1.5 s after the end in one session (131,844); still 0 at 1.5 s in two others, correct when listed 35 s later (82,715 and 18,486).
- Project usage had moved by 09:05; it mixes in the other application, so I could not time its delay.
- `GET /v1/sessions?q=user_metadata['app']:'adscan-spike'` returned exactly my 16.

**Errors** (all bodies are JSON with `statusCode`, `error`, `message`; schema failures add `"code":"FST_ERR_VALIDATION"`)

| Case | Status | Body |
|---|---|---|
| Project id not on this key | 401 | `{"statusCode":401,"error":"Unauthorized","message":"Unauthorized Project ID"}` |
| Project id not a UUID | 400 | `…"message":"Invalid Project ID"}` |
| `GET` unknown project | 404 | `…"message":"Project not found"}` |
| **Fourth session** | 429 | `{"statusCode":429,"error":"Too Many Requests","message":"You've exceeded your max concurrent sessions limit (limit 3, currently 3). Please contact support to increase your limit."}` No `retry-after`; only `ratelimit-limit: 50`, `ratelimit-remaining`, `ratelimit-reset`. |
| Invalid region | 400 | `…"code":"FST_ERR_VALIDATION"…"message":"body/region must be equal to one of the allowed values"}` |
| Proxy country `XX` | 400 | `…"message":"Validation error: Invalid country code at \"proxies[0].geolocation.country\""}` |
| External proxy, bad URL | 400 | `…"message":"Validation error: Invalid url at \"proxies[0].server\""}` |
| Proxy of unknown type, or external without `server` | 400 | `FST_ERR_VALIDATION`, a long list starting `body/proxies/0/type must be equal to constant, body/proxies/0 must have required property 'server', …` |
| `timeout` 10 / 999999 | 400 | `body/timeout must be >= 60` / `body/timeout must be <= 21600` |
| Metadata with an array | 400 | `…"message":"Value is not a string: [1,2]"}` |
| Metadata too long | 400 | `…"message":"User metadata must be at most 512 characters, current size is 632 characters"}` |
| Unknown top-level field | 400 | `…"message":"Validation error: Unrecognized key(s) in object: 'nonsense'"}` |
| `browserSettings.os: "mac"` | 400 | `…"message":"mac OS is only available for verified users, which is only available on the Enterprise plan. By default, we only support Linux."}` |
| Unknown context id | 400 | `…"message":"Context ID not found"}` |
| Malformed JSON | 400 | `…"code":"FST_ERR_CTP_INVALID_JSON_BODY"…` |
| `GET` unknown session, debug or logs | 404 | `{"statusCode":404,"error":"Not Found","message":"Not Found"}` |
| Session id not a UUID | 400 | `…"message":"Invalid Session ID"}` |
| Release an ended session | 200 | the session, unchanged (also without `projectId`) |
| **Connect to an ended session** | **HTTP 410** on the WebSocket upgrade, empty body | |
| Accepted, not refused | 201 | `browserSettings.fingerprint` (a field the docs no longer list) and a 10×10 viewport |

## 8. What recording and logging store

**Logs** (`GET /v1/sessions/{id}/logs`): a JSON array, one entry per CDP message, with `method, pageId, sessionId, timestamp, request {params, rawBody, timestamp}` and, for commands, `response {result, rawBody, timestamp}`.

| Session | Entries | Size |
|---|---|---|
| Guardian page view, 39 s | 3,824 | **11.2 MB** |
| A2, 105 s of tests | 1,260 | 4.7 MB |
| B, logging off | 0 (`[]`) | 2 bytes |

The log is a record of **our own CDP traffic**, commands and replies, as well as events:

- **Response bodies: yes.** 230 of the 246 `Network.getResponseBody` replies are stored with the body, 2.7 MB of body text. The largest stored was 125 KB; the 16 larger ones are logged without a reply. A 212 KB screenshot reply was stored, so the cut-off is somewhere above that **(inference)**.
- **Cookies: yes, when we ask for them.** The `Storage.getCookies` reply is stored whole: 154 cookies with values, 98 KB. `Network.setCookie` parameters and `Runtime.evaluate` results (including a `document.cookie` read) are stored too.
- **Credentials: yes.** The `Fetch.continueWithAuth` command is stored with `username` and `password` in clear.
- **Everything we evaluate:** expressions and their results, binding payloads, user agent overrides.
- Request and response **headers are not stored**: `Network.requestWillBeSent` entries keep only `url` and `method`, `responseReceived` keeps url, status, mime type and timing, and there are no `ExtraInfo` entries. So `Cookie` and `Set-Cookie` headers are absent. Full URLs are stored, including consent strings in query parameters.
- POST bodies: the `Network.getRequestPostData` reply (200 KB) was not stored. Smaller ones were not tested.
- `Network.dataReceived` events are stored with their base64 `data` when streaming is on.

**Recording**

- `GET /v1/sessions/{id}/recording` returns 410: `{"statusCode":410,"error":"Gone","message":"The rrweb-based Session Recording API has been removed. Use the Session Replay API for HLS video or the Recording Downloads API for MP4 files."}`
- `GET /v1/sessions/{id}/replays` returns `{"pages":[{"pageId":"0","url":"/v1/sessions/<id>/replays/0","startTimeMs":0,"endTimeMs":38095}],"pageCount":1}`: one video per tab (three for A2, which opened two more tabs).
- `GET /v1/sessions/{id}/recording/downloads` returns `{"downloads":[{"pageId":"0","status":"NOT_REQUESTED"}]}`.
- With recording off: 404 `Replay not found` and 422 `Recording was disabled for this session`.
- I did not download a video. That it shows everything rendered in the tab is an **inference** from the docs.
- `GET /v1/sessions/{id}/downloads` returns an empty zip (22 bytes) when nothing was downloaded.

---

## What this means for the integration

### Works unchanged

- The whole CDP method set the scanner uses: tracker injection with a binding, `Network` with the 256 MiB buffers on page and worker sessions, `getResponseBody`, `getRequestPostData`, the `Fetch` navigation guard with auth, scripted `Input` (trusted events), device `Emulation`, download refusal, file chooser interception.
- Workers attach paused and resume as they do locally.
- The message-level transport seam and the WebSocket client in section 4a of the plan. No compression is correct.
- Contexts as the kept profile, including consent state.
- Create, release, list by metadata, and reading the session afterwards.

### Needs a change or a workaround

1. **The fail-closed probe in section 3 of the plan would stop every scan.** `navigator.webdriver` is `false` on this account in every configuration, set by a launch flag, with no session setting that changes it. Under the owner's decision the app records it instead. Record `webdriver` (both worlds), the user agent against `Browser.getVersion`, the brands, and "headless: yes, not shown in the user agent". The plan's "Automation flag hidden" error message goes away.
2. **There are no iframe sessions.** Every frame of every site runs on the page session. The scanner must not expect per-iframe sessions, and anything that keys on "requests seen in a child session" will differ sharply from a local scan (1.8 % against 56.7 % on the same page). Page-level injection reaches cross-site frames, and the Document guard on the page session sees iframe navigations.
3. **Never ask for a body whose reply would pass about 50 MiB.** The reply is not an error; the socket dies and, without `keepAlive`, the scan with it. Decide from `encodedDataLength` or the summed `dataLength` before calling `getResponseBody`. A cap of 32 MiB of decoded body (about 44.7 MB as base64) is inside the limit; text bodies can go to about 48 MiB. For larger ones use `Network.streamResourceContent`, which works, or skip. The plan's 128 MiB tungstenite limit is more than is needed; 64 MiB covers it.
4. **Keepalive.** Send a WebSocket ping at least every 60 s; the plan's 30 s is fine. Treat 300 s of silence as fatal, not 10 minutes. The plan's "command every 4 minutes" is not needed if pings run.
5. **Exactly one client on `connectUrl`.** A second one closing ends the session. Manual visits must use the debug page socket or the live view URL, never a second connection to `connectUrl`.
6. **Ending.** Without `keepAlive`, `Browser.close` ends the session in under half a second and a dropped socket ends it in 2–10 s. Still send `REQUEST_RELEASE`: it is idempotent (200 on an ended session). If `keepAlive` is ever used, `Browser.close` does not end the session and it bills until release or timeout.
7. **Body patience.** One `getResponseBody` costs about one round trip (median 132 ms from the US to Frankfurt, p95 335 ms, max 843 ms). The 3 s `BODY_PATIENCE` held for 245 of 245 bodies with eight calls in flight; it would be tighter from further away. All bodies were still available at the end of the 30 s.
8. **Timezone and language.** Without a proxy the page reports `America/Los_Angeles` and `en-US` in every region. A scan "from Frankfurt" has a German IP and a Californian clock unless the app sets `Emulation.setTimezoneOverride`, which works. Record both values either way.
9. **Viewport.** The default is 2560×1440. The plan's "always override on remote" is right and works.
10. **Concurrency.** The limit is 3 and the other application uses the same project. A 429 here carries no `retry-after`; the plan's "honour `retry-after`" needs a fallback wait. A separate project for this app would avoid collisions **(inference: concurrency is set per project)**.
11. **`proxyBytes`** may read 0 for some seconds after the end. Read it again after 30 s before showing proxy usage.
12. **Network record with a Browserbase proxy:** `remoteIPAddress` is always `10.255.255.1`. Do not report it as the server's address.

### Not possible

- Making the page see `navigator.webdriver === true` through Browserbase settings or `Emulation.setAutomationOverride`. Only the app's own injected script could do it, which would be the app altering the page.
- Getting out-of-process iframes.
- Learning the plan or the remaining allowance from the API.
- A fixed exit IP without a proxy.

### Recommended session body for the owner's decisions

```json
{
  "projectId": "<project>",
  "region": "<chosen>",
  "timeout": "<computed cap, 60–21600 s>",
  "keepAlive": false,
  "browserSettings": {
    "solveCaptchas": true,
    "recordSession": true,
    "logSession": true,
    "blockAds": false,
    "context": { "id": "<context>", "persist": true }
  },
  "proxies": [{ "type": "browserbase", "geolocation": { "country": "BE" } }],
  "userMetadata": { "app": "adscan", "install": "<uuid>", "scan": "<uuid>" }
}
```

- `context` only for kept-profile scans; `proxies` only when a proxy is chosen (an `external` entry with `server`, `username`, `password` for the user's own proxy; not tested).
- The four `browserSettings` flags are the defaults, sent explicitly so a change of default at Browserbase cannot change a scan.
- Do not send `viewport` on the strength of this spike; its effect was not measured. Use the `Emulation` override.
- Do not send `verified`, `advancedStealth` or `os`: 403 and 400 on this account.
- Unknown top-level fields are rejected with 400, so the body must be exact.

### Limits to enforce

| Limit | Value |
|---|---|
| Largest CDP message to request | under 50 MiB; cap decoded bodies at 32 MiB for base64, 48 MiB for text |
| Client maximum message and frame | 64 MiB |
| WebSocket ping | every 30 s; fatal after 90 s without a pong |
| Idle allowance | never 300 s without traffic |
| Connect after create | docs say 5 minutes; measured connects took 0.3–1.7 s |
| Session `timeout` | 60–21600 s, validated by the API |
| Metadata | 512 characters, string values only |
| Concurrent sessions | 3, shared |
| Clock | estimate the offset at connect from the lowest round-trip sample; it held within 3 ms over 90 s |
| Context reuse | no wait was needed (1.7 s worked); keeping the plan's 5 s costs nothing |

### Noise to filter from the scanner's record

- Console: text `Browserbase keeping connection alive` with stack URL `./dist/scripts/keepalive.js`; text starting `browserbase-solving-`; a JSON string containing `"key":"browserbase-captcha-event"`.
- Execution contexts: names starting `__playwright_utility_world_`, `__bb_`, the name `Native Select Polyfill`, any origin `chrome-extension://cfiapcmfjmppkelmogiebeahmidbpdbi`, and the `w<hex>` world on the blank page.
- Log entries from source `rendering` about software WebGL fallback.
- Nothing needs filtering from the network record or the target list.

### Disclosure text: what must be true in it

The plan's sentence "The scanner turns off Browserbase's recording, logging, captcha solving and ad blocking for every scan, and stops a scan if Browserbase hides that the visit is automated" is now wrong on three counts. With the owner's defaults, the text has to say:

- Browserbase keeps a video of each tab and a log of the scanner's traffic with the browser.
- That log includes the content of scripts and data the scanner reads (bodies up to at least 125 KB each), every cookie value the scanner reads or sets, and any proxy or site password the scanner supplies. One page view produced an 11 MB log.
- Captchas on scanned pages are solved automatically.
- The browser does not tell the page it is automated, and the scan records that.

The password point deserves the owner's attention: with logging on, credentials passed through `Fetch.continueWithAuth` sit in Browserbase's logs for the plan's retention period.

### Could not determine

- Whether the 50 MiB message limit and the 300 s idle close belong to Browserbase or to this sandbox's proxy. Both should be re-measured once from the app's own network.
- The user's own external proxy: no proxy was available to test, including how its failure is reported.
- Whether service workers attach and can be paused; neither test page registered one.
- What `browserSettings.viewport` does to the window and screen values.
- The live view's own interface (clicking in it by hand), and how its input events look to a page.
- How long a Browserbase proxy keeps one exit IP beyond a minute, and whether it is the same across sessions.
- Idle behaviour beyond 6 minutes, and the 5-minute connect window.
- The exact size above which a reply is dropped from the logs, whether small POST bodies are stored, and how long logs and videos are kept.
- What the replay video contains; it was not downloaded.
- What the "Native Select Polyfill" extension changes on pages with `<select>` elements.
- The plan name, and whether concurrency 3 is a project setting or an account limit.
- Why my first script hung for three minutes locally before its first request. It did not recur.
- Fill and bid behaviour: one page load per side, from different countries, with no bids on either.