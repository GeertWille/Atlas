import { describe, expect, it } from "vitest";
import { makePage } from "@/test/fixtures";
import { makeAdLink, makeRow } from "@/test/network-fixtures";
import { indexRow } from "../query/match";
import {
  compareRows,
  DEFAULT_SORT,
  formatSort,
  gridTemplate,
  isColumnChoices,
  nextSort,
  parseSort,
  splitName,
  visibleColumns,
} from "./columns";
import { buildGrid, initiallyCollapsed, siteOf } from "./grouping";

const keys = (input: Parameters<typeof visibleColumns>[0]) => visibleColumns(input).map((column) => column.key);

describe("visibleColumns", () => {
  it("shows the full default set when the list is wide", () => {
    expect(keys({ choices: {}, listWidth: 1240, detailBeside: false })).toEqual([
      "status",
      "name",
      "vendor",
      "outcome",
      "type",
      "initiator",
      "size",
      "time",
      "waterfall",
    ]);
    // The default window leaves the room to the waterfall instead.
    expect(keys({ choices: {}, listWidth: 1040, detailBeside: false })).not.toContain("initiator");
  });

  it("drops Type, Size and Initiator in the small window", () => {
    expect(keys({ choices: {}, listWidth: 784, detailBeside: false })).toEqual([
      "status",
      "name",
      "vendor",
      "outcome",
      "time",
      "waterfall",
    ]);
  });

  it("keeps Status, Name, Ad outcome and Time beside a docked detail panel", () => {
    expect(keys({ choices: { host: true }, listWidth: 520, detailBeside: true })).toEqual([
      "status",
      "name",
      "outcome",
      "time",
    ]);
  });

  it("lets a choice of the user win over the defaults, both ways", () => {
    const chosen = keys({ choices: { vendor: false, host: true, size: true }, listWidth: 784, detailBeside: false });
    expect(chosen).not.toContain("vendor");
    expect(chosen).toContain("host");
    expect(chosen).toContain("size");
    expect(chosen.slice(0, 2)).toEqual(["status", "name"]);
  });

  it("builds one grid track per column", () => {
    const columns = visibleColumns({ choices: {}, listWidth: 784, detailBeside: false });
    expect(gridTemplate(columns)).toBe("72px minmax(168px, 2fr) 124px 108px 64px minmax(120px, 3fr)");
  });

  it("accepts only stored choices of the right shape", () => {
    expect(isColumnChoices({ host: true, vendor: false })).toBe(true);
    expect(isColumnChoices({ nope: true })).toBe(false);
    expect(isColumnChoices({ host: "yes" })).toBe(false);
    expect(isColumnChoices(["host"])).toBe(false);
  });
});

describe("sorting", () => {
  it("reads and writes the URL form, with request order as the default", () => {
    expect(parseSort(undefined)).toEqual(DEFAULT_SORT);
    expect(parseSort("-time")).toEqual({ key: "time", descending: true });
    expect(parseSort("size")).toEqual({ key: "size", descending: false });
    expect(parseSort("nonsense")).toEqual(DEFAULT_SORT);
    expect(parseSort("waterfall")).toEqual(DEFAULT_SORT);
    expect(formatSort(DEFAULT_SORT)).toBeUndefined();
    expect(formatSort({ key: "time", descending: true })).toBe("-time");
  });

  it("cycles a header through ascending, descending and back to request order", () => {
    const first = nextSort(DEFAULT_SORT, "time");
    const second = nextSort(first, "time");
    expect([first, second, nextSort(second, "time")]).toEqual([
      { key: "time", descending: false },
      { key: "time", descending: true },
      DEFAULT_SORT,
    ]);
    expect(nextSort(second, "size")).toEqual({ key: "size", descending: false });
  });

  const rows = [
    makeRow({ id: "a", seq: 1, startedAtMs: 100, durationMs: 300, transferredBytes: 10 }),
    makeRow({ id: "b", seq: 2, startedAtMs: 200, durationMs: 300, transferredBytes: 30 }),
    makeRow({ id: "c", seq: 3, startedAtMs: 300, durationMs: null, transferredBytes: 20 }),
    makeRow({ id: "d", seq: 4, startedAtMs: 50, durationMs: 100, transferredBytes: 30, page: 1 }),
  ];
  const order = (sort: string | undefined) =>
    [...rows]
      .sort(compareRows(parseSort(sort)))
      .map((row) => row.id)
      .join("");

  it("orders by page, start and sequence by default", () => {
    expect(order(undefined)).toBe("abcd");
  });

  it("keeps equal rows in request order, whichever way the sort runs", () => {
    expect(order("time")).toBe("cdab");
    expect(order("-time")).toBe("abdc");
    expect(order("-size")).toBe("bdca");
  });

  it("sorts by ad outcome", () => {
    const late = makeRow({ id: "late", ad: makeAdLink({ verdict: "lateOnNetwork" }) });
    const paid = makeRow({ id: "paid", ad: makeAdLink({ cpm: 2 }) });
    const cheap = makeRow({ id: "cheap", ad: makeAdLink({ cpm: 1 }) });
    const plain = makeRow({ id: "plain" });
    expect([late, paid, plain, cheap].sort(compareRows(parseSort("outcome"))).map((row) => row.id)).toEqual([
      "plain",
      "cheap",
      "paid",
      "late",
    ]);
  });
});

