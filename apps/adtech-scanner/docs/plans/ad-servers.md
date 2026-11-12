I did not save the plan to `/tmp/claude-0/-home-user/ee93eb55-ef69-5991-bc2c-57c8a322dcdf/scratchpad/adservers/01-plan.md` or download any library to disk: this session's read-only rule forbids creating files anywhere, scratchpad included. The libraries were fetched and read in memory, and the plan below is the full deliverable for you to save verbatim.

# Multi-ad-server support: research, design and implementation plan

## 0. What was checked, and what is memory

Read from the live libraries on 2026-11-12:

| Source | Version | URL |
|---|---|---|
| Xandr AST | `AST v0.66.0 Updated: 2024-11-18` | `https://acdn.adnxs.com/ast/ast.js` |
| Equativ smart.js | no version string found | `https://ced.sascdn.com/tag/788/smart.js` (788 is bild's network id) |
| Adform adx.js | not read for version | `https://s1.adform.net/banners/scripts/adx.js` |
| Adition srp.js | not read for version | `https://imagesrv.adition.com/js/srp.js` |
| bild.de wrapper | current | `https://www.asadcdn.com/adlib/pages/bild.js`, plus the earlier probes in `/tmp/claude-0/-home-user/ee93eb55-ef69-5991-bc2c-57c8a322dcdf/scratchpad/bild/` |

Real request/response pairs captured by hand, one or two requests each:

- **Xandr UT no-bid.** `POST https://ib.adnxs.com/ut/v3` with `{"tags":[{"id":13144370,"sizes":[{"width":300,"height":250}],"uuid":"aaaa1111bbbb2222",...}]}` answered `{"version":"3.0.0","tags":[{"uuid":"aaaa1111bbbb2222","tag_id":13144370,"auction_id":"7383364334209165474","nobid":true,"ad_profile_id":0}]}`. A first attempt without a `device.useragent` answered `{"error":"unknown"}`. The 13144370 placement id is from memory.
- **Smart onecall fill.** `POST https://www8.smartadserver.com/1/call` (307 to `?cklb=1`) with `siteId 207435, pageId 896536, formatId 62913, tagId sas_62913` (ids from memory) answered a map keyed by tag id: `{"sas_62913":{"ScriptType":"iframe","ScriptSrc":"https://www8.smartadserver.com/h/nshow?...&insid=9268001,7537582","ScriptWidth":"500","ScriptHeight":"500","ScriptNoad":"https://tracking-api-use1.smartadserver.com/h/aip?uii=...&statid=5...","FormatId":62913}}`.
- **Adhese no-fill.** `GET https://ads-demo.adhese.com/json/sl_adhese_prebid_demo_-leaderboard` answered `[]`.

Not reachable or not attempted: the AST reference on learn.microsoft.com is rendered by script (no text extracted), `documentation.adhese.com` did not resolve, and no web search was run. Market shares, the Adhese fill shape, GAM's request parameters, and everything about FreeWheel, Kevel, Revive and IMA are from memory and marked (M).

### What bild.de actually does

- **`apntag` is the standard global.** The first statement of bild.js is `window.googletag=window.googletag||{cmd:[]},window.apntag=window.apntag||{anq:[]}`. The wrapper calls `apntag.loadTags(...)`, reads `apntag.requests.tags[...]` and subscribes to `adAvailable, adLoaded, adNoBid, adError, adLoadedMediated, adNoBidMediated`. The earlier "no `apntag` global" observation was a timing or probe-list artefact, because the adlib only starts after consent. A plain AST hook will work there. I did not find where ast.js itself is loaded from.
- **The wrapper patches `XMLHttpRequest.prototype.open/send`** to read `ut/v3` answers and to retry failed ones. Our tracker must never touch XHR or fetch.
- **Smart is not called through `sas`.** A wrapper function `SmartAdServerASMI` writes a script tag: `https://www6.smartadserver.com/ac?nwid=788&siteid=..&pgname=..&fmtid=..&inSequence=true&visit=S&tmstp=..&tag=sas_<fmtid>&gdpr_consent=..&gdpr=1&async=1&tgt=<...;isAppNexus=true>`. In the probe, 8 `/ac` calls left 344 ms after the single UT POST. The source of the calls, together with `isAppNexus=true`, suggests Smart is a passback behind Xandr; that reading is an inference.
- **Seven or eight `/h/aip` requests followed the Smart calls.** In the real onecall answer, `/h/aip` is the `ScriptNoad` URL. Whether every `/h/aip` means "no ad" is not verified, so the design matches the exact noad URL from the answer instead of trusting the path.
- **Header-bidding keys are non-standard.** The Prebid module builds `hb_bidder`, `hb_size`, `hb_adid`, `hb_mediaType`, `pbAdCode`, and `hb_pb` as integer cents. Key detection must be "any `hb_*`", not exact names.
- **Video** goes to `ib.adnxs.com/ptv?...member=7823`.

## 1. Inventory

Ranking for BE/NL/DE publishers is from memory (M); verify with the owner's customer list.

| # | Ad server | Tier | JS adapter | Network entry |
|---|---|---|---|---|
| 1 | Google Ad Manager GPT | done | exists | yes (needed for iframes, merge) |
| 2 | Xandr / Microsoft Monetize AST | phase 1 | yes | yes |
| 3 | Equativ / Smart AdServer | phase 1 | yes | yes |
| 4 | Adhese (BE) | phase 2 | no (two SDK generations) | yes |
| 5 | Adition + Yieldlab (DE) | phase 2 | later, if a customer needs it | yes |
| 6 | Adform publisher tag | phase 2 | no | yes |
| 7 | Video: GAM VAST via IMA, Xandr `/ptv`, FreeWheel, SpringServe, smartclip | phase 3 | no | yes, as "video ad calls" |
| 8 | AdSense `adsbygoogle` | phase 2 | no | yes |
| 9 | Amazon APS | exists as bid source | exists | `/e/dtb/bid` already classed as Bid |
| 10 | Kevel, Revive | phase 3 | no | one table row each |
| 11 | Outbrain / Taboola | out of slots | no | new category "Content recommendation" |

Per-server facts. "V" means read in the library or captured; "M" means memory.

**GPT (done).** Request is `GET securepubads.g.doubleclick.net/gampad/ads` with `iu_parts`, `enc_prev_ius`, `prev_iu_szs`, `prev_scp`, `cust_params` for single-request mode, and `iu`, `sz`, `scp` otherwise (M). Header-bidding keys sit in `prev_scp`/`scp`; APS shows as `amznbid`/`amzniid` (M). Fill comes from `slotRenderEnded.isEmpty`.

**Xandr AST (V, 0.66.0).**
- Define: `apntag.defineTag({targetId, tagId | invCode+member, sizes, keywords, ...})`; state lives in `apntag.requests.tags[targetId]` with `uuid, targetId, invCode, member, sizes, keywords, utCalled, showTagCalled, displayed`.
- Request: `apntag.loadTags([targetIds])`, one `POST https://ib.adnxs.com/ut/v3` for all tags. If any tag has `prebid:true` the library itself uses `/ut/v3/prebid`.
- Body: `{tags:[{uuid, id | code, sizes, primary_size, keywords:[{key,value:[..]}], disable_psa, ...}], member_id?, gdpr_consent, referrer_detection, device, eids, sdk}`.
- Header bidding: `apntag.setKeywords(targetId, {hb_pb:..})`, tag `keywords`, or page keywords via `setPageOpts`.
- Events, complete list from the constants: `adRequested, adAvailable, adLoaded, adLoadedMediated, adCodeRenderStart, adNoBid, adNoBidMediated, adRequestFailure, adDefault, adError, adCollapse, adBadRequest`.
- `apntag.onEvent(name, cb)` with two arguments registers for all tags. But `adRequested` is emitted with the target id only, so a wildcard handler gets no arguments. Per-tag registration is needed for that event.
- Answer: `tags[].nobid:true`, or `tags[].ads[0]` with `cpm`, `creative_id`, `buyer_member_id`, `deal_id`, `content_source`, `ad_type`, `rtb.banner.{width,height}`.
- Refresh: `apntag.refresh([targetIds])`. Lazy load is the wrapper's job.
- The library refers to `apntag.` literally 188 times, so the global cannot be renamed.

**Equativ / Smart (V).**
- Global `sas` with `sas.cmd`; `sas.setup({networkid, domain, async, renderMode, inSequence})`; `sas.call(callType, ad, options)`; `sas.render`, `sas.refresh`, `sas.clean`, `sas.enableLazyLoading`, `sas.passback`, `sas.setHeaderBiddingWinner(tagId, bid)`.
- The tag id (`sas_<formatId>`) is the container element id.
- Endpoints: onecall is `POST {domain}/{networkId}/call` with `ads:[{formatId, tagId, target, headerBidding, isLazy, isAdRefresh, gpid}]`; std is `/genericpost`; legacy is `GET /ac?nwid&siteid&pgname|pgid&fmtid&tag&tgt` as a script.
- Events on `sas.events`: `setupCalled, call, beforeFetch, load, noad, beforeRender, render, setHeaderBiddingWinner, hbRender, error`. Whether `sas.events.on(name, fn)` without a tag id is a wildcard is not verified.
- Header bidding: the `headerBidding` object on the ad, `hb_*` inside `target`/`tgt`, and `&hb_cpm=&hb_bid=&hb_ccy=` appended to tracking URLs.
- Fill: the answer's entry has `ScriptSrc`; no-ad is the `sas.noad` callback, the `noad` event, or a later request to the entry's `ScriptNoad` URL.

**Adhese (M, except the empty answer).** Request is `GET|POST https://ads-<account>.adhese.com/json/sl<location>-<format>/...` with several slots per call, or the legacy `/ad3/`. An empty array is no-fill (V). A fill is an array of objects with `slotName`, `adType`, `width`, `height`, `tag|body`, `impressionCounter`, `extension.prebid.cpm` (M). Capture one real fill from a customer page before writing the decoder.

**Adition / Yieldlab (M, method names V).** `adition.srq.push(api => api.registerAdfarm(...).setProfile(...).configureRenderSlot(id).setContentunitId(n); api.load().completeRendering())`; `renderSlot`, `reload`, `setShb`, `events`, `modules` exist in srp.js. Yieldlab's pre-bid is `ad.yieldlab.net/yp/<ids>` and is a bid source, not an ad server. Endpoint parameters need a capture from a real page.

**Adform (partly V).** `<script data-adfscript="adx.adform.net/adx/?mid=<id>">` plus adx.js; a tag API with `setTargeting`, `resetTargeting`, `setRequestParam`, `setCustomData`, `setPrice`, `onLoaded`, `onRendered`. Placement is `mid`. `/adx/openrtb` is the Prebid endpoint and is already separated in `classify.rs`.

**Video (M).** GAM VAST is `pubads.g.doubleclick.net/gampad/ads?...output=vast|xml_vmap1`; Xandr `/ptv` (V on bild); FreeWheel `*.fwmrm.net/ad/g/1`; SpringServe `vid.springserve.com/vast/`; smartclip `ad.sxp.smartclip.net`. Fill is `<Ad>` present versus an empty `<VAST/>`. No slot lifecycle; hooking IMA is not worth it.

**Server-side setups.** Only creative loads and impression beacons are visible. Show the note "ad impressions seen with no ad call from the browser: ads are probably requested server-side" when table beacons appear without calls. No slot rules.

**Not worth supporting:** AMP, AdSense for Search, the OpenX ad server, FreeWheel/IMA SDK hooks, a slot lifecycle for Outbrain/Taboola, and JS adapters for Kevel, Revive, Adform, Adhese and AdSense (the network gives the same facts with no page risk).

## 2. Architecture

### 2.1 Three layers

1. **Tracker adapters** in `tracker.js`, phase 1 for AST and Smart, emitting the existing slot events with `adServer`.
2. **Network adapter**, a pure engine-domain function over each `NetworkRecord`, driven by a table. It runs for every ad server, GPT included.
3. **Merge** in `telemetry.rs`: one `AdSlot` list from both views.

The network adapter must run inside the engine's `NetworkSink` while records stream, because bodies are dropped afterwards. The sink's `admit_body` must admit answers of table-matched ad calls even when "keep network data" is off: read, extract, drop. The sink is not implemented yet (no `impl NetworkSink` exists in the repo), so agree this with the storage agent.

### 2.2 Contract changes (all additive)

In `crates/model/src/lib.rs`:

```rust
pub enum AdServerKind { Gpt, AdSense, XandrAst, Equativ, Adhese, Adition, Adform, Kevel, Revive, Video, Other }
// Default = Gpt, so stored scans keep rendering as GPT.

pub enum SlotEvidence { AdServerEvents, Network, Both }   // Default = AdServerEvents

pub enum AdUnitMatch { ElementId, Placement, BidAdId, BidderParam, Gpid }

pub struct AdSlot {            // new fields, all #[serde(default)]
    pub ad_server: AdServerKind,
    pub ad_server_name: Option<String>,   // for Other and user-defined servers
    pub evidence: SlotEvidence,
    pub placement: Option<String>,        // ad_unit_path stays and is filled for GPT
    pub element_known: bool,              // default true; false = element_id is a synthesized key
    pub ad_unit_code: Option<String>,
    pub ad_unit_match: Option<AdUnitMatch>,
}

pub struct SlotRequest {       // new fields
    pub ad_server: AdServerKind,
    pub evidence: SlotEvidence,
    pub outcome: Option<SlotOutcome>,     // per request; passback chains need it
    pub header_bidding_keys: Vec<String>,
    pub batch_size: Option<u32>,
    pub cpm: Option<f64>,
    pub creative_id: Option<String>,
    pub deal_id: Option<String>,
    pub passback_of: Option<u32>,         // index of the request this one follows
}

pub struct AdServerSummary { kind, name, version, evidence, slot_count, request_count, fill_rate }
pub struct UnrecognisedAdHost { host, requests, sample_request_ids, signals: Vec<String> }
// ScanMetrics gains: ad_servers: Vec<AdServerSummary>, unrecognised_ad_hosts: Vec<UnrecognisedAdHost>.
// has_gpt stays and keeps being set.
```

In `crates/model/src/capture.rs`:

- Every slot event variant gains `#[serde(default)] ad_server: AdServerKind`.
- `SlotDefined` gains `placement`.
- `SlotRequested` gains `batch: Option<u32>`.
- `SlotRenderEnded` gains `cpm`, `creative_id`, `deal_id`.
- New `AdServerDetected { ad_server, version, global_name }`; `GptDetected` stays and is still emitted.
- `BidResponse` gains `ad_id: Option<String>` for the `hb_adid` join.
- `Capture` gains `ad_calls: Vec<ObservedAdCall>`.

`ObservedAdCall` is not a frontend type: `{ request_seq, page, t, response_at, ad_server, context, initiator_script, gdpr, has_consent_string, placements: Vec<{ placement, element_hint, sizes, hb_keys, hb_ad_id, outcome, cpm, creative_id, deal_id, noad_url }> }`.

### 2.3 Tracker adapters

Rules for every adapter: use the library's own event API; only disguised pass-through wrappers of the kind already used for `defineSlot`; never patch XHR, fetch or `history`; tolerate the library arriving late or never; never create a global the page did not create, except through the existing `watchGlobal` setter.

**AST.**
- `watchGlobal('apntag', inspectAst)` plus the poll.
- If `apntag.loaded !== true` and `anq` is an array, `unshift` the attach function, as `googletag.cmd` is handled today. After load AST replaces `anq.push` with immediate execution (V).
- Attach: emit `adServerDetected` with `getAstVersion()`.
- Wrap `defineTag` (disguised) to learn target ids. For each id in `apntag.requests.tags`, emit `slotDefined` with `placement` = `tagId`, or `member:invCode`.
- Register `onEvent('adRequested', targetId, cb)` per tag. At that moment read tag plus page keywords and emit `slotRequested`.
- Wildcard handlers map the rest: `adAvailable` to `slotResponseReceived`; `adLoaded`/`adLoadedMediated` to `slotRenderEnded{isEmpty:false, size, cpm, creativeId, dealId}` plus `slotOnload`; `adNoBid`/`adNoBidMediated`/`adRequestFailure`/`adBadRequest` to `slotRenderEnded{isEmpty:true}`.
- AST has no viewability event. Leave `viewable_at_ms` empty; do not synthesize.
- A publisher `offEvent(name, targetId)` without a handler may remove ours. The network view covers that case.

**Smart.**
- `watchGlobal('sas', inspectSas)`. If `sas.events` is missing, `unshift` the attach function onto `sas.cmd`.
- `sas.events.on('call')` gives `slotDefined` and `slotRequested` per format; the tag id is `ad.tagId` or `sas_<formatId>`, and keys come from `ad.target` and a preceding `setHeaderBiddingWinner`.
- `load` maps to `slotResponseReceived`; `render` to `slotRenderEnded{isEmpty:false}`; `noad` to `slotRenderEnded{isEmpty:true}`.
- If the wildcard form of `events.on` turns out not to exist, register per tag id from the `call` payload.

### 2.4 The table

One format for built-ins and user definitions: `crates/engine/src/domain/adserver/adservers.json`, embedded with `include_str!`, parsed once into owned `AdServerDef` structs. A test asserts that it parses and that every entry passes its fixtures. An optional `adservers.json` in the app data dir is merged over it.

It replaces `AD_SERVER_CALLS` and `has_prebid_targeting` in `/home/user/Atlas/apps/adtech-scanner/crates/engine/src/domain/network/classify.rs`. `BID_CALLS` stays and is checked first.

Three full entries:

```json
[
 { "kind": "xandrAst", "name": "Xandr (Microsoft Monetize)",
   "libraries": ["acdn.adnxs.com/ast/", "adnxs-simple.com/ast/"],
   "endpoints": [
    { "id": "ut-v3", "hosts": ["adnxs.com", "adnxs-simple.com"], "path": "/ut/v3", "pathExact": true,
      "methods": ["POST"], "format": "display",
      "request": { "from": "jsonBody", "each": "/tags",
        "placement": ["/id", "/code"], "placementPrefix": { "/code": "/member_id" },
        "sizes": { "each": "/sizes", "w": "/width", "h": "/height" },
        "hbKeys": { "each": "/keywords", "key": "/key", "value": "/value/0" },
        "pageHbKeys": { "each": "/keywords", "key": "/key" },
        "join": "/uuid",
        "consent": { "gdpr": "/gdpr_consent/consent_required", "string": "/gdpr_consent/consent_string" } },
      "response": { "from": "jsonBody", "each": "/tags", "join": "/uuid",
        "noFill": { "path": "/nobid", "equals": true },
        "fill": { "exists": "/ads/0" },
        "cpm": "/ads/0/cpm", "creativeId": "/ads/0/creative_id", "dealId": "/ads/0/deal_id",
        "size": { "w": "/ads/0/rtb/banner/width", "h": "/ads/0/rtb/banner/height" },
        "error": "/error" } },
    { "id": "ptv", "hosts": ["adnxs.com"], "path": "/ptv", "methods": ["GET"], "format": "video",
      "request": { "from": "query", "placement": ["id", "inv_code"], "placementPrefix": { "inv_code": "member" } },
      "response": { "decoder": "vast" } } ],
   "beacons": [ { "hosts": ["adnxs.com"], "path": "/it" } ],
   "hbKeyPattern": "^(hb_|amzn)" },

 { "kind": "equativ", "name": "Equativ (Smart AdServer)",
   "libraries": ["sascdn.com/tag/", "sascdn.com/diff/js/smart.js"],
   "endpoints": [
    { "id": "onecall", "hosts": ["smartadserver.com"], "pathPattern": "^/\\d+/call$", "methods": ["POST"],
      "request": { "from": "jsonBody", "each": "/ads",
        "placement": ["/formatId", "/placementuuid"], "placementContext": ["/siteId", "/pageId"],
        "elementHint": "/tagId", "join": "/tagId",
        "hbKeys": { "kvString": "/target", "separator": ";" }, "hbObject": "/headerBidding",
        "refresh": "/isAdRefresh" },
      "response": { "from": "jsonBody", "eachKey": true, "join": "@key",
        "fill": { "exists": "/ScriptSrc" }, "noadUrl": "/ScriptNoad",
        "size": { "w": "/ScriptWidth", "h": "/ScriptHeight" } } },
    { "id": "ac", "hosts": ["smartadserver.com"], "path": "/ac", "methods": ["GET"],
      "request": { "from": "query", "placement": ["fmtid"], "placementContext": ["siteid", "pgname", "pgid"],
        "elementHint": "tag", "hbKeys": { "kvString": "tgt", "separator": ";" },
        "consent": { "gdpr": "gdpr", "string": "gdpr_consent" } },
      "response": { "decoder": "smart-ac-script" } },
    { "id": "genericpost", "hosts": ["smartadserver.com"], "path": "/genericpost", "methods": ["POST"],
      "request": { "decoder": "smart-genericpost" }, "response": { "decoder": "smart-genericpost" } } ],
   "beacons": [ { "hosts": ["smartadserver.com"], "path": "/h/aip" }, { "hosts": ["smartadserver.com"], "path": "/track/action" } ],
   "hbKeyPattern": "^hb_" },

 { "kind": "gpt", "name": "Google Ad Manager",
   "libraries": ["doubleclick.net/tag/js/gpt.js", "pubads_impl"],
   "endpoints": [
    { "id": "gampad", "hosts": ["doubleclick.net"], "path": "/gampad/ads", "methods": ["GET", "POST"],
      "request": { "decoder": "gpt-gampad" },
      "response": { "decoder": "gpt-gampad" } } ],
   "hbKeyPattern": "^(hb_|amzn)" }
]
```

**What must be code**, as named decoders in `domain/adserver/decoders/`:
- `gpt-gampad`: unpacking `prev_iu_szs`, `enc_prev_ius`+`iu_parts`, `prev_scp` per slot; the VAST variant.
- `smart-ac-script`: the `/ac` answer is JavaScript; find the noad URL and creative marks by pattern.
- `vast`: `<Ad>` present or not.
- `smart-genericpost`.
- The generic interpreter of the declarative locators (query, JSON pointer, `each`, `kvString`, `eachKey`).
- All merge logic.

User definition files may use declarative locators and the existing decoder names only.

### 2.5 Merge and deduplication

Run per page view and per ad server.

1. **Claim.** For each JS `slotRequested` (server A, element E, time t), the candidates are A's unclaimed network placements with `started_at` in `[t - 50 ms, t + 2000 ms]`. Among them the placement must equal the slot's placement, or the element hint must equal E. The nearest in time wins. For GPT single-request calls, the n-th slot of the same `slotRequested` burst maps to the n-th `prev_iu_szs` entry, as a fallback after the path match. A claim sets `request_id`, `evidence: Both`, the response time and the per-request outcome.
2. **Who wins on conflict.** JS events win for lifecycle times and for the rendered outcome. The wire wins for what was sent: `header_bidding_keys` and `had_prebid_targeting` come from the request when it was claimed. JS saying keys were present while the wire has none is evidence for rule N7.
3. **Unclaimed network placements** become `evidence: Network` requests. Slot identity is `(page, ad_server, element_hint)` when a hint exists (Smart `tag`, Adhese slot name). Otherwise the key is `"<placement>"` with `element_known: false`, and `in_initial_viewport` is unknown. Repeated calls for one key append to `requests`.
4. **JS requests with no network match** stay `AdServerEvents`.
5. **Network outcome.** Fill or no-fill from the answer. If only a `noad_url` is known, the outcome becomes `Empty` when a later request to exactly that URL exists, `Filled` when the creative URL (`ScriptSrc`) was requested, else `Pending` meaning "answered, outcome unknown". Never infer "filled" from a 200 alone.
6. **Invariants** (property tests): a network placement is claimed at most once; a JS request claims at most one; the merge is deterministic when the input order is shuffled.

**Confidence.** A finding whose slots are all `Network` evidence multiplies its confidence by 0.85 and gets an evidence line "read from the ad request on the network, not from the ad server's events". Rules that need element position do not run on `element_known: false` slots.

### 2.6 Slot to Prebid ad unit

A cascade; the first hit sets `ad_unit_code` and `ad_unit_match`.

1. Ad unit code equals the element id (today's rule).
2. Code equals the placement (GPT path, AST `invCode`, Smart tag id).
3. `hb_adid` on the request equals the `adId` of a bid of that ad unit (needs `BidResponse.ad_id`). This is the only reliable basis for wrapper-renamed placements such as bild's `getPlacementName`.
4. A bidder param of the ad unit equals the placement: appnexus `placementId`/`invCode` against the AST tag, smartadserver `formatId`+`siteId` against Smart's.
5. `ortb2Imp.ext.gpid` or `ext.data.pbadslot` equals the placement.

Size never matches by itself. `slot_matches_code` and `auction_covers` in `telemetry.rs` become "the slot's `ad_unit_code` is this unit". The "bids not passed" rules run for all bases. `slots-outside-header-bidding` requires that at least one slot of that ad server matched some ad unit on the page; otherwise the mapping scheme is unknown and the rule stays silent.

## 3. Rules

### Rules that do nothing today without GPT

- **Fully dead:** `slow-gpt-load`, `ad-request-before-auction-end`, `slots-never-requested`, `empty-slots`, `ad-units-without-slot` (returns when there are no slots), `slots-outside-header-bidding`, `later-page-view-without-bids`, `spa-slots-not-reset`, `refresh-with-stale-bids`, `refresh-without-auction`.
- **Degraded:** `auction-gated-on-interaction` (no waiting slots, so it only fires for the first auction and names none), `bids-requested-without-ad-units` (cannot list unbid slots).
- **Metrics:** `slot_count`, `empty_slot_count`, `fill_rate` and `time_to_first_ad_ms` are zero or Prebid-only.
- **Not read:** `won-bid-not-rendered` and the `visit.rs` bodies were not read in detail; confirm their slot use when implementing.
- **Unaffected:** timeouts, bidder checks, ads.txt, consent, Prebid setup, `consent-asked-again`, `scan-identifiable-as-automated`.

### How each generalises

| Rule | Generalisation | Per-server knowledge |
|---|---|---|
| `ad-request-before-auction-end` | A request to any ad server for a slot with a matched ad unit left before that unit's auction ended, and the wire has no key matching `hbKeyPattern`. Text names the server. | `hbKeyPattern`; where keys sit |
| Anchor requested before auction | Same rule; not GPT-specific. | none |
| `slots-never-requested` | Events evidence only: a network-only slot exists because it was requested. | AST `utCalled`, Smart "defined" equals call |
| `empty-slots` | Per request, then per slot: the last request of the chain is no-fill. A slot whose first server no-filled and whose passback filled is not empty. | fill and no-fill fields |
| `ad-units-without-slot` | Unit matches no slot on any basis. Needs one or more slots with events evidence, or all servers on the page network-covered. | none |
| `slots-outside-header-bidding` | As in 2.6; skip `Video` and 1x1. | none |
| `slow-gpt-load` | Becomes `slow-ad-server-load`, keeping the old id for GPT: first `adServerDetected` of the primary server, the one with most slots. | library names |
| `ad-library-loaded-twice` | Already hash-based. Add table `libraries` so ast.js and smart.js count however the host is categorised. | `libraries` |
| `later-page-view-without-bids` | Unchanged logic on the general slots. | none |
| `spa-slots-not-reset` | GPT keeps `slotDefinitionRejected`. For AST and Smart: after a route change the slots of the old view got no new request or definition while new ad units were auctioned. No "rejected" signal exists there; confidence 0.6. | per-server reset call in the advice text |
| `refresh-with-stale-bids` | Same `hb_adid` on two requests of one slot, any server. | none |
| `refresh-without-auction` | A second request of a slot within the same server (not a passback) without keys, when the first had them. | Smart `isAdRefresh` helps |
| `auction-gated-on-interaction`, `bids-requested-without-ad-units` | Use the general slots; replace the hard-coded host list in `lazy.rs` with the table. | none |

### New rules

Each is off unless its evidence is complete.

- **N1 `passback-chain-unfilled`** (high). Server A no-fills, server B is called for the same container within 3 s, and B no-fills too or never answers. Needs both outcomes known. The same container means the same element id, or B's initiator is A's creative frame. Report the chain with request ids.
- **N2 `passback-without-header-bids`** (medium). In an N1-shaped chain the first request carried `hb_*` keys and the passback request did not, while Prebid had a bid for the unit. A bild-shaped setup is the candidate case.
- **N3 `passback-loop`** (medium). Three or more alternating requests A, B, A for one slot within 10 s, none filled.
- **N4 `two-ad-servers-same-placement`** (medium, confidence 0.6). Two servers requested for the same element within 200 ms, neither triggered by the other's answer, both filled. Info severity unless both rendered.
- **N5 `ad-call-before-consent`** (medium). GDPR applies per `TcfState`, the ad call left before the consent choice, and the request carries no consent string. Per server; skip when the capture has no TCF data.
- **N6 `winning-bid-not-delivered`** (high). `hb_adid` went out on a request, the server answered with fill, yet no `adRenderSucceeded` or `bidWon` for that ad id, no render-endpoint hit, and no other creative rendered in the slot. Also the reverse, Prebid's top bid above the served `cpm` (AST exposes it), at info level because priorities legitimately differ.
- **N7 `header-bids-not-on-the-wire`** (high). JS showed `hb_*` on the slot at request time, the claimed request has none. Needs `Both` evidence.
- **N8 `placement-mismatch`** (medium, 0.6). The placement in the request differs from every placement the page config defines for that element, such as AST `requests.tags[target].tagId` against the UT body, or a Smart format id against the tag id suffix. Events and network both required.
- **N9 `sequential-ad-calls`** (low). Five or more ad calls to one server within 1 s, each with one placement, where the server supports batching (AST, Smart onecall, GPT SRA). On bild this would flag the 8 `/ac` calls; mark as low or info because legacy `/ac` cannot batch.
- **N10 `ad-server-call-failed`** (high). An ad call ended in a network error, 4xx/5xx, or UT `{"error":...}`. Cheap and certain.

## 4. UI

- **Overview "Detected".** Prebid first, then ad servers in order of slot count: "Prebid.js 10.25, Xandr AST 0.66, Equativ". At most three, then "+2". A server known only from the network carries a small "network" tag. `detectedStack` in `/home/user/Atlas/apps/adtech-scanner/src/features/report/key-metrics.ts` reads `metrics.adServers` and falls back to `hasGpt` for stored scans.
- **Ad slots tab.**
  - One list, not one section per server. A compact server badge sits beside the element id, shown only when the page has more than one server.
  - A filter chip row per server with counts appears only for two or more servers.
  - Passback requests are extra "requested" marks on the same row, coloured by server, with a connecting line; the tooltip lists the chain ("Xandr no-fill at 11.1 s, then Equativ at 11.5 s, no ad").
  - Network-inferred slots have a dashed outline on their phase marks and the tooltip line "Read from network requests; the ad server's own events were not available. Render and viewability are not known."
  - Slots with `element_known: false` show the placement in place of the element id, under "Placement".
- **Auction waterfall.** One "ad request" marker per request of the matched slot, labelled with the server's short name; a dashed marker when network-inferred; clicking opens the request in the Network view via `request_id`.
- **Network view.** Category "Ad server" stays one category; `vendor` carries the server name, so the vendor facet separates them. Add category "Content recommendation" for Outbrain and Taboola. `no_hb_targeting` is set from the table's `hbKeyPattern` for every server.
- **Empty and unknown states.**
  - No slots and no ad calls: "No ad server activity was seen on this page."
  - Unrecognised hosts: a card on the Ad slots tab, "Unrecognised ad server: 14 requests to ads.example.com look like ad calls (sizes and placement ids in the request, creative markup in the answer). Slots, fill and timing cannot be derived for it." with "Show requests" (Network view filtered to the host) and "Copy definition stub".
  - Network record disabled and a non-GPT server detected by library only: say that slots need the network record.

## 5. Unknown ad servers

**Detector**, in `domain/adserver/unknown.rs`, run over requests the table did not claim. A host qualifies with two or more requests scoring 3 or more of:
- the request carries two or more sizes (`\d{2,4}x\d{2,4}`) or a width/height pair;
- it carries a placement-like id parameter (`placement|zone|slot|tagid|fmtid|mid|adunit|sl`);
- it carries consent parameters (`gdpr_consent`, `gdpr`);
- the answer contains creative markup, VAST, or click/impression macros;
- the initiator is a script categorised Wrapper, Prebid or AdServer;
- the request is an XHR, fetch or script from the main frame.

Exclusions: hosts the knowledge base calls Bidder on a path matching `BID_CALLS` or OpenRTB marks, sync paths, first-party hosts unless the path contains `/ad` and the markup signal is present. Output is `UnrecognisedAdHost`. It never produces slots or findings, only one info note.

**User definition file.** Worth it, but only as a by-product: because the built-in table is the same JSON, loading `<app data dir>/adservers.json` costs about half a day. No editor UI. Ship it in phase 3, after the format has survived five built-in servers; a format that changes under the owner's own files is worse than none. An invalid file becomes a visible Settings notice; it never fails a scan.

## 6. Demo site

Add under `/home/user/Atlas/apps/adtech-scanner/crates/browser/demo-site/`:

- **`/tag/ast/ast.js`**: stand-in with `apntag.anq`, `defineTag`, `loadTags`, `refresh`, `showTag`, `setKeywords`, `setPageOpts`, `onEvent`/`offEvent` with the real argument forms (wildcard, and no arguments on `adRequested`), `requests.tags`, `getAstVersion`. It posts to the demo's own `/ut/v3`, answered by `demo.rs` in the real shape (the captured no-bid, and a fill with `ads[0].cpm` and `rtb.banner`).
- **`/tag/sas/smart.js`**: `sas.cmd`, `setup`, `call` (onecall), `render`, `refresh`, `setHeaderBiddingWinner`, `events.on`. Posts to `/<nw>/call`, answered in the captured `ScriptSrc`/`ScriptNoad` shape. Also `/ac` as a script answer, plus `/h/nshow` and `/h/aip`.
- **`/json/...`**: an Adhese-style endpoint called directly by the wrapper with `fetch`, no library. This exercises the network-only path.
- **A second demo page**, `/axel` or a `?stack=ast` mode, so the GPT demo and its tests stay untouched. The stack is Prebid plus AST as primary, Smart as passback through a wrapper function that writes `/ac` script tags, and one Adhese-style slot.

Planted bugs:
1. Prebid wins `ad-top` but the wrapper calls `loadTags` before `setKeywords` (N7 and `ad-request-before-auction-end` on AST).
2. The Smart `/ac` passback for `ad-rail` carries no `hb_*` and no-fills; AST no-filled before it (N1, N2).
3. `ad-inarticle` loops AST, Smart, AST (N3).
4. The Adhese-style call leaves before the consent choice, without a consent string (N5).
5. Eight single-placement `/ac` calls where one onecall would do (N9).
6. One AST tag defined and never loaded (`slots-never-requested` on AST).

## 7. Phasing, effort, tests

### Phase 1: bild-like stacks fully visible (about 9 to 11 working days)

1. **Model** (0.5 d): the contract changes of 2.2; regenerate bindings. A test proves that a stored scan without the new fields deserializes as GPT with events evidence.
2. **Engine table and network adapter** (3 d): `domain/adserver/{mod,table,interpret,decoders/*,merge,unknown}.rs`, `adservers.json` with GPT, Xandr and Equativ; `classify.rs` reads the table; the sink hook; `Capture.ad_calls`.
3. **Telemetry** (2 d): general `derive_slots` and the merge of 2.5; ad unit matching of 2.6; per-request outcomes; metrics `ad_servers`, with fill rate counted per final outcome.
4. **Rules** (2 d): generalise the table in section 3; add N1, N2, N7, N10. Depends on 3.
5. **Tracker** (1.5 d): AST and Smart adapters; `adServerDetected`; `adId` on `bidResponse`; a `tracker_contract.rs` row per event. Parallel to 2 to 4.
6. **Demo** (1.5 d): the stand-ins and the second page of section 6, `demo.rs` routes.
7. **Frontend** (2 d): badge, filter chips, chain marks, inferred styling, overview list, unrecognised card, fake-backend scenario with two servers. Parallel once 1 is done.
8. **One real scan of bild.de** by hand to confirm the AST hook and the `/h/aip` reading.

### Phase 2 (about 5 days)

Adhese, Adition/Yieldlab, Adform and AdSense as table entries, each from one real capture. N3, N5, N6, N9. The "Content recommendation" category.

### Phase 3 (about 3 to 4 days)

Video ad calls. N4, N8. The user definition file. Kevel and Revive rows. The server-side-ads note.

### Tests

- **Table fixtures.** Per endpoint, a request/response pair in `crates/engine/tests/fixtures/adservers/<kind>/<case>.json` plus the expected `ObservedAdCall`. Start with the three pairs captured in section 0. The bild `/ac` URL shape is in `/tmp/claude-0/-home-user/ee93eb55-ef69-5991-bc2c-57c8a322dcdf/scratchpad/bild/p2.txt`.
- **Merge.** Table-driven cases (events only, network only, both, single-request batch, refresh, passback, late answer after a route change) plus the invariants of 2.5 under shuffled input.
- **Rules.** Each new or generalised rule gets a positive case and its nearest false positive: passback that fills, legitimate server priority over a header bid, keys present under non-standard names such as bild's integer `hb_pb`.
- **Tracker.** `tracker_contract.rs` against the stand-ins: library loaded late, never loaded, stub queue only, `offEvent` by the page, define after load.
- **Demo.** A `demo_site.rs` twin for the AST page asserting the six planted bugs produce exactly their findings, and the GPT page's findings are unchanged.
- **Frontend.** Legacy-report snapshot unchanged; a two-server scenario; the inferred-evidence tooltip; the unrecognised card.
- **Classifier.** The existing table test in `classify.rs` must still pass after it reads the JSON.

## 8. Risks and open questions

1. **Sink policy when "keep network data" is off.** The network adapter still needs ad-call bodies in memory. Recommendation: always admit table-matched ad calls, extract, drop; say so in the Settings copy. Needs agreement with the storage agent.
2. **Meaning of Smart `/h/aip`.** Seen as the `ScriptNoad` URL in one real answer; on bild it follows most `/ac` calls. Recommendation: only the exact-URL match counts as no-ad until a real bild scan confirms more.
3. **AST `adRequested` gives no target id to wildcard handlers.** Recommendation: per-tag registration through a pass-through `defineTag` wrapper, as designed. Is wrapping `defineTag` acceptable under the "never change page behaviour" rule? The tracker already wraps `defineSlot` the same way; I recommend yes.
4. **Ad unit matching on wrapper-renamed placements.** Without `hb_adid` on the wire and `adId` from Prebid, bild-like pages cannot be matched. Recommendation: add `ad_id` to `BidResponse` in phase 1 and keep the "outside header bidding" rule silent when nothing matches.
5. **Passback inference is timing-based** when the second call comes from the main frame. Recommendation: require the element match or initiator proof; otherwise report nothing.
6. **Market ranking is from memory.** Recommendation: the owner names the ten customer sites that matter; scan each once with the network record on and order phase 2 by what the unrecognised-host card reports.
7. **Adhese and Adition shapes are unverified.** Recommendation: no decoder without a real capture from a customer page.
8. **Outbrain and Taboola.** Recommendation: out of slots, in as a network category.
9. **User definition file format stability.** Recommendation: ship in phase 3 only, with a `formatVersion` field.
10. **`AdServerKind::Video` mixes several vendors.** Recommendation: keep one kind plus `ad_server_name`, since video gets no slot rules.
11. **Real libraries in tests.** ast.js and smart.js are not ours to embed. Recommendation: stand-ins in the repo, plus one manual test in `manual.rs` that loads the real files from their CDNs when run by hand.

### Critical Files for Implementation

- /home/user/Atlas/apps/adtech-scanner/crates/model/src/lib.rs
- /home/user/Atlas/apps/adtech-scanner/crates/model/src/capture.rs
- /home/user/Atlas/apps/adtech-scanner/crates/browser/injected/tracker.js
- /home/user/Atlas/apps/adtech-scanner/crates/engine/src/domain/telemetry.rs
- /home/user/Atlas/apps/adtech-scanner/crates/engine/src/domain/network/classify.rs