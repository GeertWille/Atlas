import { describe, expect, it } from "vitest";
import type { Bid, BidExchange, DecodedBidResponse, NetworkRow } from "@/bindings";
import { makeAuction, makeBid } from "@/test/fixtures";
import { makeAdLink, makeRow } from "@/test/network-fixtures";
import { rowOutcome } from "../ad-outcome";
import { buildBidMatrix, type MatrixCell, summariseCell, summariseRequests } from "./bid-matrix";

const exchange = (overrides: Partial<BidExchange>): BidExchange => ({
  requestId: "r1",
  verdict: "inTime",
  lateByMs: -100,
  networkMs: 200,
  exact: true,
  ...overrides,
});
const bid = (bidder: string, overrides: Partial<Bid>) => makeBid({ bidder, cpm: null, status: "noBid", ...overrides });

const top = [
  bid("rubicon", {
    status: "timeout",
    responseTimeMs: null,
    exchange: exchange({ requestId: "r-rub", verdict: "lateOnNetwork", lateByMs: 84, networkMs: 334 }),
  }),
  bid("appnexus", { status: "won", cpm: 1.42, exchange: exchange({ requestId: "r-apn", networkMs: 188 }) }),
  bid("openx", { exchange: exchange({ requestId: "r-ox", networkMs: 212 }) }),
  bid("triplelift", { exchange: exchange({ requestId: null, verdict: "notSent", lateByMs: null, networkMs: null }) }),
  bid("pubmatic", { status: "bid", cpm: 0.88, exchange: exchange({ requestId: "r-pbs", networkMs: 301 }) }),
  bid("criteo", {
    status: "timeout",
    responseTimeMs: null,
    exchange: exchange({ requestId: "r-pbs", verdict: "inTimeButTimedOut", lateByMs: -20, networkMs: 301 }),
  }),
];
const side = [
  bid("rubicon", {
    status: "timeout",
    responseTimeMs: null,
    exchange: exchange({ requestId: "r-rub", verdict: "lateOnNetwork", lateByMs: 84, networkMs: 334 }),
  }),
  bid("appnexus", { exchange: exchange({ requestId: "r-apn", networkMs: 188 }) }),
  bid("ix", {
    status: "error",
    exchange: exchange({ requestId: "r-ix", verdict: "failed", lateByMs: null, networkMs: 41 }),
  }),
  bid("pubmatic", { exchange: exchange({ requestId: "r-pbs", networkMs: 301 }) }),
];
const auction = makeAuction([], {
  adUnits: [
    { code: "top", sizes: ["970x250"], bids: top, winningBidder: "appnexus", winningCpm: 1.42 },
    { code: "side", sizes: ["300x250"], bids: side, winningBidder: null, winningCpm: null },
  ],
});

const row = (id: string, overrides: Partial<NetworkRow>) =>
  makeRow({ id, category: "bid", ad: makeAdLink(), ...overrides });
const rows = new Map(
  [
    row("r-rub", { durationMs: 334 }),
    row("r-apn", { durationMs: 188 }),
    row("r-ox", { durationMs: 212, ad: makeAdLink({ nbr: 7 }) }),
    row("r-ix", { durationMs: 41, outcome: "failed", status: null }),
    row("r-pbs", { durationMs: 301, ad: makeAdLink({ viaServer: true }) }),
  ].map((entry) => [entry.id, entry]),
);
const preflights = new Map([
  ["r-apn", makeRow({ id: "pre", kind: "preflight", durationMs: 45, preflightOf: "r-apn" })],
]);
const serverResponse: DecodedBidResponse = {
  id: "x",
  currency: "USD",
  nbr: null,
  nbrMeaning: null,
  seats: [],
  serverTimings: [{ bidder: "pubmatic", ms: 140 }],
  serverErrors: [{ bidder: "criteo", code: 1, message: "timeout" }],
};
const matrix = buildBidMatrix(auction, { rows, preflights, serverResponses: new Map([["r-pbs", serverResponse]]) });
const [client, server] = matrix.sections;

