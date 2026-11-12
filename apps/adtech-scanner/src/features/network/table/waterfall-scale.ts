import { formatDuration } from "@/shared/lib/format";
import type { IndexedRow, MatchContext } from "../query/match";
import { axisStart, type LifecycleMarks, type StripAxis, type TimeWindow } from "../strip/strip-layout";

/** What the waterfall column needs: the stretch of time it covers, and where each row and auction falls on it. */
export type WaterfallScale = {
  startMs: number;
  endMs: number;
  /** The stretch of time the column covers, said in its heading. */
  label: string;
  /** A row's start on the scale. */
  startOf: (indexed: IndexedRow) => number;
  /** For a bid call: when its auction closed, on the scale. */
  closeOf: (indexed: IndexedRow) => number | null;
  /** Auction windows and their timeouts, drawn behind the rows. */
  bands: { id: string; startMs: number; endMs: number; timeoutAtMs: number | null }[];
};

/** A scale never shows less than this: a single 3 ms beacon is not stretched across the column. */
const MIN_SPAN_MS = 200;
/** Room left before the first and after the last bar, as a share of the span. */
const MARGIN = 0.03;

/** The stretch of `[0, limitMs]` that holds every request of `rows`; the whole of it when there are none. */
function spanOf(rows: IndexedRow[], startOf: (indexed: IndexedRow) => number, limitMs: number) {
  let from = Number.POSITIVE_INFINITY;
  let to = 0;
  for (const indexed of rows) {
    const start = startOf(indexed);
    from = Math.min(from, start);
    // A request that never finished runs to the end.
    to = Math.max(to, indexed.row.durationMs === null ? limitMs : start + indexed.row.durationMs);
  }
  if (rows.length === 0) return { startMs: 0, endMs: limitMs };
  const margin = Math.max(to - from, MIN_SPAN_MS) * MARGIN;
  const startMs = Math.max(0, Math.floor(from - margin));
  return { startMs, endMs: Math.min(Math.max(limitMs, to), Math.max(Math.ceil(to + margin), startMs + MIN_SPAN_MS)) };
}

/**
 * The waterfall's scale is the strip's window. Without one it is the stretch of the page view that the
 * listed requests take up, so a filter that leaves a handful of bid calls shows them at a readable
 * width instead of as hairlines on a 20 s axis. A visit of several page views on one axis would make
 * every bar a hairline whatever is listed, so there each request is drawn on the clock of its own page
 * view, and the auction bands, which differ per page view, are left out.
 */
export function buildWaterfallScale({
  axis,
  context,
  marks,
  window,
  rows,
}: {
  axis: StripAxis;
  context: Pick<MatchContext, "auctions">;
  marks: Pick<LifecycleMarks, "auctions">;
  window: TimeWindow | null;
  /** The requests listed. */
  rows: IndexedRow[];
}): WaterfallScale {
  const closeOnPage = ({ row }: IndexedRow) => {
    const auction = row.category === "bid" && row.ad ? context.auctions.get(row.ad.auctionId.toLowerCase()) : undefined;
    return auction ? auction.closeMs : null;
  };
  if (axis.pages.length > 0 && !window) {
    const longest = Math.max(
      ...axis.pages.map((view, index) => (axis.pages[index + 1]?.startMs ?? axis.endMs) - view.startMs),
    );
    const startOf = (indexed: IndexedRow) => indexed.row.startedAtMs;
    const { startMs, endMs } = spanOf(rows, startOf, longest);
    return {
      startMs,
      endMs,
      label: `each page view, ${formatDuration(startMs)} to ${formatDuration(endMs)}`,
      startOf,
      closeOf: closeOnPage,
      bands: [],
    };
  }
  const startOf = (indexed: IndexedRow) => axisStart(indexed, axis);
  const { startMs, endMs } = window
    ? { startMs: window.fromMs, endMs: window.toMs }
    : spanOf(rows, startOf, axis.endMs);
  return {
    startMs,
    endMs,
    label: `${formatDuration(startMs)} to ${formatDuration(endMs)}`,
    startOf,
    closeOf: (indexed) => {
      const close = closeOnPage(indexed);
      return close === null ? null : (axis.offsets.get(indexed.row.page) ?? 0) + close;
    },
    bands: marks.auctions.map(({ id, startMs: from, endMs: to, timeoutAtMs }) => ({
      id,
      startMs: from,
      endMs: to,
      timeoutAtMs,
    })),
  };
}
