import type { Finding, ScanMetrics } from "@/bindings";
import type { CapturedSite } from "./site";

/** One model-driven investigation: what it thought, which tools it ran, what it reported. */
export type Investigation = {
  thinking: string;
  tools: { name: string; input: Record<string, unknown>; result: string }[];
  finding: FindingDraft;
};

/** A finding before the scan gives it an id. `pages` is left out for findings of the entry page. */
export type FindingDraft = Omit<Finding, "id" | "scanId" | "pages"> & { pages?: number[] };

const WRAPPER_DEVICE_CHECK = `var MOBILE_LAZY_OFFSET = 2400;

// AD-2291: include tablets in the mobile experience
var IS_MOBILE = window.screen.width < 1440;

function isBelowFold(unit) {
  return unit.lazy === true;
}`;

const WRAPPER_SCHEDULE = `function scheduleAuctions(units) {
  var eager = units.filter(function (u) { return !isBelowFold(u); });
  var lazy = units.filter(isBelowFold);
  requestBids(eager);
  if (!IS_MOBILE) {
    requestBids(lazy);
    return;
  }
  // Mobile: hold below-the-fold units until the reader is two screens in.
  window.addEventListener('scroll', function onScroll() {
    if (window.pageYOffset < MOBILE_LAZY_OFFSET) return;
    window.removeEventListener('scroll', onScroll);
    requestBids(lazy);
  }, { passive: true });
}`;

const WRAPPER_FAILSAFE = `function requestBids(units) {
  var sent = false;
  function sendAdServerRequest() {
    if (sent) return;
    sent = true;
    pbjs.setTargetingForGPTAsync(codes(units));
    googletag.pubads().refresh(slotsFor(units));
  }
  pbjs.requestBids({
    adUnitCodes: codes(units),
    timeout: 1500,
    bidsBackHandler: sendAdServerRequest
  });
  setTimeout(sendAdServerRequest, FAILSAFE_TIMEOUT); // 800
}`;

const CONFIG_FLOORS = `pbjs.setConfig({
  priceGranularity: 'dense',
  floors: {
    enforcement: { enforceJS: true },
    data: {
      currency: 'USD',
      schema: { fields: ['mediaType'] },
      values: { banner: 0.01, video: 0.01 }
    }
  }
});`;

const STICKY_REFRESH = `setInterval(function () {
  if (document.hidden) return;
  // refresh the anchor every 30s
  googletag.pubads().refresh([stickySlot]);
}, 30000);`;

const DUPLICATE_PREBID = `(function () {
  var s = document.createElement('script');
  s.src = 'https://cdn.adstack-partner.example/pb/prebid.8.14.0.js';
  s.onload = function () {
    _pbjsPartner.que.push(function () {
      _pbjsPartner.addAdUnits(window.__hbUnits.slice(0, 2));
      _pbjsPartner.requestBids({ timeout: 3000 });
    });
  };
  document.head.appendChild(s);
})();`;

const TIMEOUT_CONFIG = `pbjs.setConfig({
  bidderTimeout: 3000,
  timeoutBuffer: 400,
  useBidCache: false,
  enableSendAllBids: true
});`;

function wrapperUrl(site: CapturedSite) {
  return `https://${site.hostname}/assets/ads/hb-wrapper.min.js`;
}

function configUrl(site: CapturedSite) {
  return `https://${site.hostname}/assets/ads/ad-config.js`;
}

function slotIds(site: CapturedSite, predicate: (outcome: string) => boolean) {
  return site.slots.filter((slot) => predicate(slot.outcome)).map((slot) => slot.elementId);
}

function biddersWithStatus(site: CapturedSite, status: string) {
  const counts = new Map<string, number>();
  for (const auction of site.auctions) {
    for (const unit of auction.adUnits) {
      for (const bid of unit.bids) {
        if (bid.status === status) counts.set(bid.bidder, (counts.get(bid.bidder) ?? 0) + 1);
      }
    }
  }
  return [...counts.entries()].sort((a, b) => b[1] - a[1]);
}

const pct = (ratio: number) => `${Math.round(ratio * 100)}%`;

