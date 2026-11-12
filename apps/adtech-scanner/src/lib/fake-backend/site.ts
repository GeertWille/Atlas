import type {
  AdSlot,
  Auction,
  AuctionAdUnit,
  Bid,
  DeviceProfile,
  ScanMetrics,
  ScriptCategory,
  ScriptRecord,
  SlotOutcome,
} from "@/bindings";
import { BIDDERS, PREBID_VERSIONS, SCRIPT_TEMPLATES, type ScriptTemplate } from "./catalog";
import { createRng, hashString, hexId, type Rng } from "./rng";

/** The three kinds of ad stack the fake backend knows how to simulate. */
export type Archetype = "leak" | "healthy" | "slow";

export type SlotPlan = {
  elementId: string;
  pathSuffix: string;
  sizes: string[];
  bidders: string[];
  /** Flagged `lazy: true` in the wrapper's ad-unit config. */
  lazy: boolean;
  inInitialViewport: boolean;
};

export type CapturedSite = {
  archetype: Archetype;
  hostname: string;
  device: DeviceProfile;
  prebidVersion: string;
  timeoutMs: number;
  pageLoadMs: number;
  slotPlans: SlotPlan[];
  auctions: Auction[];
  slots: AdSlot[];
};

const ROUND = (value: number) => Math.round(value);
const CPM = (value: number) => Math.round(value * 100) / 100;

const DESKTOP_SLOTS: SlotPlan[] = [
  {
    elementId: "div-gpt-top-leaderboard",
    pathSuffix: "top",
    sizes: ["970x250", "728x90"],
    bidders: ["appnexus", "rubicon", "pubmatic", "ix", "criteo", "openx"],
    lazy: false,
    inInitialViewport: true,
  },
  {
    elementId: "div-gpt-sidebar-1",
    pathSuffix: "sidebar_1",
    sizes: ["300x250", "300x600"],
    bidders: ["appnexus", "rubicon", "pubmatic", "ix", "criteo", "triplelift"],
    lazy: true,
    inInitialViewport: true,
  },
  {
    elementId: "div-gpt-incontent-1",
    pathSuffix: "incontent_1",
    sizes: ["728x90", "300x250"],
    bidders: ["appnexus", "rubicon", "pubmatic", "criteo", "triplelift"],
    lazy: false,
    inInitialViewport: false,
  },
  {
    elementId: "div-gpt-sidebar-2",
    pathSuffix: "sidebar_2",
    sizes: ["300x600"],
    bidders: ["appnexus", "rubicon", "ix", "openx"],
    lazy: true,
    inInitialViewport: false,
  },
  {
    elementId: "div-gpt-incontent-2",
    pathSuffix: "incontent_2",
    sizes: ["300x250"],
    bidders: ["appnexus", "pubmatic", "criteo", "sovrn"],
    lazy: false,
    inInitialViewport: false,
  },
  {
    elementId: "div-gpt-footer",
    pathSuffix: "footer",
    sizes: ["970x90", "728x90"],
    bidders: ["appnexus", "rubicon", "openx", "sovrn"],
    lazy: true,
    inInitialViewport: false,
  },
];

const MOBILE_SLOTS: SlotPlan[] = [
  {
    elementId: "div-gpt-top-banner",
    pathSuffix: "m_top",
    sizes: ["320x50", "320x100"],
    bidders: ["appnexus", "rubicon", "pubmatic", "ix", "criteo"],
    lazy: false,
    inInitialViewport: true,
  },
  {
    elementId: "div-gpt-incontent-1",
    pathSuffix: "m_incontent_1",
    sizes: ["300x250"],
    bidders: ["appnexus", "rubicon", "pubmatic", "criteo", "triplelift"],
    lazy: false,
    inInitialViewport: false,
  },
  {
    elementId: "div-gpt-incontent-2",
    pathSuffix: "m_incontent_2",
    sizes: ["300x250", "336x280"],
    bidders: ["appnexus", "rubicon", "ix", "sharethrough"],
    lazy: true,
    inInitialViewport: false,
  },
  {
    elementId: "div-gpt-incontent-3",
    pathSuffix: "m_incontent_3",
    sizes: ["300x250"],
    bidders: ["appnexus", "pubmatic", "criteo", "sovrn"],
    lazy: true,
    inInitialViewport: false,
  },
  {
    elementId: "div-gpt-sticky-footer",
    pathSuffix: "m_sticky",
    sizes: ["320x50"],
    bidders: ["appnexus", "rubicon", "pubmatic", "openx"],
    lazy: false,
    inInitialViewport: true,
  },
];

