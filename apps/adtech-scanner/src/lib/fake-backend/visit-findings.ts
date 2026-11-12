import type { EvidenceDraft, FindingDraft } from "./findings";
import type { CapturedVisit, VisitedPage } from "./visit";

const timeline = (label: string, body: string): EvidenceDraft => ({
  kind: "timeline",
  label,
  body,
  scriptUrl: null,
  line: null,
});

const pathOf = (page: VisitedPage) => new URL(page.view.url).pathname;
const pageName = (page: VisitedPage) => `Page ${page.view.index + 1} (${pathOf(page)})`;
const indexes = (pages: VisitedPage[]) => pages.map((page) => page.view.index);

function laterPageWithoutBids(pages: VisitedPage[]): FindingDraft {
  const slots = [...new Set(pages.flatMap((page) => page.site.slots.map((slot) => slot.elementId)))];
  return {
    severity: "high",
    category: "logicBug",
    source: "heuristic",
    title: "Later page views request their ads without header bids",
    description: `After a navigation to another page of the site, ${slots.length} slots were requested from the ad server with no Prebid auction on that page and no \`hb_*\` key-values. The same slots were auctioned on the entry page. Every page view after the first one sells its inventory without header bidding.`,
    recommendation:
      "Look for state the wrapper keeps across navigations (sessionStorage, a cookie, a last-auction timestamp) and uses to skip `pbjs.requestBids()`. A guard against double auctions must be scoped to one document, not to the session.",
    confidence: 0.8,
    impact: "No header bidding on any page view after the first",
    estimatedRevenueImpactPct: 30,
    evidence: [
      ...pages.map((page) =>
        timeline(
          pageName(page),
          `${page.site.slots.length} slotRequested, 0 auctionInit, hb_* targeting: none, ${page.site.slots.filter((slot) => slot.outcome === "empty").length} rendered empty`,
        ),
      ),
    ],
    affectedSlots: slots,
    affectedBidders: [],
    pages: indexes(pages),
  };
}

function spaSlotsNotReset(pages: VisitedPage[]): FindingDraft {
  const first = pages[0];
  const units = first?.site.auctions[0]?.adUnits.map((unit) => unit.code) ?? [];
  return {
    severity: "high",
    category: "logicBug",
    source: "heuristic",
    title: "In-page navigation re-defines ad slots without destroying the old ones",
    description: `On a route change the wrapper called \`addAdUnits\` and \`defineSlot\` again for containers that already had a slot. GPT rejected ${units.length} definitions, the auction that followed had nothing to deliver its bids to, and no ad request was made on that page view.`,
    recommendation:
      "On every route change call `googletag.destroySlots()` and `pbjs.removeAdUnit()` for the old page before defining the new slots, or keep the slots and refresh them after a new auction.",
    confidence: 0.85,
    impact: "Page views reached by in-page navigation show no ads",
    estimatedRevenueImpactPct: 18,
    evidence: pages.map((page) =>
      timeline(
        pageName(page),
        `routeChange → ${units.length} slotDefinitionRejected → auctionInit ${page.site.auctions[0]?.id ?? ""} → no slotRequested`,
      ),
    ),
    affectedSlots: units,
    affectedBidders: [],
    pages: indexes(pages),
  };
}

function refreshWithStaleBids(visit: CapturedVisit): FindingDraft | null {
  const slot = visit.staleRefresh;
  const [first, second] = slot?.requests ?? [];
  if (!slot || !first || !second) return null;
  return {
    severity: "high",
    category: "logicBug",
    source: "heuristic",
    title: `${slot.elementId} is refreshed with the bid of the previous auction`,
    description: `The slot was requested a second time ${((second.atMs - first.atMs) / 1000).toFixed(1)} s after the first, with the same \`hb_adid\` and no new auction in between. A Prebid bid can render once: the refresh either renders nothing or loses to the ad server's own demand, and the impression counts against the bidder's win rate.`,
    recommendation:
      "Run `pbjs.requestBids()` for the slot before every `googletag.pubads().refresh()`, and clear the old targeting first with `pbjs.setTargetingForGPTAsync()` after the new auction.",
    confidence: 0.9,
    impact: "Refreshed impressions carry a bid that cannot render",
    estimatedRevenueImpactPct: 5,
    evidence: [
      timeline(
        slot.elementId,
        `slotRequested at ${Math.round(first.atMs)} ms (hb_adid ${first.prebidAdId}) → slotRequested at ${Math.round(second.atMs)} ms (hb_adid ${second.prebidAdId}), no auctionInit in between`,
      ),
    ],
    affectedSlots: [slot.elementId],
    affectedBidders: [],
    pages: [0],
  };
}