export function heuristicFindings(site: CapturedSite, metrics: ScanMetrics): FindingDraft[] {
  const drafts: FindingDraft[] = [];
  const neverRequested = slotIds(site, (outcome) => outcome === "neverRequested");
  const timeouts = biddersWithStatus(site, "timeout");
  const early = site.slots.filter((slot) => slot.requestedBeforeAuctionEnd);
  const empty = slotIds(site, (outcome) => outcome === "empty");

  if (neverRequested.length > 0) {
    drafts.push({
      severity: "high",
      category: "fill",
      source: "heuristic",
      title: `${neverRequested.length} of ${site.slots.length} ad slots were defined but never requested`,
      description: `GPT defined these slots, but no ad request left the page during the observation window, even after the visit scrolled past their containers. A slot that is never requested earns **nothing**: no header bids, no direct campaigns, no backfill.`,
      recommendation:
        "Check the condition that gates `googletag.pubads().refresh()` / `display()` for these units. If they are lazy-loaded, verify the trigger actually fires on this device class.",
      confidence: 0.97,
      impact: `${neverRequested.length} slots earn nothing on ${site.device}`,
      estimatedRevenueImpactPct: Math.round((neverRequested.length / site.slots.length) * 68),
      evidence: [
        {
          kind: "metric",
          label: "Slot outcomes",
          body: `${site.slots.length} defined · ${site.slots.length - neverRequested.length} requested · ${neverRequested.length} never requested`,
          scriptUrl: null,
          line: null,
        },
        ...neverRequested.slice(0, 2).map((id) => {
          const slot = site.slots.find((candidate) => candidate.elementId === id);
          return {
            kind: "timeline" as const,
            label: id,
            body: `slotDefined at ${slot?.definedAtMs} ms → no slotRequested event in the observation window`,
            scriptUrl: null,
            line: null,
          };
        }),
      ],
      affectedSlots: neverRequested,
      affectedBidders: [],
    });
  }

  if (metrics.timeoutRate >= 0.08 && timeouts.length > 0) {
    const worst = timeouts.slice(0, 3);
    const severe = metrics.timeoutRate >= 0.2;
    drafts.push({
      severity: severe ? "high" : "medium",
      category: "timing",
      source: "heuristic",
      title: `${pct(metrics.timeoutRate)} of bid requests timed out`,
      description: `${metrics.bidTimeoutCount} of ${metrics.bidRequestCount} bid requests did not answer within the ${site.timeoutMs} ms auction timeout. Timed-out bidders still cost page weight and delay the ad server call, but can never win.`,
      recommendation: `Review ${worst.map(([bidder]) => `\`${bidder}\``).join(", ")}: check endpoint region, user-sync status and whether the adapter is still maintained. Remove or throttle bidders that time out in most auctions.`,
      confidence: 0.92,
      impact: severe ? "Auctions wait the full timeout and lose late bids" : "Late bidders never compete",
      estimatedRevenueImpactPct: severe ? 9 : 3,
      evidence: worst.map(([bidder, count]) => ({
        kind: "metric" as const,
        label: bidder,
        body: `${count} timeouts across ${site.auctions.length} auctions (timeout ${site.timeoutMs} ms)`,
        scriptUrl: null,
        line: null,
      })),
      affectedSlots: [],
      affectedBidders: worst.map(([bidder]) => bidder),
    });
  }

  if (early.length > 0) {
    drafts.push({
      severity: "medium",
      category: "timing",
      source: "heuristic",
      title: "Ad server request left before the Prebid auction ended",
      description:
        "The GPT request for these slots was sent while bidders were still answering, so it carried **no `hb_*` targeting**. Header-bidding demand cannot compete for that impression.",
      recommendation:
        "Call the ad server from `bidsBackHandler` and keep any failsafe timer above the auction timeout plus `timeoutBuffer`.",
      confidence: 0.9,
      impact: "Header bids excluded from the first impression",
      estimatedRevenueImpactPct: 6,
      evidence: early.map((slot) => ({
        kind: "timeline" as const,
        label: slot.elementId,
        body: `GPT request at ${slot.requestedAtMs} ms · auction ended at ${site.auctions[0]?.endedAtMs} ms · hb_* targeting: none`,
        scriptUrl: null,
        line: null,
      })),
      affectedSlots: early.map((slot) => slot.elementId),
      affectedBidders: [],
    });
  }

  if (empty.length > 0) {
    drafts.push({
      severity: "low",
      category: "fill",
      source: "heuristic",
      title: `${empty.length} requested slot${empty.length > 1 ? "s" : ""} came back empty`,
      description:
        "The ad server answered with no creative. With header bids present this usually means line items do not match the `hb_pb` price buckets, or the slot has no backfill.",
      recommendation:
        "Confirm price-priority line items cover the bid range for these sizes and that AdX/AdSense backfill is enabled.",
      confidence: 0.74,
      impact: "Unfilled impressions",
      estimatedRevenueImpactPct: 2,
      evidence: [
        {
          kind: "metric",
          label: "Fill rate",
          body: `${pct(metrics.fillRate)} of requested slots rendered a creative`,
          scriptUrl: null,
          line: null,
        },
      ],
      affectedSlots: empty,
      affectedBidders: [],
    });
  }

  if (site.prebidVersion.startsWith("8.")) {
    drafts.push({
      severity: "low",
      category: "hygiene",
      source: "heuristic",
      title: `Prebid.js ${site.prebidVersion} is a major version behind`,
      description: `The page runs Prebid.js **${site.prebidVersion}**. The 9.x line removed legacy code paths, shrank the core and updated most bidder adapters; several SSPs no longer test against 8.x.`,
      recommendation: "Plan an upgrade to the current 9.x release and rebuild with only the adapters in use.",
      confidence: 0.99,
      impact: "Adapter drift and extra script weight",
      estimatedRevenueImpactPct: null,
      evidence: [
        { kind: "log", label: "Tracker", body: `pbjs.version = "v${site.prebidVersion}"`, scriptUrl: null, line: null },
      ],
      affectedSlots: [],
      affectedBidders: [],
    });
  }

  drafts.push({
    severity: "info",
    category: "consent",
    source: "heuristic",
    title: "Consent signal reached every bidder",
    description:
      "A TCF v2.2 consent string was available before the first auction and all bid requests carried `gdprConsent`. Consent is not blocking demand on this page.",
    recommendation: "No action needed.",
    confidence: 0.95,
    impact: "No revenue impact",
    estimatedRevenueImpactPct: null,
    evidence: [
      {
        kind: "log",
        label: "CMP",
        body: `__tcfapi tcloaded at ${Math.round(site.pageLoadMs * 0.22)} ms · cmpId 5 (OneTrust) · purposes 1-10 consented`,
        scriptUrl: null,
        line: null,
      },
    ],
    affectedSlots: [],
    affectedBidders: [],
  });

  return drafts;
}

function leakDesktop(site: CapturedSite): Investigation[] {
  const starved = slotIds(site, (outcome) => outcome === "neverRequested");
  const wrapper = wrapperUrl(site);
  return [
    {
      thinking: `Telemetry shows ${starved.length} slots defined and never requested on a desktop viewport, while the eager slots auctioned normally. That pattern points at a gate in the wrapper, not at GPT or the bidders. I should find where the wrapper decides which units to auction immediately.`,
      tools: [
        {
          name: "search_scripts",
          input: { pattern: "requestBids\\(", scope: "wrapper" },
          result: `hb-wrapper.min.js:236  requestBids(eager);\nhb-wrapper.min.js:238    requestBids(lazy);\nhb-wrapper.min.js:245    requestBids(lazy);\nhb-wrapper.min.js:318  function requestBids(units) {`,
        },
        {
          name: "read_script",
          input: { url: wrapper, startLine: 205, endLine: 250 },
          result: `${WRAPPER_DEVICE_CHECK}\n\n…\n\n${WRAPPER_SCHEDULE}`,
        },
        {
          name: "get_page_context",
          input: { fields: ["screen.width", "innerWidth", "scrollHeight", "maxScrollY"] },
          result: `screen.width: 1280\ninnerWidth: 1280\ndocument.scrollHeight: 2960\nmax scrollY reached while scrolling: 2060`,
        },
      ],
      finding: {
        severity: "critical",
        category: "logicBug",
        source: "ai",
        title: "Desktop screens under 1440 px are treated as mobile, so below-the-fold auctions never start",
        description: `The wrapper decides the device class with \`window.screen.width < 1440\`. That is true for most laptops (1280 and 1366 px wide), so desktop visitors take the **mobile lazy-load path**: every unit flagged \`lazy\` waits for a scroll listener instead of being auctioned on load.\n\nThe listener only starts the auction once \`pageYOffset\` passes a fixed **2,400 px**. On this page the desktop layout is 2,960 px tall with a 900 px viewport, so the maximum scroll offset is 2,060 px: the threshold is **unreachable**. The sidebar and footer units are never sent to Prebid or GPT and their containers stay empty for the whole visit.`,
        recommendation: `1. Detect layout, not screen size: \`window.matchMedia('(max-width: 767px)').matches\` matches the site's own CSS breakpoint.\n2. Replace the fixed scroll offset with an \`IntersectionObserver\` on each lazy container (\`rootMargin: '600px 0px'\`), so a unit is auctioned shortly before it enters view on any device.\n3. Do not flag units that sit in the initial desktop viewport (\`${starved[0] ?? "div-gpt-sidebar-1"}\`) as lazy.`,
        confidence: 0.94,
        impact: `${starved.length} of ${site.slots.length} desktop slots never bid or render`,
        estimatedRevenueImpactPct: 34,
        evidence: [
          {
            kind: "code",
            label: "Device class is derived from screen width",
            body: WRAPPER_DEVICE_CHECK,
            scriptUrl: wrapper,
            line: 208,
          },
          {
            kind: "code",
            label: "Lazy units wait for a 2,400 px scroll offset",
            body: WRAPPER_SCHEDULE,
            scriptUrl: wrapper,
            line: 231,
          },
          {
            kind: "metric",
            label: "Page geometry during capture",
            body: "screen.width 1280 → IS_MOBILE = true · max scrollY 2,060 px < MOBILE_LAZY_OFFSET 2,400 px",
            scriptUrl: null,
            line: null,
          },
          {
            kind: "timeline",
            label: starved[0] ?? "div-gpt-sidebar-1",
            body: "slotDefined 412 ms → in viewport at load → no auctionInit, no slotRequested in 20 s",
            scriptUrl: null,
            line: null,
          },
        ],
        affectedSlots: starved,
        affectedBidders: [],
      },
    },
    {
      thinking:
        "The leaderboard was requested 800 ms after auctionInit while the auction ran until the 1,500 ms timeout. There must be a failsafe timer shorter than the bidder timeout. Let me read requestBids().",
      tools: [
        { name: "read_script", input: { url: wrapper, startLine: 316, endLine: 334 }, result: WRAPPER_FAILSAFE },
        {
          name: "get_slot_timeline",
          input: { slot: "div-gpt-top-leaderboard" },
          result: `auctionInit a1            ${site.auctions[0]?.startedAtMs} ms\nslotRequested             ${(site.auctions[0]?.startedAtMs ?? 0) + 800} ms   targeting: {} (no hb_*)\nauctionEnd a1             ${site.auctions[0]?.endedAtMs} ms\nbidWon                    none`,
        },
      ],
      finding: {
        severity: "high",
        category: "configuration",
        source: "ai",
        title: "800 ms failsafe fires the ad server before the 1,500 ms auction can finish",
        description:
          "`requestBids()` arms `setTimeout(sendAdServerRequest, FAILSAFE_TIMEOUT)` with **800 ms**, but asks Prebid for a **1,500 ms** auction. Whenever a single bidder is slower than 800 ms, which happened in every captured auction because `ix` timed out, the failsafe wins the race: GPT is called before `bidsBackHandler`, with no `hb_*` key-values.\n\nEvery bid that arrived in time is discarded for that impression, and the highest-value above-the-fold slot is sold without header-bidding competition.",
        recommendation:
          "Set the failsafe to `timeout + timeoutBuffer + 200` (about 2,100 ms here), or lower `bidderTimeout` to a value the failsafe respects. Keep one source of truth for both numbers.",
        confidence: 0.91,
        impact: "Top leaderboard is sold without header bids",
        estimatedRevenueImpactPct: 11,
        evidence: [
          {
            kind: "code",
            label: "Failsafe is shorter than the auction timeout",
            body: WRAPPER_FAILSAFE,
            scriptUrl: wrapper,
            line: 318,
          },
          {
            kind: "timeline",
            label: "div-gpt-top-leaderboard",
            body: `GPT request at +800 ms with empty targeting · auction ended at +${site.auctions[0]?.durationMs} ms`,
            scriptUrl: null,
            line: null,
          },
        ],
        affectedSlots: ["div-gpt-top-leaderboard"],
        affectedBidders: ["ix"],
      },
    },
    {
      thinking:
        "While reading the config I noticed the floors module is enabled with enforcement but the values look like placeholders. Worth confirming against the bid data.",
      tools: [
        { name: "read_script", input: { url: configUrl(site), startLine: 40, endLine: 54 }, result: CONFIG_FLOORS },
        {
          name: "query_bids",
          input: { where: "cpm < 0.10", groupBy: "bidder" },
          result: "0 rows. Lowest received CPM: 0.27 (criteo, div-gpt-incontent-2)",
        },
      ],
      finding: {
        severity: "low",
        category: "pricing",
        source: "ai",
        title: "Price floors are enforced at $0.01, which filters nothing",
        description:
          "The Price Floors module is loaded and enforcing, but the only rule is a flat **$0.01** for every media type. No received bid was below $0.27, so the module adds weight and a `floorData` payload to every request without protecting yield.",
        recommendation:
          "Either remove the floors module from the build, or supply real floors per `adUnitCode` and `size` (a dynamic floor provider or values derived from GAM unified pricing rules).",
        confidence: 0.83,
        impact: "No yield protection; minor script overhead",
        estimatedRevenueImpactPct: 1,
        evidence: [
          { kind: "code", label: "Flat one-cent floor", body: CONFIG_FLOORS, scriptUrl: configUrl(site), line: 41 },
        ],
        affectedSlots: [],
        affectedBidders: [],
      },
    },
  ];
}