describe("buildBidMatrix", () => {
  it("has a column per ad unit and puts the bidders with the best outcome first", () => {
    expect(matrix.adUnits.map((unit) => unit.code)).toEqual(["top", "side"]);
    expect(client?.bidders.map((bidder) => bidder.bidder)).toEqual([
      "appnexus",
      "openx",
      "triplelift",
      "rubicon",
      "ix",
    ]);
  });

  it("leaves the cell empty where a bidder was not asked for an ad unit", () => {
    const ix = client?.bidders.find((bidder) => bidder.bidder === "ix");
    expect(ix?.cells.map((cell) => cell?.bid.status ?? null)).toEqual([null, "error"]);
  });

  it("ties the cells that one request answers", () => {
    const rubicon = client?.bidders.find((bidder) => bidder.bidder === "rubicon");
    expect(rubicon?.requests).toMatchObject([{ id: "r-rub", cells: 2, preflightMs: null }]);
    expect(rubicon?.cells.map((cell) => cell?.requestId)).toEqual(["r-rub", "r-rub"]);
  });

  it("has no request for a bid Prebid never sent", () => {
    const triplelift = client?.bidders.find((bidder) => bidder.bidder === "triplelift");
    expect(triplelift?.requests).toEqual([]);
    expect(triplelift?.cells[0]).toMatchObject({ requestId: null, verdict: "notSent" });
  });

  it("brackets the bidders of one Prebid Server request", () => {
    expect(server).toMatchObject({ kind: "server", request: { id: "r-pbs", cells: 3 } });
    expect(server?.bidders.map((bidder) => bidder.bidder)).toEqual(["pubmatic", "criteo"]);
    const [pubmatic, criteo] = server?.bidders ?? [];
    expect(pubmatic?.cells[0]).toMatchObject({ serverMs: 140, serverError: null });
    expect(criteo?.cells[0]).toMatchObject({ serverMs: null, serverError: "timeout" });
  });

  it("takes the no-bid reason from the request's answer", () => {
    const openx = client?.bidders.find((bidder) => bidder.bidder === "openx");
    expect(openx?.cells[0]?.nbr).toBe(7);
  });

  it("stands the best of several bids for one cell", () => {
    const twice = makeAuction([bid("a", {}), bid("a", { status: "bid", cpm: 2 })]);
    const [section] = buildBidMatrix(twice, {
      rows: new Map(),
      preflights: new Map(),
      serverResponses: new Map(),
    }).sections;
    expect(section?.bidders[0]?.cells[0]?.bid.cpm).toBe(2);
  });

  it("is one client section without a network record", () => {
    const plain = buildBidMatrix(makeAuction([makeBid()]), {
      rows: new Map(),
      preflights: new Map(),
      serverResponses: new Map(),
    });
    expect(plain.sections).toMatchObject([{ kind: "client", bidders: [{ bidder: "appnexus", requests: [] }] }]);
    expect(plain.sections[0]?.bidders[0]?.cells[0]).toMatchObject({ requestId: null, verdict: null });
  });
});

describe("summariseCell", () => {
  const cellOf = (bidder: string, unit = 0): MatrixCell => {
    const all = [...(client?.bidders ?? []), ...(server?.bidders ?? [])];
    return all.find((entry) => entry.bidder === bidder)?.cells[unit] as MatrixCell;
  };

  it("gives a price and the network time for a bid", () => {
    expect(summariseCell(cellOf("appnexus"), false)).toEqual({ primary: "$1.42", secondary: "188 ms" });
  });

  it("names the no-bid reason when the exchange gave one", () => {
    expect(summariseCell(cellOf("openx"), false)).toEqual({ primary: "no-bid 7", secondary: "212 ms" });
    expect(summariseCell(cellOf("appnexus", 1), false)).toEqual({ primary: "no bid", secondary: "188 ms" });
  });

  it("lets the network's verdict speak for a late, failed or unsent bid", () => {
    expect(summariseCell(cellOf("rubicon"), false)).toEqual({ primary: "late +84 ms", secondary: "334 ms" });
    expect(summariseCell(cellOf("ix", 1), false)).toEqual({ primary: "failed", secondary: "41 ms" });
    expect(summariseCell(cellOf("triplelift"), false)).toEqual({ primary: "not sent", secondary: null });
  });

  it("shows the server-side time and error for a Prebid Server bidder", () => {
    expect(summariseCell(cellOf("pubmatic"), true)).toEqual({ primary: "$0.88", secondary: "server 140 ms" });
    expect(summariseCell(cellOf("criteo"), true)).toEqual({ primary: "server timeout", secondary: null });
  });

  it("gives Prebid's own record and calls the request unknown when none could be tied to the bid", () => {
    const timedOut = { ...cellOf("rubicon"), requestId: null, verdict: null, lateByMs: null, networkMs: null };
    expect(summariseCell(timedOut, false)).toEqual({ primary: "timeout", secondary: "request unknown" });
    const priced = { ...cellOf("appnexus"), requestId: null, verdict: null, lateByMs: null, networkMs: null };
    expect(summariseCell(priced, false)).toEqual({ primary: "$1.42", secondary: "request unknown" });
  });
});

describe("summariseRequests", () => {
  const of = (name: string) =>
    client?.bidders.find((bidder) => bidder.bidder === name) as NonNullable<typeof client>["bidders"][number];

  it("says how a bidder's cells were asked for", () => {
    expect(summariseRequests(of("rubicon"), false)).toBe("1 request for 2 ad units, 334 ms");
    expect(summariseRequests(of("appnexus"), false)).toBe("1 request for 2 ad units, 188 ms, preflight +45 ms");
    expect(summariseRequests(of("openx"), false)).toBe("1 request, 212 ms");
    expect(summariseRequests(of("ix"), false)).toBe("failed, 41 ms");
    expect(summariseRequests(of("triplelift"), false)).toBe("no request");
    expect(summariseRequests(server?.bidders[0] as NonNullable<typeof server>["bidders"][number], true)).toBe("shared");
  });
});

