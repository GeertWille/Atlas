import type { PageView, ScanDetail } from "@/bindings";
import { pagePath } from "@/shared/domain/page-views";
import { niceStep } from "@/shared/lib/time-scale";
import type { IndexedRow } from "../query/match";

/**
 * The time axis of the overview strip. With one page view selected it is that page's own clock; with
 * the whole visit shown, every time is moved by its page view's start so the pages follow each other.
 */
export type StripAxis = {
  endMs: number;
  /** What to add to a time on a page's clock to place it on the axis. */
  offsets: Map<number, number>;
  /** Where each page view begins, when the whole visit is shown and it has more than one. */
  pages: { index: number; startMs: number; label: string }[];
};

const MIN_AXIS_MS = 1000;

export function buildAxis(pages: PageView[], selectedPage: number | undefined, rows: IndexedRow[]): StripAxis {
  const selected = pages.find((page) => page.index === selectedPage);
  const shown = selected ? [selected] : pages;
  const offsets = new Map(shown.map((page) => [page.index, selected ? 0 : page.startedAtMs]));
  let endMs = MIN_AXIS_MS;
  for (const page of shown) {
    endMs = Math.max(endMs, (offsets.get(page.index) ?? 0) + (page.loadMs ?? 0) + page.dwellMs);
  }
  for (const { row, endMs: rowEnd } of rows) {
    if (selected && row.page !== selected.index) continue;
    endMs = Math.max(endMs, (offsets.get(row.page) ?? 0) + (rowEnd ?? row.startedAtMs));
  }
  return {
    endMs,
    offsets,
    pages:
      selected || pages.length < 2
        ? []
        : pages.map((page) => ({
            index: page.index,
            startMs: page.startedAtMs,
            label: `${page.index + 1} ${pagePath(page.url)}`,
          })),
  };
}

/** A row's start on the axis. */
export function axisStart(indexed: IndexedRow, axis: StripAxis): number {
  return (axis.offsets.get(indexed.row.page) ?? 0) + indexed.row.startedAtMs;
}

export type StripBins = {
  /** Requests that started in each bin. */
  counts: number[];
  /** Whether a failed request started in the bin. */
  failed: boolean[];
  peak: number;
};

/** The histogram of the network lane: request starts per slice of the axis. */
export function binRequests(rows: IndexedRow[], axis: StripAxis, binCount: number): StripBins {
  const counts = Array<number>(binCount).fill(0);
  const failed = Array<boolean>(binCount).fill(false);
  if (binCount < 1) return { counts, failed, peak: 0 };
  for (const indexed of rows) {
    const bin = Math.min(binCount - 1, Math.max(0, Math.floor((axisStart(indexed, axis) / axis.endMs) * binCount)));
    counts[bin] = (counts[bin] ?? 0) + 1;
    const { outcome } = indexed.row;
    if (outcome === "failed" || outcome === "httpError") failed[bin] = true;
  }
  return { counts, failed, peak: Math.max(0, ...counts) };
}

export type TimeWindow = { fromMs: number; toMs: number };

/** A window narrower than this is a slip of the hand, not a selection. */
const MIN_WINDOW_MS = 20;

/** The window a link or a drag asks for, inside the axis and in order; `null` when there is none worth having. */
export function normalizeWindow(from: number | undefined, to: number | undefined, axis: StripAxis): TimeWindow | null {
  if (from === undefined || to === undefined) return null;
  const fromMs = Math.max(0, Math.min(from, to));
  const toMs = Math.min(axis.endMs, Math.max(from, to));
  if (toMs - fromMs < MIN_WINDOW_MS) return null;
  if (fromMs <= 0 && toMs >= axis.endMs) return null;
  return { fromMs, toMs };
}

/** Whether a request was under way at any point of the window. One that never finished runs to the end. */
export function inWindow(indexed: IndexedRow, axis: StripAxis, window: TimeWindow): boolean {
  const start = axisStart(indexed, axis);
  const end = indexed.row.durationMs === null ? Number.POSITIVE_INFINITY : start + indexed.row.durationMs;
  return start <= window.toMs && end >= window.fromMs;
}

export type AuctionMark = {
  id: string;
  /** As the report numbers auctions. */
  number: number;
  page: number;
  startMs: number;
  endMs: number;
  /** Where the configured timeout falls; `null` when the auction has none. */
  timeoutAtMs: number | null;
  /** Bidders that had not answered when it closed. */
  timedOutBids: number;
};

export type LifecycleMarks = {
  auctions: AuctionMark[];
  /** Slot renders: when, and which slot. */
  renders: { atMs: number; slot: string }[];
  /** Page load events. */
  loads: { atMs: number; page: number }[];
  /** When the consent choice was known; `null` when it is not on the axis. */
  consentAtMs: number | null;
};

/** The ad lifecycle on the same axis as the requests: what the page's ad stack was doing meanwhile. */
export function lifecycleMarks(
  detail: Pick<ScanDetail, "auctions" | "slots" | "pages">,
  /** When the consent choice was known, in ms since the start of the visit. */
  consentResolvedAtMs: number | null,
  axis: StripAxis,
): LifecycleMarks {
  const place = (page: number, ms: number) => {
    const offset = axis.offsets.get(page);
    return offset === undefined ? null : offset + ms;
  };
  const auctions = detail.auctions.flatMap((auction, index): AuctionMark[] => {
    const startMs = place(auction.page, auction.startedAtMs);
    if (startMs === null) return [];
    const duration = auction.durationMs ?? auction.timeoutMs ?? 0;
    return [
      {
        id: auction.id,
        number: index + 1,
        page: auction.page,
        startMs,
        endMs: startMs + duration,
        timeoutAtMs: auction.timeoutMs === null ? null : startMs + auction.timeoutMs,
        timedOutBids: auction.adUnits.reduce(
          (sum, unit) => sum + unit.bids.filter((bid) => bid.status === "timeout").length,
          0,
        ),
      },
    ];
  });
  const renders = detail.slots.flatMap((slot) => {
    const atMs = slot.renderedAtMs === null ? null : place(slot.page, slot.renderedAtMs);
    return atMs === null ? [] : [{ atMs, slot: slot.elementId }];
  });
  const loads = detail.pages.flatMap((page) => {
    const atMs = page.loadMs === null ? null : place(page.index, page.loadMs);
    return atMs === null ? [] : [{ atMs, page: page.index }];
  });
  // The consent time counts from the start of the visit. On one page's own clock it is moved to that page.
  const [onlyPage] = axis.offsets.size === 1 ? [...axis.offsets.keys()] : [];
  const shownPage = axis.pages.length === 0 ? detail.pages.find((page) => page.index === onlyPage) : undefined;
  const consent = consentResolvedAtMs === null ? null : consentResolvedAtMs - (shownPage?.startedAtMs ?? 0);
  return {
    auctions,
    renders,
    loads,
    consentAtMs: consent !== null && consent >= 0 && consent <= axis.endMs ? consent : null,
  };
}

/** Round tick positions for an axis of `endMs`, at most `maxTicks` of them after zero. */
export function axisTicks(endMs: number, maxTicks: number): number[] {
  const step = niceStep(endMs, maxTicks);
  const ticks: number[] = [];
  for (let tick = 0; tick <= endMs; tick += step) ticks.push(Math.round(tick));
  return ticks;
}
