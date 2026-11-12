import { expect, test } from "vitest";
import { makeAuction, makeBid } from "@/test/fixtures";
import { summarizeBidders } from "./bidder-summary";

test("totals each bidder across auctions and ranks by wins, then bids", () => {
  const first = makeAuction([
    makeBid({ bidder: "appnexus", status: "rendered", cpm: 2, responseTimeMs: 200 }),
    makeBid({ bidder: "ix", status: "timeout", cpm: null, responseTimeMs: null }),
    makeBid({ bidder: "rubicon", status: "bid", cpm: 1, responseTimeMs: 300 }),
  ]);
  const second = makeAuction(
    [
      makeBid({ bidder: "appnexus", status: "noBid", cpm: null, responseTimeMs: 100 }),
      makeBid({ bidder: "ix", status: "timeout", cpm: null, responseTimeMs: null }),
      makeBid({ bidder: "rubicon", status: "won", cpm: 3, responseTimeMs: 500 }),
    ],
    { id: "a2" },
  );

  expect(summarizeBidders([first, second])).toEqual([
    { bidder: "rubicon", requests: 2, bids: 2, wins: 1, timeouts: 0, avgLatencyMs: 400, avgCpm: 2, winRate: 0.5 },
    { bidder: "appnexus", requests: 2, bids: 1, wins: 1, timeouts: 0, avgLatencyMs: 150, avgCpm: 2, winRate: 0.5 },
    { bidder: "ix", requests: 2, bids: 0, wins: 0, timeouts: 2, avgLatencyMs: null, avgCpm: null, winRate: 0 },
  ]);
});

test("no auctions means no rows", () => {
  expect(summarizeBidders([])).toEqual([]);
});
