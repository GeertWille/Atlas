# Ad servers on BE/NL/DE publisher sites: live measurements (2026-11-13)

All paths below are under `/tmp/claude-0/-home-user/ee93eb55-ef69-5991-bc2c-57c8a322dcdf/scratchpad/adserver-sites/`.

## 0. What was done, in short

- 80 distinct sites probed with headless Chromium (Playwright), 89 page loads in total. That is well above the 30 to 45 sites asked for: the first 44 sites produced no Adhese, Adition or Adform publisher request at all, so two more batches were added. No site was loaded more than twice.
- Exit IP is in the US, and the browser announced itself as `HeadlessChrome/153`. Both shaped the results (see section 7).
- Headline results:
  - **Adhese**: real request/response pairs captured on dhnet.be and lalibre.be (account `ipm`), all no-fill. There it is a **Prebid bidder in front of Google Ad Manager**, not the primary ad server. No site with Adhese as primary ad server could be observed, because every Mediahuis Belgium site blocked the probe.
  - **Adform publisher tag**: one real batch request/response captured on tv3.lt, no-fill for both placements.
  - **Adition publisher ad server**: **not seen on any German-language site** (26 tried, 22 loaded). Only cookie syncs and one DSP creative. Yieldlab is everywhere in Germany, but as a bidder.
  - **AdSense direct**: captured on synoniemen.net and golem.de, no-fill only.
  - Google Ad Manager is the primary ad server on almost every site that loaded. Xandr AST is the primary on all DPG Media sites, welt.de and iltalehti.fi.

Tags used below: **(V)** seen in a capture from this session, **(D)** read in vendor documentation or library source fetched this session, **(M)** from memory, not verified.

## 1. Results table

Columns: primary ad server with evidence (request counts are for the one page load) · header bidding · consent platform and whether consent was given · whether ads loaded · confidence in the ad-server classification.

"TCF id" is the `cmpId` reported by `__tcfapi`. A platform name is given only where a page global or host confirmed it.

### Belgium

| Site | Primary ad server(s), evidence | Header bidding | Consent | Ads loaded? | Confidence |
|---|---|---|---|---|---|
| hln.be | Xandr AST, member 9998: 5 × `POST ib.adnxs.com/ut/v3`, 17 tags in `apntag` | Prebid 11.31: adform, criteo, gps, gumgum, ix, rubicon. `adheseBidAdapter` is compiled into the build but no ad unit uses it | DPG privacy gate (TCF id 411). No dialog shown to this visitor; TCF string present, 8 purposes true | No: every UT call answered `{"error":"unknown"}` | High |
| demorgen.be | Xandr AST, member 9998: 2 UT calls, 10 tags | Prebid 11.31: adform, criteo, gumgum, ix, rubicon | Same as hln.be | No (same error) | High |
| 7sur7.be | Xandr AST, member 9998: 3 UT calls, 12 tags | Prebid 11.31: adform, criteo, gps, gumgum, ix, rubicon | Same as hln.be | No (same error) | High |
| dhnet.be (2 loads) | Google Ad Manager, network 22133500099: 2 × `gampad/ads`, 7 slots. Video: 4 GAM VAST calls and 2 Equativ `/ac?...out=vast4` | PubMatic OpenWrap (`owpbjs` 10.15): **adhese** (account `ipm`, 4 calls), equativ, msft, ix, pubmatic, rubicon, proxistore, seedtag, teads, triplelift, gumgumskins | Didomi. No dialog for this visitor (`shouldConsentBeCollected()` false). Consent given through `Didomi.setUserAgreeToAll()` on the second load | Yes on the second load (GAM creatives and 7 impression beacons). First load, no consent: no display requests | High |
| lalibre.be | Same stack as dhnet.be (same GAM network, same Adhese account): 2 gampad, 4 Adhese, 4 GAM VAST, 2 Equativ `/ac` | Same as dhnet.be | Didomi, consent through the API | Yes (GAM) | High |
| rtbf.be | Equativ, network 3491: 4 × `POST www8.smartadserver.com/genericpost`. No GAM | None seen (`pbjs` object without version or ad units) | Didomi, no dialog; API consent set late in the load | Partly: 4 calls, at least 2 no-ad beacons (`/h/aip`) | High |
| rtl.be (2 loads) | Google Ad Manager likely: 11 GPT slots defined under `/81985301/RTLINFO/...`, but **no ad request left**. A `sas` global is also present | `pbjs` object, no ad units | Didomi, no dialog; API consent had no effect | No | Medium (inconclusive) |
| 2dehands.be | GPT library and OpenWrap (`owpbjs` 10.15) loaded, 0 slots, no ad request | OpenWrap, no ad units | Sourcepoint (`_sp_` global); TCF call timed out; not accepted | No | Low (inconclusive) |
| goplay.be (2 loads) | Redirects to play.tv. First probe crashed; second: no ad requests | None seen | Didomi, not accepted | No | Inconclusive |
| pickx.be | No ad technology seen on the landing page | None | OneTrust, `AllowAll()` through the API | No | Low |
| nieuwsblad.be, standaard.be, gva.be, hbvl.be | **Blocked**: HTTP 403, Cloudflare "Attention Required" page | – | – | – | No data |
| zimmo.be, jobat.be | Blocked: 403 (zimmo.be shows a Cloudflare challenge) | – | – | – | No data |
| lesoir.be, sudinfo.be | Blocked: 403 on the document, nothing else loaded | – | – | – | No data |
| knack.be | Blocked: HTTP 405 | – | – | – | No data |
| tijd.be | Blocked: 403 | – | – | – | No data |
| bol.com (BE) | Blocked: 403 | – | – | – | No data |

### Netherlands

