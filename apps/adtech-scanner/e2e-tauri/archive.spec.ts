import { afterAll, beforeAll, expect, test } from "vitest";
import type {
  ArchiveStorage,
  NetworkBodyChunk,
  NetworkRequestDetail,
  NetworkRow,
  NetworkRowChunk,
  NetworkSummary,
  ScanDetail,
  ScanSummary,
  ScriptSource,
  Settings,
} from "../src/bindings";
import { type App, goTo, invokeOk, launchApp, seeText, textOf } from "./support/app";
import { cspViolations, watchCspViolations } from "./support/csp";
import { AI_FINDING_TITLE, MOCK_API_KEY, type MockAnthropic, startMockAnthropic } from "./support/mock-anthropic";

const CODE = '//*[@data-testid="code"]';
const MAIN = "//main";

let api: MockAnthropic;
let app: App;
let scan: ScanDetail;
let rows: NetworkRow[] = [];

async function scanDemoSite(aiEnabled: boolean): Promise<ScanDetail> {
  const { browser } = app;
  const url = await invokeOk<string>(browser, "get_demo_url");
  const started = await invokeOk<ScanSummary>(browser, "start_scan", {
    options: { url, device: "desktop", observeSeconds: 6, autoScroll: true, acceptConsent: true, aiEnabled },
  });
  let detail = await invokeOk<ScanDetail>(browser, "get_scan", { scanId: started.id });
  await browser.waitUntil(
    async () => {
      detail = await invokeOk<ScanDetail>(browser, "get_scan", { scanId: started.id });
      return !["queued", "running"].includes(detail.summary.status);
    },
    { timeout: 120_000, interval: 500 },
  );
  expect(detail.summary).toMatchObject({ status: "completed", error: null });
  return detail;
}

const bidsOf = (detail: ScanDetail) =>
  detail.auctions.flatMap((auction) =>
    auction.adUnits.flatMap((unit) => unit.bids.map((bid) => ({ auction: auction.id, unit: unit.code, ...bid }))),
  );

beforeAll(async () => {
  api = await startMockAnthropic();
  app = await launchApp({ ADSCAN_ANTHROPIC_BASE_URL: api.baseUrl, ADSCAN_API_KEY: MOCK_API_KEY });
  await watchCspViolations(app.browser);
  scan = await scanDemoSite(true);
  const chunk = await invokeOk<NetworkRowChunk>(app.browser, "list_network_requests", { scanId: scan.summary.id });
  rows = chunk.rows;
}, 180_000);

afterAll(async () => {
  await app?.close();
  await api?.close();
});

test("a demo scan keeps its network record next to the report", async () => {
  const { browser } = app;
  expect(scan.network).toMatchObject({ availability: "available", headersRedacted: true, truncated: false });
  expect(scan.network?.requestCount).toBe(rows.length);
  expect(rows.length).toBeGreaterThan(20);
  const starts = rows.map((row) => row.startedAtMs);
  expect(starts).toEqual([...starts].sort((a, b) => a - b));

  // The list comes in chunks that add up to the same rows.
  const first = await invokeOk<NetworkRowChunk>(browser, "list_network_requests", {
    scanId: scan.summary.id,
    limit: 10,
  });
  const second = await invokeOk<NetworkRowChunk>(browser, "list_network_requests", {
    scanId: scan.summary.id,
    cursor: first.nextCursor,
    limit: 10,
  });
  expect([...first.rows, ...second.rows].map((row) => row.id)).toEqual(rows.slice(0, 20).map((row) => row.id));
  expect(first.total).toBe(rows.length);

  const summary = await invokeOk<NetworkSummary>(browser, "get_network_summary", { scanId: scan.summary.id });
  expect(summary.totalRequests).toBe(rows.length);
  expect(summary.categories.find((count) => count.category === "bid")?.requests).toBeGreaterThan(0);

  // An image has a row and a size, and no body.
  const image = rows.find((row) => row.kind === "image");
  expect(image).toMatchObject({ hasResponseBody: false });
  const pixels = await invokeOk<NetworkBodyChunk>(browser, "get_network_body", {
    scanId: scan.summary.id,
    requestId: image?.id,
    side: "response",
  });
  expect(pixels).toMatchObject({ state: "notKeptType", text: null });

  const storage = await invokeOk<ArchiveStorage>(browser, "get_archive_storage");
  expect(storage).toMatchObject({ scans: 1, evictedScans: 0, storedBytes: scan.network?.storedBytes });
});

