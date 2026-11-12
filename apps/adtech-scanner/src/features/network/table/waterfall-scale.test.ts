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

function scaleFor(page: number | undefined, window: { fromMs: number; toMs: number } | null) {
  const axis = buildAxis(pages, page, [bid]);
  return buildWaterfallScale({
    axis,
    context,
    marks: lifecycleMarks({ auctions, slots: [], pages }, null, axis),
    window,
  });
}

describe("buildWaterfallScale", () => {
  it("draws each request of a multi-page visit on its own page view's clock, as long as the longest page view", () => {
    const scale = scaleFor(undefined, null);
    expect([scale.startMs, scale.endMs]).toEqual([0, 23_000]);
    expect(scale.label).toBe("each page view, 0 ms to 23.0 s");
    expect(scale.startOf(bid)).toBe(500);
    expect(scale.closeOf(bid)).toBe(1900);
    // The auctions of different page views would lie on top of each other.
    expect(scale.bands).toEqual([]);
  });

  it("uses the selected page view as the scale, with its auctions behind the rows", () => {
    const scale = scaleFor(1, null);
    expect([scale.startMs, scale.endMs]).toEqual([0, 12_000]);
    expect(scale.label).toBe("0 ms to 12.0 s");
    expect(scale.startOf(bid)).toBe(500);
    expect(scale.bands.map((band) => [band.id, band.startMs, band.timeoutAtMs])).toEqual([["a2", 400, 1900]]);
  });

  it("follows a window on the visit's axis", () => {
    const scale = scaleFor(undefined, { fromMs: 23_200, toMs: 25_400 });
    expect([scale.startMs, scale.endMs]).toEqual([23_200, 25_400]);
    expect(scale.label).toBe("23.2 s to 25.4 s");
    expect(scale.startOf(bid)).toBe(23_500);
    expect(scale.closeOf(bid)).toBe(24_900);
    expect(scale.bands).toHaveLength(2);
  });
});
