import { expect, test } from "vitest";
import { makeAuction, makeBid } from "@/test/fixtures";
import { layoutAuction } from "./waterfall-layout";

const auction = makeAuction([
  makeBid({ bidder: "appnexus", status: "bid", cpm: 1.2, requestedAtMs: 1010, responseTimeMs: 300 }),
  makeBid({ bidder: "ix", status: "timeout", cpm: null, requestedAtMs: 1005, responseTimeMs: null }),
  makeBid({ bidder: "rubicon", status: "won", cpm: 2, requestedAtMs: 1020, responseTimeMs: 400 }),
  makeBid({ bidder: "openx", status: "noBid", cpm: null, requestedAtMs: 1010, responseTimeMs: 200 }),
]);

test("the axis covers the timeout with headroom and marks it", () => {
  const layout = layoutAuction(auction);
  expect(layout.scale.domainMs).toBe(2000);
  expect(layout.timeoutPercent).toBe(75);
});

test("bars are ordered winner, bids by price, then failures", () => {
  const [group] = layoutAuction(auction).groups;
  expect(group?.bars.map((bar) => bar.bid.bidder)).toEqual(["rubicon", "appnexus", "openx", "ix"]);
  expect(group?.bars.map((bar) => bar.isWinner)).toEqual([true, false, false, false]);
});

test("an answered bid spans request to response, relative to auction start", () => {
  const winner = layoutAuction(auction).groups[0]?.bars[0];
  expect(winner).toMatchObject({ offsetMs: 20, startPercent: 1, widthPercent: 20, openEnded: false });
});

test("a timed-out bid runs open-ended up to the timeout line", () => {
  const timedOut = layoutAuction(auction).groups[0]?.bars[3];
  expect(timedOut).toMatchObject({ offsetMs: 5, startPercent: 0.25, widthPercent: 74.75, openEnded: true });
});

test("without a timeout, unanswered bids run to the end of the axis", () => {
  const pending = makeAuction(
    [
      makeBid({ bidder: "appnexus", requestedAtMs: 1000, responseTimeMs: 800 }),
      makeBid({ bidder: "sovrn", status: "pending", cpm: null, requestedAtMs: 1000, responseTimeMs: null }),
    ],
    { timeoutMs: null, durationMs: null, endedAtMs: null },
  );
  const layout = layoutAuction(pending);
  expect(layout.timeoutPercent).toBeNull();
  expect(layout.groups[0]?.bars[1]).toMatchObject({ startPercent: 0, widthPercent: 100, openEnded: true });
});

test("very fast responses keep a visible minimum width", () => {
  const fast = makeAuction([makeBid({ requestedAtMs: 1000, responseTimeMs: 1 })]);
  expect(layoutAuction(fast).groups[0]?.bars[0]?.widthPercent).toBe(0.75);
});

test("a bid without a request time starts at the auction start", () => {
  const unknown = makeAuction([makeBid({ requestedAtMs: null, responseTimeMs: 500 })]);
  expect(layoutAuction(unknown).groups[0]?.bars[0]).toMatchObject({ offsetMs: 0, startPercent: 0, widthPercent: 25 });
});
