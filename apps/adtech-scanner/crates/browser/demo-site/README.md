# Demo publisher site

A fictional news article ("The Meridian Post") with a realistic header-bidding
stack. It is embedded in the binary, served by `start_demo_site()` on a random
loopback port, and loads nothing from the internet. It is the in-app "Try the
demo" target and the fixture of `tests/demo_site.rs`.

## What is served

| Path | What it is |
|---|---|
| `/` | The article page (`index.html`), with one inline script (page config and theme) |
| `/static/ads/prebid.js` | Prebid.js stand-in: public API, event payloads and debug logging of the real library; bidders are simulated in the page |
| `/tag/js/gpt.js` | Google Publisher Tag stand-in: `googletag` API and slot events |
| `/static/ads/apstag.js` | Amazon TAM stand-in, loaded by the wrapper |
| `/static/ads/hb-wrapper.min.js` | The publisher's wrapper ("adstack"), minified. **This is where the bugs are.** |
| `/static/js/app.9b3e7f.js`, `/static/js/vendors-ui.4c1d2a.chunk.js`, `/static/js/analytics.js` | Application bundle, UI library chunk and first-party analytics: not ad related |
| `/collect` | Sink for the analytics beacons (204) |

`static/ads/hb-wrapper.src.js` is the readable source of the wrapper. It is not
served. After changing it, regenerate the served file:

```sh
npx terser@5 static/ads/hb-wrapper.src.js --compress --mangle --comments '/^!/' -o static/ads/hb-wrapper.min.js
```

## Simulated ad stack

Slots on the page: `ad-leaderboard` (top, in the first viewport),
`ad-sidebar-1`, `ad-inarticle-1`, `ad-inarticle-2` (lazy, far down the article)
and `ad-sticky-footer` (anchor).

Simulated bidders, with outcomes seeded per bidder, ad unit and request count so
every run is identical:

| Bidder | Latency | CPM | Behaviour |
|---|---|---|---|
| appnexus | 70-150 ms | 0.35-1.90 | bids on almost everything |
| rubicon | 105-195 ms | 0.45-2.40 | bids on most requests |
| ix | 130-215 ms | 0.30-1.60 | passes on about half |
| pubmatic | 330-620 ms | 1.80-4.60 | highest CPMs, slower than the misconfigured timeout |
| criteo | 1500-2100 ms | 0.60-2.20 | slower than any sensible timeout |

The GPT stand-in has demand only for header-bidding line items: a slot whose ad
request carries `hb_adid` is filled with that bid's creative (which triggers
Prebid's `bidWon` and `adRenderSucceeded`); any other request renders empty.

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

What works correctly, for contrast: `ad-inarticle-2` is lazy-loaded with an
`IntersectionObserver` and auctioned when it comes within 250 px of the viewport;
Prebid and Amazon bids are awaited together with a 2.5 s failsafe.
