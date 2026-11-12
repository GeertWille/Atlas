import { describe, expect, it } from "vitest";
import { makeAuction, makePage } from "@/test/fixtures";
import { makeAdLink, makeRow } from "@/test/network-fixtures";
import { buildMatchContext, indexRow } from "../query/match";
import { buildAxis, lifecycleMarks } from "../strip/strip-layout";
import { buildWaterfallScale } from "./waterfall-scale";

const pages = [
  makePage({ index: 0, startedAtMs: 0, loadMs: 2000, dwellMs: 20_000 }),
  makePage({ index: 1, startedAtMs: 23_000, loadMs: 3000, dwellMs: 9000 }),
];
const auctions = [
  makeAuction([], { id: "a1", page: 0, startedAtMs: 900, timeoutMs: 1500 }),
  makeAuction([], { id: "a2", page: 1, startedAtMs: 400, timeoutMs: 1500 }),
];
const bid = indexRow(
  makeRow({
    id: "bid",
    page: 1,
    category: "bid",
    startedAtMs: 500,
    durationMs: 1600,
    ad: makeAdLink({ auctionId: "a2" }),
  }),
);
const context = buildMatchContext({ auctions });

const beacon = indexRow(makeRow({ id: "beacon", page: 1, startedAtMs: 8000, durationMs: 40 }));
const pending = indexRow(makeRow({ id: "poll", page: 1, startedAtMs: 3000, durationMs: null, outcome: "pending" }));

function scaleFor(page: number | undefined, window: { fromMs: number; toMs: number } | null, rows = [bid, beacon]) {
  const axis = buildAxis(pages, page, [bid, beacon]);
  return buildWaterfallScale({
    axis,
    context,
    marks: lifecycleMarks({ auctions, slots: [], pages }, null, axis),
    window,
    rows,
  });
}

describe("buildWaterfallScale", () => {
  it("draws each request of a multi-page visit on its own page view's clock", () => {
    const scale = scaleFor(undefined, null);
    expect([scale.startMs, scale.endMs]).toEqual([273, 8267]);
    expect(scale.label).toBe("each page view, 273 ms to 8.27 s");
    expect(scale.startOf(bid)).toBe(500);
    expect(scale.closeOf(bid)).toBe(1900);
    // The auctions of different page views would lie on top of each other.
    expect(scale.bands).toEqual([]);
  });

  it("uses the selected page view's clock, with its auctions behind the rows", () => {
    const scale = scaleFor(1, null);
    expect([scale.startMs, scale.endMs]).toEqual([273, 8267]);
    expect(scale.label).toBe("273 ms to 8.27 s");
    expect(scale.startOf(bid)).toBe(500);
    expect(scale.bands.map((band) => [band.id, band.startMs, band.timeoutAtMs])).toEqual([["a2", 400, 1900]]);
  });

  it("fits the requests that are listed: a filter down to one bid call shows it wide", () => {
    const scale = scaleFor(1, null, [bid]);
    expect([scale.startMs, scale.endMs]).toEqual([452, 2148]);
    // The bar of the 1.6 s call takes most of the column instead of a seventh of it.
    expect((1600 / (scale.endMs - scale.startMs)) * 100).toBeGreaterThan(90);
  });

  it("keeps the whole page view for a request that never finished, and for an empty list", () => {
    expect(scaleFor(1, null, [pending])).toMatchObject({ startMs: 2730, endMs: 12_000 });
    expect(scaleFor(1, null, [])).toMatchObject({ startMs: 0, endMs: 12_000 });
    expect(scaleFor(undefined, null, [])).toMatchObject({ startMs: 0, endMs: 23_000 });
  });

  it("does not stretch a single short request across the column", () => {
    const scale = scaleFor(1, null, [beacon]);
    expect(scale.endMs - scale.startMs).toBeGreaterThanOrEqual(200);
    expect(scale.startMs).toBeLessThanOrEqual(8000);
    expect(scale.endMs).toBeGreaterThanOrEqual(8040);
  });

  it("follows a window on the visit's axis, whatever is listed", () => {
    const scale = scaleFor(undefined, { fromMs: 23_200, toMs: 25_400 }, [bid]);
    expect([scale.startMs, scale.endMs]).toEqual([23_200, 25_400]);
    expect(scale.label).toBe("23.2 s to 25.4 s");
    expect(scale.startOf(bid)).toBe(23_500);
    expect(scale.closeOf(bid)).toBe(24_900);
    expect(scale.bands).toHaveLength(2);
  });
});