| Site | Primary ad server(s), evidence | Header bidding | Consent | Ads loaded? | Confidence |
|---|---|---|---|---|---|
| ad.nl | Xandr AST, member 7463: 4 UT calls, 14 tags. Video: smartclip (`ad.sxp.smartclip.net/select`, VMAP then VAST) | Prebid 11.31: adf, criteo, gps, ix | DPG gate (TCF id 411), no dialog, 7 purposes true | No display (UT error). smartclip answered a VAST wrapper | High |
| nu.nl | Xandr AST, member 7463: 2 UT calls, 7 tags. smartclip `/ads?type=dyn&plc=109248` | Prebid 11.31: adf, criteo, gps, ix | Same | No (UT error) | High |
| volkskrant.nl | Xandr AST, member 7463: 2 UT calls, 9 tags | Prebid 11.31: adf, criteo, gps, ix | Same | No (UT error) | High |
| rtl.nl | Xandr AST, member 7463: 2 UT calls, 8 tags. Also `POST advertising-module.api.dpgmedia.cloud/web/rtl` | Prebid 11.31: adf, criteo, gps, ix | Same | No (UT error) | High |
| tweakers.net | Xandr AST, member 7463: 1 UT call, 4 tags | `pbjs` object, no ad units | Same (the gate redirected and came straight back) | No (UT error) | High |
| limburger.nl (2 loads) | Google Ad Manager: 8 GPT slots under `/75037320/limnl/home`, **no request left**. `apntag` global present but not loaded | Prebid 11.31 configured: adform, **adhese**, criteo, improvedigital, ix, msft, pubmatic, rubicon, seedtag, teads. None requested | Didomi, no dialog; API consent registered in Didomi (13 purposes) but the page did not start its ads | No | Medium for GAM + Adhese-as-bidder; no request seen |
| buienradar.nl (2 loads) | Google Ad Manager, network 21730500021: 5 gampad calls, 4 slots | None seen | OneTrust. First load: no dialog, 0 purposes, no ads. Second: `OneTrust.AllowAll()` | Yes on the second load | High |
| nrc.nl | No GAM or Xandr request. One `POST adserving.optoutadvertising.com/prebid/display` | – | Didomi, `gdprApplies` false | No | Low |
| vertalen.nu | Google Ad Manager, network 1010985: 1 gampad call | None | Own banner ("Ok!" clicked) | Yes (1 creative) | Medium |
| synoniemen.net | **AdSense direct**, `ca-pub-8850378122772120`: 3 × `googleads.g.doubleclick.net/pagead/ads` | None | No consent platform | No: both slot requests came back unfilled | High |
| mijnwoordenboek.nl, encyclo.nl | GPT and Prebid (10.28) with Relevant Digital present; no ad request | – | consentmanager.net, not accepted | No | Inconclusive |
| telegraaf.nl, dvhn.nl, noordhollandsdagblad.nl, lc.nl | Blocked: 403 (Cloudflare) | – | – | – | No data |

### Germany and Austria

| Site | Primary ad server(s), evidence | Header bidding | Consent | Ads loaded? | Confidence |
|---|---|---|---|---|---|
| t-online.de | Google Ad Manager, network 4444 (Ströer): 1 gampad call, 3 slots; Ströer `SDG` wrapper | Prebid 11.41 (`pbjsYLHH`, yieldlove): 19 bidders incl. stroeerCore, smartadserver, appnexus, adform, orbidder | Sourcepoint, "ZUSTIMMEN" clicked, 11 purposes | Yes | High |
| golem.de | Google Ad Manager, network 4444,1033050 (Ströer metaTag): 3 gampad. **Plus AdSense direct**: 14 `pagead/ads`, 4 `ins.adsbygoogle`, `ca-pub-2342548244565729` and `ca-pub-8813347375005258` | Prebid 11.41: 23 bidders incl. yieldlab, stroeerCore | Sourcepoint, accepted | Yes (GAM) | High |
| focus.de | Google Ad Manager, network 3673 (Burda): 5 gampad, 7 slots; wrapper `a.bf-ad.net/adengine` | Prebid 11.29 through Relevant Digital: 28 bidders incl. yieldlab, adform, equativ, orbidder; Amazon APS | TCF id 35, "AKZEPTIEREN" clicked, 11 purposes | Yes | High |
| chip.de | Same stack as focus.de, network 3673: 5 gampad | Prebid 11.29: 31 bidders; APS | Same | Yes | High |
| spiegel.de | Google Ad Manager, network 183 (iq digital): 1 gampad, 5 slots; `IQD` wrapper | Prebid 11.35 through Relevant Digital: 28 bidders incl. yieldlab, adform, equativ; APS | Sourcepoint, accepted | Yes | High |
| sueddeutsche.de | Google Ad Manager, network 183: 1 gampad, 5 slots | Prebid 11.35: 23 bidders incl. yieldlab; APS | Sourcepoint, accepted | Yes | High |
| handelsblatt.com | Google Ad Manager, network 183: 3 gampad, 7 slots | Prebid 11.35 (plus a second `ihowpbjs` 8.19): 24 bidders incl. yieldlab; APS | Sourcepoint, accepted | Yes | High |
| tagesspiegel.de | Google Ad Manager, network 183: 7 gampad, 11 slots | Prebid 11.14: 24 bidders incl. yieldlab; APS | Sourcepoint, accepted | Yes | High |
| n-tv.de | Google Ad Manager, network 6032 (Ad Alliance): 5 gampad, 17 slots. **Yieldlab called directly** (6 × `/yp/<ids>`, `yl` global), not through Prebid | Prebid 11.39: connectad, criteo, pubmatic, seedtag; APS; Yieldlab direct | Sourcepoint, accepted | Yes | High |
| stern.de | Google Ad Manager, network 6032: 5 gampad, 23 slots. Yieldlab direct (3 × `/yp`). 2 AdSense requests (`ca-pub-9519890336884096`) | APS; Yieldlab direct; no `pbjs` object found | Sourcepoint, accepted | Yes | High |
| heise.de | Google Ad Manager, network 6514: 3 gampad, 6 slots | Prebid 11.42: 18 bidders (adf, stroeerCore, msft, ...) | Sourcepoint, accepted | Yes | High |
| wetter.com | Google Ad Manager, network 16486012: 4 gampad | Prebid 11.45: 18 bidders incl. yieldlab (`POST /ortb`), appnexus; APS | TCF id 329, accepted | Yes | High |
| merkur.de | Google Ad Manager, network 307308315,5176: 4 gampad, 10 slots | Prebid 10.29: 22 bidders incl. smartadserver, appnexus, yieldlab; APS | TCF id 21, `gdprApplies` false, no dialog | Yes | High |
| welt.de | **Xandr AST**, members 7823 and 7791: 2 UT calls, 6 tags, `ASCDP` wrapper (same as bild.de). GPT library loaded but 0 slots | Prebid 10.26: adf, aduptech, aps, criteo, ix, openx, orbidder, outbrain, pubmatic, rubicon, smartadserver, ttd, yieldlab | TCF id 345, "ALLE AKZEPTIEREN" clicked | No (UT error) | High |
| ran.de | Redirects to ran.joyn.de. Google Ad Manager, network 22605252184: 1 gampad, 3 slots | Prebid 11.16, bidders prefixed `cpg_`: adf, connectad, equativ, msft, netads, pubmatic, sovrn, triplelift, yieldlab | TCF id 318, accepted | Yes (1 creative) | Medium |
| prosieben.de | Redirects to joyn.de. No display ad call seen | – | TCF id 318, accepted | No | Low |
| dict.cc | Google Ad Manager, network 22152718 through Snigel (`adserv.snigelweb.com`): 2 gampad | Prebid 11.29: 23 bidders; APS | TCF id 9, `gdprApplies` false | Yes | High |
| sport1.de | GPT library loaded, 0 slots, no ad request | – | Sourcepoint clicked, but TCF reports `gdprApplies` false and 0 purposes | No | Inconclusive |
| web.de | Stopped on `web.de/consent-management/`; no ad technology beyond one Adition 1×1 image | – | TCF id 167, not accepted | No | Inconclusive |
| kleinanzeigen.de | Prebid 11.41 present, no ad request | – | TCF id 309, dialog not accepted | No | Inconclusive |
| finanzen.net | No ad request | – | consentmanager.net (TCF id 31), not accepted | No | Inconclusive |
| vol.at (AT) | No ad request after "Einverstanden" | – | consentmanager.net host seen | No | Inconclusive |
| zeit.de | Blocked: 403 | – | – | – | No data |
| kicker.de | Blocked: 403 with a `captcha-delivery.com` challenge | – | – | – | No data |
| mobile.de | Blocked: 403 | – | – | – | No data |
| faz.net (2 loads) | Probe hung and was killed at 150 s both times; nothing saved | – | – | – | No data |

