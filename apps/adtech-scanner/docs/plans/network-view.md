# Network view for AdTech Scanner: design and implementation plan

The Network view should be a full-height workbench route, plus a small digest tab in the report that opens it. Filtering, sorting and facet counts run in the webview on a compact row index that Rust delivers in chunks. Headers, bodies and decoded bid exchanges are fetched per request.

Two things I could not do:
- **Web research was blocked.** `developer.chrome.com` and `firefox-source-docs.mozilla.org` do not resolve in this session; they must be allowed in the network settings to check details. The patterns below come from my knowledge of DevTools, Netmonitor, Proxyman, HTTP Toolkit and trace waterfalls.
- **`storage/01-design.md` does not exist yet** (checked three times, last at the end). I designed against the obvious shape and the storage agent's interim files `storage/work/compress.txt` and `layout.txt`. What I need from storage is listed in section 10.

## What the code and the samples say

- **Real pages are bigger than the brief assumes.** The storage agent's eight captures in `storage/downloads/har/*/requests.json` hold 345 to 1,674 requests per single page load. Their average URL is 172 to 266 characters, and the longest URL in each capture runs from 5.8 KB to 19 KB.
- **3,000 rows is about two pages.** A 6-page browse scan can pass 8,000 rows, and a 15-minute manual session can pass 20,000. I set budgets at 3,000 and 10,000 rows.
- **Metadata with headers is 2.2 to 3.0 KB per request** in those captures. Headers therefore stay out of the list rows.
- **Body reads are cheap.** `layout.txt` measures a median read of 0.3 to 0.44 ms for one zstd-framed body in each seekable layout. Kept classes in the benchmark are css, html, js, json, text and POST bodies.
- **Today's recorder keeps almost nothing.** `crates/browser/src/capture/recorder.rs` holds url, initiator URL, status and the bid match per request, then drops it. It does not read timing phases, headers, sizes, the cache source or the initiator stack; it uses `redirectResponse` only to spot navigation redirects and skips preflights for bid matching.
- **Two clocks are involved.** Tracker times are page-relative; CDP times are monotonic, converted through `wall_clock_offset_ms`. The "late bid" insight depends on aligning them, so it needs a stated tolerance.
- **The report is a scrolling stack** capped at 1180 px (`PageContent`), beside a 240 px sidebar (`Sidebar.tsx`, `w-60`). A 3,000-row grid with a split panel cannot live inside it.
- **Reusable pieces exist.** `createTimeScale`, `BID_STATUS_STYLE`, the `hatch` utility, `FilterMenu`, `SearchInput`, the TanStack Virtual pattern in `ScriptsTable.tsx`, and `scopeReport` with the `page` search param.
- **The script viewer route is not in `router.tsx` yet.** Whichever of the two features lands first must create the shared CodeMirror code.

## 1. Information architecture

**Decision: both, with different jobs.**

- **Report tab "Network" is a digest, not a table.** It shows the summary bar, the overview strip, category chips with counts, and up to eight notable exchanges (failed bid calls, late answers, blocked requests, the slowest request before the first auction). Every element links into the workbench pre-filtered. It costs one small call.
- **Workbench route** `/scans/$scanId/network`, full height with `Page scroll={false}`, like the viewer. It has two views: Requests, and Bid exchanges (the bid inspector).
- **The page selection is shared.** The route uses the same `page` search param as the report, so switching between them keeps the page view.

Cross-links use stable ids stored on the records rather than a lookup command:

| From | To | Mechanism |
|---|---|---|
| Bid bar in `AuctionWaterfall` | Its HTTP exchange | New `Bid.exchange.requestId`. The tooltip gains "arrived +84 ms after close". |
| Auction header | Bid inspector | "Inspect exchanges" button: `?view=bids&auction=<id>` |
| Bidder check "endpoint said no-bid reason 7" | That exact response | New `DiagnosticCheck.requestIds`, opening `?req=<id>&dt=response` |
| Slot request in the Slots tab | The ad-server call | New `SlotRequest.requestId` |
| Script row and viewer header | Its request | New `ScriptRecord.requestId` |
| Finding evidence | Request | New `Evidence.requestId`, shown as a "View request" chip in `EvidenceBlock` |
| Request initiator | Script at the line | Viewer route with `line`, or `view=original&line&col` as fallback |
| Request | Auction, slot, finding | Summary tab links back to `tab=auctions`, `tab=slots`, `finding=<id>` |

- **History:** cross-links push a history entry, so Back returns to where the user was. Selection and filter edits inside the workbench use `replace`.
- **Initiator line mapping:** CDP gives line and column in the minified source, while the viewer shows formatted text. The backend should take about 80 characters at that offset, `squash` them, and run the existing whitespace-insensitive `locate` on the formatted lines, with the proportional offset as the hint. No source map is needed. When the source is not retained, the link goes to the Scripts tab row.

