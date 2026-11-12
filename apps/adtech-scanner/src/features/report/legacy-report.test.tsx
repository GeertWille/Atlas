import { expect, test } from "vitest";
import type { ScanDetail } from "@/bindings";
import {
  makeAuction,
  makeBid,
  makeDetail,
  makeFinding,
  makePage,
  makeScript,
  makeSlot,
  makeStep,
  makeSummary,
} from "@/test/fixtures";
import { renderWithApp } from "@/test/render";
import { AuctionsTab } from "./auctions/AuctionsTab";
import { EnvironmentNotice } from "./EnvironmentNotice";
import { FindingsTab } from "./findings/FindingsTab";
import { PageSwitcher } from "./pages/PageSwitcher";
import { isMultiPage, scopeReport } from "./pages/page-scope";
import { ReportHeader } from "./ReportHeader";
import { ReportOverview } from "./ReportOverview";
import { ScriptsTab } from "./scripts/ScriptsTab";
import { SlotsTab } from "./slots/SlotsTab";

/**
 * A scan as it was stored before visits had pages: no page views, no environment, only the options
 * of that time, and none of the per-page fields filled in.
 */
const legacy: ScanDetail = makeDetail({
  summary: makeSummary({ findingCounts: { critical: 1, high: 0, medium: 1, low: 0, info: 0 }, healthScore: 64 }),
  overview:
    "2 of 3 ad slots rendered a creative across 2 Prebid auctions on desktop.\n\n**Note:** Analysis stopped at the budget cap.",
  findings: [
    makeFinding({
      id: "f1",
      severity: "critical",
      category: "logicBug",
      source: "ai",
      title: "Desktop is treated as mobile",
      description: "The wrapper checks `screen.width`.",
      recommendation: "Use `matchMedia`.",
      estimatedRevenueImpactPct: 34,
      evidence: [
        {
          kind: "code",
          label: "Device check",
          body: "var IS_MOBILE = screen.width < 1440;",
          scriptUrl: "https://www.example.com/ads/hb.js",
          line: 211,
        },
        {
          kind: "timeline",
          label: "div-gpt-side",
          body: "slotDefined at 450 ms, never requested",
          scriptUrl: null,
          line: null,
        },
      ],
      affectedSlots: ["div-gpt-side"],
      affectedBidders: ["appnexus"],
    }),
    makeFinding({ id: "f2", severity: "medium", title: "Bidders time out" }),
  ],
  auctions: [
    makeAuction([
      makeBid({ status: "rendered", cpm: 2.1 }),
      makeBid({ bidder: "rubicon", status: "timeout", cpm: null, responseTimeMs: null }),
    ]),
    makeAuction([makeBid({ bidder: "ix", status: "noBid", cpm: null })], {
      id: "a2",
      startedAtMs: 8200,
      endedAtMs: 8900,
      durationMs: 700,
    }),
  ],
  slots: [
    makeSlot(),
    makeSlot({
      elementId: "div-gpt-side",
      outcome: "neverRequested",
      requestedAtMs: null,
      responseAtMs: null,
      renderedAtMs: null,
      viewableAtMs: null,
      renderedSize: null,
      hadPrebidTargeting: null,
      requestedBeforeAuctionEnd: null,
      inInitialViewport: false,
    }),
    makeSlot({
      elementId: "div-gpt-footer",
      outcome: "empty",
      renderedAtMs: null,
      viewableAtMs: null,
      renderedSize: null,
      hadPrebidTargeting: false,
      requestedBeforeAuctionEnd: true,
    }),
  ],
  scripts: [
    makeScript(),
    makeScript({
      id: "s2",
      url: "https://www.example.com/ads/hb.js",
      category: "wrapper",
      adRelated: true,
      analyzed: true,
    }),
    makeScript({
      id: "s3",
      url: "https://cdn.vendor.example/gpt.js",
      hostname: "cdn.vendor.example",
      firstParty: false,
      category: "adServer",
      adRelated: true,
      classifiedBy: "heuristic",
      classifierModel: null,
      vendor: "Google",
    }),
  ],
  steps: [makeStep(), makeStep({ id: "scan-1-analyzeLogic", node: "analyzeLogic", model: "claude-sonnet-5-5" })],
});

/** The whole report body the way the scan route composes it, for whatever page is selected. */
function Report({ detail }: { detail: ScanDetail }) {
  const scope = scopeReport(detail, undefined);
  return (
    <>
      <ReportHeader detail={detail} />
      <ReportOverview detail={detail} />
      <PageSwitcher pages={detail.pages} selected={scope.selected} onSelect={() => {}} />
      <EnvironmentNotice detail={detail} />
      <FindingsTab findings={scope.findings} pages={detail.pages} onOpenScript={() => {}} />
      <AuctionsTab auctions={scope.auctions} diagnostics={detail.bidderDiagnostics} pages={scope.groups} />
      <SlotsTab slots={scope.slots} metrics={scope.markers} pages={scope.groups} />
      <ScriptsTab scripts={scope.scripts} showPages={isMultiPage(detail)} />
    </>
  );
}

/** The markup with what differs from run to run taken out: generated ids and the local-time date. */
function normalise(html: string): string {
  return html
    .replace(/(radix-)?_r_\w+_/g, "ID")
    .replace(/<time([^>]*)>[^<]*<\/time>/g, "<time$1>DATE</time>")
    .replace(/></g, ">\n<");
}

async function reportHtml(detail: ScanDetail): Promise<string> {
  const { container, unmount } = await renderWithApp(<Report detail={detail} />);
  const html = normalise(container.innerHTML);
  unmount();
  return html;
}

/**
 * `legacy-report.reference.txt` was produced by the report as it was before visits, pages, proxies and
 * configurations existed (commit 21e4856), from this same fixture. It is a reference, not a snapshot
 * to update: a difference means an old scan no longer renders as it did.
 */
test("a scan stored before page views renders exactly as it did before they existed", async () => {
  await expect(await reportHtml(legacy)).toMatchFileSnapshot("./legacy-report.reference.txt");
});

test("a single-page scan renders exactly like one stored without pages", async () => {
  const singlePage: ScanDetail = {
    ...legacy,
    pages: [makePage({ url: legacy.summary.url })],
    findings: legacy.findings.map((finding) => ({ ...finding, pages: [0] })),
    scripts: legacy.scripts.map((script) => ({ ...script, pages: [0] })),
    slots: legacy.slots.map((slot) => ({
      ...slot,
      requests:
        slot.requestedAtMs === null
          ? []
          : [{ atMs: slot.requestedAtMs, hadPrebidTargeting: slot.hadPrebidTargeting ?? false, prebidAdId: null }],
    })),
  };
  expect(await reportHtml(singlePage)).toBe(await reportHtml(legacy));
});
