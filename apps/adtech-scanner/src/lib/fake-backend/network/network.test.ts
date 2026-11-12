import { beforeEach, describe, expect, it } from "vitest";
import type { NetworkRow, ScanDetail } from "@/bindings";
import type { ApiPort } from "@/lib/api/port";
import { createFakeBackend } from "..";

const DESKTOP = "seed-sportsdesk-desktop";
const MOBILE = "seed-sportsdesk-mobile";
const BROWSE = "seed-sportsdesk-browse";
const EVICTED = "seed-techpulse-desktop";
const DISABLED = "seed-morningledger-desktop";
const LEGACY = "seed-morningledger-mobile";

async function allRows(api: ApiPort, scanId: string): Promise<NetworkRow[]> {
  const rows: NetworkRow[] = [];
  let cursor: string | undefined;
  do {
    const chunk = await api.listNetworkRequests(scanId, { cursor });
    rows.push(...chunk.rows);
    cursor = chunk.nextCursor ?? undefined;
  } while (cursor);
  return rows;
}

const bidsOf = (detail: ScanDetail) =>
  detail.auctions.flatMap((auction) =>
    auction.adUnits.flatMap((unit) => unit.bids.map((bid) => ({ auction, unit, bid }))),
  );

describe("the simulated network record", () => {
  let api: ApiPort;

  beforeEach(() => {
    localStorage.clear();
    api = createFakeBackend({ profile: "seeded", speed: 1000 });
  });

  it("gives the three-page visit about 400 requests, most of them on the entry page", async () => {
    const rows = await allRows(api, BROWSE);
    const perPage = [0, 1, 2].map((page) => rows.filter((row) => row.page === page).length);
    expect(rows.length).toBeGreaterThan(360);
    expect(rows.length).toBeLessThan(440);
    expect(perPage[0]).toBeGreaterThan(170);
    expect(perPage[0]).toBeLessThan(215);
    expect(perPage[1]).toBeGreaterThan(110);
    expect(perPage[1]).toBeLessThan(150);
    expect(perPage[2]).toBeGreaterThan(60);
    expect(perPage[2]).toBeLessThan(100);
  });

  it("orders the list by page view, start time and sequence, with unique ids", async () => {
    const rows = await allRows(api, BROWSE);
    for (const [index, row] of rows.entries()) {
      const previous = rows[index - 1];
      if (!previous) continue;
      const ordered =
        previous.page < row.page ||
        (previous.page === row.page &&
          (previous.startedAtMs < row.startedAtMs ||
            (previous.startedAtMs === row.startedAtMs && previous.seq < row.seq)));
      expect(ordered, `${previous.id} before ${row.id}`).toBe(true);
    }
    expect(new Set(rows.map((row) => row.id)).size).toBe(rows.length);
    expect(rows.every((row) => row.id === `r${row.seq}`)).toBe(true);
  });

  it("serves the list in chunks that add up", async () => {
    const first = await api.listNetworkRequests(BROWSE, { limit: 150 });
    expect(first.rows).toHaveLength(150);
    expect(first.nextCursor).not.toBeNull();
    const second = await api.listNetworkRequests(BROWSE, { limit: 150, cursor: first.nextCursor ?? undefined });
    expect(second.rows[0]?.id).not.toBe(first.rows[0]?.id);
    expect(first.total).toBe((await allRows(api, BROWSE)).length);
    const onePage = await api.listNetworkRequests(BROWSE, { pages: [2] });
    expect(onePage.rows.every((row) => row.page === 2)).toBe(true);
    expect(onePage.total).toBe(onePage.rows.length);
  });

  it("links every record of the report to a request that fits it", async () => {
    for (const scanId of [DESKTOP, MOBILE, BROWSE]) {
      const detail = await api.getScan(scanId);
      const rows = new Map((await allRows(api, scanId)).map((row) => [row.id, row]));
      for (const { auction, unit, bid } of bidsOf(detail)) {
        expect(bid.exchange, `${scanId} ${bid.bidder}`).not.toBeNull();
        const requestId = bid.exchange?.requestId;
        if (requestId === null || requestId === undefined) {
          expect(bid.exchange?.verdict).toBe("notSent");
          continue;
        }
        const row = rows.get(requestId);
        expect(row?.category).toBe("bid");
        expect(row?.ad?.auctionId).toBe(auction.id);
        expect(row?.ad?.adUnits).toContain(unit.code);
        expect(row?.page).toBe(auction.page);
      }
      for (const slot of detail.slots) {
        for (const request of slot.requests) {
          const row = rows.get(request.requestId ?? "");
          expect(row?.category).toBe("adServer");
          expect(row?.startedAtMs).toBe(request.atMs);
          expect(row?.noHbTargeting).toBe(!request.hadPrebidTargeting);
        }
      }
      for (const script of detail.scripts) {
        if (script.inline) expect(script.requestId).toBeNull();
        else expect(rows.get(script.requestId ?? "")?.url).toBe(script.url);
      }
      for (const row of rows.values()) {
        if (row.redirectOf) expect(rows.has(row.redirectOf)).toBe(true);
        if (row.preflightOf) expect(rows.has(row.preflightOf)).toBe(true);
      }
      const checkIds = (detail.bidderDiagnostics?.bidders ?? []).flatMap((bidder) =>
        bidder.checks.flatMap((check) => check.requestIds),
      );
      expect(checkIds.length).toBeGreaterThan(0);
      expect(checkIds.every((id) => rows.has(id))).toBe(true);
      const evidenceIds = detail.findings.flatMap((finding) =>
        finding.evidence.flatMap((evidence) => evidence.requestId ?? []),
      );
      expect(evidenceIds.every((id) => rows.get(id)?.category === "adServer")).toBe(true);
    }
  });

  it("points the evidence of the early ad request at that request", async () => {
    const detail = await api.getScan(DESKTOP);
    const early = detail.findings.find((finding) => finding.title.startsWith("Ad server request left before"));
    const requestId = early?.evidence[0]?.requestId;
    expect(requestId).toBeTruthy();
    const request = await api.getNetworkRequest(DESKTOP, requestId ?? "");
    expect(request.row.noHbTargeting).toBe(true);
    // An ad-server call carries no auction link of its own: its slot names it.
    expect(request.row.ad).toBeNull();
    expect(request.adContext).toBeNull();
    const slot = detail.slots.find((candidate) => candidate.elementId === "div-gpt-top-leaderboard");
    expect(slot?.requests[0]?.requestId).toBe(requestId);
  });

  it("carries the cases a network view has to show", async () => {
    const rows = await allRows(api, BROWSE);
    const some = (predicate: (row: NetworkRow) => boolean) => rows.some(predicate);
    expect(some((row) => row.kind === "document" && row.status === 301)).toBe(true);
    expect(some((row) => row.kind === "document" && row.redirectOf !== null)).toBe(true);
    expect(some((row) => row.kind === "preflight" && row.preflightOf !== null)).toBe(true);
    expect(some((row) => row.ad?.viaServer === true)).toBe(true);
    expect(some((row) => row.ad?.verdict === "lateOnNetwork" && row.ad.lateByMs === 84)).toBe(true);
    expect(some((row) => row.ad?.verdict === "inTimeButTimedOut")).toBe(true);
    expect(some((row) => row.ad?.nbr === 5)).toBe(true);
    expect(some((row) => row.category === "adServer" && row.noHbTargeting)).toBe(true);
    expect(some((row) => row.outcome === "pending")).toBe(true);
    expect(some((row) => row.outcome === "cancelled")).toBe(true);
    expect(some((row) => row.outcome === "blocked")).toBe(true);
    expect(some((row) => row.outcome === "httpError")).toBe(true);
    expect(some((row) => row.servedFrom === "diskCache" && row.transferredBytes === 0)).toBe(true);
    expect(some((row) => row.servedFrom === "serviceWorker")).toBe(true);
    expect(some((row) => row.kind === "webSocket")).toBe(true);
    expect(some((row) => row.context === "iframe")).toBe(true);
    // A chain of three: two redirects and where they land.
    const landing = rows.find((row) => row.host === "cm.g.doubleclick.net");
    const chain = (await api.getNetworkRequest(BROWSE, landing?.id ?? "")).redirectChain;
    expect(chain.map((hop) => hop.status)).toEqual([302, 302, 200]);
    // The page that never auctioned asks the ad server without Prebid keys, six times.
    expect(rows.filter((row) => row.page === 1 && row.category === "adServer" && row.noHbTargeting)).toHaveLength(6);
  });

  it("has a failed bid call and an exchange that never answers on the phone", async () => {
    const rows = await allRows(api, MOBILE);
    expect(rows.some((row) => row.category === "bid" && row.outcome === "failed")).toBe(true);
    expect(rows.some((row) => row.ad?.verdict === "neverAnswered" && row.durationMs === null)).toBe(true);
    expect(rows.some((row) => row.ad?.nbr === 7)).toBe(true);
  });

  it("marks the bid Prebid never sent, and says why in the bidder checks", async () => {
    const detail = await api.getScan(DESKTOP);
    const unsent = bidsOf(detail).filter(({ bid }) => bid.exchange?.verdict === "notSent");
    expect(unsent.map(({ bid }) => bid.bidder)).toEqual(["triplelift"]);
    const checks = detail.bidderDiagnostics?.bidders.find((bidder) => bidder.bidder === "triplelift")?.checks ?? [];
    expect(checks[0]).toMatchObject({ area: "params", status: "problem", adUnits: ["div-gpt-incontent-1"] });
  });

  it("decodes OpenRTB exchanges and leaves the others raw", async () => {
    const detail = await api.getScan(DESKTOP);
    const rows = await allRows(api, DESKTOP);
    const sovrn = rows.find((row) => row.bidder === "sovrn");
    const declined = await api.getNetworkRequest(DESKTOP, sovrn?.id ?? "");
    expect(declined.decoded?.response).toMatchObject({ nbr: 5, nbrMeaning: "Cloud, Data Center, or Proxy IP" });
    expect(declined.adContext?.checkCodes).toContain("response-no-bid-reason");
    const body = await api.getNetworkBody(DESKTOP, sovrn?.id ?? "", { side: "response" });
    expect(JSON.parse(body.text ?? "")).toMatchObject({ nbr: 5 });

    const appnexus = rows.find((row) => row.bidder === "appnexus" && row.method === "POST");
    const proprietary = await api.getNetworkRequest(DESKTOP, appnexus?.id ?? "");
    expect(proprietary.decoded).toBeNull();
    expect(proprietary.preflightMs).toBe(45);
    expect(proprietary.stack.at(-1)?.scriptId).toBe(
      detail.scripts.find((script) => script.url.includes("hb-wrapper"))?.id,
    );

    const server = rows.find((row) => row.ad?.viaServer);
    const fanOut = await api.getNetworkRequest(DESKTOP, server?.id ?? "");
    expect(fanOut.decoded?.protocol).toBe("prebidServer");
    expect(fanOut.decoded?.response?.serverTimings.map((timing) => timing.bidder).sort()).toEqual([
      "criteo",
      "pubmatic",
    ]);
  });

  it("redacts cookie and authorization values, and says so", async () => {
    const rows = await allRows(api, DESKTOP);
    const session = rows.find((row) => row.url.endsWith("/api/session"));
    const { requestHeaders } = await api.getNetworkRequest(DESKTOP, session?.id ?? "");
    const cookie = requestHeaders.find((header) => header.name === "Cookie");
    expect(cookie?.redaction).toBe("cookie");
    expect(cookie?.value).toMatch(/^_ga=\[\d+ bytes\]; _sd_session=\[\d+ bytes\]/);
    const authorization = requestHeaders.find((header) => header.name === "Authorization");
    expect(authorization).toMatchObject({ redaction: "credential" });
    expect(authorization?.value).toMatch(/^Bearer \[\d+ bytes\]$/);
    expect((await api.getScan(DESKTOP)).network?.headersRedacted).toBe(true);
  });

  it("states what became of each body", async () => {
    const rows = await allRows(api, DESKTOP);
    const state = async (match: (row: NetworkRow) => boolean) => {
      const row = rows.find(match);
      return (await api.getNetworkRequest(DESKTOP, row?.id ?? "")).responseBody;
    };
    expect(await state((row) => row.url.includes("archive.json"))).toMatchObject({ state: "kept" });
    expect(await state((row) => row.url.includes("full-season.json"))).toMatchObject({
      state: "truncated",
      bytes: 7_759_462,
      storedBytes: 5 * 1024 * 1024,
    });
    expect(await state((row) => row.url.includes("standings.json"))).toMatchObject({
      state: "dropped",
      storedBytes: 0,
    });
    expect(await state((row) => row.kind === "font")).toMatchObject({ state: "notKeptType" });
    expect(await state((row) => row.outcome === "pending" && row.category === "site")).toMatchObject({
      state: "pending",
    });
    expect(await state((row) => row.url.endsWith("/metrics/vitals"))).toMatchObject({ state: "empty" });
    expect(await state((row) => row.status === 301)).toMatchObject({ state: "unavailable" });
    for (const row of rows.filter((candidate) => candidate.hasResponseBody).slice(0, 40)) {
      const chunk = await api.getNetworkBody(DESKTOP, row.id, { side: "response", maxBytes: 64 });
      expect(["kept", "truncated"]).toContain(chunk.state);
      expect(chunk.text).not.toBeNull();
    }
  });

  it("reads a large body in stretches", async () => {
    const rows = await allRows(api, DESKTOP);
    const archive = rows.find((row) => row.url.includes("archive.json"));
    const first = await api.getNetworkBody(DESKTOP, archive?.id ?? "", { side: "response" });
    expect(first.text).toHaveLength(2 * 1024 * 1024);
    expect(first.nextOffset).toBe(2 * 1024 * 1024);
    const second = await api.getNetworkBody(DESKTOP, archive?.id ?? "", {
      side: "response",
      offset: first.nextOffset ?? 0,
    });
    expect(second.nextOffset).toBeNull();
    expect(JSON.parse(`${first.text}${second.text}`).fixtures.length).toBeGreaterThan(1000);
  });

  it("serves a script's captured text as the body of its request", async () => {
    const detail = await api.getScan(DESKTOP);
    const wrapper = detail.scripts.find((script) => script.url.includes("hb-wrapper"));
    const request = await api.getNetworkRequest(DESKTOP, wrapper?.requestId ?? "");
    expect(request.responseBody.scriptId).toBe(wrapper?.id);
    const body = await api.getNetworkBody(DESKTOP, wrapper?.requestId ?? "", { side: "response" });
    const source = await api.getScriptSource(DESKTOP, wrapper?.id ?? "", "original");
    expect(body.text).toBe(source.text);
  });

  it("keeps a comment that tries to inject markup exactly as it was sent", async () => {
    const rows = await allRows(api, DESKTOP);
    const comments = rows.find((row) => row.url.includes("/comments/latest.json"));
    const body = await api.getNetworkBody(DESKTOP, comments?.id ?? "", { side: "response" });
    expect(body.text).toContain("<img src=x onerror=");
    expect(body.text).toContain("‮");
  });

  it("summarises the record for the digest", async () => {
    const summary = await api.getNetworkSummary(BROWSE);
    const rows = await allRows(api, BROWSE);
    expect(summary.archive.availability).toBe("available");
    expect(summary.totalRequests).toBe(rows.length);
    expect(summary.categories.reduce((sum, category) => sum + category.requests, 0)).toBe(rows.length);
    expect(summary.pages.map((page) => page.page)).toEqual([0, 1, 2]);
    expect(summary.lateBids).toBe(1);
    expect(summary.failed).toBeGreaterThan(2);
    expect(summary.vendors[0]?.vendor).toBe("Google");
    expect(summary.notable.length).toBeGreaterThan(3);
    expect(summary.notable.length).toBeLessThanOrEqual(8);
    expect(summary.notable.some((row) => row.ad?.verdict === "lateOnNetwork")).toBe(true);
  });

  it("is honest about scans without a record", async () => {
    for (const [scanId, availability] of [
      [DISABLED, "disabled"],
      [LEGACY, "notRecorded"],
    ] as const) {
      const summary = await api.getNetworkSummary(scanId);
      expect(summary.archive.availability).toBe(availability);
      expect(summary.totalRequests).toBe(0);
      expect(await api.listNetworkRequests(scanId)).toEqual({ rows: [], nextCursor: null, total: 0 });
      await expect(api.getNetworkRequest(scanId, "r1")).rejects.toMatchObject({ code: "notFound" });
    }
    expect((await api.getScan(LEGACY)).network).toBeNull();
    // Bid calls are read as they pass even when nothing is kept: the verdict is there, the request is not.
    const unkept = bidsOf(await api.getScan(DISABLED));
    expect(unkept.every(({ bid }) => bid.exchange !== null && bid.exchange.requestId === null)).toBe(true);
    expect(unkept.some(({ bid }) => bid.exchange?.verdict === "inTime")).toBe(true);
    expect(bidsOf(await api.getScan(LEGACY)).every(({ bid }) => bid.exchange === null)).toBe(true);
  });

  it("keeps the list of an evicted scan, without query strings, headers or bodies", async () => {
    const rows = await allRows(api, EVICTED);
    expect(rows.length).toBeGreaterThan(150);
    expect(rows.every((row) => !row.url.includes("?"))).toBe(true);
    expect(rows.every((row) => !row.hasResponseBody && !row.hasRequestBody && row.phases === null)).toBe(true);
    const bid = rows.find((row) => row.category === "bid" && row.method === "POST");
    const detail = await api.getNetworkRequest(EVICTED, bid?.id ?? "");
    expect(detail).toMatchObject({ available: false, requestHeaders: [], decoded: null });
    expect(detail.responseBody.state).toBe("evicted");
    expect((await api.getNetworkBody(EVICTED, bid?.id ?? "", { side: "response" })).text).toBeNull();
  });

  it("removes network data on request and leaves the reports", async () => {
    const before = await api.getArchiveStorage();
    expect(before.scans).toBe(3);
    expect(before.evictedScans).toBe(1);
    expect(before.storedBytes).toBeGreaterThan(1_000_000);
    const after = await api.clearArchive();
    expect(after).toMatchObject({ scans: 0, storedBytes: 0, oldestAt: null });
    const detail = await api.getScan(DESKTOP);
    expect(detail.findings.length).toBeGreaterThan(0);
    expect(detail.network).toMatchObject({ availability: "evicted", requestCount: 0 });
    expect((await api.listNetworkRequests(DESKTOP)).total).toBe(0);
    expect((await api.getScriptSource(DESKTOP, "s7", "formatted")).availability).toBe("evicted");
    // It stays removed after a reload.
    const reloaded = createFakeBackend({ profile: "seeded", speed: 1000 });
    expect((await reloaded.getScan(DESKTOP)).network?.availability).toBe("evicted");
  });

  it("records nothing for a scan made with recording off, and full headers when asked", async () => {
    await api.updateSettings({ storeFullHeaders: true });
    await expect(api.updateSettings({ archiveLimitBytes: 1024 })).rejects.toMatchObject({ code: "invalidInput" });
  });
});

describe("padded request lists", () => {
  it.each([3000, 10_000])("reach %i rows without renumbering the visit's own requests", async (rows) => {
    localStorage.clear();
    const plain = createFakeBackend({ profile: "seeded", speed: 1000 });
    const padded = createFakeBackend({ profile: "seeded", speed: 1000, rows });
    const list = await allRows(padded, DESKTOP);
    expect(list).toHaveLength(rows);
    const own = await allRows(plain, DESKTOP);
    const byId = new Map(list.map((row) => [row.id, row]));
    expect(own.every((row) => byId.get(row.id)?.url === row.url)).toBe(true);
    expect((await padded.getScan(DESKTOP)).auctions[0]?.adUnits[0]?.bids[0]?.exchange?.requestId).toBe(
      (await plain.getScan(DESKTOP)).auctions[0]?.adUnits[0]?.bids[0]?.exchange?.requestId,
    );
  });
});