function archetypeFor(hostname: string): Archetype {
  if (/sportsdesk|localhost|127\.0\.0\.1/.test(hostname)) return "leak";
  if (/ledger/.test(hostname)) return "healthy";
  if (/techpulse/.test(hostname)) return "slow";
  return (["healthy", "leak", "slow"] as const)[hashString(hostname) % 3] ?? "healthy";
}

function siteSlug(hostname: string): string {
  return hostname.replace(/^www\./, "").split(".")[0] ?? "site";
}

type AuctionSpec = {
  id: string;
  startedAtMs: number;
  timeoutMs: number;
  units: SlotPlan[];
  latencyFactor: number;
  alwaysTimeout: string[];
};

function buildBid(rng: Rng, bidderCode: string, unit: SlotPlan, spec: AuctionSpec): Bid {
  const profile = BIDDERS[bidderCode];
  if (!profile) throw new Error(`Unknown bidder ${bidderCode}`);
  const requestedAtMs = ROUND(spec.startedAtMs + rng.between(4, 26));
  const latency = ROUND(rng.between(...profile.latency) * spec.latencyFactor);
  const base = {
    bidder: bidderCode,
    currency: null,
    size: null,
    cpm: null,
    dealId: null,
    requestedAtMs,
    exchange: null,
  };

  if (spec.alwaysTimeout.includes(bidderCode) || latency >= spec.timeoutMs) {
    return { ...base, status: "timeout", responseTimeMs: null };
  }
  if (rng.chance(0.02)) return { ...base, status: "error", responseTimeMs: latency };
  if (!rng.chance(profile.bidRate)) return { ...base, status: "noBid", responseTimeMs: latency };
  return {
    ...base,
    status: "bid",
    cpm: CPM(rng.between(...profile.cpm)),
    currency: "USD",
    size: rng.pick(unit.sizes),
    responseTimeMs: latency,
    dealId: rng.chance(0.08) ? `PMP-${hexId(rng, 6).toUpperCase()}` : null,
  };
}

function buildAuction(rng: Rng, spec: AuctionSpec): Auction {
  const adUnits: AuctionAdUnit[] = spec.units.map((unit) => {
    const bids = unit.bidders.map((bidder) => buildBid(rng, bidder, unit, spec));
    const winner = bids.filter((bid) => bid.status === "bid").sort((a, b) => (b.cpm ?? 0) - (a.cpm ?? 0))[0];
    if (winner) winner.status = "won";
    return {
      code: unit.elementId,
      sizes: unit.sizes,
      bids,
      winningBidder: winner?.bidder ?? null,
      winningCpm: winner?.cpm ?? null,
    };
  });
  const allBids = adUnits.flatMap((unit) => unit.bids);
  const timedOut = allBids.some((bid) => bid.status === "timeout");
  const lastResponse = Math.max(...allBids.map((bid) => (bid.requestedAtMs ?? 0) + (bid.responseTimeMs ?? 0)));
  const endedAtMs = timedOut ? spec.startedAtMs + spec.timeoutMs : lastResponse + 6;
  return {
    id: spec.id,
    page: 0,
    startedAtMs: spec.startedAtMs,
    endedAtMs,
    durationMs: endedAtMs - spec.startedAtMs,
    timeoutMs: spec.timeoutMs,
    adUnits,
  };
}

type SlotFate = {
  outcome: SlotOutcome;
  /** GPT request left this many ms after auction start instead of waiting for it to end. */
  failsafeAfterMs?: number;
};