## 2. The table

**Rows** are 28 px single-line by default, 36 px in comfortable density.

| Column | Default | Notes |
|---|---|---|
| Status | on, 52 px | Icon plus code. Failed, blocked, cancelled and pending have distinct icons, never colour alone. |
| Name | on, flexible | Last path segment in medium weight, then dimmed query. Full URL in the tooltip. Navigation requests are bold with a page icon. Each redirect hop is its own row, marked with a redirect glyph. |
| Vendor | on, 128 px | Vendor or bidder badge; falls back to the registrable domain. First party is a neutral "Site" badge. |
| Ad outcome | on, 96 px | For bid and ad-server calls only: `BID_STATUS_STYLE` icon plus price, "no-bid 7", or "late +84 ms". This is the column no generic tool has. |
| Type | on, 56 px | doc, script, xhr, img, media, font, css, ping, ws, preflight, other |
| Initiator | on at 1200 px and wider | File name and line, as a link |
| Size | on, 64 px | Transferred; "(cache)", "(memory)" or "(SW)" when not from the network |
| Time | on, 60 px | Total duration |
| Waterfall | on, flexible, at least 200 px | See below |
| Method, Host, Page, Start, Protocol, Priority, Auction, Bidder, Decoded size, MIME | off | In the column chooser, a `DropdownMenu` of checkbox items saved to localStorage |

- **Sorting:** click a header, default is start time ascending. Sorting applies within groups.
- **Left out of v1:** column resize and reorder.
- **Grouping:** by page view (default when all pages of a multi-page scan are shown), by vendor, by type, or flat. Group headers are sticky and show count, transferred bytes, failed count and time span. Left and Right collapse and expand.
- **Virtualisation:** TanStack Virtual with fixed row height and overscan 8. The focused row is always kept rendered through `rangeExtractor`.
- **Waterfall bar is one DOM node per row.** Phases are painted with a `linear-gradient` background, so 40 visible rows cost 40 nodes.
- **Phases:** queued (2 px hairline), connecting (DNS, TCP and TLS merged), waiting, receiving.
- **Phase colours are one hue in three lightness steps,** so they are readable without hue and never collide with the bid-status colours. Add tokens `--net-connect`, `--net-wait`, `--net-receive` in `src/styles/app.css` and run them through the dataviz validator in both themes.
- **Status colours mean status only.** Red is failed. An amber hatch marks the part of a bid response after its auction closed.
- **The waterfall axis is the strip's window,** not the whole visit. Otherwise a 300 ms bid call is sub-pixel in a 60 s visit. The window defaults to the selected page view. The auction window is drawn as a faint band behind the rows, with the timeout line in amber as in `AuctionWaterfall`.

**Overview strip** (64 px, two lanes on one axis):
- **Network lane:** request starts per bin, with failed requests as red ticks beneath.
- **Ad lifecycle lane:** auction spans with their timeout tick, bidder timeouts, slot renders, consent resolved, and page load.
- **Boundaries:** page views are labelled dividers. Off-site gaps are hatched and labelled "not recorded".
- **Interaction:** dragging selects a window, which filters the table and rescales the waterfall. Clicking an auction span sets the window to that auction. The selected request is drawn as a span, so the alignment survives when the waterfall column is collapsed.
- **Data:** every marker is already in `ScanDetail` (auctions, slots, pages, `consent.resolvedAtMs`). The histogram comes from the loaded rows. No extra command is needed.

## 3. Filtering

**One source of truth: the query string `q`.** Chips are shortcuts that add and remove tokens, so links, recents and saved filters are plain strings.

- **Syntax:**
  - Tokens are separated by spaces.
  - `key:value` narrows by a property.
  - A leading `-` negates a token.
  - Commas mean OR within one key.
  - Quotes hold phrases.
  - `/…/` is a regular expression on the URL.
  - Bare text matches URL, vendor and bidder.
- **Keys:** `host:` (suffix match), `url:`, `path:`, `method:`, `status:` (`204`, `4xx`), `type:`, `cat:`, `vendor:`, `bidder:`, `unit:`, `page:` (1-based, as displayed), `auction:`, `initiator:`, `mime:`, `larger-than:` (`50k`), `slower-than:` (`500ms`).
- **`is:` values:** `failed`, `blocked`, `cancelled`, `pending`, `bid`, `late`, `nobid`, `won`, `cached`, `sw`, `redirect`, `post`, `first-party`, `third-party`, `before-auction`, `no-hb-targeting`.
- **Unknown keys** fall back to text and show an inline note. Autocomplete of keys and values comes from the facets, using the existing `cmdk`.
- **Category chips (single select):** All, Bid requests, Ad server, Wrapper and Prebid, Consent, Identity and sync, Measurement, First-party. I add **Creative**, because ad creative assets are a large share of a page's requests and otherwise pollute "other".
- **State chips (toggles, AND-ed):** Failed, Slow, Blocked or cancelled.
  - Failed is a status of 400 or more, or a network error.
  - Blocked or cancelled is separate, because cancellation on navigation is normal noise.
  - Slow is over 1,000 ms, or a bid call that used over 80% of its auction timeout. The tooltip states the rule.
