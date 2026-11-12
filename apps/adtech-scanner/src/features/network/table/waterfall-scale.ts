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

/**
 * The waterfall's scale is the strip's window, and without one the page view shown. A visit of several
 * page views on one axis would make every bar a hairline, so there each request is drawn on the clock
 * of its own page view, and the auction bands, which differ per page view, are left out.
 */
export function buildWaterfallScale({
  axis,
  context,
  marks,
  window,
}: {
  axis: StripAxis;
  context: Pick<MatchContext, "auctions">;
  marks: Pick<LifecycleMarks, "auctions">;
  window: TimeWindow | null;
}): WaterfallScale {
  const closeOnPage = ({ row }: IndexedRow) => {
    const auction = row.category === "bid" && row.ad ? context.auctions.get(row.ad.auctionId.toLowerCase()) : undefined;
    return auction ? auction.closeMs : null;
  };
  if (axis.pages.length > 0 && !window) {
    const endMs = Math.max(
      ...axis.pages.map((view, index) => (axis.pages[index + 1]?.startMs ?? axis.endMs) - view.startMs),
    );
    return {
      startMs: 0,
      endMs,
      label: `each page view, ${formatDuration(0)} to ${formatDuration(endMs)}`,
      startOf: (indexed) => indexed.row.startedAtMs,
      closeOf: closeOnPage,
      bands: [],
    };
  }
  const startMs = window?.fromMs ?? 0;
  const endMs = window?.toMs ?? axis.endMs;
  return {
    startMs,
    endMs,
    label: `${formatDuration(startMs)} to ${formatDuration(endMs)}`,
    startOf: (indexed) => axisStart(indexed, axis),
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