function buildSlot(
  rng: Rng,
  site: { hostname: string; networkCode: string },
  plan: SlotPlan,
  auction: Auction | undefined,
  fate: SlotFate,
): AdSlot {
  const definedAtMs = ROUND(rng.between(360, 540));
  const slot: AdSlot = {
    page: 0,
    elementId: plan.elementId,
    adUnitPath: `/${site.networkCode}/${siteSlug(site.hostname)}/${plan.pathSuffix}`,
    sizes: plan.sizes,
    outcome: fate.outcome,
    renderedSize: null,
    definedAtMs,
    requestedAtMs: null,
    responseAtMs: null,
    renderedAtMs: null,
    viewableAtMs: null,
    inInitialViewport: plan.inInitialViewport,
    hadPrebidTargeting: null,
    requestedBeforeAuctionEnd: null,
    advertiserId: null,
    lineItemId: null,
    requests: [],
  };
  if (fate.outcome === "neverRequested" || !auction) return slot;

  const auctionEnd = auction.endedAtMs ?? auction.startedAtMs;
  const early = fate.failsafeAfterMs !== undefined;
  slot.requestedAtMs = ROUND(
    early ? auction.startedAtMs + (fate.failsafeAfterMs ?? 0) : auctionEnd + rng.between(8, 30),
  );
  slot.requestedBeforeAuctionEnd = early && slot.requestedAtMs < auctionEnd;
  slot.hadPrebidTargeting = !slot.requestedBeforeAuctionEnd;
  const unit = auction.adUnits.find((candidate) => candidate.code === plan.elementId);
  const winner = unit?.bids.find((bid) => bid.status === "won");
  slot.requests = [
    {
      atMs: slot.requestedAtMs,
      hadPrebidTargeting: slot.hadPrebidTargeting,
      // `hb_adid` is only on the slot when a winning bid's targeting was set before the request.
      prebidAdId:
        winner && slot.hadPrebidTargeting
          ? hashString(`${site.hostname}|${plan.elementId}|${auction.id}`).toString(16).padStart(8, "0")
          : null,
      requestId: null,
    },
  ];
  if (fate.outcome === "pending") return slot;

  slot.responseAtMs = ROUND(slot.requestedAtMs + rng.between(110, 290));
  if (fate.outcome === "empty") return slot;

  slot.renderedAtMs = ROUND(slot.responseAtMs + rng.between(50, 190));
  slot.viewableAtMs = ROUND(slot.renderedAtMs + rng.between(1010, 1600));
  slot.advertiserId = String(rng.int(4_500_000_000, 5_900_000_000));
  slot.lineItemId = String(rng.int(6_100_000_000, 6_900_000_000));

  const prebidRendered = winner && slot.hadPrebidTargeting && (winner.cpm ?? 0) > 0.9;
  if (winner && prebidRendered) winner.status = "rendered";
  slot.renderedSize = (prebidRendered ? winner.size : null) ?? plan.sizes[0] ?? null;
  return slot;
}