- **Counts on chips** are computed with every other filter applied except the chip's own dimension.
- **Ad lifecycle menu:** "Before first auction" (`is:before-auction`, per page; on a page that never auctioned every request matches, which is exactly the reference bug) and "During auction N" (`auction:N`, requests overlapping that auction's window).
- **Recipes and recents in v1:** five built-in recipes and the last eight queries in localStorage. Examples: "Bid calls that failed or came late" is `cat:bid is:failed,late`; "Ad requests without Prebid keys" is `cat:adserver is:no-hb-targeting`. User-named saved filters are v1.1.
- **Summary bar (footer):** "412 of 3,104 requests · 1.8 of 14.2 MB transferred · 9 failed · 3 late bids · Google 212, Rubicon 31, …". When a persistent profile was used it adds "61% from cache: not a cold load".

## 4. Request detail panel

- **Split:** resizable, docked right or bottom, with dock and size saved to localStorage. I recommend the shadcn `resizable` component (react-resizable-panels); the viewer needs the same thing. Check the current version with `npm view`.
- **Default dock:** right when the content area is at least 1,000 px wide, bottom below that. With a right dock the table collapses to Status, Name, Ad outcome and Time.
- **Maximise** hides the table; Esc restores it.

**Tabs, in order:**

1. **Summary.** I merge DevTools' "General" block and the brief's "Ad context" into the first tab, so non-ad requests never show an empty tab.
   - General facts: URL, method, status, vendor, page view, served from, redirect chain as hop list.
   - For ad requests: auction, ad units, bidder, outcome per ad unit, and a plain-language verdict line (section 5).
   - Related findings and bidder checks as links.
   - Match quality is stated: "matched by request id" or "matched by timing".
2. **Headers.** Response then request. Redacted values render as `•••• redacted` with the reason, for example "credential: not stored". A filter box narrows by header name.
3. **Payload.** Query string parsed into a key and value table, with nested decoding for `cust_params` and `prev_scp` on GAM requests. The POST body is pretty-printed. Bid requests get a Decoded and Raw toggle.
   - Decoded shows the imp list with sizes and floors, `tmax`, schain nodes and completeness, and regs flags.
   - Consent and identity are shown as presence only: consent string present, eids sources. The string and the ids are never rendered in the decoded view.
4. **Response.** Decoded and Raw for bid responses.
   - Decoded has a seat and bid table: imp, price, size, adomain, crid, deal, creative bytes.
   - `nbr` is decoded to words.
   - Prebid Server `ext.responsetimemillis` and `ext.errors` are listed per bidder.
   - JavaScript bodies show "Open in script viewer". Body-not-kept states are in section 7.
5. **Timing.** Labelled phase bars with milliseconds, plus preflight time when there was one, and where the request sits relative to its auction.
6. **Initiator.** Call stack with script links to the line; "parser" or "preload" when there is no stack.

**Cookies tab: not built** unless storage keeps cookies. I recommend it does not.

**Body viewer recommendation: CodeMirror 6 read-only only, no separate JSON tree.**
- CM6 gives viewport rendering, folding, Cmd+F search and copy for free.
- "Copy path" comes from the Lezer JSON tree at the cursor, shown in the status line (`seatbid[0].bid[2].price`).
- Add `@codemirror/lang-json` and `@codemirror/lang-xml` (VAST) in v1; HTML highlighting is v1.1 and plain text until then.
- JSON is pretty-printed in the webview up to 2 MB.

**Untrusted content:**
- Text nodes only. No HTML rendering, no image preview, no link detection, no remote loads. The CSP is unchanged.
- `highlightSpecialChars` and forced `direction: ltr`, as in the viewer.
- Bodies arrive in 2 MB chunks with "Load more", up to 16 MB.
- Lines over 10,000 characters turn highlighting off and force wrap.
- Binary bodies show type and size only in v1.

**Copy:**
- Copy URL, copy as cURL, copy payload and copy response. cURL is a pure TypeScript function.
- When a header was redacted, cURL contains a placeholder and a toast says the command will not reproduce a signed-in request.
- When the content carries user identifiers (bid requests, cookies if stored), an `AlertDialog` warns once per session.
- No "open in browser" for bid or ad endpoints, and no edit-and-resend: both would send real traffic to exchanges.

## 5. Bid inspector

A view of the workbench for one auction: `?view=bids&auction=<id>`.

- **Layout:** rows are bidders, columns are ad units, with a sticky bidder column.
- **Cell content:** status icon, price or no-bid reason, and network response time.
- **Shared requests:** cells that share one HTTP request are visually tied. Most adapters send one request with several imps.
- **Clicking a cell** opens the same detail panel.
- **The assembly is client-side** from `ScanDetail.auctions` plus the rows filtered to that auction; verdicts are computed in Rust.

Verdicts, shown as sentences:

| Verdict | Sentence |
|---|---|
| In time | "Answered in 212 ms, 38 ms before the auction closed" |
| Late on the network | "Response arrived after the auction closed (+84 ms)" |
| In time but timed out in the page | "The response arrived 40 ms before the timeout, but Prebid recorded a timeout: the page was busy or the adapter was slow to parse" |
| Never answered | "No response before the visit ended" |
| Failed | The error or blocked reason |
| Not sent | "Prebid announced this bid but no request left the page", linking to the params check. This covers the demo's `triplelift` bug. |

- **Tolerance:** the two clocks can disagree by a few milliseconds. Below a tolerance (start at 10 ms, uncalibrated) the text is "arrived as the auction closed", with no number.
- **Preflight:** when a CORS preflight preceded the bid call, the cell shows "preflight +45 ms".
- **Prebid Server:** one request fans out to many bidders. Those rows sit under a "Prebid Server, 1 request" bracket. Each cell shows the server-side time from `ext.responsetimemillis`, labelled "server-side", and errors from `ext.errors`. All of those cells open the same exchange, scrolled to that seat.

## 6. Multi-page scans

- **The log is always preserved across navigations,** with page views as groups.
- **A request belongs to the page view it started on,** the same rule as auctions.
- **Time base:** page-relative when one page is selected; visit-absolute (`pages[page].startedAtMs + t`) on the strip when all pages are shown.
- **Navigation requests** are the first row of each group. A route-change page has no document request and its header says so.
- **`[` and `]` move between page views.** This conflicts with the viewer plan, which uses `[` and `]` to toggle side panels. I recommend the viewer move to a different key so the two routes agree.
- **Large manual sessions:** above 5,000 rows, page groups other than the selected one start collapsed. Above 20,000, rows load per page on expand. Above 50,000, the list is cut with a banner that says so.

## 7. States and honesty

| Situation | What the user sees |
|---|---|
| Scan predates network storage | Digest tab: "No network record for this scan. Network recording started with version X." plus "Scan again". Cross-link buttons do not render. |
| Retention was off | "Network recording was off for this scan" with a link to Settings |
| Whole archive evicted by the cap | "The network record of this scan was removed to stay under the storage limit" |
| Body not kept by type | "Body not kept (image, 48 KB)" |
| Body too large, evicted, or lost in Chrome's buffer | A separate sentence for each |
| Empty body (204) | "No content" |
| Body truncated at capture | "First 2 MB of 7.4 MB kept" |
| Redacted header | Placeholder with the reason |
| Never finished | Hourglass status, open-ended hatched bar, "No response before the visit ended" |
| Cancelled by navigation | "Cancelled when the page navigated" |
| Cache or service worker | Stated in Size and Summary; transferred bytes shown as 0 |
| WebSocket and EventSource | One row per connection with duration; "Frames are not recorded" |
| Left the scanned site | Gap on the strip |
| Timing-only match to an auction | Labelled as such |
| No decoder for the adapter | "No decoder for this adapter yet", Raw only |

## 8. Performance and accessibility

**Decision: load rows into the webview; Rust does not filter.**

- **Row size:** a list row without headers is about 400 to 450 bytes of JSON. That is my estimate from the measured URL lengths plus about 170 bytes of fields, and it is not measured.
- **Payload:** about 1.3 MB for 3,000 rows and about 4.5 MB for 10,000.
- **Client cost:** filtering 10,000 rows against a precomputed lowercase haystack is a few milliseconds in JavaScript (estimate). A round trip per keystroke would cost more and would force two implementations of the query language, Rust and the fake backend.
- **Chunking:** rows arrive in time-ordered chunks of 2,000 (about 0.9 MB), so the first paint does not wait for the rest.
- **What Rust must serve** is anything the rows cannot answer: headers, bodies, decoded exchanges, and later a body search that returns an id set to intersect.
- **When to revisit:** if the storage design puts requests in SQLite with indexes and sessions regularly exceed 50,000 rows.

Budgets, asserted in Playwright on Chromium with the fake backend. WKWebView must be checked by hand on a Mac.

| Action | 3,000 rows | 10,000 rows |
|---|---|---|
| Route open to first rows painted | 250 ms | 250 ms |
| All rows loaded | 600 ms | 1.5 s |
| Filter keystroke to repaint (p95) | 50 ms | 100 ms |
| Sort | 60 ms | 150 ms |
| Select row to panel facts (no IPC) | 50 ms | 50 ms |
| Headers and decoded exchange | 150 ms | 150 ms |
| Body up to 256 KB shown | 300 ms | 300 ms |

- **DOM:** at most about 45 rows mounted.
- **URL:** writes of `q` are debounced by 300 ms.
- **Live updating: not v1.** The report does not exist until the scan completes, and the live view already has the Visit panel. A live tail during manual sessions is the one place it earns its keep; put it in "later", fed by a batched event, never one event per request.

**Keyboard:**
- Up, Down, Home, End, PageUp, PageDown move the active row.
- Enter or Space opens the detail; Tab moves into it; Esc closes the detail, then clears the window.
- Cmd+F or `/` focuses the filter.
- `[` and `]` move between page views.
- Left and Right collapse and expand groups.
- Cmd+C copies the active row's URL.

**Screen readers:**
- `role="grid"` with `aria-rowcount` and `aria-rowindex`.
- Focus stays on the scroll container with `aria-activedescendant`, so virtualisation never drops focus.
- Group headers are rows with `aria-expanded`.
- The waterfall cell has a text label ("started 1.24 s, took 312 ms: waiting 280, receiving 32").
- A polite live region announces the filtered count.
- The strip is `role="img"` with a summary. Its markers are also in an "Events" menu, and the window is a two-thumb `Slider`, so nothing is pointer-only.

## 9. Wireframes

**Requests view, 1280x820 (content about 1040 px), detail docked right**

```
+----------------------------------------------------------------------------------------------------+
| < Report  pub.example > Network      [Requests | Bid exchanges]   Pages [All|1|2|3]  [Columns] [..]|
| [/ cat:bid is:late                                  x]  Lifecycle v  Recipes v  Group: Page v      |
| (All 3104) (Bid 96) (Ad server 22) (Wrapper 14) (Consent 9) (Identity 141) (Measure 212) (Creative |
|  388) (First-party 310)   | [Failed 9] [Slow 17] [Blocked 4]                                       |
+----------------------------------------------------------------------------------------------------+
| net  ..:|||:::.. . ..:|:.   | P2 .:||:.. .  .:|.          | P3 .:|:.                               |
| ads   C  [==A1==|t]  r r    |    [=A2=|t] r               |    (no auction)                        |
|      0s      2s   [####window####]   8s      12s                 20s                 28s           |
+--------------------------------------------------+-------------------------------------------------+
| St  Name                  Ad outcome      Time   | POST rtb.openx.net/openrtbb/prebidjs        [x] |
| v Page 1  /  (entry)   1,204 req  6.1 MB  3 fail | Summary  Headers  Payload  Response  Timing  In |
| 200 prebidjs  openx       - no-bid 7      212ms  |-------------------------------------------------|
|>200 fastlane.json rubicon ! late +84ms    334ms  | Auction 1 - ad units top, sidebar_1             |
| 200 translator pubmatic   $ 1.42          188ms  | Bidder openx - client-side - matched by id      |
| ERR pbjs  ix              x failed        41ms   | Outcome: no bid, reason 7 (Blocked Publisher    |
| 200 auction  PBS (3)      $ 0.88          301ms  |  or Site)                                       |
| ...                                              | Verdict: answered in 212 ms, 38 ms before the   |
| v Page 2  /climate/kelp  (link)  980 req         |  auction closed                                 |
| ...                                              | Related: bidder check "endpoint said no-bid" >  |
|                                                  |          finding "openx never bids" >           |
|                                                  | [Copy URL] [Copy as cURL] [Open in Auctions]    |
+--------------------------------------------------+-------------------------------------------------+
| 96 of 3,104 requests - 0.4 of 14.2 MB - 9 failed - 3 late bids - Google 212, Rubicon 31, OpenX 12  |
+----------------------------------------------------------------------------------------------------+
```

**With the detail closed, the full column set**

```
| St  Name               Vendor     Ad outcome    Type  Initiator      Size    Time   Waterfall      |
| 200 fastlane.json      Rubicon    ! late +84ms  xhr   prebid.js:411  1.2 KB  334ms  -..====##|t    |
| 200 ads?gdfp_req=1...  Google     no hb keys    xhr   gpt.js:88      9.8 KB  142ms       .===-     |
```

**Bid exchanges view**

```
| Auction 1 v   started 1.24 s - timeout 250 ms - closed 1.49 s - 6 bidders - 3 ad units             |
+-------------+---------------------+---------------------+---------------------+--------------------+
| Bidder      | top (970x250)       | sidebar_1 (300x250) | anchor (320x50)     | Request            |
| rubicon     | ! late +84 ms       | ! late +84 ms       | ! late +84 ms       | 1 request, 334 ms  |
| pubmatic    | $ 1.42  188 ms      | - no bid            | $ 0.61  188 ms      | 1 request          |
| openx       | - no-bid 7  212 ms  | - no-bid 7          |                     | preflight +45 ms   |
| ix          | x net error         | x net error         | x net error         | failed 41 ms       |
| triplelift  | $ 0.40  120 ms      | o not sent (params) | o not sent (params) | 1 request          |
| [Prebid Server: 1 request, 301 ms]                                                                 |
|   appnexus  | $ 0.88  server 140  | - no bid            |                     | shared             |
|   sovrn     | t server timeout    | t server timeout    |                     | shared             |
+-------------+---------------------+---------------------+---------------------+--------------------+
| Legend: $ bid  W won  - no bid  ! late  x failed  o not sent  t timeout     click a cell to open   |
```

**OpenRTB response, Decoded**

```
| Response   [Decoded | Raw]                                   application/json - 3.1 KB    [Copy]   |
| id 7f3a..  currency USD  nbr none                                                                  |
| Seat      Imp (ad unit)     Price   Size      Advertiser     Creative   Deal      Markup           |
| pubmatic  2a (top)          1.42    970x250   acme.example   cr-88121   -         2.4 KB           |
| pubmatic  2c (anchor)       0.61    320x50    shoes.example  cr-1077    PM-DEAL9  1.1 KB           |
| Prebid Server timing: appnexus 140 ms - sovrn error "timeout" (code 1)                             |
| No-bid form:  nbr 7 - Blocked Publisher or Site (OpenRTB 2.5, list 5.24)                           |
| Raw:  2   "seatbid": [                           path: seatbid[0].bid[1].price  [Copy path]        |
```

**Unavailable states**

```
| (icon)  No network record for this scan                                                            |
|         Network recording started with version X. This scan was made before that.                  |
|         [Scan again]   [Scan again without AI (free)]                                              |

| Response:  Body not kept (image, 48 KB). Images, video and fonts are recorded as metadata only.    |
| Response:  Body removed to stay under the storage limit (was 212 KB).  [Storage settings]          |
```

**At 1024x680 (content about 780 px)**
- The detail docks to the bottom at 50%, with a maximise button.
- Category chips collapse into one `FilterMenu`; state chips stay.
- The strip drops to 40 px and keeps both lanes.
- Columns are Status, Name, Vendor, Ad outcome, Time and a 120 px waterfall. Type, Size and Initiator move to the chooser.
- The bid inspector scrolls horizontally past three ad units with the bidder column sticky.

## 10. Contract, structure, tests, plan

### Contract (additive, in `crates/model/src/lib.rs`)

```rust
pub enum NetworkAvailability { Available, NotRecorded, Disabled, Evicted }
pub struct NetworkArchiveInfo { availability, request_count: u32, truncated: bool }
// ScanDetail.network: Option<NetworkArchiveInfo>   (#[serde(default)]; None reads as NotRecorded)

pub enum ResourceKind { Document, Script, Stylesheet, Image, Media, Font, Xhr, Fetch, Ping,
                        WebSocket, EventSource, Preflight, Other }
pub enum NetworkCategory { Bid, AdServer, Wrapper, Consent, Identity, Measurement, Creative,
                           Site, OtherThirdParty }
pub enum RequestOutcome { Ok, HttpError, Failed, Blocked, Cancelled, Pending }
pub enum ServedFrom { Network, DiskCache, MemoryCache, ServiceWorker, Prefetch }
pub enum ExchangeVerdict { InTime, LateOnNetwork, InTimeButTimedOut, NeverAnswered, Failed, NotSent }
pub enum BodyState { Kept, Empty, NotKeptType, TooLarge, Truncated, Evicted, Unavailable,
                     RetentionOff, Pending }

pub struct NetworkRow {
    id: String, seq: u32, page: u32, method: String, url: String, host: String,
    kind: ResourceKind, mime: Option<String>, status: Option<u16>, outcome: RequestOutcome,
    error: Option<String>, category: NetworkCategory, vendor: Option<String>,
    bidder: Option<String>, first_party: bool, initiator: Option<InitiatorRef>,
    started_at_ms: f64 /* page-relative */, duration_ms: Option<f64>,
    phases: Option<RowPhases /* queue, connect, wait, receive */>,
    transferred_bytes: f64, decoded_bytes: f64, served_from: ServedFrom,
    redirect_of: Option<String>, preflight_of: Option<String>,
    has_request_body: bool, has_response_body: bool, no_hb_targeting: bool,
    ad: Option<AdLink>,
}
pub struct AdLink { auction_id: String, ad_units: Vec<String>, via_server: bool, exact: bool,
                    verdict: ExchangeVerdict, late_by_ms: Option<f64>, cpm: Option<f64>,
                    nbr: Option<u32> }
pub struct NetworkRequestDetail {
    row: NetworkRow, request_headers: Vec<Header>, response_headers: Vec<Header>,
    protocol: Option<String>, priority: Option<String>, remote_address: Option<String>,
    timing: Option<TimingDetail>, redirect_chain: Vec<RedirectHop>, preflight_ms: Option<f64>,
    stack: Vec<StackFrame /* function, url, script_id, line, column, formatted_line */>,
    request_body: BodyRef, response_body: BodyRef /* state, bytes, mime, text|binary, script_id */,
    ad_context: Option<AdContext /* units with BidStatus, auction end, timeout, finding_ids, check codes */>,
    decoded: Option<DecodedExchange /* request: imps, floors, schain, regs, eid sources;
                                       response: seats, bids, nbr, server timings and errors */>,
}
pub struct Header { name: String, value: Option<String>, redaction: Option<String> }
```

Additive link fields:
- `Bid.exchange: Option<{ request_id, verdict, late_by_ms, network_ms }>`
- `SlotRequest.request_id`
- `ScriptRecord.request_id`
- `Evidence.request_id`
- `DiagnosticCheck.request_ids: Vec<String>`

Commands:

| Command | Args | Returns |
|---|---|---|
| `get_network_summary` | `{ scanId }` | Totals, per-category counts and bytes, failed, late bids, up to 8 notable rows |
| `list_network_requests` | `{ scanId, pages?, cursor?, limit? }` | `{ rows, nextCursor, total }`, time-ordered |
| `get_network_request` | `{ scanId, requestId }` | `NetworkRequestDetail` |
| `get_network_body` | `{ scanId, requestId, side, offset?, maxBytes? }` | `{ state, text?, offset, totalBytes, complete }` |
| `export_network_har` (v1.1) | `{ scanId, requestIds?, path }` | Path written |

Deviations from the brief:
- **No filter, sort or facet arguments** on the list command, for the reasons in section 8.
- **No bid-exchange lookup command.** The ids on `Bid`, `SlotRequest` and `DiagnosticCheck` make it unnecessary, and they also let the Auctions tab show the late-bid sentence without loading any rows.
- **Decoding lives in Rust,** next to `bid_endpoints.rs`, so the bid inspector, the detail panel, the rules and Phase 2 agree. v1 decodes generic OpenRTB 2.x and Prebid Server `ext`.

**Needed from the storage design:**
- A stable request id and one record per redirect hop.
- The preflight link.
- Full CDP timing.
- The `ExtraInfo` headers.
- The initiator stack with script ids.
- A per-body state enum that distinguishes every row of the table in section 7.
- A redaction reason per header.
- Whether cookies are stored (I recommend not).
- Random read of one body by reference with a byte range.
- The archive available on failed and cancelled scans.

### Frontend structure

```
src/features/network/
  index.ts
  NetworkWorkbench.tsx        route body: toolbar, strip, split
  NetworkDigest.tsx           the report tab
  query/  parse.ts  match.ts  facets.ts  recipes.ts          (pure, tested)
  table/  RequestGrid.tsx  columns.ts  grouping.ts  waterfall-bar.ts  use-grid-keys.ts
  strip/  OverviewStrip.tsx  strip-layout.ts  lifecycle-markers.ts
  detail/ DetailPanel.tsx  SummaryTab  HeadersTab  PayloadTab  ResponseTab  TimingTab
          InitiatorTab  DecodedRequest.tsx  DecodedResponse.tsx  BodyView.tsx  curl.ts
  bids/   BidInspector.tsx  bid-matrix.ts  verdict-text.ts
  use-network-rows.ts         chunked loading through TanStack Query
src/shared/editor/            useCodeMirror, theme, .tok-* CSS, search panel (shared with the viewer)
src/shared/ui/resizable.tsx   shared with the viewer
src/app/routes/NetworkRoute.tsx
```

- **Search schema:** `q`, `page`, `group`, `sort`, `req`, `dt`, `view`, `auction`, `from`, `to`.
- **localStorage only:** dock, sizes, columns, density, recents.
- **Edits to existing files:**
  - `router.tsx`: add `"network"` to `REPORT_TABS` and the new route.
  - `ScanRoute.tsx`: render the digest tab.
  - `AuctionWaterfall.tsx`: exchange link and verdict in the tooltip.
  - `BidderChecks.tsx`, `EvidenceBlock.tsx`, `SlotsTab.tsx`, `ScriptDetails.tsx`: link chips.
  - `src/lib/api/port.ts` and both adapters: the new commands.

### Fake backend

`src/lib/fake-backend/network.ts`, generated from the existing scenario so every link resolves. A 3-page browse scan of about 400 requests (190, 130, and 80 on a route change) containing:

- Documents, and scripts matching the `ScriptRecord`s.
- GAM calls per `SlotRequest`, one without `hb_` keys.
- Bid calls per `Bid` with OpenRTB payloads: `openx` with `nbr` 7, `rubicon` late by 84 ms, `ix` failed, and one answered in time but marked timeout.
- One Prebid Server call fanning out to three bidders, and one bid call with a preflight.
- A three-hop cookie-sync redirect chain.
- CMP calls and beacons.
- Metadata-only images.
- One pending, one cancelled by navigation, one from disk cache, one from a service worker, one WebSocket.
- A 3 MB JSON body, a truncated body, an evicted body, a redacted Cookie header.
- A body containing `<img onerror>` and bidi characters.

Also: scan ids for `notRecorded`, `disabled` and `evicted`, and a generator knob for 3,000 and 10,000 rows.

### Tests

- **Vitest:**
  - Query parser (tokens, negation, quotes, OR, units, unknown keys, error positions), matcher and facet counts.
  - Grouping and sort stability; `is:before-auction` on a page without auctions.
  - Strip binning and marker positions across page boundaries.
  - Phase gradient maths with minimum widths.
  - Verdict text including the tolerance.
  - Bid matrix with shared and Prebid Server exchanges.
  - cURL with redactions.
  - JSON path at cursor.
- **Playwright (fake backend):**
  - The budgets in section 8 at 3,000 and 10,000 rows.
  - A keyboard-only path: filter, arrows, Enter, tabs, Esc, `[` and `]`.
  - Every cross-link in both directions, with Back returning.
  - URL reload restoring filter, selection and tab.
  - axe on each state in both themes.
  - The inert-content case.
  - The layout at 1024x680.
- **Rust:**
  - Classification table; exchange matching (exact and timing).
  - Verdicts with clock tolerance; redirect and preflight linking.
  - OpenRTB decode fixtures.
  - Page scoping and cursor paging of the list command.
- **e2e-tauri (demo site):**
  - The `openx` check opens the response with `nbr` 7 decoded.
  - A 250 ms timeout shows two late or never-answered cells.
  - `triplelift` shows "not sent".
  - No CSP violations.
  - A scan with retention off shows the honest state.
  - This needs one demo change: a stand-in endpoint that answers after the timeout.

### Ordered plan

**v1**
1. **Contract:** types, link fields, bindings, `ApiPort` methods with stubs, route and search schema.
2. **Capture and storage (backend):** full request records, classification, exchange matching, verdicts, decode, the four commands.
3. **Fake data set** and `use-network-rows`.
4. **Grid:** columns, virtualisation, keyboard, grouping, sort, summary bar.
5. **Query language,** chips with counts, lifecycle menu, recipes, URL state.
6. **Overview strip** and waterfall bars with the shared window.
7. **Detail panel:** Summary, Headers, Payload, Response, Timing, Initiator; `shared/editor`; body states; copy actions.
8. **Decoded OpenRTB views** and the bid inspector.
9. **Digest tab** and all cross-links in the existing tabs.
10. **Unavailable states,** the 1024x680 pass, tests, then the simplify and review passes.

**v1.1**
- HAR export. I recommend building it: publishers hand a HAR to their SSP contact as evidence. It is cheap once the archive exists and covers every generic feature I left out.
- User-named saved filters.
- Body and header text search served by Rust.
- HTML highlighting.
- AppNexus and Rubicon decoders.

**Later**
- Live tail in manual sessions.
- Column resize.
- Grouping by ad slot or frame.
- WebSocket frames.
- Hex view.
- Comparing the network of two scans.

**Not this product:** throttling, request blocking, HAR import, edit-and-resend, rendered previews.

### Questions for the product owner

1. **Do you accept that the Network tab in the report is a digest that opens the workbench, not an inline table?** An inline table would get about 400 px of height inside the scrolling report.
2. **Should "Copy as cURL" exist for bid endpoints at all?** I kept it with a warning; removing it would be the stricter reading of "never send traffic to exchanges on the user's behalf".
3. **Is 10 ms an acceptable starting tolerance for the late-bid number?** It is uncalibrated, like the other thresholds in the handover.

### Critical Files for Implementation
- /home/user/Atlas/apps/adtech-scanner/crates/model/src/lib.rs
- /home/user/Atlas/apps/adtech-scanner/crates/browser/src/capture/recorder.rs
- /home/user/Atlas/apps/adtech-scanner/src/app/router.tsx
- /home/user/Atlas/apps/adtech-scanner/src/features/report/auctions/AuctionWaterfall.tsx
- /home/user/Atlas/apps/adtech-scanner/src/lib/api/port.ts