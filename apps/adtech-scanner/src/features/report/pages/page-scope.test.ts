import { expect, test } from "vitest";
import { makeAuction, makeDetail, makeFinding, makePage, makeScript, makeSlot } from "@/test/fixtures";
import { isMultiPage, pageMarkers, pageShowingScript, scopeReport } from "./page-scope";

const pages = [
  makePage(),
  makePage({
    index: 1,
    url: "https://www.example.com/news/second",
    reachedBy: "linkClick",
    startedAtMs: 23_000,
    loadMs: 1800,
    metrics: { ...makePage().metrics, timeToFirstAuctionMs: null, timeToFirstAdMs: 950 },
  }),
  makePage({ index: 2, url: "https://www.example.com/news/third", reachedBy: "routeChange", loadMs: null }),
];

const visit = makeDetail({
  pages,
  findings: [
    makeFinding({ id: "entry", pages: [0] }),
    makeFinding({ id: "later", pages: [1, 2] }),
    makeFinding({ id: "whole-visit", pages: [] }),
  ],
  auctions: [makeAuction([], { id: "a1", page: 0 }), makeAuction([], { id: "a2", page: 2 })],
  slots: [makeSlot({ page: 0 }), makeSlot({ page: 1 }), makeSlot({ page: 1, elementId: "div-gpt-side" })],
  scripts: [
    makeScript({ id: "s1", pages: [0, 1] }),
    makeScript({ id: "s2", url: "https://www.example.com/ads/entry-only.js", pages: [0] }),
  ],
});

const ids = <T extends { id: string }>(items: T[]) => items.map((item) => item.id);

test("only a visit of more than one page view has pages to tell apart", () => {
  expect(isMultiPage(makeDetail())).toBe(false);
  expect(isMultiPage(makeDetail({ pages: [makePage()] }))).toBe(false);
  expect(isMultiPage(visit)).toBe(true);
});

test("a multi-page visit shown whole keeps everything and groups by every page view", () => {
  const scope = scopeReport(visit, undefined);
  expect(scope.selected).toBeNull();
  expect(scope.groups).toBe(pages);
  expect(scope.findings).toBe(visit.findings);
  expect(scope.auctions).toBe(visit.auctions);
  expect(scope.slots).toBe(visit.slots);
  expect(scope.scripts).toBe(visit.scripts);
  expect(scope.markers).toBe(visit.metrics);
});

test("a selected page keeps its own auctions, slots and scripts, and the findings seen there or everywhere", () => {
  const second = scopeReport(visit, 1);
  expect(second.selected).toBe(pages[1]);
  expect(second.groups).toEqual([]);
  expect(ids(second.findings)).toEqual(["later", "whole-visit"]);
  expect(second.auctions).toEqual([]);
  expect(second.slots).toHaveLength(2);
  expect(ids(second.scripts)).toEqual(["s1"]);
  // The slot timeline is marked with this page's own load and first-ad times.
  expect(second.markers).toEqual({ pageLoadMs: 1800, timeToFirstAuctionMs: null, timeToFirstAdMs: 950 });

  const third = scopeReport(visit, 2);
  expect(ids(third.findings)).toEqual(["later", "whole-visit"]);
  expect(ids(third.auctions)).toEqual(["a2"]);
  expect(third.slots).toEqual([]);
  expect(third.scripts).toEqual([]);
  expect(pageMarkers(pages[2] ?? makePage()).pageLoadMs).toBeNull();
});

test("a page that does not exist selects nothing", () => {
  expect(scopeReport(visit, 7)).toEqual(scopeReport(visit, undefined));
});

test("a single-page scan and a scan stored without pages are never narrowed or grouped", () => {
  for (const detail of [
    makeDetail({ findings: visit.findings }),
    makeDetail({ pages: [makePage()], findings: visit.findings }),
  ]) {
    for (const page of [undefined, 0, 3]) {
      const scope = scopeReport(detail, page);
      expect(scope.selected).toBeNull();
      expect(scope.groups).toEqual([]);
      expect(scope.findings).toBe(detail.findings);
      expect(scope.markers).toBe(detail.metrics);
    }
  }
});

test("opening a script from a finding keeps the page only when that page loaded the script", () => {
  const onSecondPage = scopeReport(visit, 1);
  // The whole-visit finding is listed on page 2, but its script was only loaded by the entry page.
  expect(ids(onSecondPage.findings)).toContain("whole-visit");
  expect(pageShowingScript(onSecondPage, "https://www.example.com/ads/entry-only.js")).toBeUndefined();
  expect(pageShowingScript(onSecondPage, makeScript().url)).toBe(1);
  expect(pageShowingScript(scopeReport(visit, undefined), makeScript().url)).toBeUndefined();
});