/** `salt` varies what one site does between the page views of a visit; the entry page has none. */
export function captureSite(hostname: string, device: DeviceProfile, salt = ""): CapturedSite {
  const archetype = archetypeFor(hostname);
  const rng = createRng(hashString(`${hostname}|${device}${salt}`));
  const slotPlans = device === "desktop" ? DESKTOP_SLOTS : MOBILE_SLOTS;
  const site = { hostname, networkCode: String(21_000_000_000 + (hashString(hostname) % 900_000_000)) };
  const pageLoadMs = ROUND(rng.between(1900, 3300) * (device === "mobile" ? 1.25 : 1));
  const firstAuctionAt = ROUND(rng.between(640, 980));
  const scrollAuctionAt = ROUND(rng.between(7800, 9600));

  const eager = slotPlans.filter((plan) => !plan.lazy);
  const lazy = slotPlans.filter((plan) => plan.lazy);
  const auctions: Auction[] = [];
  const fates = new Map<string, { auction?: Auction; fate: SlotFate }>();

  if (archetype === "leak") {
    const timeoutMs = 1500;
    const first = buildAuction(rng, {
      id: "a1",
      startedAtMs: firstAuctionAt,
      timeoutMs,
      units: eager,
      latencyFactor: 1,
      alwaysTimeout: ["ix"],
    });
    auctions.push(first);
    for (const plan of eager) fates.set(plan.elementId, { auction: first, fate: { outcome: "filled" } });
    // The wrapper's 800 ms failsafe fires GPT for the leaderboard before bids are back.
    fates.set(eager[0]?.elementId ?? "", { auction: first, fate: { outcome: "filled", failsafeAfterMs: 800 } });
    if (device === "desktop") {
      // The reference bug: lazy units wait for a scroll offset a desktop page never reaches.
      for (const plan of lazy) fates.set(plan.elementId, { fate: { outcome: "neverRequested" } });
    } else {
      const second = buildAuction(rng, {
        id: "a2",
        startedAtMs: scrollAuctionAt,
        timeoutMs,
        units: lazy,
        latencyFactor: 1.1,
        alwaysTimeout: [],
      });
      auctions.push(second);
      lazy.forEach((plan, index) => {
        fates.set(plan.elementId, { auction: second, fate: { outcome: index === 1 ? "empty" : "filled" } });
      });
    }
    return finish(PREBID_VERSIONS.outdated, timeoutMs);
  }

  if (archetype === "slow") {
    const timeoutMs = 3000;
    const first = buildAuction(rng, {
      id: "a1",
      startedAtMs: firstAuctionAt,
      timeoutMs,
      units: eager,
      latencyFactor: 3.4,
      alwaysTimeout: ["triplelift", "sovrn"],
    });
    // A second Prebid instance re-auctions the same units 40 ms later.
    const duplicate = buildAuction(rng, {
      id: "a2",
      startedAtMs: firstAuctionAt + 40,
      timeoutMs,
      units: eager.slice(0, 2),
      latencyFactor: 3.1,
      alwaysTimeout: ["ix"],
    });
    const second = buildAuction(rng, {
      id: "a3",
      startedAtMs: scrollAuctionAt,
      timeoutMs,
      units: lazy,
      latencyFactor: 3.2,
      alwaysTimeout: ["openx"],
    });
    auctions.push(first, duplicate, second);
    eager.forEach((plan, index) => {
      fates.set(plan.elementId, { auction: first, fate: { outcome: index === 2 ? "empty" : "filled" } });
    });
    lazy.forEach((plan, index) => {
      fates.set(plan.elementId, {
        auction: second,
        fate: { outcome: index === lazy.length - 1 ? "pending" : "filled" },
      });
    });
    return finish(PREBID_VERSIONS.recent, timeoutMs);
  }

  const timeoutMs = 1200;
  const first = buildAuction(rng, {
    id: "a1",
    startedAtMs: firstAuctionAt,
    timeoutMs,
    units: eager,
    latencyFactor: 0.9,
    alwaysTimeout: [],
  });
  const second = buildAuction(rng, {
    id: "a2",
    startedAtMs: scrollAuctionAt,
    timeoutMs,
    units: lazy,
    latencyFactor: 0.95,
    alwaysTimeout: ["triplelift"],
  });
  auctions.push(first, second);
  for (const plan of eager) fates.set(plan.elementId, { auction: first, fate: { outcome: "filled" } });
  for (const plan of lazy) fates.set(plan.elementId, { auction: second, fate: { outcome: "filled" } });
  return finish(PREBID_VERSIONS.current, timeoutMs);

  function finish(prebidVersion: string, siteTimeoutMs: number): CapturedSite {
    const slots = slotPlans.map((plan) => {
      const entry = fates.get(plan.elementId) ?? { fate: { outcome: "neverRequested" as const } };
      return buildSlot(rng, site, plan, entry.auction, entry.fate);
    });
    return {
      archetype,
      hostname,
      device,
      prebidVersion,
      timeoutMs: siteTimeoutMs,
      pageLoadMs,
      slotPlans,
      auctions,
      slots,
    };
  }
}

function scriptHostname(url: string): string {
  return new URL(url.replace(/^inline:/, "https:")).hostname;
}

/** Jev routing cut-offs, mirroring the backend's (uncalibrated) triage constants. */
export const JEV_THRESHOLDS = { analyse: 0.5, drop: 0.15, publisherSpecific: 0.5 } as const;

export type JevRule = "jev-yes" | "jev-unsure" | "jev-generic" | "jev-no";

/** Between the two cut-offs, the publisher's own code is kept and generic library code is dropped. */
export function jevRule(adProbability: number, publisherSpecific: number): JevRule {
  if (adProbability >= JEV_THRESHOLDS.analyse) return "jev-yes";
  if (adProbability < JEV_THRESHOLDS.drop) return "jev-no";
  return publisherSpecific >= JEV_THRESHOLDS.publisherSpecific ? "jev-unsure" : "jev-generic";
}

/** No script bodies exist here: first-party code scores as the publisher's own, everything else as generic. */
export function jevPublisherSpecific(firstParty: boolean, adProbability: number): number {
  return ROUND((firstParty ? Math.max(0.55, Math.min(0.97, adProbability + 0.02)) : adProbability / 3) * 100) / 100;
}

/** How many parts of a script Jev was asked about, out of how many it was split into. */
export function jevParts(sizeBytes: number): { total: number; judged: number } {
  const total = Math.max(1, Math.ceil(sizeBytes / 8_000));
  return { total, judged: Math.min(total, 3) };
}