function leakMobile(site: CapturedSite): Investigation[] {
  const wrapper = wrapperUrl(site);
  return [
    {
      thinking:
        "On mobile the lazy path is the intended one and the second auction did start after scrolling. Still, the trigger is a fixed 2,400 px offset. I should check how that relates to where the lazy containers actually are.",
      tools: [
        {
          name: "read_script",
          input: { url: wrapper, startLine: 205, endLine: 250 },
          result: `${WRAPPER_DEVICE_CHECK}\n\n…\n\n${WRAPPER_SCHEDULE}`,
        },
        {
          name: "get_slot_timeline",
          input: { slot: "div-gpt-incontent-2" },
          result: `slotDefined               438 ms\nentered viewport          6,120 ms (scrollY 1,480)\nauctionInit a2            ${site.auctions[1]?.startedAtMs} ms (scrollY 2,400)\nslotRenderEnded           ${(site.auctions[1]?.endedAtMs ?? 0) + 380} ms (container above viewport)`,
        },
      ],
      finding: {
        severity: "high",
        category: "lazyLoad",
        source: "ai",
        title: "Lazy auction starts 2,400 px down the page, after the first lazy slot has scrolled out of view",
        description:
          "The mobile lazy-load trigger is a fixed scroll offset (`MOBILE_LAZY_OFFSET = 2400`) rather than each container's position. `div-gpt-incontent-2` sits around 1,500 px: it entered the viewport **about 3 s before** its auction even started and had left it again when the creative rendered.\n\nThe impression is served but is not viewable, which depresses viewability-optimised demand and CPMs for that unit.",
        recommendation:
          "Observe each lazy container with `IntersectionObserver` and start its auction when it is roughly one viewport away (`rootMargin: '800px 0px'`). Auction units individually instead of all lazy units together.",
        confidence: 0.88,
        impact: "In-content slot renders out of view on mobile",
        estimatedRevenueImpactPct: 8,
        evidence: [
          {
            kind: "code",
            label: "Single fixed offset for all lazy units",
            body: WRAPPER_SCHEDULE,
            scriptUrl: wrapper,
            line: 231,
          },
          {
            kind: "timeline",
            label: "div-gpt-incontent-2",
            body: "in view at 6,120 ms · auction started 3 s later · rendered above the viewport",
            scriptUrl: null,
            line: null,
          },
        ],
        affectedSlots: ["div-gpt-incontent-2", "div-gpt-incontent-3"],
        affectedBidders: [],
      },
    },
    {
      thinking:
        "The sticky footer keeps rendering new creatives without any new auctionInit. Let me look for a refresh timer.",
      tools: [
        {
          name: "search_scripts",
          input: { pattern: "setInterval|refresh\\(", scope: "wrapper" },
          result:
            "hb-wrapper.min.js:402  setInterval(function () {\nhb-wrapper.min.js:405    googletag.pubads().refresh([stickySlot]);",
        },
        { name: "read_script", input: { url: wrapper, startLine: 400, endLine: 408 }, result: STICKY_REFRESH },
      ],
      finding: {
        severity: "medium",
        category: "logicBug",
        source: "ai",
        title: "Sticky footer refreshes every 30 s without a new Prebid auction",
        description:
          "The anchor slot is refreshed with a bare `googletag.pubads().refresh()`. No `pbjs.requestBids()` runs first, so every refreshed impression reuses the **stale `hb_*` targeting** from the first auction. GAM either matches a bid that has already been rendered or expired, or falls through to backfill.",
        recommendation:
          "Route the timer through the wrapper's `requestBids([stickyUnit])` so each refresh gets a fresh auction, and clear targeting with `pbjs.setTargetingForGPTAsync` after `bidWon`.",
        confidence: 0.86,
        impact: "Refreshed anchor impressions sell without live bids",
        estimatedRevenueImpactPct: 5,
        evidence: [
          { kind: "code", label: "Refresh without auction", body: STICKY_REFRESH, scriptUrl: wrapper, line: 402 },
        ],
        affectedSlots: ["div-gpt-sticky-footer"],
        affectedBidders: [],
      },
    },
  ];
}