describe("rowOutcome", () => {
  it("is empty for requests that are no ad call", () => {
    expect(rowOutcome(makeRow())).toBeNull();
    expect(rowOutcome(makeRow({ category: "bid", ad: null }))).toBeNull();
  });

  it("gives the price, the no-bid reason or the verdict of a bid call", () => {
    const of = (ad: Parameters<typeof makeAdLink>[0]) => rowOutcome(makeRow({ category: "bid", ad: makeAdLink(ad) }));
    expect(of({ cpm: 1.42 })).toEqual({ kind: "bid", text: "$1.42" });
    expect(of({ nbr: 7 })).toEqual({ kind: "noBid", text: "no-bid 7" });
    expect(of({})).toEqual({ kind: "noBid", text: "no bid" });
    expect(of({ verdict: "lateOnNetwork", lateByMs: 84 })).toEqual({ kind: "late", text: "late +84 ms" });
    expect(of({ verdict: "atDeadline", lateByMs: null })).toEqual({ kind: "atDeadline", text: "at deadline" });
    expect(of({ verdict: "inTimeButTimedOut" })).toEqual({ kind: "timedOutInPage", text: "timed out in page" });
    expect(of({ verdict: "neverAnswered" })).toEqual({ kind: "noAnswer", text: "no answer" });
    expect(of({ verdict: "failed" })).toEqual({ kind: "failed", text: "failed" });
  });

  it("says whether an ad-server call carried Prebid's keys", () => {
    expect(rowOutcome(makeRow({ category: "adServer", noHbTargeting: true }))).toEqual({
      kind: "noHbKeys",
      text: "no hb keys",
    });
    expect(rowOutcome(makeRow({ category: "adServer", ad: makeAdLink() }))).toEqual({
      kind: "hbKeys",
      text: "hb keys",
    });
  });
});

const serverResponses = new Map([["r-pbs", serverResponse]]);

describe("columns", () => {
  // As a scan stored before ad units were merged by code holds them: one entry per registered ad unit.
  const twins = makeAuction([], {
    adUnits: [
      { code: "banner", sizes: ["980x150", "970x250"], bids: top, winningBidder: "appnexus", winningCpm: 1.42 },
      { code: "banner", sizes: ["970x250", "840x150"], bids: [], winningBidder: null, winningCpm: null },
      { code: "rect", sizes: ["300x600"], bids: side, winningBidder: null, winningCpm: null },
      { code: "banner", sizes: ["728x90"], bids: [], winningBidder: null, winningCpm: null },
    ],
  });

  it("gives every bidder one cell per column, whatever the ad unit list holds", () => {
    for (const subject of [auction, twins]) {
      const matrix = buildBidMatrix(subject, { rows, preflights, serverResponses });
      expect(matrix.adUnits.map((unit) => unit.code)).toEqual(subject.adUnits.map((unit) => unit.code));
      const bidders = matrix.sections.flatMap((section) => section.bidders);
      expect(bidders.length).toBeGreaterThan(0);
      for (const bidder of bidders) expect(bidder.cells, bidder.bidder).toHaveLength(matrix.adUnits.length);
    }
  });
});

describe("currencies", () => {
  const priced = makeAuction([], {
    adUnits: [
      {
        code: "banner",
        sizes: ["970x250"],
        bids: [
          bid("gumgum", { status: "won", cpm: 10.01, currency: "EUR", exchange: exchange({ requestId: "r-gg" }) }),
          bid("rubicon", { status: "bid", cpm: 2.05, currency: "USD", exchange: exchange({ requestId: "r-rub" }) }),
          bid("appnexus", { exchange: exchange({ requestId: "r-apn" }) }),
        ],
        winningBidder: "gumgum",
        winningCpm: 10.01,
      },
    ],
  });
  const primaryOf = (subject: typeof priced, bidder: string) => {
    const matrix = buildBidMatrix(subject, { rows, preflights, serverResponses });
    const cell = matrix.sections.flatMap((section) => section.bidders).find((row) => row.bidder === bidder)?.cells[0];
    return summariseCell(cell as MatrixCell, false, matrix.currencies.length > 1).primary;
  };

  it("names the currency of every price once an auction has bids in two", () => {
    expect(buildBidMatrix(priced, { rows, preflights, serverResponses }).currencies).toEqual(["EUR", "USD"]);
    expect(primaryOf(priced, "gumgum")).toBe("10.01 EUR");
    expect(primaryOf(priced, "rubicon")).toBe("2.05 USD");
    expect(primaryOf(priced, "appnexus")).toBe("no bid");
  });

  it("keeps the short form while all prices are in one currency", () => {
    expect(buildBidMatrix(auction, { rows, preflights, serverResponses }).currencies).toEqual(["USD"]);
    expect(primaryOf(auction, "appnexus")).toBe("$1.42");
  });
});