test("the openx exchange opens with its request and the decoded no-bid reason 7", async () => {
  const { browser } = app;
  const openx = rows.find((row) => row.bidder === "openx" && row.kind !== "preflight");
  expect(openx).toMatchObject({ category: "bid", method: "POST", hasRequestBody: true, hasResponseBody: true });
  expect(openx?.ad).toMatchObject({ nbr: 7, cpm: null });

  const opened = await invokeOk<NetworkRequestDetail>(browser, "get_network_request", {
    scanId: scan.summary.id,
    requestId: openx?.id,
  });
  expect(opened.available).toBe(true);
  expect(opened.decoded?.protocol).toBe("openRtb");
  expect(opened.decoded?.response).toMatchObject({ nbr: 7, nbrMeaning: "Blocked Publisher or Site", seats: [] });
  expect(opened.decoded?.request?.imps.length).toBeGreaterThan(0);
  expect(opened.adContext?.bidder).toBe("openx");

  const answer = await invokeOk<NetworkBodyChunk>(browser, "get_network_body", {
    scanId: scan.summary.id,
    requestId: openx?.id,
    side: "response",
  });
  expect(answer.state).toBe("kept");
  expect(JSON.parse(answer.text ?? "")).toMatchObject({ nbr: 7 });

  // The bidder check that reported the reason points at this request.
  const check = scan.bidderDiagnostics?.bidders
    .find((diagnosis) => diagnosis.bidder === "openx")
    ?.checks.find((candidate) => candidate.code === "response-reason");
  expect(check?.requestIds).toContain(openx?.id);
});

test("under the 250 ms timeout a bid is late or never answered, and triplelift's incomplete bids were not sent", async () => {
  const bids = bidsOf(scan).filter((bid) => bid.exchange);
  expect(scan.auctions[0]?.timeoutMs).toBe(250);

  const missed = bids.filter((bid) => ["lateOnNetwork", "neverAnswered"].includes(bid.exchange?.verdict ?? ""));
  expect(missed.length).toBeGreaterThan(0);
  for (const bid of missed) {
    const row = rows.find((candidate) => candidate.id === bid.exchange?.requestId);
    expect(row?.ad?.auctionId).toBe(bid.auction);
    if (bid.exchange?.verdict === "lateOnNetwork") expect(bid.exchange.lateByMs).toBeGreaterThan(10);
    else expect(row?.outcome).toBe("pending");
  }

  const triplelift = bids.filter((bid) => bid.bidder === "triplelift");
  const notSent = triplelift.filter((bid) => bid.exchange?.verdict === "notSent");
  expect(notSent.length).toBeGreaterThan(0);
  expect(notSent.every((bid) => bid.exchange?.requestId === null)).toBe(true);
  // The one bid with an inventory code did leave the page.
  expect(triplelift.some((bid) => bid.exchange?.requestId)).toBe(true);
});

test("the network view opens the openx exchange with no-bid reason 7 decoded to words", async () => {
  const { browser } = app;
  const openx = rows.find((row) => row.bidder === "openx" && row.kind !== "preflight");
  await goTo(browser, `/scans/${scan.summary.id}/network?req=${openx?.id}&dt=response`);
  const decoded = await seeText(browser, "No bid, reason 7", {
    within: '//*[@aria-label="Decoded bid response"]',
    timeout: 30_000,
  });
  expect(decoded).toContain("Blocked Publisher or Site");
  // The list behind the panel holds every request of the scan.
  expect(await textOf(browser, '//*[@data-testid="request-count"]')).toContain(String(rows.length));
  const details = await textOf(browser, '//*[@aria-label="Request details"]');
  expect(details).toContain("/openrtbb/prebidjs");
});

