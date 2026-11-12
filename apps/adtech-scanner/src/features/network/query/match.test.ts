import { describe, expect, it } from "vitest";
import type { NetworkRow } from "@/bindings";
import { makeAuction, makeBid } from "@/test/fixtures";
import { makeAdLink, makeRow } from "@/test/network-fixtures";
import { activeCategory, hasToken, STATE_TOKENS, toggleToken, withCategory } from "./edit";
import { categoryCounts, stateCounts, summariseList } from "./facets";
import { buildMatchContext, compileQuery, indexRow } from "./match";
import { parseQuery } from "./parse";
import { pushRecent } from "./recipes";

const won = makeBid({
  bidder: "rubicon",
  status: "won",
  cpm: 1.4,
  exchange: { requestId: "bid-ok", verdict: "inTime", lateByMs: -200, networkMs: 300, exact: true },
});
const detail = {
  auctions: [
    makeAuction([won], { id: "a1", page: 0, startedAtMs: 1000, endedAtMs: 2500, timeoutMs: 1500 }),
    makeAuction([], { id: "a2", page: 2, startedAtMs: 800, endedAtMs: 1400, timeoutMs: 1500 }),
  ],
};
const context = buildMatchContext(detail);

const rows: NetworkRow[] = [
  makeRow({ id: "doc", kind: "document", url: "https://www.example.com/news", startedAtMs: 0, durationMs: 300 }),
  makeRow({ id: "app", url: "https://cdn.example.com/static/app.js?v=3", transferredBytes: 600_000 }),
  makeRow({
    id: "bid-ok",
    url: "https://fastlane.rubiconproject.com/a/api/fastlane.json?size_id=2",
    category: "bid",
    kind: "xhr",
    vendor: "Magnite",
    bidder: "rubicon",
    firstParty: false,
    startedAtMs: 1010,
    durationMs: 300,
    ad: makeAdLink({ cpm: 1.4 }),
  }),
  makeRow({
    id: "bid-late",
    url: "https://htlb.casalemedia.com/openrtb/pbjs",
    method: "POST",
    category: "bid",
    kind: "xhr",
    vendor: "Index Exchange",
    bidder: "ix",
    firstParty: false,
    startedAtMs: 1012,
    durationMs: 1572,
    ad: makeAdLink({ verdict: "lateOnNetwork", lateByMs: 84 }),
  }),
  makeRow({
    id: "bid-nobid",
    url: "https://rtb.openx.net/openrtbb/prebidjs",
    method: "POST",
    category: "bid",
    kind: "fetch",
    vendor: "OpenX",
    bidder: "openx",
    firstParty: false,
    startedAtMs: 1015,
    durationMs: 1300,
    ad: makeAdLink({ nbr: 7, adUnits: ["div-gpt-top", "div-gpt-side"] }),
  }),
  makeRow({
    id: "gam",
    url: "https://securepubads.g.doubleclick.net/gampad/ads?gdfp_req=1",
    category: "adServer",
    kind: "xhr",
    vendor: "Google",
    firstParty: false,
    startedAtMs: 1800,
    noHbTargeting: true,
    initiator: {
      kind: "script",
      url: "https://securepubads.g.doubleclick.net/pubads_impl.js",
      scriptId: "s13",
      line: 1,
      column: 400,
    },
  }),
  makeRow({
    id: "sync",
    url: "https://ib.adnxs.com/setuid?entity=315",
    category: "identity",
    kind: "image",
    mime: "image/gif",
    vendor: "Xandr",
    firstParty: false,
    status: 302,
    startedAtMs: 4000,
  }),
  makeRow({
    id: "beacon-blocked",
    url: "https://api.segment.io/v1/p",
    method: "POST",
    category: "measurement",
    kind: "fetch",
    vendor: "Segment",
    firstParty: false,
    status: null,
    outcome: "blocked",
    startedAtMs: 5000,
  }),
  makeRow({ id: "img-404", url: "https://img.example.com/a.jpg", kind: "image", status: 404, outcome: "httpError" }),
  makeRow({
    id: "cached",
    url: "https://cdn.example.com/static/base.css",
    kind: "stylesheet",
    servedFrom: "diskCache",
    transferredBytes: 0,
    page: 1,
  }),
  makeRow({
    id: "sw",
    url: "https://www.example.com/offline.json",
    kind: "fetch",
    servedFrom: "serviceWorker",
    transferredBytes: 0,
    page: 1,
  }),
  makeRow({
    id: "poll",
    url: "https://www.example.com/api/poll",
    kind: "fetch",
    status: null,
    outcome: "pending",
    durationMs: null,
    page: 1,
  }),
  makeRow({ id: "p3", url: "https://www.example.com/api/article.json", kind: "fetch", page: 2, startedAtMs: 700 }),
  makeRow({ id: "p3-late", url: "https://www.example.com/api/more.json", kind: "fetch", page: 2, startedAtMs: 3000 }),
];
const indexed = rows.map(indexRow);

