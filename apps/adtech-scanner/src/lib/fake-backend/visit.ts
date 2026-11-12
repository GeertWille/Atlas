import type {
  AdSlot,
  Auction,
  DeviceProfile,
  PageMetrics,
  PageReach,
  PageView,
  ProxyUse,
  ScanEnvironment,
  ScanMetrics,
  ScriptRecord,
  VisitorBehavior,
} from "@/bindings";
import { createRng, hashString } from "./rng";
import { type CapturedSite, captureSite, computeMetrics } from "./site";

/** How a scan visits its site, with every default already resolved. */
export type VisitPlan = {
  url: string;
  device: DeviceProfile;
  visitor: VisitorBehavior;
  /** Page views of the visit, the entry page included. */
  pageCount: number;
  observeSeconds: number;
  seed: number;
  headless: boolean;
  persistentProfile: boolean;
  proxy: ProxyUse;
};

/** What went wrong on a later page view, for the rules that only a multi-page visit can trigger. */
type PageStory = "normal" | "noBidsAfterNavigation" | "slotsNotReset" | "consentAskedAgain";

export type VisitedPage = { view: PageView; site: CapturedSite; story: PageStory };

export type CapturedVisit = {
  plan: VisitPlan;
  /** The entry page, as a single-page scan would have captured it. */
  entry: CapturedSite;
  pages: VisitedPage[];
  auctions: Auction[];
  slots: AdSlot[];
  environment: ScanEnvironment;
  /** The slot the wrapper refreshed without a new auction, on the entry page of a visit that interacted. */
  staleRefresh: AdSlot | null;
};

/** A browse visit splits five minutes over its pages, like the backend. */
const VISIT_BUDGET_MS = 300_000;
const NAVIGATION_GAP_MS = 900;

/** Where the simulated visitor goes after the entry page, in order. */
const LATER_PAGES: { path: string; title: string; reach: "linkClick" | "routeChange" }[] = [
  { path: "/2026/11/council-approves-harbour-plan", title: "Council approves harbour plan", reach: "linkClick" },
  { path: "/2026/11/weekend-weather-outlook", title: "Weekend weather outlook", reach: "routeChange" },
  { path: "/sport/cup-final-preview", title: "Cup final preview", reach: "linkClick" },
  { path: "/business/markets-open-higher", title: "Markets open higher", reach: "linkClick" },
  { path: "/culture/festival-line-up-announced", title: "Festival line-up announced", reach: "routeChange" },
];

export const MAX_LATER_PAGES = LATER_PAGES.length;

function pageMetrics(site: CapturedSite): PageMetrics {
  const bids = site.auctions.flatMap((auction) => auction.adUnits.flatMap((unit) => unit.bids));
  const requested = site.slots.filter((slot) => slot.requestedAtMs !== null);
  const rendered = site.slots.flatMap((slot) => (slot.renderedAtMs === null ? [] : [slot.renderedAtMs]));
  return {
    auctionCount: site.auctions.length,
    bidRequestCount: bids.length,
    bidResponseCount: bids.filter((bid) => ["bid", "won", "rendered"].includes(bid.status)).length,
    slotCount: site.slots.length,
    emptySlotCount: site.slots.filter((slot) => slot.outcome === "empty").length,
    fillRate: requested.length ? site.slots.filter((slot) => slot.outcome === "filled").length / requested.length : 0,
    timeToFirstAuctionMs: site.auctions[0]?.startedAtMs ?? null,
    timeToFirstAdMs: rendered.length ? Math.min(...rendered) : null,
  };
}

/**
 * The wrapper's "double-fire guard" remembers the last auction across navigations, so the next document
 * requests its slots without bidding: no auction, no `hb_*` keys, and only the house line item fills.
 */
function withoutBids(site: CapturedSite): CapturedSite {
  const slots = site.slots.map((slot, index): AdSlot => {
    const requestedAtMs = (slot.definedAtMs ?? 400) + 180 + index * 12;
    const filled = index === 0;
    return {
      ...slot,
      outcome: filled ? "filled" : "empty",
      requestedAtMs,
      responseAtMs: requestedAtMs + 160,
      renderedAtMs: filled ? requestedAtMs + 260 : null,
      viewableAtMs: filled ? requestedAtMs + 1400 : null,
      renderedSize: filled ? (slot.sizes[0] ?? null) : null,
      hadPrebidTargeting: false,
      requestedBeforeAuctionEnd: false,
      requests: [{ atMs: requestedAtMs, hadPrebidTargeting: false, prebidAdId: null, requestId: null }],
    };
  });
  return { ...site, auctions: [], slots };
}

/** After a route change the wrapper auctions again, but GPT rejects the re-defined slots: nothing is requested. */
function withoutSlotReset(site: CapturedSite): CapturedSite {
  return { ...site, auctions: site.auctions.slice(0, 1), slots: [] };
}

/** The banner comes back on the next page and the wrapper waits for it before bidding. */
function withConsentDelay(site: CapturedSite): CapturedSite {
  const delayMs = 2600;
  const shift = (value: number | null) => (value === null ? null : value + delayMs);
  return {
    ...site,
    auctions: site.auctions.map((auction) => ({
      ...auction,
      startedAtMs: auction.startedAtMs + delayMs,
      endedAtMs: shift(auction.endedAtMs),
      adUnits: auction.adUnits.map((unit) => ({
        ...unit,
        bids: unit.bids.map((bid) => ({ ...bid, requestedAtMs: shift(bid.requestedAtMs) })),
      })),
    })),
    slots: site.slots.map((slot) => ({
      ...slot,
      requestedAtMs: shift(slot.requestedAtMs),
      responseAtMs: shift(slot.responseAtMs),
      renderedAtMs: shift(slot.renderedAtMs),
      viewableAtMs: shift(slot.viewableAtMs),
      requests: slot.requests.map((request) => ({ ...request, atMs: request.atMs + delayMs })),
    })),
  };
}