function slow(site: CapturedSite): Investigation[] {
  const wrapper = wrapperUrl(site);
  return [
    {
      thinking:
        "Two auctionInit events 40 ms apart for the same ad unit codes, from different Prebid globals. One of the scripts must be loading a second Prebid build.",
      tools: [
        {
          name: "search_scripts",
          input: { pattern: "prebid.*\\.js|_pbjs", scope: "all" },
          result:
            "hb-wrapper.min.js:512    s.src = 'https://cdn.adstack-partner.example/pb/prebid.8.14.0.js';\nhb-wrapper.min.js:515      _pbjsPartner.que.push(function () {",
        },
        { name: "read_script", input: { url: wrapper, startLine: 509, endLine: 522 }, result: DUPLICATE_PREBID },
      ],
      finding: {
        severity: "high",
        category: "configuration",
        source: "ai",
        title: "A second Prebid instance re-auctions the top two ad units",
        description:
          "A partner snippet inside the wrapper loads **Prebid 8.14.0** under the global `_pbjsPartner` and calls `requestBids` for the first two ad units, 40 ms after the main auction. Bidders receive two requests for the same impression; SSPs dedupe or throttle the duplicate, which lowers bid rates for both.\n\nOnly the main instance sets GPT targeting, so every bid in the partner auction is wasted.",
        recommendation: "Remove the partner Prebid build and add its bidders as adapters in the main Prebid instance.",
        confidence: 0.9,
        impact: "Duplicate bid requests lower bid rate; 290 KB of dead script",
        estimatedRevenueImpactPct: 7,
        evidence: [
          {
            kind: "code",
            label: "Partner snippet loads its own Prebid",
            body: DUPLICATE_PREBID,
            scriptUrl: wrapper,
            line: 509,
          },
          {
            kind: "metric",
            label: "Auctions",
            body: `a1 at ${site.auctions[0]?.startedAtMs} ms and a2 at ${site.auctions[1]?.startedAtMs} ms request the same ad unit codes`,
            scriptUrl: null,
            line: null,
          },
        ],
        affectedSlots: site.auctions[1]?.adUnits.map((unit) => unit.code) ?? [],
        affectedBidders: [],
      },
    },
    {
      thinking:
        "Median bid latency is far above normal and the timeout is 3,000 ms. I want to see how the timeout is configured and what the first ad render looks like.",
      tools: [
        { name: "read_script", input: { url: configUrl(site), startLine: 12, endLine: 20 }, result: TIMEOUT_CONFIG },
        {
          name: "query_bids",
          input: { select: "p50(responseTimeMs), p90(responseTimeMs)", where: "status != 'timeout'" },
          result: "p50 1,180 ms · p90 2,410 ms",
        },
      ],
      finding: {
        severity: "medium",
        category: "timing",
        source: "ai",
        title: "3,000 ms auction timeout delays the first ad by more than 3 s",
        description:
          "`bidderTimeout` is **3,000 ms** with a 400 ms buffer. Because two bidders time out in every auction, each auction runs to the full limit and the first creative only renders well after the reader has started scrolling. The above-the-fold impression is often not viewable by then.",
        recommendation:
          "Lower `bidderTimeout` to 1,200 to 1,500 ms and drop the bidders that never answer in time. Bids later than that rarely win enough to pay for the lost viewability.",
        confidence: 0.84,
        impact: "Late first render lowers viewability above the fold",
        estimatedRevenueImpactPct: 6,
        evidence: [
          { kind: "code", label: "Auction timeout", body: TIMEOUT_CONFIG, scriptUrl: configUrl(site), line: 12 },
        ],
        affectedSlots: [],
        affectedBidders: ["triplelift", "sovrn"],
      },
    },
  ];
}

