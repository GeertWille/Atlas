import type { Auction } from "@/bindings";

export type BidderSummary = {
  bidder: string;
  requests: number;
  bids: number;
  wins: number;
  timeouts: number;
  avgLatencyMs: number | null;
  avgCpm: number | null;
  /** wins / requests, 0-1. */
  winRate: number;
};

type Tally = Omit<BidderSummary, "avgLatencyMs" | "avgCpm" | "winRate"> & { latencies: number[]; cpms: number[] };

const average = (values: number[]) =>
  values.length === 0 ? null : values.reduce((sum, value) => sum + value, 0) / values.length;

/** Per-bidder totals across all auctions, strongest performer first. */
export function summarizeBidders(auctions: Auction[]): BidderSummary[] {
  const tallies = new Map<string, Tally>();
  for (const auction of auctions) {
    for (const unit of auction.adUnits) {
      for (const bid of unit.bids) {
        const tally = tallies.get(bid.bidder) ?? {
          bidder: bid.bidder,
          requests: 0,
          bids: 0,
          wins: 0,
          timeouts: 0,
          latencies: [],
          cpms: [],
        };
        tally.requests += 1;
        if (bid.status === "bid" || bid.status === "won" || bid.status === "rendered") tally.bids += 1;
        if (bid.status === "won" || bid.status === "rendered") tally.wins += 1;
        if (bid.status === "timeout") tally.timeouts += 1;
        if (bid.responseTimeMs !== null) tally.latencies.push(bid.responseTimeMs);
        if (bid.cpm !== null) tally.cpms.push(bid.cpm);
        tallies.set(bid.bidder, tally);
      }
    }
  }
  return [...tallies.values()]
    .map(({ latencies, cpms, ...tally }) => ({
      ...tally,
      avgLatencyMs: average(latencies),
      avgCpm: average(cpms),
      winRate: tally.requests === 0 ? 0 : tally.wins / tally.requests,
    }))
    .sort((a, b) => b.wins - a.wins || b.bids - a.bids || a.bidder.localeCompare(b.bidder));
}