const ids = (query: string) => {
  const matches = compileQuery(parseQuery(query), context);
  return indexed.filter(matches).map(({ row }) => row.id);
};

describe("the filter", () => {
  it("matches bare text against URL, vendor and bidder", () => {
    expect(ids("rubicon")).toEqual(["bid-ok"]);
    expect(ids("INDEX")).toEqual(["bid-late"]);
    expect(ids("openx")).toEqual(["bid-nobid"]);
    expect(ids("example.com static")).toEqual(["app", "cached"]);
  });

  it("matches a host by suffix, never by a fragment of a label", () => {
    expect(ids("host:example.com")).toHaveLength(8);
    expect(ids("host:cdn.example.com")).toEqual(["app", "cached"]);
    expect(ids("host:ample.com")).toEqual([]);
  });

  it("matches url, path, method, mime and initiator", () => {
    expect(ids("url:gdfp_req")).toEqual(["gam"]);
    expect(ids("path:/openrtb")).toEqual(["bid-late", "bid-nobid"]);
    expect(ids("path:gdfp_req")).toEqual([]);
    expect(ids("method:post")).toEqual(["bid-late", "bid-nobid", "beacon-blocked"]);
    expect(ids("mime:gif")).toEqual(["sync"]);
    expect(ids("initiator:pubads_impl")).toEqual(["gam"]);
  });

  it("matches a status or a class of statuses", () => {
    expect(ids("status:404")).toEqual(["img-404"]);
    expect(ids("status:3xx,4xx")).toEqual(["sync", "img-404"]);
  });

  it("matches types, with xhr covering fetch", () => {
    expect(ids("type:doc")).toEqual(["doc"]);
    expect(ids("type:fetch")).toEqual(["bid-nobid", "beacon-blocked", "sw", "poll", "p3", "p3-late"]);
    expect(ids("type:xhr cat:bid")).toEqual(["bid-ok", "bid-late", "bid-nobid"]);
  });

  it("matches category, vendor, bidder, ad unit and page", () => {
    expect(ids("cat:adserver")).toEqual(["gam"]);
    expect(ids('vendor:"index exchange"')).toEqual(["bid-late"]);
    expect(ids("bidder:ix")).toEqual(["bid-late"]);
    expect(ids("unit:div-gpt-side")).toEqual(["bid-nobid"]);
    expect(ids("page:2")).toEqual(["cached", "sw", "poll"]);
  });

  it("compares sizes and durations", () => {
    expect(ids("larger-than:500k")).toEqual(["app"]);
    expect(ids("slower-than:1s")).toEqual(["bid-late", "bid-nobid"]);
    expect(ids("slower-than:1s -is:late")).toEqual(["bid-nobid"]);
  });

  it("knows the states", () => {
    expect(ids("is:failed")).toEqual(["img-404"]);
    expect(ids("is:blocked,cancelled")).toEqual(["beacon-blocked"]);
    expect(ids("is:pending")).toEqual(["poll"]);
    expect(ids("is:late")).toEqual(["bid-late"]);
    expect(ids("is:nobid")).toEqual(["bid-nobid"]);
    expect(ids("is:won")).toEqual(["bid-ok"]);
    expect(ids("is:cached")).toEqual(["cached"]);
    expect(ids("is:sw")).toEqual(["sw"]);
    expect(ids("is:redirect")).toEqual(["sync"]);
    expect(ids("is:no-hb-targeting")).toEqual(["gam"]);
    expect(ids("is:bid is:post")).toEqual(["bid-late", "bid-nobid"]);
    expect(ids("is:first-party type:img")).toEqual(["img-404"]);
  });

  it("calls a request slow past a second, and a bid call past 80% of its auction's timeout", () => {
    expect(ids("is:slow")).toEqual(["bid-late", "bid-nobid"]);
    const nearTimeout = indexRow(makeRow({ category: "bid", durationMs: 1250, ad: makeAdLink() }));
    const quick = indexRow(makeRow({ category: "bid", durationMs: 900, ad: makeAdLink() }));
    const matches = compileQuery(parseQuery("is:slow"), context);
    expect(matches(nearTimeout)).toBe(true);
    expect(matches(quick)).toBe(false);
  });

  it("finds what started before a page's first auction, and everything on a page that never auctioned", () => {
    expect(ids("is:before-auction page:1")).toEqual(["doc", "app", "img-404"]);
    // Page 2 never auctioned: all of it counts.
    expect(ids("is:before-auction page:2")).toEqual(["cached", "sw", "poll"]);
    expect(ids("is:before-auction page:3")).toEqual(["p3"]);
  });

  it("finds an auction's own calls and what else was under way during it, by number or id", () => {
    expect(ids("auction:1")).toEqual(["bid-ok", "bid-late", "bid-nobid", "gam"]);
    expect(ids("auction:A1")).toEqual(ids("auction:1"));
    const matches = compileQuery(parseQuery("auction:1"), context);
    // Started before the auction and still loading when it began.
    expect(matches(indexRow(makeRow({ startedAtMs: 400, durationMs: 900 })))).toBe(true);
    expect(matches(indexRow(makeRow({ startedAtMs: 400, durationMs: null })))).toBe(true);
    expect(matches(indexRow(makeRow({ startedAtMs: 400, durationMs: 900, page: 1 })))).toBe(false);
    expect(ids("auction:2")).toEqual(["p3"]);
    expect(ids("auction:9")).toEqual([]);
  });

  it("applies every word, and negation to the word it is on", () => {
    expect(ids("cat:bid -bidder:ix,openx")).toEqual(["bid-ok"]);
    expect(ids("-host:example.com -cat:bid")).toEqual(["gam", "sync", "beacon-blocked"]);
  });

  it("applies a regular expression to the URL only", () => {
    expect(ids("/\\/api\\/(poll|more)/")).toEqual(["poll", "p3-late"]);
    expect(ids("/magnite/")).toEqual([]);
  });

  it("shows everything for an empty filter", () => {
    expect(ids("")).toHaveLength(rows.length);
  });
});