test("the bid inspector shows a late or unanswered bid under the 250 ms timeout and not sent for triplelift", async () => {
  const { browser } = app;
  // The auction in which a bidder missed the deadline.
  const missedBid = bidsOf(scan).find((bid) =>
    ["lateOnNetwork", "neverAnswered"].includes(bid.exchange?.verdict ?? ""),
  );
  const auction = scan.auctions.find((candidate) => candidate.id === missedBid?.auction);
  expect(auction?.timeoutMs).toBe(250);
  await goTo(browser, `/scans/${scan.summary.id}/network?view=bids&auction=${auction?.id}`);
  const matrix = '//*[starts-with(@aria-label, "Bid exchanges of auction")]';
  await browser.$(matrix).waitForDisplayed({ timeout: 30_000 });
  await seeText(browser, "250 ms", { within: MAIN });

  // A cell reads "<bidder> for <ad unit>: <what happened>": a button when it opens an exchange, a
  // focusable span when there is none to open.
  const cells = await browser.execute((path: string) => {
    const found = document.evaluate(
      `${path}//td//*[self::button or self::span[@tabindex]]`,
      document,
      null,
      XPathResult.ORDERED_NODE_SNAPSHOT_TYPE,
      null,
    );
    return Array.from({ length: found.snapshotLength }, (_, index) =>
      ((found.snapshotItem(index) as HTMLElement).textContent ?? "").replace(/\s+/g, " ").trim(),
    );
  }, matrix);
  const text = (await textOf(browser, matrix)) ?? "";
  // What the backend worked out is what the cells say.
  const missed = bidsOf(scan).filter(
    (bid) => bid.auction === auction?.id && ["lateOnNetwork", "neverAnswered"].includes(bid.exchange?.verdict ?? ""),
  );
  expect(missed.length).toBeGreaterThan(0);
  for (const bid of missed) {
    const cell = cells.find((label) => label?.startsWith(`${bid.bidder} for ${bid.unit}:`));
    expect(cell, `${bid.bidder} for ${bid.unit} in ${JSON.stringify(cells)}`).toMatch(/late|no answer/);
  }
  // Header and body are one grid: every row has the header's cells, each ad unit has one column, and the
  // bidder names start where their heading starts.
  const grid = await browser.execute((path: string) => {
    const table = document.evaluate(path, document, null, XPathResult.FIRST_ORDERED_NODE_TYPE, null)
      .singleNodeValue as HTMLTableElement;
    const textLeft = (cell: Element) => {
      const range = document.createRange();
      range.selectNodeContents(cell);
      return Math.round(range.getBoundingClientRect().left);
    };
    const header = [...table.querySelectorAll("thead th")];
    const bidders = [...table.querySelectorAll("tbody tr")].filter((row) => row.querySelector("td"));
    return {
      columns: header.length,
      codes: header.slice(1, -1).map((cell) => cell.firstElementChild?.textContent),
      rowCells: bidders.map((row) => row.children.length),
      headingLeft: textLeft(header[0] as Element),
      nameLefts: bidders.map((row) => textLeft(row.children[0] as Element)),
      tableLeft: Math.round(table.getBoundingClientRect().left),
    };
  }, matrix);
  expect(grid.codes).toEqual(auction?.adUnits.map((unit) => unit.code));
  expect(new Set(grid.codes).size).toBe(grid.codes.length);
  expect(grid.rowCells.length).toBeGreaterThan(0);
  expect(grid.rowCells.every((cells) => cells === grid.columns)).toBe(true);
  expect(grid.nameLefts.every((left) => left === grid.headingLeft)).toBe(true);
  expect(grid.headingLeft - grid.tableLeft).toBe(12);
  expect(text).toContain("not sent");
  expect(text).toMatch(/triplelift[\s\S]*not sent/);
  expect(text).toMatch(/openx[\s\S]*no-bid 7/);
  expect(await cspViolations(browser)).toEqual([]);
});