function jevVerdict(script: Pick<ScriptRecord, "firstParty" | "sizeBytes">, adProbability: number) {
  const p = adProbability.toFixed(2);
  const { total, judged } = jevParts(script.sizeBytes);
  const publisherSpecific = jevPublisherSpecific(script.firstParty, adProbability);
  const own = publisherSpecific.toFixed(2);
  const rule = jevRule(adProbability, publisherSpecific);
  if (rule === "jev-yes") {
    return {
      category: publisherSpecific >= JEV_THRESHOLDS.publisherSpecific ? "wrapper" : "unknown",
      adRelated: true,
      classificationConfidence: adProbability,
      classificationReason: `Jev: ad orchestration ${p} in its best-scoring part, publisher-specific ${own}; rule jev-yes (>= 0.50)`,
    } as const;
  }
  const screened = `Jev: ad orchestration ${p} at most over ${judged} screened parts of ${total}`;
  if (rule === "jev-unsure") {
    return {
      category: "unknown",
      adRelated: true,
      classificationConfidence: adProbability,
      classificationReason: `${screened}, publisher-specific ${own}; rule jev-unsure (0.15 to 0.50 and site-specific), kept for analysis and ranked last`,
    } as const;
  }
  return {
    category: script.firstParty ? "application" : "library",
    adRelated: false,
    classificationConfidence: ROUND((1 - adProbability) * 100) / 100,
    classificationReason:
      rule === "jev-generic"
        ? `${screened}, publisher-specific ${own}; rule jev-generic (0.15 to 0.50 and a generic library, publisher-specific < 0.50)`
        : `${screened}; rule jev-no (< 0.15)`,
  } as const;
}

/** Who classifies the scripts no vendor rule decides, and whether Phase 2 follows. */
export type TriagePlan = {
  /** Jev System One goes first and answers for this many undecided scripts (`Infinity` unless it fails mid-scan). */
  jev: { model: string; answers: number } | null;
  /** The Claude filter model takes whatever Jev did not answer. `null` without an Anthropic key. */
  claudeModel: string | null;
  logicAnalysis: boolean;
};

const AD_CATEGORIES: readonly ScriptCategory[] = ["prebid", "wrapper", "adServer", "bidder", "cmp", "adQuality"];

/** `documentPages`: the page views that loaded a document, and with it every script of the site. */
export function buildScripts(site: CapturedSite, plan: TriagePlan, documentPages: number[]): ScriptRecord[] {
  const rng = createRng(hashString(`${site.hostname}|scripts`));
  // A separate stream, so switching Jev on changes nothing else about a site's scripts.
  const jevRng = createRng(hashString(`${site.hostname}|jev`));
  const templates: ScriptTemplate[] = [...SCRIPT_TEMPLATES];
  if (site.archetype === "slow") {
    templates.push({
      url: "https://cdn.adstack-partner.example/pb/prebid.8.14.0.js",
      category: "prebid",
      vendor: "Prebid.org",
      sizeKb: 287.4,
      byRule: true,
      analyzed: true,
      reason: "Second Prebid.js build (global `_pbjsPartner`) loaded by a partner tag.",
    });
  }
  const rootDomain = site.hostname.replace(/^www\./, "");
  const isNamedHost = /[a-z]$/i.test(site.hostname) && site.hostname.includes(".");
  const cdnHost = isNamedHost ? `cdn.${rootDomain}` : site.hostname;
  let loadedAt = 180;
  let undecidedSeen = 0;

  return templates.map((template, index): ScriptRecord => {
    const url = template.url.replace("cdn.{host}", cdnHost).replaceAll("{host}", site.hostname);
    const hostname = scriptHostname(url);
    loadedAt += rng.between(20, 190);
    const base = {
      id: `s${index + 1}`,
      url,
      hostname,
      inline: template.inline ?? false,
      sizeBytes: Math.round(template.sizeKb * 1024),
      hash: hexId(rng, 16),
      vendor: template.vendor,
      loadedAtMs: ROUND(loadedAt),
      firstParty: hostname.endsWith(rootDomain),
      // The inline bootstrap differs per document, so later documents carry their own copy.
      pages: template.inline ? documentPages.slice(0, 1) : documentPages,
      initiator: null,
      requestId: null,
      sha256: null,
    };
    const byTemplate = {
      category: template.category,
      adRelated: AD_CATEGORIES.includes(template.category),
      classificationReason: template.reason,
      analyzed: plan.logicAnalysis && Boolean(template.analyzed),
      adProbability: null,
    };
    if (template.byRule) {
      return { ...base, ...byTemplate, classificationConfidence: 1, classifiedBy: "heuristic", classifierModel: null };
    }

    undecidedSeen += 1;
    if (plan.jev && undecidedSeen <= plan.jev.answers) {
      const adProbability = ROUND(jevProbability(jevRng, template.category) * 100) / 100;
      return {
        ...base,
        ...byTemplate,
        ...jevVerdict(base, adProbability),
        classifiedBy: "ai",
        classifierModel: plan.jev.model,
        adProbability,
        analyzed:
          plan.logicAnalysis &&
          (Boolean(template.analyzed) ||
            jevRule(adProbability, jevPublisherSpecific(base.firstParty, adProbability)) === "jev-unsure"),
      };
    }
    if (plan.claudeModel) {
      return {
        ...base,
        ...byTemplate,
        classificationConfidence: ROUND(rng.between(0.78, 0.99) * 100) / 100,
        classifiedBy: "ai",
        classifierModel: plan.claudeModel,
      };
    }
    const adPath = url.includes("/ads/");
    return {
      ...base,
      category: adPath ? template.category : "unknown",
      adRelated: adPath && AD_CATEGORIES.includes(template.category),
      classificationReason: adPath
        ? "Path matches the first-party ad asset rule (/ads/)."
        : "No vendor rule matched and AI classification did not run.",
      classificationConfidence: null,
      classifiedBy: "heuristic",
      classifierModel: null,
      adProbability: null,
      analyzed: false,
    };
  });
}

