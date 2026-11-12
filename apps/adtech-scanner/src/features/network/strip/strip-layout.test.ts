import { describe, expect, it } from "vitest";
import { makeAuction, makeBid, makePage, makeSlot } from "@/test/fixtures";
import { makeRow } from "@/test/network-fixtures";
import { indexRow } from "../query/match";
import {
  axisStart,
  axisTicks,
  binRequests,
  buildAxis,
  inWindow,
  lifecycleMarks,
  normalizeWindow,
} from "./strip-layout";

const pages = [
  makePage({ index: 0, url: "https://x.example/", startedAtMs: 0, loadMs: 2000, dwellMs: 20_000 }),
  makePage({ index: 1, url: "https://x.example/kelp", startedAtMs: 23_000, loadMs: 3000, dwellMs: 20_000 }),
  makePage({
    index: 2,
    url: "https://x.example/weather",
    startedAtMs: 47_000,
    loadMs: null,
    dwellMs: 20_000,
    reachedBy: "routeChange",
  }),
];
const rows = [
  makeRow({ id: "a", page: 0, startedAtMs: 100, durationMs: 200 }),
  makeRow({ id: "b", page: 0, startedAtMs: 21_900, durationMs: 50, status: 500, outcome: "httpError" }),
  makeRow({ id: "c", page: 1, startedAtMs: 100, durationMs: 400 }),
  makeRow({ id: "d", page: 2, startedAtMs: 19_000, durationMs: null, outcome: "pending" }),
].map(indexRow);

describe("buildAxis", () => {
  it("lays the page views of a visit end to end", () => {
    const axis = buildAxis(pages, undefined, rows);
    expect(axis.endMs).toBe(67_000);
    expect(axis.pages.map((page) => [page.startMs, page.label])).toEqual([
      [0, "1 /"],
      [23_000, "2 /kelp"],
      [47_000, "3 /weather"],
    ]);
    expect(rows.map((row) => axisStart(row, axis))).toEqual([100, 21_900, 23_100, 66_000]);
  });

  it("uses a page's own clock when one page is selected", () => {
    const axis = buildAxis(pages, 1, rows);
    expect(axis.endMs).toBe(23_000);
    expect(axis.pages).toEqual([]);
    expect(axisStart(rows[2] as (typeof rows)[number], axis)).toBe(100);
  });

  it("reaches past the dwell time for a request that ended later", () => {
    const late = indexRow(makeRow({ page: 1, startedAtMs: 24_000, durationMs: 2000 }));
    expect(buildAxis(pages, 1, [late]).endMs).toBe(26_000);
  });

  it("draws no page dividers for a single-page scan, and still has an axis without pages", () => {
    expect(buildAxis(pages.slice(0, 1), undefined, rows.slice(0, 1)).pages).toEqual([]);
    const legacy = buildAxis([], undefined, [indexRow(makeRow({ startedAtMs: 4000, durationMs: 500 }))]);
    expect(legacy.endMs).toBe(4500);
  });
});

describe("binRequests", () => {
  it("counts request starts per slice, across page boundaries", () => {
    const axis = buildAxis(pages, undefined, rows);
    const bins = binRequests(rows, axis, 67);
    expect(bins.counts[0]).toBe(1);
    expect(bins.counts[21]).toBe(1);
    expect(bins.counts[23]).toBe(1);
    expect(bins.counts[66]).toBe(1);
    expect(bins.counts.reduce((sum, count) => sum + count, 0)).toBe(4);
    expect(bins.peak).toBe(1);
  });

  it("flags the slices where a request failed", () => {
    const bins = binRequests(rows, buildAxis(pages, undefined, rows), 67);
    expect(bins.failed.flatMap((failed, index) => (failed ? [index] : []))).toEqual([21]);
  });

  it("puts a start at the very end in the last slice", () => {
    const axis = buildAxis(pages, undefined, rows);
    const atEnd = indexRow(makeRow({ page: 2, startedAtMs: 20_000, durationMs: 0 }));
    expect(binRequests([atEnd], axis, 10).counts[9]).toBe(1);
    expect(binRequests(rows, axis, 0)).toEqual({ counts: [], failed: [], peak: 0 });
  });
});