function consentAskedAgain(pages: VisitedPage[]): FindingDraft {
  return {
    severity: "medium",
    category: "consent",
    source: "heuristic",
    title: "The consent banner came back on a later page",
    description:
      "The banner was accepted on the entry page, and the consent platform showed its dialog again after the next navigation. The wrapper waits for the choice, so the first auction of that page started more than two seconds late.",
    recommendation:
      "Check that the consent cookie is written for the whole site (domain and path), and that it is not blocked or cleared between page views.",
    confidence: 0.75,
    impact: "Auctions start late on every page view after the first",
    estimatedRevenueImpactPct: null,
    evidence: pages.map((page) =>
      timeline(
        pageName(page),
        `cmpuishown after useractioncomplete on page 1 → first auctionInit at ${Math.round(page.site.auctions[0]?.startedAtMs ?? 0)} ms`,
      ),
    ),
    affectedSlots: [],
    affectedBidders: [],
    pages: indexes(pages),
  };
}

const metric = (label: string, body: string): EvidenceDraft => ({
  kind: "metric",
  label,
  body,
  scriptUrl: null,
  line: null,
});

/**
 * Not a fault of the page: the visit was recognisable as automated and a detection vendor was there
 * to act on it. The backend also raises this when a bidder was asked twice or more and never bid, and
 * then adds a note to the overview; the fake keeps to the vendor, so its other scans stay as they are.
 */
function identifiableAsAutomated(visit: CapturedVisit): FindingDraft | null {
  const { environment } = visit;
  const indicators = [
    environment.webdriver ? "navigator.webdriver was true" : null,
    environment.headless ? "the browser ran headless" : null,
  ].filter((indicator) => indicator !== null);
  const vendors = environment.detectors.map((detector) => detector.vendor);
  if (vendors.length === 0 || indicators.length === 0) return null;
  return {
    severity: "info",
    category: "configuration",
    source: "heuristic",
    title: "This scan was identifiable as automated traffic",
    description: `A scripted visit (scroll, or "Browse like a visitor") is automated by design, and the pages could tell: ${indicators.join(", ")}. ${vendors.join(" and ")} ${vendors.length === 1 ? "is" : "are"} on the page, a service that tells automated visits apart. Every bidder that was asked more than once did bid, so nothing in this scan points at filtering; this only says how the visit looked to the page. To rule it out, run the scan again as "I'll browse myself": a person drives the browser and it is not flagged as automated.`,
    recommendation:
      "Nothing to fix on the site. Treat missing bids in this scan with caution and confirm them with a visit a person drives.",
    confidence: 0.5,
    impact: "Bid counts and fill in this scan may be lower than what real visitors get",
    estimatedRevenueImpactPct: null,
    evidence: [
      metric("What the page could see", indicators.join(", ")),
      metric(
        "Detection vendors on the page",
        environment.detectors.map((detector) => `${detector.vendor} (${detector.host})`).join("\n"),
      ),
    ],
    affectedSlots: [],
    affectedBidders: [],
  };
}

/** The findings only a visit can produce: what later page views, refreshes and the environment showed. */
export function visitFindings(visit: CapturedVisit): FindingDraft[] {
  const withStory = (story: VisitedPage["story"]) => visit.pages.filter((page) => page.story === story);
  const noBids = withStory("noBidsAfterNavigation");
  const notReset = withStory("slotsNotReset");
  const consent = withStory("consentAskedAgain");
  return [
    noBids.length > 0 ? laterPageWithoutBids(noBids) : null,
    notReset.length > 0 ? spaSlotsNotReset(notReset) : null,
    refreshWithStaleBids(visit),
    consent.length > 0 ? consentAskedAgain(consent) : null,
    identifiableAsAutomated(visit),
  ].filter((draft) => draft !== null);
}