function healthy(site: CapturedSite): Investigation[] {
  return [
    {
      thinking:
        "Telemetry looks clean: every slot was requested after its auction ended and rendered. I will still read the wrapper's scheduling and refresh logic for latent problems.",
      tools: [
        {
          name: "read_script",
          input: { url: wrapperUrl(site), startLine: 180, endLine: 240 },
          result:
            "function observeLazyUnits(units) {\n  var io = new IntersectionObserver(onIntersect, { rootMargin: '600px 0px' });\n  units.forEach(function (u) { io.observe(document.getElementById(u.code)); });\n}\n…",
        },
        {
          name: "read_script",
          input: { url: configUrl(site), startLine: 1, endLine: 30 },
          result:
            "pbjs.setConfig({\n  bidderTimeout: 1200,\n  enableSendAllBids: false,\n  targetingControls: { alwaysIncludeDeals: true },\n  userSync: { syncsPerBidder: 3, syncDelay: 3000 }\n});",
        },
      ],
      finding: {
        severity: "low",
        category: "configuration",
        source: "ai",
        title: "User syncs are capped at 3 per bidder with a 3 s delay",
        description:
          "The wrapper is well structured: lazy units use `IntersectionObserver`, GPT is called from `bidsBackHandler`, and the failsafe is above the auction timeout. The only tunable is `userSync`: with `syncDelay: 3000`, readers who bounce in the first seconds are never synced, which slowly lowers match rates.",
        recommendation: "Consider `syncDelay: 1000` and enabling `iframe` syncs for the top three bidders by revenue.",
        confidence: 0.71,
        impact: "Slightly lower match rates on short visits",
        estimatedRevenueImpactPct: 1,
        evidence: [
          {
            kind: "code",
            label: "User sync config",
            body: "userSync: { syncsPerBidder: 3, syncDelay: 3000 }",
            scriptUrl: configUrl(site),
            line: 5,
          },
        ],
        affectedSlots: [],
        affectedBidders: [],
      },
    },
  ];
}