test("the script viewer opens the wrapper from the archive with the planted min-width line marked", async () => {
  const { browser } = app;
  const finding = scan.findings.find((candidate) => candidate.title === AI_FINDING_TITLE);
  const evidence = finding?.evidence.find((candidate) => candidate.kind === "code");
  expect(evidence).toMatchObject({ scriptId: expect.stringMatching(/^s\d+$/), line: expect.any(Number) });
  const scriptId = evidence?.scriptId ?? "";

  const source = await invokeOk<ScriptSource>(browser, "get_script_source", {
    scanId: scan.summary.id,
    scriptId,
    view: "formatted",
  });
  expect(source).toMatchObject({ availability: "available", format: "parser" });
  const anchor = source.anchors.find((candidate) => candidate.findingId === finding?.id);
  expect(anchor).toMatchObject({ located: true, startLine: evidence?.line });
  expect(source.text?.split("\n")[(anchor?.startLine ?? 0) - 1]).toContain("min-width");
  // What the model searched is not what it read; the wrapper was never read with read_script.
  expect(source.adCalls.some((call) => call.name === "requestBids")).toBe(true);

  await goTo(browser, `/scans/${scan.summary.id}/source/${scriptId}?finding=${finding?.id}`);
  await browser.$(CODE).waitForDisplayed({ timeout: 20_000 });
  const marked = `${CODE}//*[contains(@class, "cm-line") and contains(@class, "cm-findingLine-critical")]`;
  await seeText(browser, "min-width", { within: marked, timeout: 20_000 });
  // The gutter marker names the finding in its tooltip.
  const marker = `${CODE}//*[contains(@class, "cm-findingMarker")]`;
  expect(await browser.$(marker).getAttribute("title")).toContain(AI_FINDING_TITLE);
  // The text is the page's script, shown as text: nothing of it ran or was fetched.
  expect(await cspViolations(browser)).toEqual([]);
});

test("with keep network data off the next scan says so everywhere", async () => {
  const { browser } = app;
  const settings = await invokeOk<Settings>(browser, "update_settings", { patch: { keepNetworkData: false } });
  expect(settings.keepNetworkData).toBe(false);
  const unstored = await scanDemoSite(false);

  expect(unstored.network).toMatchObject({ availability: "disabled", requestCount: 0, storedBytes: 0 });
  expect(await invokeOk<NetworkRowChunk>(browser, "list_network_requests", { scanId: unstored.summary.id })).toEqual({
    rows: [],
    nextCursor: null,
    total: 0,
  });
  const summary = await invokeOk<NetworkSummary>(browser, "get_network_summary", { scanId: unstored.summary.id });
  expect(summary.archive.availability).toBe("disabled");
  const wrapper = unstored.scripts.find((script) => script.url.includes("hb-wrapper"));
  const source = await invokeOk<ScriptSource>(browser, "get_script_source", {
    scanId: unstored.summary.id,
    scriptId: wrapper?.id,
    view: "formatted",
  });
  expect(source).toMatchObject({ availability: "notRetained", text: null });
  // The verdicts are still worked out; there is just no request to open.
  const verdicts = bidsOf(unstored).filter((bid) => bid.exchange);
  expect(verdicts.length).toBeGreaterThan(0);
  expect(verdicts.every((bid) => bid.exchange?.requestId === null)).toBe(true);
  expect((await invokeOk<ArchiveStorage>(browser, "get_archive_storage")).scans).toBe(1);

  // The report's Network tab and the workbench both say that recording was off.
  await goTo(browser, `/scans/${unstored.summary.id}?tab=network`);
  await seeText(browser, "Network recording was off for this scan", { within: MAIN, timeout: 20_000 });
  await goTo(browser, `/scans/${unstored.summary.id}/network`);
  const off = await seeText(browser, "Network recording was off for this scan", { within: MAIN, timeout: 20_000 });
  expect(off).toContain("Keep network data");

  // The viewer says why there is no source instead of showing an empty editor.
  await goTo(browser, `/scans/${unstored.summary.id}/source/${wrapper?.id}`);
  const shown = await seeText(browser, /not (kept|retained|stored)|was off/i, { within: MAIN, timeout: 20_000 });
  expect(shown).not.toContain("isMobileViewport");
  expect(await textOf(browser, CODE)).toBeNull();

  await invokeOk(browser, "update_settings", { patch: { keepNetworkData: true } });
});

test("deleting the scan removes its stored data", async () => {
  const { browser } = app;
  expect(await invokeOk<null>(browser, "delete_scan", { scanId: scan.summary.id })).toBeNull();
  expect(await invokeOk<ArchiveStorage>(browser, "get_archive_storage")).toMatchObject({ scans: 0, storedBytes: 0 });
  expect(await cspViolations(browser)).toEqual([]);
});