function storyFor(entry: CapturedSite, reach: PageReach): PageStory {
  if (entry.archetype === "leak") return reach === "routeChange" ? "slotsNotReset" : "noBidsAfterNavigation";
  if (entry.archetype === "slow" && reach !== "routeChange") return "consentAskedAgain";
  return "normal";
}

const TELL_STORY: Record<PageStory, (site: CapturedSite) => CapturedSite> = {
  normal: (site) => site,
  noBidsAfterNavigation: withoutBids,
  slotsNotReset: withoutSlotReset,
  consentAskedAgain: withConsentDelay,
};

/** The engagement refresh of the reference wrapper: the same `hb_adid` is sent again five seconds on. */
function refreshWithStaleBid(site: CapturedSite): AdSlot | null {
  const slot = site.slots.find((candidate) => candidate.viewableAtMs !== null && candidate.requests[0]?.prebidAdId);
  const first = slot?.requests[0];
  if (!slot || !first || slot.viewableAtMs === null) return null;
  slot.requests.push({
    atMs: slot.viewableAtMs + 5200,
    hadPrebidTargeting: true,
    prebidAdId: first.prebidAdId,
    requestId: null,
  });
  return slot;
}

function environmentOf(plan: VisitPlan, entry: CapturedSite): ScanEnvironment {
  const mobile = plan.device === "mobile";
  const realWindow = !plan.headless && !mobile && (plan.visitor === "browse" || plan.visitor === "manual");
  return {
    visitor: plan.visitor,
    seed: plan.seed,
    headless: plan.headless,
    persistentProfile: plan.persistentProfile,
    // A scripted visit never hides that it is automated; in a manual session a person drives the browser.
    webdriver: plan.visitor !== "manual",
    userAgentOverridden: mobile,
    viewportOverridden: !realWindow,
    emulatedMobile: mobile,
    detectors:
      entry.archetype === "slow"
        ? [{ vendor: "DoubleVerify", kind: "invalidTrafficMeasurement", host: "cdn.doubleverify.com" }]
        : [],
    proxy: plan.proxy,
  };
}

export function captureVisit(plan: VisitPlan): CapturedVisit {
  const { hostname, origin } = new URL(plan.url);
  const entry = captureSite(hostname, plan.device);
  const rng = createRng(hashString(`${hostname}|${plan.device}|visit|${plan.seed}`));
  const interacts = plan.visitor === "browse" || plan.visitor === "manual";
  const dwellMs = Math.min(plan.observeSeconds * 1000, Math.round(VISIT_BUDGET_MS / plan.pageCount));

  const pages: VisitedPage[] = [];
  let startedAtMs = 0;
  let auctionNumber = 0;
  for (let index = 0; index < plan.pageCount; index += 1) {
    const later = LATER_PAGES[index - 1];
    let reachedBy: PageReach = "entry";
    if (later) reachedBy = plan.visitor === "manual" && later.reach === "linkClick" ? "userNavigation" : later.reach;
    const story = later ? storyFor(entry, reachedBy) : "normal";
    const captured = later ? TELL_STORY[story](captureSite(hostname, plan.device, `|page${index}`)) : entry;
    const site: CapturedSite = {
      ...captured,
      auctions: captured.auctions.map((auction) => {
        auctionNumber += 1;
        return { ...auction, id: `a${auctionNumber}`, page: index };
      }),
      slots: captured.slots.map((slot) => ({ ...slot, page: index })),
    };
    const url = later ? `${origin}${later.path}` : plan.url;
    const loadMs = reachedBy === "routeChange" ? null : site.pageLoadMs;
    pages.push({
      story,
      site,
      view: {
        index,
        url,
        title: later ? later.title : `${hostname.replace(/^www\./, "")}: latest news`,
        startedAtMs,
        dwellMs,
        reachedBy,
        // A scripted visitor reaches every later page through a link, in-page navigations included.
        link:
          later && plan.visitor === "browse"
            ? {
                text: later.title,
                href: url,
                reason: `content link in <article>, article-like path; best of ${rng.int(3, 5)}`,
              }
            : null,
        loadMs,
        maxScrollDepth: plan.visitor === "none" ? 0 : Math.round(rng.between(0.55, 1) * 100) / 100,
        metrics: pageMetrics(site),
      },
    });
    startedAtMs += (loadMs ?? 0) + dwellMs + NAVIGATION_GAP_MS;
  }

  const entrySite = pages[0]?.site ?? entry;
  return {
    plan,
    entry: entrySite,
    pages,
    auctions: pages.flatMap((page) => page.site.auctions),
    slots: pages.flatMap((page) => page.site.slots),
    environment: environmentOf(plan, entry),
    staleRefresh: interacts && entry.archetype === "leak" ? refreshWithStaleBid(entrySite) : null,
  };
}

/** Counts are sums over the page views; the load and first-ad timings are the entry page's. */
export function visitMetrics(visit: CapturedVisit, scripts: ScriptRecord[]): ScanMetrics {
  const whole = computeMetrics({ ...visit.entry, auctions: visit.auctions, slots: visit.slots }, scripts);
  const entry = computeMetrics(visit.entry, scripts);
  return { ...whole, timeToFirstAuctionMs: entry.timeToFirstAuctionMs, timeToFirstAdMs: entry.timeToFirstAdMs };
}