### Nordics, Baltics, Poland, others

| Site | Primary ad server(s), evidence | Header bidding | Consent | Ads loaded? | Confidence |
|---|---|---|---|---|---|
| tv3.lt (LT) | **Adform publisher tag**: 1 batch `GET adx.adform.net/adx/?rp=3...` for 2 placements (mid 1650591, 1650592). Also 3 GPT slots (`/147246189,22579612491/...`) with no gampad request | None seen | No TCF platform answered | No: both Adform placements empty | High |
| delfi.lt (LT, 2 loads) | Google Ad Manager (1 slot defined). **Adform tags in the markup** (`data-adfscript="adx.adform.net/adx/?mid=1544005"` and 9 more; `adformtag` queue of 40 entries) but adx.js was never loaded and no Adform request left | Prebid 9.53, no ad units | Cookiebot, `gdprApplies` false, API consent had no effect | No | Medium |
| 15min.lt (LT) | Google Ad Manager, network 146733436: 7 gampad, 40 slots | None seen | Cookiebot, API consent | Yes | High |
| ekstrabladet.dk (DK) | Google Ad Manager, network 83172459: 6 gampad | Prebid 11.19 (Livewrapped): adnuntius, appnexusAst, criteo, ebadform, ix, pubmatic, rubicon, teads, ... | Cookiebot, "Tillad alle" clicked | Yes | High |
| bt.dk (DK) | Google Ad Manager, network 135050247: 3 gampad | Prebid 9.53: 20 bidders incl. adform (8 `/adx/openrtb`), adnuntius, appnexus | Cookiebot, accepted | Yes | High |
| tv2.dk (DK) | Google Ad Manager, network 19784602: 8 gampad | Prebid 8.52: 18 bidders incl. adf (16 `/adx/openrtb`), adnuntius | OneTrust, accepted | Yes | High |
| seoghoer.dk (DK) | Google Ad Manager, network 159150954: 3 display gampad, plus video (`output=xml_vmap1` then `xml_vast4`) | None seen | Didomi, `gdprApplies` false | Yes | High |
| jv.dk (DK, 2 loads) | GPT, Prebid 8.49 and Assertive Yield loaded; 0 slots, no ad request | – | Didomi, not accepted | No | Inconclusive |
| dagbladet.no (NO) | Google Ad Manager, network 8578: 3 gampad | Prebid 11.37: 19 bidders incl. adf, adnuntius, kobler | Sourcepoint, `gdprApplies` false | Yes | High |
| iltalehti.fi (FI) | **Xandr AST**, member 6931: 6 UT calls, 28 tags. No gampad | Prebid 10.23: adform, adnuntius, criteo, pubmatic, rubicon, teads | Sourcepoint, accepted | No (UT error) | High |
| postimees.ee (EE, 2 loads) | 9 GPT slots under network 82349501, no request | – | Didomi, not accepted | No | Medium (inconclusive) |
| onet.pl (PL) | Google Ad Manager: 9 gampad, 17 slots. Own SSP `ssp.ringieraxelspringer.tech/openrtb2/auction` | Prebid 11.27: 13 bidders incl. `das` (the publisher's own) | TCF id 280, accepted | Yes | High |
| borsen.dk, tv3.lv, tv3.ee, independent.ie, wort.lu | Blocked: 403 (tv3.lv and tv3.ee show a Cloudflare challenge) | – | – | – | No data |

## 2. Phase-2 ad servers

### 2.1 Adhese

**Confirmed sites (V)**

| Site | Role | Why useful |
|---|---|---|
| dhnet.be | Prebid bidder `adhese`, account `ipm`, in front of GAM | Display batch of 5 slots, a single-slot refresh, and a video slot (`...preroll-640x480`) called from the video player iframe |
| lalibre.be | Same account, same structure | Second title on the same account; slot names carry the site name |
| limburger.nl | Prebid bidder `adhese` configured (not requested in this session) | Mediahuis Netherlands; the same "Adhese as bidder beside GAM" pattern outside IPM |

Not confirmed: any site where Adhese is the **primary** ad server. The Mediahuis Belgium titles (nieuwsblad.be, standaard.be, gva.be, hbvl.be) are the obvious candidates (M) and all returned 403. DPG Media does **not** use Adhese as ad server on any of the 8 DPG sites tested: they are on Xandr AST, with only the `adheseBidAdapter` module compiled into their Prebid build.

**Request shape (V, 8 captures)**

```
POST https://ads-ipm.adhese.com/json
content-type: application/json
referer: https://www.dhnet.be/

{"slots":[
   {"slotname":"dhnet.be_ros_desktop_halfpage2-300x600","parameters":{}},
   {"slotname":"dhnet.be_ros_desktop_buttonad1-300x120","parameters":{}},
   {"slotname":"dhnet.be_ros_desktop_halfpage1-300x600","parameters":{}},
   {"slotname":"dhnet.be_ros_desktop_leaderboard1-970x250","parameters":{}},
   {"slotname":"dhnet.be_ros_desktop_rectangle1-300x250","parameters":{}}],
 "parameters":{"xt":["CQtnz4AQtnz4AAHABBENC4Fs…"],"xf":["aHR0cHM6Ly93d3cuZGhuZXQuYmUv"]},
 "vastContentAsUrl":true,
 "user":{"ext":{"eids":[{"source":"pubcid.org","uids":[{"id":"2c14ed85-…","atype":1}]}, …]}}}
```

- Endpoint: `POST https://ads-<account>.adhese.com/json`, no trailing slash, no path segments. The account is in the host name.
- Placement: `slots[].slotname` = `<location>-<format>`. In Prebid the two halves come from the bid params `location` (`dhnet.be_ros_desktop_leaderboard1`) and `format` (`970x250`). Here the format is a size; in other accounts it is a format code such as `leaderboard` (D).
- Sizes: not a separate field. Only present when the format half of the slot name happens to be a size.
- Slot-level targeting: `slots[].parameters`, a map of two-letter keys to string arrays. Empty in all captures.
- Request-level targeting: `parameters`, same structure.
- Consent: `parameters.xt[0]` is the TCF string. `tl` (`all` or `none`) is the binary consent flag (D); not sent here.
- Referrer: `parameters.xf[0]`, base64url of the page URL (`aHR0cHM6Ly93d3cuZGhuZXQuYmUv` = `https://www.dhnet.be/`). The `referer` header is the page, or the player iframe for the video slot.
- User ids: `user.ext.eids`, OpenRTB shape.
- Header-bidding keys: none in the request. Adhese is itself the bidder here.
- Batching: several slots per request. Each slot name may appear once; a duplicate gives HTTP 442 (D).
- A user sync follows: `GET https://user-sync.adhese.com/iframe/user_sync.html?account=ipm&gdpr=1&consentString=…` (failed through this proxy).

A hand-made `GET https://ads-ipm.adhese.com/json/sldhnet.be_ros_desktop_rectangle1-300x250/sldhnet.be_ros_desktop_leaderboard1-970x250/tlall/dtdesktop?t=…` answered `200 []`, so the documented GET form works on this account too (V, one request).

**Response shape**

- No-fill (V, all 8 captures, plus the demo account): HTTP 200, `content-type: application/json`, body `[]`.
- Fill: **not captured**. From the vendor documentation (D) and the Prebid adapter source (D), a fill is a JSON array with one object per delivered ad:

```
[{"slotName":"homepage_banner","adFormat":"714x224","width":"714","height":"224",
  "tag":"<div>…ad markup…</div>",
  "trackedImpressionCounter":"https://ads-[account].adhese.com/track/…",
  "viewableImpressionCounter":"https://ads-[account].adhese.com/track/…-Adhese_IABview/…",
  "clickTag":"https://ads-[account].adhese.com/raylene/…/UR",
  "orderName":"Example Campaign","creativeName":"Example Creative",
  "extension":{"mediaType":"banner","prebid":{"cpm":{"amount":"13.047","currency":"EUR"}}}}]
```

  - Join key back to the request: `slotName`. Slots with no ad are simply absent from the array.
  - Markup: `tag`, or `body` when the ad comes from another source (`origin` set and not `JERLICIA`) or when `ext` is `js` and `body` holds script/HTML. Video: VAST in `tag`/`body`, or a URL in `cachedBodyUrl`.
  - Size: `width`, `height` as **strings**.
  - Ids: `id` (creative-to-booking link), `orderId` (campaign; the Prebid adapter uses it as deal id), `libId`, `adspaceId`, `advertiserId`, `slotID`.
  - Price: `extension.prebid.cpm.amount` (string) and `.currency`.
  - For ads from Adhese Gateway: `origin`, `originInstance`, and `originData.seatbid[0].bid[0]` with `crid`, `dealid`.
  - Tracking: `impressionCounter`, `trackedImpressionCounter`, `viewableImpressionCounter`, `tracker`, `trackingUrl`, `clickTag`.

**Differences from the plan**

| Plan (from memory) | Found |
|---|---|
| Endpoint `ads-<account>.adhese.com/json/sl<location>-<format>/...` with slots in the path | True for GET only (D, one manual V). Every page capture was `POST .../json` with slots in a **JSON body**. A decoder needs both: path segments for GET, `/slots[*]/slotname` for POST |
| Host `ads-<account>.adhese.com` | Seen. The current Prebid adapter (11.49 source) uses a second host form, `https://<account>.ads.adhese.eu/json` (D). `demo.ads.adhese.eu` answers (V). Match both `adhese.com` and `adhese.eu`, and allow first-party CNAMEs (D) |
| Adhese listed as an ad server (tier phase 2) | On the only sites where it was observed, it is a **bid source**: the call is made by the Prebid adapter and the slot is rendered by GAM. Classifying every `/json` call as "ad server call" would double-count slots on these sites. The Prebid POST carries `vastContentAsUrl` and `user.ext.eids`, which could be used as a hint; whether a non-Prebid POST lacks them is not verified |
| Fill fields `slotName, adType, width, height, tag\|body, impressionCounter, extension.prebid.cpm` | Consistent with the documentation. Add: `cpm` is an object `{amount, currency}` with a string amount; `width`/`height` are strings; `adFormat` is the format code and `adType` the requested format; three counters exist, not one |
| Empty array is no-fill | Confirmed (V). An older documentation page says an empty **object** `{}` for the mobile endpoint (D); not seen |
| "legacy `/ad3/`" | Not seen anywhere in the sample |
| Consent and referrer | Not in the plan. `xt` = TCF string, `xf` = base64url page URL, `tl` = binary consent (D/V) |

**Still unknown**

- A real fill. Only `[]` was seen, from a US IP.
- How a site with Adhese as primary ad server calls it (SDK generation, GET or POST, first-party host). No such site could be loaded.
- Whether the Mediahuis Belgium sites still use Adhese as primary ad server at all (M, untested).

**Capture files**

- `captures/dhnet.be-retry/ad/002-adhese.json` (batch of 5), `011-adhese.json` and `023-adhese.json` (video slot, from the player iframe), `018-adhese.json` (single-slot refresh)
- `captures/lalibre.be/ad/001-adhese.json`, `012-adhese.json`, `022-adhese.json`, `027-adhese.json`
- `captures/_adhese-ipm-manual/` (the one manual GET), `captures/_adhese-demo/` (5 manual calls to the public demo account, all `[]`)
- `captures/_docs-adhese/*.html` (vendor documentation pages as fetched)
- Bid params as configured on the page: `captures/dhnet.be-retry/summary.json` → `state.pbjs[0].bidParams.adhese`

### 2.2 Adition and Yieldlab

**Adition publisher ad server: not found.** No request to `imagesrv.adition.com/js/srp.js`, no `adition` global, no `ad*.adfarm1.adition.com/banner` call on any of the 26 German and Austrian sites that were tried (22 loaded, 16 of them with ad activity). What was seen under Adition host names:

| Request | Sites | What it is |
|---|---|---|
| `dsp-cookie.adfarm1.adition.com/?ssp=…` | 22 sites, incl. Danish, Polish and Belgian ones | Cookie sync of the Adition DSP, fired from other vendors' sync frames |
| `dspcluster.adfarm1.adition.com/show_banner?…` plus `imagesrv.adition.com/banners/3784/…` | focus.de, once | A creative delivered by the Adition DSP inside another ad server's slot |
| `imagesrv.adition.com/1x1.gif` | web.de | A single pixel on the consent page |

The sites the brief named as likely Adition users all turned out to be on Google Ad Manager (t-online.de, focus.de, chip.de, spiegel.de, sueddeutsche.de, n-tv.de, handelsblatt.com, heise.de, golem.de, wetter.com) or Xandr AST (welt.de). zeit.de, kicker.de, mobile.de and faz.net gave no data.

**Yieldlab: confirmed as a bid source on 12 sites**, in three forms.

1. Direct pre-bid by the publisher's own script, JavaScript answer. Seen on n-tv.de and stern.de (Ad Alliance), global `yl`.

```
GET https://ad.yieldlab.net/yp/11761102,11761103,11761104,14885105,11761108
      ?t=consent=yes&clienttc=ebov,dxbq,…&schain=1.0,1!ad-alliance.de,csid_22395447709,1,,,
      &pvid=1&pubref=https://www.n-tv.de/&gdpr_consent=CQtnz4…&ids=pubcid.org:501fd778-…&ts=0.0547…
→ 200 text/javascript
var yl=yl||{};yl.YpResult=yl.YpResult||function(){var a={};return{add:function(b){a[b.id]=b},…}}();
```

   The answer above is the no-bid form: only the `YpResult` prelude. A bid would add `yl.YpResult.add({...})` lines (inferred from the prelude, not captured).

2. Prebid adapter, older form. Seen on chip.de, focus.de, spiegel.de, sueddeutsche.de, handelsblatt.com, tagesspiegel.de, welt.de, merkur.de, ran.de.

```
GET https://ad.yieldlab.net/yp/21730,5108424,399073,12435406
      ?ts=…&json=true&t=kw=…&section=homepage&adSlot=iqadtile5
      &ids=pubcid.org:…&atypes=pubcid.org:1&schain=1.0,1!iqdigital.de,iqd-5026830680-1,1,,,,
      &iab_content=language:de&pubref=https://www.handelsblatt.com/&gdpr=true&gdpr_consent=…
      &sizes=21730:1920x333|970x250|…,5108424:728x90|…&floor=21730:75,5108424:75,399073:300,12435406:75
→ 200 application/json   []
```

3. Prebid adapter, current form: `POST https://ad.yieldlab.net/ortb` with an OpenRTB body, answered `204` (wetter.com, 5 calls).

- Placement: the comma-separated adslot ids in the path. Sizes and floors are keyed by adslot id in `sizes` and `floor`.
- Targeting: `t`, a URL-encoded key=value string.
- Consent: `gdpr`, `gdpr_consent`. Referrer: `pubref`.
- No-bid: `[]` (JSON form), prelude only (JS form), or 204 (`/ortb`).
- `ad.yieldlab.net/d/6846326/766/2x2?…` is **not** an ad delivery call here: it is the adapter's user-sync frame and returns an HTML page of sync pixels. `/m` and `/mr` are match redirects.

**Differences from the plan**

- The plan puts "Adition + Yieldlab" in phase 2 as a German ad server. In this sample the Adition ad server does not appear at all; treating it as a priority is not supported by these measurements.
- The plan says Yieldlab's pre-bid is `ad.yieldlab.net/yp/<ids>` and a bid source. Confirmed, with two additions: the `/ortb` POST form, and the direct non-Prebid use on Ad Alliance sites.
- The brief lists `/d/…` as a Yieldlab ad endpoint. In these captures `/d/6846326/766/2x2` is a sync frame; a decoder must not count it as an ad call.

**Still unknown**: every Yieldlab answer was empty, so the bid shape is not captured. The Adition request and response shape remain unverified: there was nothing to capture.

**Capture files**: `captures/n-tv.de/ad/003-yieldlab.json` to `009-yieldlab.json`, `captures/stern.de/ad/003-yieldlab.json` to `005-yieldlab.json`, `captures/handelsblatt.com/ad/002-yieldlab.json`, `captures/chip.de/ad/009-yieldlab.json`, `captures/wetter.com/ad/002-yieldlab.json` (`/ortb`), `captures/wetter.com/ad/019-yieldlab.json` (`/d/` sync frame), `captures/focus.de/ad/*adition*.json`.

### 2.3 Adform publisher tag

**Confirmed sites**

| Site | Evidence | Why useful |
|---|---|---|
| tv3.lt (V) | `s1.adform.net/banners/scripts/adx.js` loaded, one batch request, `adformtag` and `Adform` globals, 2 `data-adfscript` tags | The only site where the request left; also has GPT slots, so it tests two ad servers on one page |
| delfi.lt (markup only) | 10+ `data-adfscript="adx.adform.net/adx/?mid=…"` tags and a 40-entry `adformtag` queue, but adx.js was not loaded in this session | Many placements; needs a visit with consent from Europe |

None of the Danish, Norwegian, Finnish, Dutch or Belgian sites tested uses the Adform publisher tag. There Adform appears only as a Prebid bidder (`/adx/openrtb`, 24 sites) and as an advertiser-side ad server (`track.adform.net/adfscript`, `a1.adform.net/adfserve`). tv3.lv and tv3.ee, sister sites of tv3.lt, were blocked.

**Request shape (V, plus adx.js source (D))**

```
GET https://adx.adform.net/adx/?rp=3&pv=1
      &bWt3PXRpdHVsaW5pcyZtaWQ9MTY1MDU5Mg
      &bWt3PXRpdHVsaW5pcyZtaWQ9MTY1MDU5MQ
      &schain=1.0,1!tv3.lt,1,1&gdpr=0&url=https%3A%2F%2Fwww.tv3.lt%2F
      &callback=_adform_cb_1794559685323_34511014759379977
→ 302 to the same URL with CC=1 prepended (cookie check), then 200 text/javascript
```

- One request for all placements on the page (single-request mode is the default; `adformtag.disableSingleRequest` exists).
- **Each placement is one query parameter with no value, whose name is base64url of that placement's own query string.** `bWt3PXRpdHVsaW5pcyZtaWQ9MTY1MDU5Mg` decodes to `mkw=titulinis&mid=1650592`.
- Inside the decoded string: `mid` = placement id; `mkw` = keywords; `mkv` = key:value targeting; `msw` = search words; `dpr` = price, set by `adformtag.setPrice` (the place where a header-bidding price would be passed); `mcr` = custom data.
- Page-level parameters are plain: `rp=3` (constant in adx.js), `pv=1`, `schain`, `gdpr`, `gdpr_consent`, `gpp`, `gpp_sid`, `url` (page URL), `callback` (JSONP function name).
- The first request is always redirected once (`CC=1`). A decoder should take the second one, or de-duplicate on the `callback` value.

**Response shape (V)**

```
_adform_cb_1794559685323_34511014759379977([
 {"Response":" ",
  "UnloadParameters":{"AdSlotId":"e7431e0cdfc743e1bdb96800fc7ac987","CookieId":"…",
    "SerializedUserIdentity":"…","Width":300,"Height":250,"TestId":null,"SerializedAdxVars":"…",
    "PublisherVisibilityTime":1,"PublisherVisibilityArea":50,
    "UnloadHandlerUrl":"https://adx.adform.net/adx/unload/?","Gdpr":false,"GdprConsent":null,
    "Gpp":null,"GppSid":null,"IsMobile":false,"AdEventsUrl":null,"BeginToRender":false,
    "DistributionChannel":"Site"}},
 {"Response":" ","UnloadParameters":{"AdSlotId":"bfe8c576…","Width":1200,"Height":150, …}}]);
```

- JSONP: a call to the `callback` function with an **array, one entry per placement, in request order**. There is no placement id in the answer; the join is by position.
- Fill versus no-fill, from adx.js: `isEmpty = !/\S/.test(Response)`. A `Response` that is empty or only white space is a no-fill. Both entries here are `" "`, so both are no-fills.
- On a fill, `Response` holds the creative markup (D, from the same code path: `content(Response)`); not captured.
- Size: `UnloadParameters.Width` and `Height`, present even on a no-fill (it is the placement size).
- Ids: `UnloadParameters.AdSlotId` (per impression opportunity). No creative, campaign, price or deal field in the no-fill answer.
- Tracking: `UnloadHandlerUrl` (`POST https://adx.adform.net/adx/unload/?…` when the page closes), and `AdEventsUrl` with `SerializedAdxVars` when `BeginToRender` is true.

**Differences from the plan**

| Plan | Found |
|---|---|
| `<script data-adfscript="adx.adform.net/adx/?mid=<id>">`, placement is `mid` | The markup is exactly that. But the **network request does not contain `mid=` in clear text**: it is inside a base64url parameter name. A declarative `"placement": ["mid"]` query locator will find nothing. This needs a small named decoder: for every query parameter with an empty value, try base64url, then parse the result as a query string |
| No statement about batching | One request for all placements, answers joined by position |
| Tag API `setTargeting, resetTargeting, setRequestParam, setCustomData, setPrice, onLoaded, onRendered` | Confirmed on the page, plus `setSearchWord`, `disableSingleRequest`, `sticky`, `floating`, `setRequestLimit`, `addRequestGroup`, `addStrictGroup`, `setExtendedIDs`, `setGlobalTargeting` |
| `/adx/openrtb` is the Prebid endpoint | Confirmed, 24 sites |

**Still unknown**: a fill; where price or deal data would appear on a fill; the request with `gdpr=1` and a consent string (tv3.lt sent `gdpr=0` to this US visitor).

**Capture files**: `captures/tv3.lt/ad/002-adform-adx.json` (302), `003-adform-adx.json` (the answer), `001-adform-other.json` (adx.js, 72 KB); `captures/tv3.lt/summary.json` and `captures/delfi.lt-retry/summary.json` → `state.adform`.

### 2.4 AdSense direct tags

**Confirmed sites (V)**

| Site | Evidence | Why useful |
|---|---|---|
| synoniemen.net | 3 `pagead/ads` requests, nothing else on the page (47 requests in total) | Smallest possible test page, no consent platform |
| golem.de | 14 `pagead/ads` requests, 4 `ins.adsbygoogle` (3 with `data-ad-slot`), next to GAM and Prebid | AdSense beside GPT on one page: the merge case |
| stern.de | 2 requests | Minor |
| dhnet.be, lalibre.be | 4 and 10 requests, `ca-pub-3884220855289250`, with `slotname` set; 0 `ins.adsbygoogle` in the top document | Whether this is a direct tag or an AdSense tag delivered as a GAM creative was not determined |

**Request shape**

```
GET https://googleads.g.doubleclick.net/pagead/ads
      ?client=ca-pub-8850378122772120&output=html&h=280&w=336&format=336x280
      &slotname=…&adk=3365045730&adf=1852631761&url=https://synoniemen.net/
      &gdpr=0&correlator=2838500267484&pvsid=7034568266788100&ifi=2&frm=20&…
resource type: document (it is the src of the ad iframe)
```

- Publisher: `client` (`ca-pub-…`). Placement: `slotname` (the page attribute `data-ad-slot` and this parameter were not compared value by value). `adk` stayed the same for two requests of the same unit type (`adk=3365045730`).
- Size: `format` = `WxH`, also `w` and `h`.
- Page URL: `url`. Consent: `gdpr`, `gdpr_consent`, `addtl_consent` (seen on stern.de).
- Page grouping: `correlator` and `pvsid` are shared by all requests of one page view; `ifi` counts the frames (1, 2, 3).
- **One request per ad unit, no batching.**
- **The first request of a page has `format=0x0` and no size.** It is the page-level (auto ads) configuration request, not a slot. Its answer is a small script (`"googMsgType":"apcnf"` or `"sc-cnf"`). It must not be counted as an ad slot.

**Response shape**: an HTML document for the iframe. Only no-fills were seen:

- synoniemen.net, 64 KB: starts with `window.top.postMessage('{"msg_type":"unfill-fb", …` and carries a fallback content block instead of an ad.
- stern.de, 430 bytes: `window.top.postMessage('{"msg_type":"resize-me","key_value":[{"key":"r_nh","value":"0"}, …,{"key":"qid","value":"CInRztq_…"}],"googMsgType":"sth"}', '*')`, which collapses the frame to height 0.
- golem.de: the bodies could not be read (some answers were 302 redirects, the rest came back empty from the browser API).
- In the page, golem.de has an `ins.adsbygoogle` with `data-ad-status="unfilled"`. That attribute is the only fill signal that was observed directly.

**Differences from the plan**: the plan has one line for AdSense and no shape. New: one request per unit; the `format=0x0` configuration request; the answer is HTML, so there is no structured fill field, and price or deal data is not exposed at all.

**Still unknown**: what a filled answer looks like and how to tell it from the network alone. `data-ad-status` on the `ins` element is the practical signal.

**Capture files**: `captures/synoniemen.net/ad/001-adsense-ads.json` to `003-adsense-ads.json`, `captures/stern.de/ad/010-adsense-ads.json` and `011-adsense-ads.json`, `captures/golem.de/ad/005-adsense-ads.json` to `010-adsense-ads.json` (request side only).

## 3. Phase-3 sightings

| What | Where | Notes |
|---|---|---|
| Google video, `pubads.g.doubleclick.net/gampad/ads?...output=xml_vmap1` then `xml_vast4` | seoghoer.dk (14 requests), dhnet.be and lalibre.be (4 each) | seoghoer.dk: a VMAP whose `AdTagURI` points to `gampad/live/ads`. dhnet.be: an empty `<VAST version="4.0"/>`, a clean no-fill reference. Files: `captures/seoghoer.dk/ad/003-gam-video.json`, `004-gam-video.json`, `captures/dhnet.be-retry/ad/012-gam-video.json` |
| smartclip `ad.sxp.smartclip.net` | ad.nl (5 × `/select`, 2 × `stats…/count`), nu.nl (2 × `/ads?type=dyn&plc=109248&sz=1x1`) | ad.nl: first a VMAP, then `<VAST version="3.0"><Ad id="ADFILL:0"><Wrapper>…`. Files: `captures/ad.nl/ad/006-smartclip.json`, `007-smartclip.json` |
| Equativ video, `www8.smartadserver.com/ac?siteid=358063&pgid=1257013&fmtid=84995&…&out=vast4` | dhnet.be, lalibre.be | Legacy `/ac` used as a VAST tag; 302 to `&cklb=1` first |
| Equativ display, `POST www8.smartadserver.com/genericpost` | rtbf.be (network 3491, 4 calls) | Request is JSON like onecall (`networkId, siteId, pageId, ads:[{formatId, tagId, target, headerBidding, isLazy, isAdRefresh}]`). The answer is **JavaScript**, starting `sas.events.fire("ad", { tagId: "ads__R_2bavknqlb__105399", formatId: 105399 }, …)`. Useful for the plan's `smart-genericpost` decoder. File: `captures/rtbf.be/ad/001-equativ-call.json` |
| Equativ as bidder, `POST prg.smartadserver.com/prebid/v1` | t-online.de (3), welt.de (3), merkur.de (14) | Bid source, not an ad call |
| Xandr AST as primary | hln.be, demorgen.be, 7sur7.be (member 9998); ad.nl, nu.nl, volkskrant.nl, rtl.nl, tweakers.net (member 7463); welt.de (7823, 7791); iltalehti.fi (6931) | Tags use `code` (for example `hln_wmc_home_home_h1`) with `member_id` on DPG and welt; numeric ids on iltalehti.fi |
| Xandr as bidder, `/ut/v3/prebid` | 13 sites | Same host, different path |
| Xandr `/ptv` | Not seen | |
| FreeWheel | Only `user-sync.fwmrm.net/ad/u` (22 sites) and `7cf90.v.fwmrm.net/ad/u` (wetter.com, joyn.de) | User syncs. No `/ad/g/1` ad request anywhere |
| SpringServe | Only `sync.springserve.com` | No VAST call |
| Kevel/Adzerk, Revive | Not seen on any site | |
| Amazon APS `/e/dtb/bid` | chip, focus, golem, handelsblatt, spiegel, sueddeutsche, tagesspiegel, n-tv, stern, wetter.com, merkur, dict.cc, onet.pl | 12 of the 16 German sites with ad activity, and onet.pl |

## 4. Ad-server-like hosts that are not in the plan

| Host and path | Where | What it looks like |
|---|---|---|
| `europe.delivery.adnuntius.com/i`, `ads.adnuntius.delivery/i` (`?format=prebid&…`), library `cdn.adnuntius.com/adn.js` | bt.dk, ekstrabladet.dk, tv2.dk, dagbladet.no, iltalehti.fi | Adnuntius, a Nordic ad server that is also a Prebid bidder (`adnuntius`, and `adnuntiusadserver` on ekstrabladet.dk). All calls here were the Prebid form and failed in this session. Relevant only if Nordic customers appear |
| `prebid.optinadserving.com/prebid/display`, `adserving.optoutadvertising.com/prebid/display`, library `cdn.optoutadvertising.com/script/ootag.v2.min.js` | DPG sites (hln.be, demorgen.be, 7sur7.be, ad.nl, nu.nl, volkskrant.nl, rtl.nl, tweakers.net) and nrc.nl | Opt Out Advertising: an ad server with its own tag (`ootag`) and a Prebid endpoint. On nrc.nl it was the only ad request. **Worth a look for NL/BE**: it is on the DPG pages |
| `advertising-module.api.dpgmedia.cloud/web/<site>` and `/video/ad` (POST), `advertising-cdn.dpgmedia.cloud` | DPG sites | DPG's own ad configuration and video ad decision service, in front of Xandr. Bodies were not saved |
| `ssp.ringieraxelspringer.tech/openrtb2/auction` | onet.pl | The publisher's own SSP (bidder `das`) |
| `te.technical-service.net/api?pa=43&gdpr_consent=…`, `native.emsservice.de/teasers/<id>.json`, `static.emsservice.de/molten/live/render.*.js` | n-tv.de, stern.de, welt.de | Ad Alliance: an identity/targeting service and a native teaser delivery with numeric ids. The teaser JSON calls behave like ad calls for native slots |
| `a.bf-ad.net/adengine/<site>/adengine.js`, `/sad/<site>.json`, `/vbuckets/…` | focus.de, chip.de | Burda's wrapper and configuration, not an ad server |
| `cdn.stroeerdigitalgroup.de/metatag/live/<site>/metaTag.min.js` | golem.de (t-online.de has the `SDG` global from another host) | Ströer wrapper in front of GAM network 4444 |
| `ads.videoadex.com/jw/advertisement.js` | dhnet.be, lalibre.be | Video ad helper script |
| `adserv.snigelweb.com` | dict.cc | Snigel wrapper in front of GAM |
| `ad-delivery.net/px.gif` | nu.nl, rtl.nl, ekstrabladet.dk, delfi.lt | Looks like ad-block detection or recovery; vendor not identified |
| `*.ay.delivery` | jv.dk, n-tv.de | Assertive Yield analytics (matches the `assertive` global) |

## 5. Which ad servers are most common here, against the plan's ranking

Primary ad server, counted over the 43 sites where a primary could be named (requests seen, or slots defined in the page):

| Ad server | Sites | Plan's rank |
|---|---|---|
| Google Ad Manager | 30 | 1 (done) |
| Xandr AST | 10 (8 of them DPG Media) | 2 |
| Equativ | 1 as primary display (rtbf.be); video on 2; bidder on many | 3 |
| Adform publisher tag | 1 with a request (tv3.lt), 1 in markup (delfi.lt) | 6 |
| AdSense direct | 1 as only ad source (synoniemen.net), 2 to 4 beside GAM | 8 |
| Adhese | 0 as primary. Bidder on 2 sites (requests seen) and 1 more (configured) | 4 |
| Adition | 0 | 5 |

What this says about the plan's ranking:

- GAM first and Xandr second match. For Belgian and Dutch news, Xandr matters mostly because of one publisher, DPG Media.
- **Adition at rank 5 is not supported.** Zero sightings on the German sites where it was expected. The effort would be better spent elsewhere unless a customer is known to use it.
- **Adhese at rank 4 could not be confirmed or refuted as a primary ad server**, because the sites most likely to use it were unreachable. What is confirmed is that it shows up as a bid source next to GAM (IPM, Mediahuis Netherlands), and the plan does not model that role.
- **Yieldlab deserves its own entry as a bid source**: 12 of the 16 German sites with ad activity, in three request forms.
- Adform as a bidder is on 24 sites; as a publisher ad server only in Lithuania here.
- Two things not in the plan are common in this market: Amazon APS on German sites (already handled as a bid source) and Opt Out Advertising on the DPG pages.

Caveat: this is a hand-picked sample of 80 sites, 23 of which were blocked and about 14 more gave no ad activity, measured from a US data-centre IP with a headless browser. It is not market data. Mediahuis Belgium, Rossel, Roularta and Mediafin, which together are a large part of the Belgian market (M), are absent.

## 6. Recommended reference sites for a scan from Belgium

| Ad server | Sites | What to look for |
|---|---|---|
| Adhese as bidder | dhnet.be, lalibre.be (account `ipm`), limburger.nl | `POST ads-ipm.adhese.com/json`; whether a Belgian IP gets non-empty arrays is the thing to find out |
| Adhese as primary (unverified, from memory) | nieuwsblad.be, standaard.be, gva.be, hbvl.be | Whether any `adhese` host, a first-party `ads.<site>.be/json/…` call, or an `adhese`/`Adhese` global exists at all. This is the main open question of this report |
| Adform publisher tag | tv3.lt, delfi.lt | `adx.adform.net/adx/?rp=3&…` with base64url parameter names; delfi.lt has many placements |
| AdSense direct | synoniemen.net (pure), golem.de (beside GAM) | `pagead/ads` per unit, `data-ad-status` on the `ins` elements |
| Yieldlab | n-tv.de or stern.de (direct `/yp`, JS answer), handelsblatt.com or spiegel.de (`/yp?json=true`), wetter.com (`/ortb`) | A non-empty answer in each of the three forms |
| Adition | No site found. Do not plan work on this before a customer page shows `srp.js` | – |
| Xandr AST | hln.be (BE, member 9998), nu.nl or ad.nl (NL, member 7463), welt.de (same wrapper as bild.de), iltalehti.fi (numeric tag ids) | A real UT answer; this session only got `{"error":"unknown"}` |
| Equativ | rtbf.be (`/genericpost`, Equativ only), dhnet.be (video `/ac?out=vast4`) | – |
| GAM + Prebid, German wrappers | t-online.de (Ströer), focus.de (Burda), spiegel.de (iq digital), n-tv.de (Ad Alliance) | Wrapper-specific targeting keys |
| Video | seoghoer.dk (GAM VMAP/VAST), ad.nl (smartclip), dhnet.be (GAM VAST + Equativ VAST) | – |
| Opt Out Advertising | nrc.nl, any DPG site | `…/prebid/display` and the `ootag` library |

## 7. Limits and what could not be verified

- **US exit IP.** Consent platforms behaved differently from what a Belgian visitor sees: DPG's gate showed no dialog and returned a TCF string by itself; Didomi sites showed no dialog at all (`shouldConsentBeCollected()` false) and then loaded no ads; several sites reported `gdprApplies: false`. Where no dialog existed, consent was given through the platform's own API (`Didomi.setUserAgreeToAll()`, `OneTrust.AllowAll()`, `Cookiebot.submitCustomConsent`). That worked on dhnet.be, lalibre.be and buienradar.nl and had no effect on rtl.be, jv.dk, postimees.ee, limburger.nl and delfi.lt. Those five are inconclusive, not negative.
- **Headless browser.** The user agent was left as `HeadlessChrome/153.0.8010.12`. 23 sites answered 403 or 405, most with a Cloudflare page. Whether the block is on the IP range or on the user agent was not tested. All of Mediahuis (BE, NL, IE, LU), Rossel, Roularta, Mediafin, zeit.de, kicker.de, mobile.de and bol.com are missing for this reason.
- **Xandr answered `{"error":"unknown"}` to every `/ut/v3` call** on all 10 AST sites, and to most `/ut/v3/prebid` calls. So no Xandr ad was delivered in this session, and the AST sites count as "ad server identified, ads not loaded". The plan notes the same answer for a request without a `device.useragent`; here the browser did send one, so the cause is more likely the headless user agent or the IP. Not investigated.
- **No fill was captured for any phase-2 ad server.** Adhese `[]`, Adform `" "`, AdSense unfilled, Yieldlab empty. Every fill shape in section 2 comes from documentation or library source and is marked (D).
- **One page load per visit, 30 to 40 seconds, homepage only.** Lazy slots far down the page, article pages and video players that need a click were mostly not exercised. No article page was loaded for any site.
- **Adhese as primary ad server and the whole Adition ad server remain unverified.**
- faz.net hung the probe twice and goplay.be crashed it once; no data for faz.net.
- Response bodies of iframe documents (golem.de AdSense) and of some 204 and redirect answers could not be read by the browser API.
- Sample size went to 80 sites and 89 loads, above the requested 30 to 45, to chase the three ad servers that the first 44 sites did not show. Extra direct requests: 5 to the Adhese public demo account, 1 to `ads-ipm.adhese.com`, 4 documentation pages on `documentation.adhese.eu`.
- Market statements marked (M) are from memory and were not checked against any source.

## 8. Files

| Path | Content |
|---|---|
| `scripts/probe.js` | The probe: pattern table, consent handling, globals, capture rules |
| `scripts/sites1.txt`, `sites2.txt`, `sites3.txt`, `run.sh`, `run1.log`, `run2.log`, `run3.log` | Site lists and one-line results per load |
| `scripts/digest.py`, `table.py`, `evidence.py`, `urls.py`, `show.py` | Read-only analysis of the captures |
| `captures/<site>/requests.jsonl` | Every request: time, method, type, URL, frame, status, label |
| `captures/<site>/summary.json` | Navigation result, consent actions, TCF state, globals, GPT slots, Prebid versions, bidders and bid params (bid params only in the third batch), host counts |
| `captures/<site>/ad/NNN-<label>.json` | Full request (cookie and authorization headers removed) and response body for ad-server matches |
| `captures/<site>/final.jpg` | Screenshot at the end of the load |
| `captures/<site>-retry/` | Second load of the same site |
