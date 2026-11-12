import type { Auction, Bid, BidStatus } from "@/bindings";
import { BID_STATUSES } from "@/shared/domain/vocabulary";
import { createTimeScale, type TimeScale } from "@/shared/lib/time-scale";

export type WaterfallBar = {
  bid: Bid;
  isWinner: boolean;
  /** ms after the auction started when the request went out. */
  offsetMs: number;
  /** Left edge and width on the shared axis, in percent. */
  startPercent: number;
  widthPercent: number;
  /** The bar has no real end (timeout, pending): it is drawn up to the deadline. */
  openEnded: boolean;
};

export type WaterfallGroup = {
  code: string;
  sizes: string[];
  winningBidder: string | null;
  winningCpm: number | null;
  bars: WaterfallBar[];
};

export type WaterfallLayout = {
  scale: TimeScale;
  /** Position of the configured auction timeout on the axis, when it has one. */
  timeoutPercent: number | null;
  groups: WaterfallGroup[];
};

const MIN_BAR_PERCENT = 0.75;
const AXIS_HEADROOM = 1.06;

function compareBids(a: Bid, b: Bid): number {
  return (
    BID_STATUSES.indexOf(a.status) - BID_STATUSES.indexOf(b.status) ||
    (b.cpm ?? 0) - (a.cpm ?? 0) ||
    (a.responseTimeMs ?? Number.POSITIVE_INFINITY) - (b.responseTimeMs ?? Number.POSITIVE_INFINITY)
  );
}

function isWinningStatus(status: BidStatus): boolean {
  return status === "won" || status === "rendered";
}

/**
 * Places every bid of an auction on one time axis that starts at auctionInit.
 * Answered bids span request → response; unanswered ones run to the timeout
 * (or the end of the axis when the auction has none).
 */
export function layoutAuction(auction: Auction): WaterfallLayout {
  const bids = auction.adUnits.flatMap((unit) => unit.bids);
  const offsetOf = (bid: Bid) => Math.max(0, (bid.requestedAtMs ?? auction.startedAtMs) - auction.startedAtMs);
  const lastResponseMs = Math.max(0, ...bids.map((bid) => offsetOf(bid) + (bid.responseTimeMs ?? 0)));
  const scale = createTimeScale(
    Math.max(lastResponseMs, auction.timeoutMs ?? 0, auction.durationMs ?? 0) * AXIS_HEADROOM,
  );
  const deadlineMs = auction.timeoutMs ?? scale.domainMs;

  const groups = auction.adUnits.map((unit) => ({
    code: unit.code,
    sizes: unit.sizes,
    winningBidder: unit.winningBidder,
    winningCpm: unit.winningCpm,
    bars: [...unit.bids].sort(compareBids).map((bid): WaterfallBar => {
      const offsetMs = offsetOf(bid);
      const openEnded = bid.responseTimeMs === null;
      const endMs = openEnded ? Math.max(deadlineMs, offsetMs) : offsetMs + (bid.responseTimeMs ?? 0);
      const startPercent = scale.toPercent(offsetMs);
      return {
        bid,
        isWinner: isWinningStatus(bid.status),
        offsetMs,
        startPercent,
        widthPercent: Math.max(MIN_BAR_PERCENT, scale.toPercent(endMs) - startPercent),
        openEnded,
      };
    }),
  }));

  return {
    scale,
    timeoutPercent: auction.timeoutMs === null ? null : scale.toPercent(auction.timeoutMs),
    groups,
  };
}
