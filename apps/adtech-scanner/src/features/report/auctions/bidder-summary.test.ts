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

  const inDollars = { currency: "USD", mixedCurrencies: false };
  expect(summarizeBidders([first, second])).toEqual([
    {
      bidder: "rubicon",
      requests: 2,
      bids: 2,
      wins: 1,
      timeouts: 0,
      avgLatencyMs: 400,
      avgCpm: 2,
      winRate: 0.5,
      ...inDollars,
    },
    {
      bidder: "appnexus",
      requests: 2,
      bids: 1,
      wins: 1,
      timeouts: 0,
      avgLatencyMs: 150,
      avgCpm: 2,
      winRate: 0.5,
      ...inDollars,
    },
    {
      bidder: "ix",
      requests: 2,
      bids: 0,
      wins: 0,
      timeouts: 2,
      avgLatencyMs: null,
      avgCpm: null,
      winRate: 0,
      currency: null,
      mixedCurrencies: false,
    },
  ]);
});

test("a bidder's prices are averaged in their currency, and not at all across two", () => {
  const euros = makeAuction([
    makeBid({ bidder: "gumgum", cpm: 10, currency: "EUR" }),
    makeBid({ bidder: "rubicon", cpm: 2, currency: "USD" }),
  ]);
  const more = makeAuction(
    [makeBid({ bidder: "gumgum", cpm: 6, currency: "EUR" }), makeBid({ bidder: "rubicon", cpm: 4, currency: "EUR" })],
    { id: "a2" },
  );
  const byBidder = Object.fromEntries(summarizeBidders([euros, more]).map((row) => [row.bidder, row]));
  expect(byBidder.gumgum).toMatchObject({ avgCpm: 8, currency: "EUR", mixedCurrencies: false });
  expect(byBidder.rubicon).toMatchObject({ avgCpm: null, mixedCurrencies: true });
});

test("no auctions means no rows", () => {
  expect(summarizeBidders([])).toEqual([]);
});