/** Clear ad scripts score high, plain application code low, and the categories Jev cannot tell apart land in between. */
function jevProbability(rng: Rng, category: ScriptCategory): number {
  if (AD_CATEGORIES.includes(category)) return rng.between(0.62, 0.98);
  if (category === "unknown" || category === "analytics") return rng.between(0.18, 0.44);
  return rng.between(0.01, 0.12);
}

export function computeMetrics(site: CapturedSite, scripts: ScriptRecord[]): ScanMetrics {
  const bids = site.auctions.flatMap((auction) => auction.adUnits.flatMap((unit) => unit.bids));
  const count = (...statuses: Bid["status"][]) => bids.filter((bid) => statuses.includes(bid.status)).length;
  const winners = site.auctions.flatMap((auction) =>
    auction.adUnits.flatMap((unit) => (unit.winningCpm === null ? [] : [unit.winningCpm])),
  );
  const requested = site.slots.filter((slot) => slot.requestedAtMs !== null);
  const filled = site.slots.filter((slot) => slot.outcome === "filled");
  const rendered = filled.flatMap((slot) => (slot.renderedAtMs === null ? [] : [slot.renderedAtMs]));
  const totalWinningCpm = CPM(winners.reduce((sum, cpm) => sum + cpm, 0));
  const timeouts = count("timeout");

  return {
    pageLoadMs: site.pageLoadMs,
    auctionCount: site.auctions.length,
    adUnitCount: new Set(site.auctions.flatMap((a) => a.adUnits.map((unit) => unit.code))).size,
    bidderCount: new Set(bids.map((bid) => bid.bidder)).size,
    bidRequestCount: bids.length,
    bidResponseCount: count("bid", "won", "rendered"),
    bidTimeoutCount: timeouts,
    noBidCount: count("noBid"),
    timeoutRate: bids.length ? timeouts / bids.length : 0,
    slotCount: site.slots.length,
    emptySlotCount: site.slots.filter((slot) => slot.outcome === "empty").length,
    fillRate: requested.length ? filled.length / requested.length : 0,
    avgWinningCpm: winners.length ? CPM(totalWinningCpm / winners.length) : null,
    totalWinningCpm,
    timeToFirstAuctionMs: site.auctions[0]?.startedAtMs ?? null,
    timeToFirstAdMs: rendered.length ? Math.min(...rendered) : null,
    scriptCount: scripts.length,
    adScriptCount: scripts.filter((script) => script.adRelated).length,
    totalScriptBytes: scripts.reduce((sum, script) => sum + script.sizeBytes, 0),
    prebidTimeoutMs: site.timeoutMs,
    hasPrebid: true,
    hasGpt: true,
    hasAmazonTam: true,
  };
}