describe("chip counts", () => {
  it("counts categories under the rest of the filter, with the category itself left out", () => {
    const counts = categoryCounts(indexed, "cat:bid is:post", context);
    expect(counts.all).toBe(3);
    expect(counts.byCategory).toEqual({ bid: 2, measurement: 1 });
  });

  it("counts each state under the filter without that state", () => {
    expect(stateCounts(indexed, "", context)).toEqual({ failed: 1, slow: 2, blocked: 1 });
    expect(stateCounts(indexed, "cat:bid is:slow", context)).toEqual({ failed: 0, slow: 2, blocked: 0 });
    expect(stateCounts(indexed, "is:failed", context)).toEqual({ failed: 1, slow: 0, blocked: 0 });
  });
});

describe("editing the filter through chips", () => {
  it("adds and removes a state word", () => {
    const once = toggleToken("cat:bid", STATE_TOKENS.failed);
    expect(once).toBe("cat:bid is:failed");
    expect(hasToken(once, STATE_TOKENS.failed)).toBe(true);
    expect(toggleToken(once, STATE_TOKENS.failed)).toBe("cat:bid");
    expect(toggleToken("", STATE_TOKENS.blocked)).toBe("is:blocked,cancelled");
  });

  it("swaps the category and keeps the rest", () => {
    expect(withCategory("rubicon cat:bid is:late", "adServer")).toBe("rubicon is:late cat:adserver");
    expect(withCategory("rubicon cat:bid", null)).toBe("rubicon");
    expect(withCategory("", "site")).toBe("cat:site");
  });

  it("reads back the one category a filter narrows to", () => {
    expect(activeCategory("x cat:bid")).toBe("bid");
    expect(activeCategory("cat:bid,adserver")).toBeNull();
    expect(activeCategory("-cat:bid")).toBeNull();
    expect(activeCategory("x")).toBeNull();
  });
});

describe("summariseList", () => {
  it("says what is shown out of what", () => {
    const matches = compileQuery(parseQuery("cat:bid"), context);
    const summary = summariseList(indexed.filter(matches), indexed);
    expect(summary).toMatchObject({ shown: 3, total: rows.length, failed: 0, lateBids: 1, cachedShare: 0 });
    expect(summary.vendors.map((entry) => entry.vendor)).toEqual(["Index Exchange", "Magnite", "OpenX"]);
    expect(summary.shownBytes).toBe(36_000);
    expect(summary.totalBytes).toBeGreaterThan(600_000);
  });

  it("gives the share that did not come from the network", () => {
    const page2 = indexed.filter(({ row }) => row.page === 1);
    expect(summariseList(page2, indexed).cachedShare).toBeCloseTo(2 / 3);
    expect(summariseList([], indexed).cachedShare).toBe(0);
  });
});

describe("pushRecent", () => {
  it("puts the newest first, without duplicates or blanks, and keeps eight", () => {
    expect(pushRecent(["a", "b"], " b ")).toEqual(["b", "a"]);
    expect(pushRecent(["a"], "  ")).toEqual(["a"]);
    const many = Array.from({ length: 8 }, (_, index) => `q${index}`);
    expect(pushRecent(many, "new")).toEqual(["new", ...many.slice(0, 7)]);
  });
});
