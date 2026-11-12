# Demo publisher site

A fictional news site ("The Meridian Post") of five articles with a realistic
header-bidding stack. It is embedded in the binary, served by
`start_demo_site()` on random loopback ports, and loads nothing from the
internet. It is the in-app "Try the demo" target and the fixture of
`tests/demo_site.rs` (one page view), `tests/demo_browse.rs` (a visitor who
follows links) and `tests/network.rs` (every request of a visit).

## Three origins, all on this machine

| Origin | What it is |
|---|---|
| `http://127.0.0.1:<site port>` | The site: `DemoSite.url`. Everything in "What is served" is here |
| `http://localhost:<partner port>` | The site's ad-tech partners: the same server under another name and port. To the browser it is another site, so a frame from it runs in a process of its own and a JSON POST to it needs a CORS preflight |
| `http://localhost:<identity port>` | An identity provider that accepts every connection and closes it without an answer |

The pages and `prebid.js` are served with the two partner origins filled in
(`%PARTNER_ORIGIN%`, `%IDENTITY_ORIGIN%` in the sources). A scan of the demo
therefore lists `localhost` as the one host outside the site.

## What is served

| Path | What it is |
|---|---|
| `/` | The entry article: the page shell (`index.html`) around `fragments/tide-powered-town.html`, with one inline script (page config and theme) |
| `/climate/village-substation`, `/climate/kelp-carbon`, `/climate/white-roofs`, `/climate/permit-queue` | The other four articles, each a full page: the same shell around its fragment |
| `/fragments/<slug>.html` | The bare `<article>` of each of the five, fetched by the page when it shows an article in place |
| `/static/ads/prebid.js` | Prebid.js stand-in: public API, event payloads, param validation and debug logging of the real library. Each bidder posts an OpenRTB request to its demo exchange, which answers with the bids the page's seeded simulation decided (see "Simulated ad stack") |
| `/tag/js/gpt.js` | Google Publisher Tag stand-in: `googletag` API and slot events |
| `/static/ads/apstag.js` | Amazon TAM stand-in, loaded by the wrapper |
| `/static/ads/hb-wrapper.min.js` | The publisher's wrapper ("adstack"), minified. **This is where the bugs are.** |
| `/static/js/app.9b3e7f.js`, `/static/js/vendors-ui.4c1d2a.chunk.js`, `/static/js/analytics.js` | Application bundle, UI library chunk and first-party analytics: not ad related |
| `/static/js/analytics-worker.js` | A dedicated worker of the analytics script. It fetches `/collect/sampling.json` and hands the rules to the page |
| `/static/img/harbour-dusk.png`, `/static/fonts/meridian-initial.ttf` | One image (498 bytes, in the footer) and one font (728 bytes, the "M" of the masthead): the two requests of a page view that are recorded without a body |
| `/api/archive-index.json` | The search index the application fetches 1.5 s after `load`: one JSON document of about 1.3 MB |
| `/collect` | Sink for the analytics beacons (204) |
| `/collect/sampling.json` | The sampling rules the analytics worker asks for |
| `/__demo/hits` | How often each path was requested since the site started, on any of the origins, as JSON (`{"/": 1, ...}`); its own requests are not counted. For tests |
| `/ads.txt` | The publisher's ads.txt (`ads.txt`): six authorised ad systems, `text/plain` |
| `/ut/v3/prebid`, `/a/api/fastlane.json`, `/openrtb/pbjs`, `/translator`, `/openrtb_2_5/pbjs/auction/request`, `/header/auction` | The demo exchanges of `appnexus`, `rubicon`, `ix`, `pubmatic`, `criteo` and `triplelift` (POST, at the paths of the real adapters' endpoints). `appnexus` is asked at the partner origin, the others at the site's own |
| `/openrtbb/prebidjs` | The demo exchange behind the `openx` bidder (POST). Always answers `{"id":"meridian-post","nbr":7}`: OpenRTB no-bid reason 7, "blocked publisher or site" |
| `/usersync/start`, `/usersync/match`, `/usersync/done.gif` | A cookie sync in three hops: the site redirects to the partner origin, which sets `uid` and redirects back to the site's pixel |
| `/usersync/frame.html`, `/usersync/ids.json` | The partner's sync frame (loaded from the partner origin) and the ids its script fetches before it requests a pixel of its own |

`static/ads/hb-wrapper.src.js` is the readable source of the wrapper. It is not
served. After changing it, regenerate the served file:

```sh
npx terser@5 static/ads/hb-wrapper.src.js --compress --mangle --comments '/^!/' -o static/ads/hb-wrapper.min.js
```

## Getting from one article to another

- **Loading a page.** The "Most read" list in the rail and the links inside
  the article texts are plain anchors.
- **In place.** The four story cards under "More from Climate" are
  `a[data-route]`: `app.9b3e7f.js` fetches the article's fragment, replaces the
  `<article>`, calls `history.pushState` and dispatches `meridian:route` on
  `window`. The address changes; no document is loaded.
- **In a new tab.** One "Most read" link (`/climate/white-roofs`) has
  `target="_blank"`. A new tab has a session of its own (`sessionStorage`), so
  the session-dependent bugs (i) and the recirculation unit do not apply there.

`static/js/analytics.js` counts the reader's `pointermove`, `wheel` and
`touchstart` events and whether all of them were trusted (caused by an input
device, not by script). It prints
`ENGAGEMENT {"pointerMoves":12,"wheels":3,"touches":0,"trusted":true}` to the
console after three seconds without input and when the page is left.

## Links a visitor must not follow

Every page carries these; none of the paths is served (404), so a request for
one shows up in `/__demo/hits`:

| Link | Why not |
|---|---|
| `/account/login` ("Sign in"), `/logout` ("Log out") | account actions |
| `/files/media-kit.pdf` | a file |
| `mailto:newsdesk@meridianpost.example` | not a web address |
| `https://partner.example/` | another site |
| `/sponsored/partner-story` | `rel="sponsored"` |
| `/sponsored/offer` | inside the native ad card `div.ad-slot--native` |
| `/ads/click?bidder=...` | inside an ad creative's iframe |
| `#`, `#newsletter` | the page itself |

## Simulated ad stack

Slots on every article page: `ad-leaderboard` (top, in the first viewport),
`ad-sidebar-1` (rail, sticky on desktop), `ad-inarticle-1`, `ad-inarticle-2`
(lazy, far down the article) and `ad-sticky-footer` (anchor). The container
`ad-recirc`, under the story cards, gets a slot only from a reader's second
page of a session on (see "What works correctly").

Simulated bidders, with outcomes seeded per bidder, ad unit and request count so
every run is identical:

| Bidder | Exchange latency | CPM | Behaviour |
|---|---|---|---|
| appnexus | 40-100 ms | 0.35-1.90 | bids on almost everything. Asked at the partner origin with a JSON body, so its first request of a page view follows a CORS preflight |
| rubicon | 60-120 ms | 0.45-2.40 | bids on most requests |
| ix | 70-130 ms | - | never bids, see (e) |
| pubmatic | 330-620 ms | 1.80-4.60 | highest CPMs; its answer arrives after the misconfigured timeout |
| criteo | 1500-2100 ms | 0.60-2.20 | its answer arrives long after any sensible timeout |
| triplelift | 60-110 ms | 0.40-1.50 | bids on every request that reaches it |
| openx | none | - | the exchange never bids and says why |

Every bidder request with at least one valid bid is one real HTTP round trip:
a POST of an OpenRTB request (`id` is Prebid's `bidderRequestId`, each
`imp[].id` a `bidId`, `source.tid` the auction id, `tmax` the timeout) to the
bidder's demo exchange, and an OpenRTB answer: `seatbid` with one seat and a
bid per impression that was bid on, `"seatbid": []` when the bidder passes,
`"nbr": 7` from `openx`. The exchanges are mirrors. The seeded simulation in
`prebid.js` decides what the bidder answers and how long it takes, sends that
along as `ext.standin` (`latencyMs`, `bids`), and the exchange waits that long
and answers with it. The page does not wait for that answer: it acts on the
simulation's own result when the same latency has passed on its own clock, as
it did before the exchanges existed. A round trip on a busy machine, or while
the page is busy with a scroll, can take 100 ms longer than on an idle one,
and which bidder is late must not depend on that. So Prebid's events are as
repeatable as ever, and the network shows each answer a few milliseconds
after the page acted on it. `openx` is the exception: its exchange decides,
and the page waits for the answer. The four bidders that answer in time have
more than 100 ms of headroom below the wrapper's 250 ms, so that their answers
are in time on the network too.

The GPT stand-in has demand only for header-bidding line items: a slot whose ad
request carries `hb_adid` is filled with that bid's creative (which triggers
Prebid's `bidWon` and `adRenderSucceeded`); any other request renders empty.
Like GPT, it refuses a second slot for a container (`defineSlot` returns
`null` and logs "is already associated with another slot") until the first is
destroyed, and reports `impressionViewable` after one second at least half in
view.

A bid buys one impression. Asked to render a bid it rendered before, the
Prebid stand-in draws nothing, warns "Ad id ... has been rendered before" and
emits `adRenderFailed` (reason `exception`).

## Planted bugs (ground truth for tests)

**(a) Scroll gate on the wrong device class.** `scrollGatedAuction` is meant to
delay the first auction on phones until the reader scrolls. `isMobileViewport()`
tests `(min-width: 768px)` where `max-width` was intended, so the gate applies to
desktop and tablet, and phones skip it.
- Desktop, no scroll: no auction for `ad-leaderboard`, `ad-sidebar-1`,
  `ad-sidebar-2`, `ad-inarticle-1`; their slots are defined but never requested,
  although `ad-leaderboard` is in the first viewport.
- Desktop, with scroll: that auction starts a few milliseconds after
  `pageLifecycle: firstScroll`, seconds after `load`.
- Mobile: the same auction starts at page load with no scroll at all.

**(b) Ad request races its auction.** `startAnchor()` calls
`googletag.pubads().refresh()` right after `pbjs.requestBids()` instead of inside
`bidsBackHandler`. `ad-sticky-footer` gets `slotRequested` with no `hb_*` keys
before its `auctionEnd`, and renders empty although bids came back. Both devices.

**(c) Timeout misconfiguration.** The config defines `prebid.auctionTimeout: 1200`
but the wrapper reads `CONFIG.prebid.timeout || 250`. `prebidConfig.bidderTimeout`
and every `auctionInit.timeout` are 250. `pubmatic` (which would fit in 1200 ms
and pays most) and `criteo` time out on every request and never produce a
`bidResponse`; Prebid logs a "Late response from pubmatic" warning.

**(d) Ad unit without a slot, and GPT loaded twice.** `ad-sidebar-2` is a Prebid
ad unit, but the page has no such container, so no GPT slot is defined: bidders
are asked (and bid) for it in every gated auction, and nothing is ever requested
from the ad server. Separately, `ensureLibrary()` matches script URLs exactly, so
it loads `/tag/js/gpt.js?v=adstack-4.2.1` although the page already includes
`/tag/js/gpt.js`: two scripts with identical bodies, plus a console warning.

**(e) Bidder missing from ads.txt.** `ix` is called for `ad-leaderboard`,
`ad-inarticle-1` and `ad-sticky-footer`, but `/ads.txt` has no
`indexexchange.com` line. It answers every request with `noBid`.

**(f) Required param missing.** `BIDDER_PARAMS.triplelift` reads
`slot.inventoryCode`, which only the `ad-inarticle-2` slot defines. On
`ad-sidebar-1` and `ad-inarticle-1` the bid has no `inventoryCode`: Prebid
warns "Invalid bid sent to bidder triplelift", sends nothing and reports
`noBid` for both. On `ad-inarticle-2` the bidder is asked and bids.

**(g) Exchange blocks the publisher.** The one `openx` bid request, sent in
the gated auction for `ad-leaderboard`, `ad-sidebar-1` and `ad-inarticle-1`,
gets HTTP 200 with `"nbr": 7` from `/openrtbb/prebidjs`. The capture's
`bidResponses` carry, for `openx`, status 200 and the reason
"no-bid reason 7 (blocked publisher or site)", and for every other bidder
status 200 and no reason; Prebid reports `noBid` for all three ad units. `openx.com` is authorised in ads.txt, so the endpoint's answer
is the cause.

The three bugs below need a visitor who does more than load and scroll one
page. A scan with the `none` or `scroll` behaviour never shows them.

**(h) Engagement refresh with a used bid.** `watchEngagementRefresh()` waits
for `impressionViewable` of `ad-sidebar-1`; once the slot has then been at
least half in view for 5 s and the reader moved the pointer or touched the
screen within the last 5 s, it calls `googletag.pubads().refresh([slot])`
without a new auction. The slot still carries the `hb_*` keys of the bid it
showed.
- Ground truth: a second `slotRequested` for `ad-sidebar-1` with the same
  `prebidAdId` as the first and no `auctionInit` for that ad unit in between;
  GPT reports the slot as filled (the line item matched), the creative stays
  blank and Prebid emits `adRenderFailed` for `ad-sidebar-1`.
- `browse`, desktop: about 6 s after the rail ad rendered, which on desktop is
  after the first scroll (bug (a)). On a phone the rail ad is below the
  article and rarely stays in view for 5 s, so the refresh rarely happens.
- `manual`: whenever the person keeps the rail ad in view and moves the mouse.

**(i) A later page of the session never bids.** A "double-fire guard" was
meant to stop a consent reload from starting the first auction twice. It
remembers the time of the last auction in `sessionStorage` and compares it
with `doubleFireGuardMs: 120000`: for two minutes after an auction, every page
the reader loads skips `pbjs.requestBids` and goes straight to the ad server.
- Ground truth on a page view reached by loading a document within 120 s of
  an auction on an earlier page: no `auctionInit` for any of the five slots;
  each slot that is requested (`ad-sticky-footer` at once, the gated three
  after the first scroll on desktop, `ad-inarticle-2` when scrolled to) has no
  `hb_*` keys and renders empty. The only auction on the page is the
  recirculation unit's.
- `browse` and `manual`: on every page reached through a plain link.
- Not in a new tab, which starts a new session.

**(j) A route change does not reset the stack.** On `meridian:route` the
wrapper runs its page-load setup again: `pbjs.addAdUnits` and
`googletag.defineSlot` for every slot, without `removeAdUnit` or
`destroySlots` first. GPT refuses all five definitions, the wrapper stores the
`null`s over its slot references, and the ad server request that follows the
auction has no slot left to refresh.
- Ground truth on a page view reached by a story card: five
  `slotDefinitionRejected` (one per container), no `slotsDestroyed`, no
  `slotRequested` at all. When the route changes on the entry document there
  is also one auction, with every ad unit in it twice, whose bids are never
  used. When it changes on a document where bug (i) applies, that auction is
  skipped too.
- `browse` and `manual`: on every page view reached through a story card.

## What each visitor behaviour shows

| Behaviour | Page views | Bugs visible |
|---|---|---|
| `none` | 1 | (a) as "the gated auction never starts" on desktop, (b), (c), the double GPT load of (d), (e) for the anchor. (f) and (g) need the gated auction, which a phone runs at load |
| `scroll` | 1 | (a) to (g) |
| `browse` | 1 + pages followed | (a) to (g) on the entry page; (h) on desktop; (i) or (j) on each further page view, by how it was reached |
| `manual` | as browsed | whatever the person's path leads to |

The page has no consent platform, so no consent event is ever emitted.

## On the network (ground truth for the request record)

What every page view that loads a document requests besides its bid requests,
and what a scan with network recording must show for it:

| Request | When | What it shows |
|---|---|---|
| The document, `site.css`, seven scripts (GPT twice, see (d)) | at load | Kept bodies. A script's response body is its body in the script list |
| `/static/img/harbour-dusk.png`, `/static/fonts/meridian-initial.ttf` | at load | An image and a font: status, sizes and type, no body |
| `GET <identity>/identity/envelope` | when `prebid.js` runs | A failed request: the provider hangs up (`net::ERR_EMPTY_RESPONSE` or a like error), no status, no body. Chrome logs it to the console |
| `/static/js/analytics-worker.js`, then `/collect/sampling.json` | at load | The worker's script is a request of the page; the JSON is requested by the worker, in a target of its own |
| `POST /collect` | at load and on scroll milestones | Beacons (resource type "ping") with their JSON body, answered 204 |
| `/api/archive-index.json` | 1.5 s after `load` | One large JSON body, about 1.3 MB |
| `OPTIONS <partner>/ut/v3/prebid` | before the first `appnexus` bid request | A CORS preflight, linked to that bid request. Chrome remembers the answer for a few seconds, so an `appnexus` request that follows soon has none |
| `pubmatic` and `criteo` bid requests | in every auction that asks them | Answers that arrive after their auction closed (250 ms, bug (c)): 330-620 ms and 1.5-2.1 s after the request |
| `/usersync/start` → `<partner>/usersync/match` → `/usersync/done.gif` | 3 s (`userSync.syncDelay`) after the first auction of the document ended | A redirect chain of three hops: 302, 302 with `Set-Cookie: uid=...`, 200 |
| `<partner>/usersync/frame.html`, its `/usersync/ids.json` and `/usersync/done.gif?from=frame...` | with the sync | A cross-site frame in a process of its own: the document is requested by the page, the JSON and the pixel by the frame |

A `triplelift` bid request is only sent for `ad-inarticle-2` (bug (f)); the
other two ad units have no valid bid and nothing leaves the page for them.
A page view reached by a story card (a route change) loads no document: of
the above it only has the beacon, the fragment fetch and, when an auction
runs, bid requests.

What works correctly, for contrast: `ad-inarticle-2` is lazy-loaded with an
`IntersectionObserver` and auctioned when it comes within 250 px of the viewport;
Prebid and Amazon bids are awaited together with a 2.5 s failsafe. The
recirculation unit `ad-recirc` is meant for engaged readers only: it is
defined and auctioned from the second page a reader loads in a session
(`sessionStorage` counter), with the ad request sent from `bidsBackHandler`.
Its auction is not behind the double-fire guard.