describe("splitName", () => {
  it("separates the last path segment from the query", () => {
    expect(splitName("https://x.example/gampad/ads?gdfp_req=1&a=2")).toEqual({ name: "ads", query: "?gdfp_req=1&a=2" });
    expect(splitName("https://x.example/")).toEqual({ name: "x.example", query: "" });
    expect(splitName("not a url")).toEqual({ name: "not a url", query: "" });
  });
});

describe("buildGrid", () => {
  const pages = [
    makePage({ index: 0, url: "https://www.example.com/" }),
    makePage({ index: 1, url: "https://www.example.com/kelp", reachedBy: "linkClick" }),
    makePage({ index: 2, url: "https://www.example.com/weather", reachedBy: "routeChange" }),
  ];
  const rows = [
    makeRow({ id: "doc", kind: "document", startedAtMs: 0, durationMs: 200, transferredBytes: 1000 }),
    makeRow({
      id: "g1",
      vendor: "Google",
      firstParty: false,
      kind: "xhr",
      startedAtMs: 400,
      durationMs: 100,
      status: 500,
      outcome: "httpError",
    }),
    makeRow({
      id: "g2",
      vendor: "Google",
      firstParty: false,
      kind: "image",
      startedAtMs: 500,
      durationMs: null,
      outcome: "pending",
    }),
    makeRow({ id: "x", url: "https://t.unknown-cdn.example/t.js", firstParty: false, startedAtMs: 600, page: 1 }),
    makeRow({ id: "r", kind: "fetch", startedAtMs: 100, page: 2 }),
  ].map(indexRow);
  const base = { sort: DEFAULT_SORT, collapsed: new Set<string>(), pages };
  const outline = (items: ReturnType<typeof buildGrid>) =>
    items.map((item) => (item.kind === "group" ? `[${item.group.label}|${item.group.count}]` : item.indexed.row.id));

  it("is a flat list without grouping", () => {
    expect(outline(buildGrid(rows, { ...base, grouping: "none" }))).toEqual(["doc", "g1", "g2", "x", "r"]);
  });

  it("groups by page view in the order of the visit, and says how each was reached", () => {
    const items = buildGrid(rows, { ...base, grouping: "page" });
    expect(outline(items)).toEqual([
      "[Page 1  /|3]",
      "doc",
      "g1",
      "g2",
      "[Page 2  /kelp|1]",
      "x",
      "[Page 3  /weather|1]",
      "r",
    ]);
    const groups = items.flatMap((item) => (item.kind === "group" ? [item.group] : []));
    expect(groups.map((group) => group.note)).toEqual([
      "entry page",
      "followed link",
      "in-page navigation, no document request",
    ]);
  });

  it("totals each group: bytes, failures and time span", () => {
    const [first] = buildGrid(rows, { ...base, grouping: "page" });
    expect(first).toMatchObject({
      kind: "group",
      group: { count: 3, failed: 1, startMs: 0, endMs: null, transferredBytes: 25_000 },
    });
    const flatSpan = buildGrid(rows.slice(0, 2), { ...base, grouping: "page" })[0];
    expect(flatSpan?.kind === "group" && flatSpan.group.endMs).toBe(500);
  });

  it("groups by vendor, largest first, naming unknown third parties by their site", () => {
    expect(outline(buildGrid(rows, { ...base, grouping: "vendor" }))).toEqual([
      "[Google|2]",
      "g1",
      "g2",
      "[Site|2]",
      "doc",
      "r",
      "[unknown-cdn.example|1]",
      "x",
    ]);
  });

  it("groups by type", () => {
    expect(outline(buildGrid(rows, { ...base, grouping: "type" }))[0]).toBe("[doc|1]");
  });

  it("sorts inside the groups", () => {
    const items = buildGrid(rows, { ...base, grouping: "page", sort: { key: "start", descending: true } });
    expect(outline(items).slice(0, 4)).toEqual(["[Page 1  /|3]", "g2", "g1", "doc"]);
  });

  it("keeps the heading of a folded group and hides its rows", () => {
    const items = buildGrid(rows, { ...base, grouping: "page", collapsed: new Set(["page:0"]) });
    expect(outline(items)).toEqual(["[Page 1  /|3]", "[Page 2  /kelp|1]", "x", "[Page 3  /weather|1]", "r"]);
    expect(items[0]).toMatchObject({ group: { collapsed: true } });
  });
});

describe("initiallyCollapsed", () => {
  const pages = [makePage({ index: 0 }), makePage({ index: 1 }), makePage({ index: 2 })];

  it("folds nothing in a list of ordinary size", () => {
    expect(initiallyCollapsed(5000, pages, undefined).size).toBe(0);
  });

  it("folds every page but the one in view in a very large list", () => {
    expect([...initiallyCollapsed(8000, pages, undefined)]).toEqual(["page:1", "page:2"]);
    expect([...initiallyCollapsed(8000, pages, 1)]).toEqual(["page:0", "page:2"]);
  });
});

describe("siteOf", () => {
  it("is the last two labels of a host", () => {
    expect(siteOf("securepubads.g.doubleclick.net")).toBe("doubleclick.net");
    expect(siteOf("localhost")).toBe("localhost");
  });
});