export function aiInvestigations(site: CapturedSite): Investigation[] {
  if (site.archetype === "leak") return site.device === "desktop" ? leakDesktop(site) : leakMobile(site);
  if (site.archetype === "slow") return slow(site);
  return healthy(site);
}

const SEVERITY_PENALTY = { critical: 30, high: 14, medium: 6, low: 2, info: 0 } as const;

export function healthScore(findings: Pick<Finding, "severity">[]): number {
  const penalty = findings.reduce((sum, finding) => sum + SEVERITY_PENALTY[finding.severity], 0);
  return Math.max(4, 100 - penalty);
}

const OVERVIEW_MODE = {
  analysed: "",
  // A scan with only a TypeSafe key: the model classified scripts, nothing read their logic.
  triaged: " Scripts were classified by AI, but wrapper logic was not analysed: that needs an Anthropic API key.",
  deterministic: " Only deterministic checks ran for this scan, so wrapper logic was not analysed.",
} as const;

/** `ai` is how far the AI steps got: logic analysed, scripts only triaged, or nothing but the rules. */
export function overviewFor(site: CapturedSite, findings: Finding[], ai: keyof typeof OVERVIEW_MODE): string {
  const top = [...findings].sort((a, b) => SEVERITY_PENALTY[b.severity] - SEVERITY_PENALTY[a.severity])[0];
  const filled = site.slots.filter((slot) => slot.outcome === "filled").length;
  const base = `${filled} of ${site.slots.length} ad slots rendered a creative across ${site.auctions.length} Prebid auction${site.auctions.length === 1 ? "" : "s"} on ${site.device}.`;
  const lead =
    top && SEVERITY_PENALTY[top.severity] >= 6
      ? ` The most important issue: ${top.title.charAt(0).toLowerCase()}${top.title.slice(1)}.`
      : " No revenue-critical problems were found; the remaining items are tuning opportunities.";
  return `${base}${lead}${OVERVIEW_MODE[ai]}`;
}