describe("the time window", () => {
  const axis = buildAxis(pages, undefined, rows);

  it("is put in order and kept inside the axis", () => {
    expect(normalizeWindow(30_000, 20_000, axis)).toEqual({ fromMs: 20_000, toMs: 30_000 });
    expect(normalizeWindow(60_000, 90_000, axis)).toEqual({ fromMs: 60_000, toMs: 67_000 });
  });

  it("is no window when it is missing, a slip, or the whole axis", () => {
    expect(normalizeWindow(undefined, 500, axis)).toBeNull();
    expect(normalizeWindow(500, 510, axis)).toBeNull();
    expect(normalizeWindow(0, 67_000, axis)).toBeNull();
  });

  it("keeps the requests under way during it, including ones that never finished", () => {
    const window = { fromMs: 23_000, toMs: 70_000 };
    expect(rows.filter((row) => inWindow(row, axis, window)).map((row) => row.row.id)).toEqual(["c", "d"]);
    const early = { fromMs: 150, toMs: 250 };
    expect(rows.filter((row) => inWindow(row, axis, early)).map((row) => row.row.id)).toEqual(["a"]);
    // The pending request began after this window: it is not in it.
    expect(inWindow(rows[3] as (typeof rows)[number], axis, { fromMs: 60_000, toMs: 65_000 })).toBe(false);
  });
});

describe("lifecycleMarks", () => {
  const timedOut = makeBid({ bidder: "ix", status: "timeout", responseTimeMs: null });
  const detail = {
    pages,
    auctions: [
      makeAuction([timedOut, makeBid()], { id: "a1", page: 0, startedAtMs: 900, durationMs: 1500, timeoutMs: 1500 }),
      makeAuction([makeBid()], { id: "a2", page: 2, startedAtMs: 800, durationMs: 600, timeoutMs: 1500 }),
    ],
    slots: [
      makeSlot({ page: 0, elementId: "top", renderedAtMs: 2700 }),
      makeSlot({ page: 1, elementId: "side", renderedAtMs: 900 }),
      makeSlot({ page: 1, elementId: "footer", renderedAtMs: null }),
    ],
  };

  it("places auctions, renders, loads and consent on the visit's axis", () => {
    const marks = lifecycleMarks(detail, 410, buildAxis(pages, undefined, rows));
    expect(marks.auctions).toEqual([
      { id: "a1", number: 1, page: 0, startMs: 900, endMs: 2400, timeoutAtMs: 2400, timedOutBids: 1 },
      { id: "a2", number: 2, page: 2, startMs: 47_800, endMs: 48_400, timeoutAtMs: 49_300, timedOutBids: 0 },
    ]);
    expect(marks.renders).toEqual([
      { atMs: 2700, slot: "top" },
      { atMs: 23_900, slot: "side" },
    ]);
    expect(marks.loads).toEqual([
      { atMs: 2000, page: 0 },
      { atMs: 26_000, page: 1 },
    ]);
    expect(marks.consentAtMs).toBe(410);
  });

  it("shows only the selected page's marks, on its own clock", () => {
    const marks = lifecycleMarks(detail, 410, buildAxis(pages, 2, rows));
    expect(marks.auctions.map((auction) => [auction.number, auction.startMs])).toEqual([[2, 800]]);
    expect(marks.renders).toEqual([]);
    expect(marks.loads).toEqual([]);
    // Consent was resolved on the entry page, long before this page view began.
    expect(marks.consentAtMs).toBeNull();
    expect(lifecycleMarks(detail, 410, buildAxis(pages, 0, rows)).consentAtMs).toBe(410);
  });
});

describe("axisTicks", () => {
  it("are round and stay on the axis", () => {
    expect(axisTicks(67_000, 6)).toEqual([0, 20_000, 40_000, 60_000]);
    expect(axisTicks(2400, 6)).toEqual([0, 500, 1000, 1500, 2000]);
  });
});
