import type { AdSlot, Auction, Finding, PageView, ScanDetail, ScanMetrics, ScriptRecord } from "@/bindings";

/** The page lifecycle times a slot timeline marks. All count from the start of one page view. */
export type PageMarkers = Pick<ScanMetrics, "pageLoadMs" | "timeToFirstAuctionMs" | "timeToFirstAdMs">;

export function pageMarkers(page: PageView): PageMarkers {
  return {
    pageLoadMs: page.loadMs,
    timeToFirstAuctionMs: page.metrics.timeToFirstAuctionMs,
    timeToFirstAdMs: page.metrics.timeToFirstAdMs,
  };
}

/** A visit has pages to tell apart only when it has more than one; scans stored before page views have none. */
export function isMultiPage(detail: Pick<ScanDetail, "pages">): boolean {
  return detail.pages.length > 1;
}

export type ReportScope = {
  /** The one page view the report is narrowed to; `null` shows the whole visit. */
  selected: PageView | null;
  /** The page views to group auctions and slots under: every one of a multi-page visit shown whole, else none. */
  groups: PageView[];
  findings: Finding[];
  auctions: Auction[];
  slots: AdSlot[];
  scripts: ScriptRecord[];
  /** Markers for the slot timeline when it is not grouped: the selected page's, or the scan's. */
  markers: PageMarkers;
};

/**
 * What the report tabs show for a page selection. A finding that names no page concerns the whole
 * visit and stays visible on every page. An unknown page index, or any index on a single-page scan,
 * selects nothing.
 */
export function scopeReport(detail: ScanDetail, pageIndex: number | undefined): ReportScope {
  const multiPage = isMultiPage(detail);
  const selected = multiPage ? (detail.pages.find((page) => page.index === pageIndex) ?? null) : null;
  if (!selected) {
    return {
      selected: null,
      groups: multiPage ? detail.pages : [],
      findings: detail.findings,
      auctions: detail.auctions,
      slots: detail.slots,
      scripts: detail.scripts,
      markers: detail.metrics,
    };
  }
  const { index } = selected;
  return {
    selected,
    groups: [],
    findings: detail.findings.filter((finding) => finding.pages.length === 0 || finding.pages.includes(index)),
    auctions: detail.auctions.filter((auction) => auction.page === index),
    slots: detail.slots.filter((slot) => slot.page === index),
    scripts: detail.scripts.filter((script) => script.pages.includes(index)),
    markers: pageMarkers(selected),
  };
}

/**
 * The page selection a deep link to a script keeps. A finding of the whole visit stays visible on
 * every page and can cite a script the selected page never loaded: the link then shows all pages.
 */
export function pageShowingScript(scope: ReportScope, scriptUrl: string): number | undefined {
  return scope.scripts.some((script) => script.url === scriptUrl) ? scope.selected?.index : undefined;
}
