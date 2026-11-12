import { beforeEach, describe, expect, test } from "vitest";
import type { LogEntry, ScanEvent, ScanOptions, ScanStatus } from "@/bindings";
import { ApiError } from "@/lib/api/error";
import type { ApiPort } from "@/lib/api/port";
import { visitorActivity } from "@/shared/domain/visitor-log";
import { createFakeBackend } from "./index";

const options: ScanOptions = {
  url: "https://www.sportsdesk.example/football",
  device: "desktop",
  observeSeconds: 20,
  autoScroll: true,
  acceptConsent: true,
  aiEnabled: false,
};

const fresh = (speed = 5000) => createFakeBackend({ profile: "fresh", speed });
const seeded = () => createFakeBackend({ profile: "seeded", speed: 5000 });

/** Resolves with the scan's final status once it has one. Subscribe before starting the scan. */
async function untilFinished(api: ApiPort) {
  let finish: (status: ScanStatus) => void = () => {};
  const finished = new Promise<ScanStatus>((resolve) => {
    finish = resolve;
  });
  const events: ScanEvent[] = [];
  const unsubscribe = await api.subscribeScanEvents((event) => {
    events.push(event);
    if (event.type === "scanUpdated" && event.summary.status !== "running") finish(event.summary.status);
  });
  return { finished: finished.finally(unsubscribe), events };
}

async function run(api: ApiPort, scanOptions: ScanOptions) {
  const { finished } = await untilFinished(api);
  const summary = await api.startScan(scanOptions);
  expect(await finished).toBe("completed");
  return { detail: await api.getScan(summary.id), logs: await api.getLogs(summary.id) };
}

const activities = (logs: LogEntry[]) => logs.flatMap((entry) => visitorActivity(entry) ?? []);
const rejects = (promise: Promise<unknown>, code: string) =>
  expect(promise).rejects.toSatisfy((error) => error instanceof ApiError && error.code === code);

beforeEach(() => {
  localStorage.clear();
});

describe("what the backend fills in when a scan starts", () => {
  test("options from before visitor behaviours resolve to scroll with the settings' defaults", async () => {
    const { detail } = await run(fresh(), options);
    expect(detail.options).toMatchObject({
      visitor: "scroll",
      autoScroll: true,
      extraPages: 0,
      headless: true,
      persistentProfile: false,
      proxy: { kind: "system" },
      budgetUsd: 0.5,
    });
    expect(detail.options.seed).toEqual(expect.any(Number));
    expect(detail.summary.proxy).toEqual({ kind: "system", name: null, host: null });
    expect(detail.summary.configuration).toBeNull();
  });

  test("a browse visit is shown by default and takes its page count from the settings", async () => {
    const api = fresh();
    await api.updateSettings({ defaultExtraPages: 1 });
    const { detail } = await run(api, { ...options, visitor: "browse" });
    expect(detail.options).toMatchObject({ visitor: "browse", extraPages: 1, headless: false });
    expect(detail.pages).toHaveLength(2);
  });

  test("extra pages only count for browse, and more than five are refused", async () => {
    const api = fresh();
    expect((await run(api, { ...options, visitor: "scroll", extraPages: 4 })).detail.options.extraPages).toBe(0);
    await rejects(api.startScan({ ...options, visitor: "browse", extraPages: 6 }), "invalidInput");
  });

  test("a budget outside the allowed range and an unknown proxy profile are refused", async () => {
    const api = fresh();
    // The backend's own range is wider than what the composer offers: $0 to $50.
    await rejects(api.startScan({ ...options, budgetUsd: 50.01 }), "invalidInput");
    await rejects(api.startScan({ ...options, budgetUsd: -1 }), "invalidInput");
    await rejects(api.startScan({ ...options, proxy: { kind: "profile", id: "proxy-nowhere" } }), "invalidInput");
    expect((await api.listScans({})).total).toBe(0);
  });

  test("a seed that is given is kept, so a visit can be repeated", async () => {
    const { detail } = await run(fresh(), { ...options, visitor: "browse", extraPages: 1, seed: 4711 });
    expect(detail.options.seed).toBe(4711);
    expect(detail.environment?.seed).toBe(4711);
  });
});

describe("a browse visit of the reference-bug site", () => {
  test("covers the entry page and the followed pages, each with its own auctions, slots and metrics", async () => {
    const { detail } = await run(fresh(), { ...options, visitor: "browse", extraPages: 2 });

    expect(detail.pages.map((page) => [page.index, page.reachedBy])).toEqual([
      [0, "entry"],
      [1, "linkClick"],
      [2, "routeChange"],
    ]);
    expect(detail.pages[0]?.startedAtMs).toBe(0);
    expect(detail.pages[1]?.startedAtMs).toBeGreaterThan(detail.pages[0]?.dwellMs ?? 0);
    expect(detail.pages[1]?.link).toMatchObject({ href: detail.pages[1]?.url });
    // A route change loads no document, but the visitor still got there through a link.
    expect(detail.pages[2]?.loadMs).toBeNull();
    expect(detail.pages[2]?.link).toMatchObject({ href: detail.pages[2]?.url });

    for (const page of detail.pages) {
      expect(page.metrics.auctionCount).toBe(detail.auctions.filter((auction) => auction.page === page.index).length);
      expect(page.metrics.slotCount).toBe(detail.slots.filter((slot) => slot.page === page.index).length);
    }
    expect(new Set(detail.auctions.map((auction) => auction.id)).size).toBe(detail.auctions.length);
    // Scan metrics are sums over the page views; the timings are the entry page's.
    expect(detail.metrics.slotCount).toBe(detail.slots.length);
    expect(detail.metrics.auctionCount).toBe(detail.auctions.length);
    expect(detail.metrics.timeToFirstAuctionMs).toBe(detail.pages[0]?.metrics.timeToFirstAuctionMs);
    // Every script was loaded by the documents, not by the route change.
    expect(detail.scripts.every((script) => script.pages.every((page) => page !== 2))).toBe(true);
    expect(detail.scripts.some((script) => script.pages.join() === "0,1")).toBe(true);
  });

  test("reports what only later pages and refreshes can show, each on its pages", async () => {
    const { detail } = await run(fresh(), { ...options, visitor: "browse", extraPages: 2 });
    const byTitle = (pattern: RegExp) => detail.findings.find((finding) => pattern.test(finding.title));

    // Page 2: slots requested with no auction and no hb_* keys.
    expect(byTitle(/Later page views request their ads without header bids/)?.pages).toEqual([1]);
    const second = detail.slots.filter((slot) => slot.page === 1);
    expect(second.length).toBeGreaterThan(1);
    expect(second.every((slot) => slot.requests.length === 1 && slot.hadPrebidTargeting === false)).toBe(true);
    expect(detail.auctions.some((auction) => auction.page === 1)).toBe(false);

    // Page 3: an auction ran, but the re-defined slots were rejected and nothing was requested.
    expect(byTitle(/In-page navigation re-defines ad slots/)?.pages).toEqual([2]);
    expect(detail.auctions.filter((auction) => auction.page === 2)).toHaveLength(1);
    expect(detail.slots.filter((slot) => slot.page === 2)).toEqual([]);

    // Entry page: one slot refreshed with the hb_adid it had already used.
    const refreshed = detail.slots.filter((slot) => slot.requests.length > 1);
    expect(refreshed).toHaveLength(1);
    expect(refreshed[0]?.page).toBe(0);
    expect(refreshed[0]?.requests[1]?.prebidAdId).toBe(refreshed[0]?.requests[0]?.prebidAdId);
    expect(byTitle(/is refreshed with the bid of the previous auction/)).toMatchObject({
      pages: [0],
      affectedSlots: [refreshed[0]?.elementId],
    });

    // Findings of the entry page say so; the visit's own environment is not tied to a page.
    expect(byTitle(/never requested/)?.pages).toEqual([0]);
    // Like the backend, the overview counts the whole visit and says so.
    expect(detail.overview).toContain(
      `of ${detail.slots.length} ad slots rendered a creative across ${detail.auctions.length} Prebid auctions`,
    );
    expect(detail.overview).toContain("The visit covered 3 page views; the counts are their sum.");
  });

  test("tells its story in visitor entries: environment, page views, gestures, link choices and a block", async () => {
    const { detail, logs } = await run(fresh(), { ...options, visitor: "browse", extraPages: 2 });
    const visit = activities(logs);
    const kinds = visit.map((activity) => activity.activity);

    expect(kinds.filter((kind) => kind === "pageView")).toHaveLength(3);
    expect(kinds.filter((kind) => kind === "environment")).toHaveLength(1);
    expect(kinds.filter((kind) => kind === "linkChoice")).toHaveLength(2);
    expect(kinds).toContain("blocked");
    // A page view is announced before anything else that happens on it.
    for (const page of [0, 1, 2]) {
      const first = visit.find((activity) => activity.page === page);
      expect(first?.activity).toBe("pageView");
    }
    const pageViews = visit.filter((activity) => activity.activity === "pageView");
    expect(pageViews.map((view) => view.url)).toEqual(detail.pages.map((page) => page.url));

    const choice = visit.find((activity) => activity.activity === "linkChoice");
    expect(choice).toMatchObject({ chosen: { href: detail.pages[1]?.url, score: expect.any(Number) } });
    expect(choice?.activity === "linkChoice" && choice.alternatives.length).toBeLessThanOrEqual(4);
    expect(choice?.activity === "linkChoice" && choice.skipped[0]).toEqual({ reason: "off-site", count: 9 });

    const environment = logs.find((entry) => visitorActivity(entry)?.activity === "environment");
    expect(environment?.message).toBe("This visit is identifiable as automated: navigator.webdriver is true");
    expect(logs.find((entry) => visitorActivity(entry)?.activity === "blocked")?.level).toBe("warn");
    // The visitor's entries belong to the capture step and nowhere else.
    expect(new Set(logs.filter((entry) => entry.kind === "visitor").map((entry) => entry.node))).toEqual(
      new Set(["capture"]),
    );
    expect(detail.environment).toMatchObject({
      visitor: "browse",
      webdriver: true,
      headless: false,
      viewportOverridden: false,
    });
  });

  test("on a phone the visitor swipes and has no pointer", async () => {
    const { logs } = await run(fresh(), { ...options, device: "mobile", visitor: "browse", extraPages: 1 });
    const gestures = activities(logs).flatMap((activity) => (activity.activity === "action" ? [activity.action] : []));
    expect(gestures).toContain("swipe");
    expect(gestures).not.toContain("pointerMove");
    expect(gestures).not.toContain("hover");
  });

  test("observe only does nothing on the page; scroll only scrolls", async () => {
    const actionsOf = async (visitor: "none" | "scroll") =>
      activities((await run(fresh(), { ...options, visitor })).logs).flatMap((activity) =>
        activity.activity === "action" ? [activity.action] : [],
      );
    expect(await actionsOf("none")).toEqual(["consent"]);
    expect(await actionsOf("scroll")).toEqual(["consent", "scroll", "scroll", "scroll", "scroll"]);
  });
});

describe("the automated-traffic finding", () => {
  const SLOW = "https://www.techpulse.example/reviews";
  test("fires for a scripted visit to a site with an invalid-traffic vendor, and names what was seen", async () => {
    const { detail } = await run(fresh(), { ...options, url: SLOW });
    const finding = detail.findings.find((candidate) => /automated/.test(candidate.title));
    expect(finding).toMatchObject({ severity: "info", category: "configuration", source: "heuristic", pages: [] });
    expect(finding?.description).toContain(
      "the pages could tell: navigator.webdriver was true, the browser ran headless. DoubleVerify is on the page",
    );
    expect(detail.environment?.detectors).toEqual([
      { vendor: "DoubleVerify", kind: "invalidTrafficMeasurement", host: "cdn.doubleverify.com" },
    ]);
  });

  test("does not fire without such a vendor on the page", async () => {
    const { detail } = await run(fresh(), options);
    expect(detail.environment).toMatchObject({ webdriver: true, detectors: [] });
    expect(detail.findings.some((candidate) => /automated/.test(candidate.title))).toBe(false);
  });
});

describe("a manual session", () => {
  const manual: ScanOptions = { ...options, visitor: "manual" };

  /** Resolves once the scan has logged a page view with this index. */
  async function untilPage(api: ApiPort, scanId: string, index: number) {
    await expect
      .poll(async () => activities(await api.getLogs(scanId)).filter((a) => a.activity === "pageView").length, {
        timeout: 5000,
      })
      .toBeGreaterThan(index);
  }
  const capturing = async (api: ApiPort, scanId: string) =>
    (await api.getScan(scanId)).steps.find((step) => step.node === "capture")?.status;

  test("opens a window, clicks nothing, and records until the user finishes it", async () => {
    const api = fresh(200);
    const { finished } = await untilFinished(api);
    const summary = await api.startScan({ ...manual, acceptConsent: true, headless: true });
    expect((await api.getScan(summary.id)).options).toMatchObject({ headless: false, acceptConsent: false });

    // All three pages of the scripted session have come and gone: the capture still waits.
    await untilPage(api, summary.id, 2);
    await new Promise((resolve) => setTimeout(resolve, 150));
    expect(await capturing(api, summary.id)).toBe("running");
    expect((await api.getScan(summary.id)).summary.status).toBe("running");

    await api.finishCapture(summary.id);
    expect(await finished).toBe("completed");
    const detail = await api.getScan(summary.id);
    const logs = await api.getLogs(summary.id);
    expect(detail.pages.map((page) => page.reachedBy)).toEqual(["entry", "userNavigation", "routeChange"]);
    // The person chose where to go: no link is attributed to the scanner.
    expect(detail.pages.every((page) => page.link === null)).toBe(true);
    expect(logs.some((entry) => entry.message.startsWith("This visit is not flagged as automated"))).toBe(true);
    expect(detail.environment).toMatchObject({ visitor: "manual", webdriver: false, headless: false });
    // Nobody but the person acted: no gestures, no link choices, no consent click.
    expect(activities(logs).filter((activity) => activity.activity === "action")).toEqual([]);
    expect(activities(logs).filter((activity) => activity.activity === "linkChoice")).toEqual([]);
    expect(logs.some((entry) => entry.message === "Capture finished by you after 3 pages")).toBe(true);
    expect(detail.findings.some((finding) => /automated/.test(finding.title))).toBe(false);
  });

  test("finished early, the report covers only the pages seen so far", async () => {
    const api = fresh(40);
    const { finished } = await untilFinished(api);
    const summary = await api.startScan(manual);
    await untilPage(api, summary.id, 0);
    await api.finishCapture(summary.id);
    expect(await finished).toBe("completed");

    const detail = await api.getScan(summary.id);
    expect(detail.pages).toHaveLength(1);
    expect(detail.slots.every((slot) => slot.page === 0)).toBe(true);
    const pageViews = activities(await api.getLogs(summary.id)).filter((activity) => activity.activity === "pageView");
    expect(pageViews).toHaveLength(1);
  });

  test("finishing does nothing for a scripted scan, after the capture, or once the scan has ended", async () => {
    const api = fresh();
    const { finished } = await untilFinished(api);
    const scripted = await api.startScan(options);
    await expect(api.finishCapture(scripted.id)).resolves.toBeUndefined();
    expect(await finished).toBe("completed");
    await expect(api.finishCapture(scripted.id)).resolves.toBeUndefined();
    expect((await api.getScan(scripted.id)).pages).toHaveLength(1);
    await rejects(api.finishCapture("scan-unknown"), "notFound");
  });

  test("cancelling a session that waits for the user ends it, and a later finish changes nothing", async () => {
    const api = fresh(200);
    const { finished } = await untilFinished(api);
    const summary = await api.startScan(manual);
    await untilPage(api, summary.id, 2);
    await api.cancelScan(summary.id);
    expect(await finished).toBe("cancelled");
    await api.finishCapture(summary.id);
    expect((await api.getScan(summary.id)).summary.status).toBe("cancelled");
  });
});

describe("the scan profile", () => {
  test("is empty until a scan keeps it, grows with each such scan, and can be cleared when idle", async () => {
    const api = fresh();
    expect(await api.getScanProfile()).toEqual({ exists: false, sizeBytes: 0, lastUsedAt: null, inUse: false });

    await run(api, options);
    expect((await api.getScanProfile()).exists).toBe(false);

    await api.updateSettings({ persistentProfile: true });
    const { detail } = await run(api, options);
    expect(detail.options.persistentProfile).toBe(true);
    expect(detail.environment?.persistentProfile).toBe(true);
    const once = await api.getScanProfile();
    expect(once).toMatchObject({ exists: true, inUse: false, lastUsedAt: detail.summary.startedAt });
    expect(once.sizeBytes).toBeGreaterThan(0);

    // A scan can also ask for it, or refuse it, on its own.
    await run(api, { ...options, persistentProfile: false });
    expect((await api.getScanProfile()).sizeBytes).toBe(once.sizeBytes);
    await api.updateSettings({ persistentProfile: false });
    await run(api, { ...options, persistentProfile: true });
    expect((await api.getScanProfile()).sizeBytes).toBeGreaterThan(once.sizeBytes);

    expect(await api.clearScanProfile()).toEqual({ exists: false, sizeBytes: 0, lastUsedAt: null, inUse: false });
  });

  test("cannot be cleared while a scan is using it, and survives a restart", async () => {
    const api = fresh(1);
    const summary = await api.startScan({ ...options, persistentProfile: true });
    expect((await api.getScanProfile()).inUse).toBe(true);
    await rejects(api.clearScanProfile(), "profileBusy");
    expect((await api.getScanProfile()).exists).toBe(true);

    await api.cancelScan(summary.id);
    expect((await api.getScanProfile()).inUse).toBe(false);
    expect((await fresh().getScanProfile()).exists).toBe(true);
    await api.clearScanProfile();
    expect((await fresh().getScanProfile()).exists).toBe(false);
  });
});

describe("proxy profiles", () => {
  const belgium = {
    name: " Belgium ",
    scheme: "http",
    host: "be.proxy.example",
    port: 8080,
    username: "scanner",
  } as const;

  test("are saved by name, edited in place, and keep their password out of the settings", async () => {
    const api = fresh();
    let settings = await api.saveProxyProfile(belgium);
    const [proxy] = settings.proxies;
    expect(proxy).toMatchObject({ name: "Belgium", host: "be.proxy.example", username: "scanner", hasPassword: false });
    const id = proxy?.id ?? "";

    settings = await api.setProxyPassword(id, "s3cret");
    expect(settings.proxies[0]?.hasPassword).toBe(true);
    expect(JSON.stringify(settings)).not.toContain("s3cret");
    expect(localStorage.getItem("adscan.fake.db.fresh")).not.toContain("s3cret");

    settings = await api.saveProxyProfile({ ...belgium, id, name: "Belgium (Brussels)", port: 3128 });
    expect(settings.proxies).toHaveLength(1);
    expect(settings.proxies[0]).toMatchObject({ id, name: "Belgium (Brussels)", port: 3128, hasPassword: true });

    expect((await api.clearProxyPassword(id)).proxies[0]?.hasPassword).toBe(false);
    await rejects(api.setProxyPassword(id, ""), "invalidInput");
    await rejects(api.setProxyPassword("proxy-nowhere", "x"), "notFound");
    await rejects(api.saveProxyProfile({ ...belgium, id: "proxy-nowhere" }), "notFound");
    expect((await fresh().getSettings()).proxies).toHaveLength(1);
  });

  test.each([
    ["no name", { name: "  " }],
    ["a name of more than 60 characters", { name: "x".repeat(61) }],
    ["a URL instead of a host", { host: "http://be.proxy.example" }],
    ["a host with a port", { host: "be.proxy.example:8080" }],
    ["a host with credentials", { host: "user:pw@be.proxy.example" }],
    ["a port of zero", { port: 0 }],
    ["a port above 65535", { port: 70_000 }],
    ["a user name with SOCKS5", { scheme: "socks5" as const }],
  ])("%s is refused and nothing is saved", async (_name, patch) => {
    const api = fresh();
    await rejects(api.saveProxyProfile({ ...belgium, ...patch }), "invalidInput");
    expect((await api.getSettings()).proxies).toEqual([]);
  });

  test("names are unique without regard to case, and SOCKS5 and IP addresses are fine without a user", async () => {
    const api = fresh();
    await api.saveProxyProfile(belgium);
    await rejects(api.saveProxyProfile({ ...belgium, name: "BELGIUM" }), "invalidInput");
    const settings = await api.saveProxyProfile({
      name: "Local",
      scheme: "socks5",
      host: "10.0.0.7",
      port: 1080,
      username: null,
    });
    await api.saveProxyProfile({ name: "v6", scheme: "https", host: "2001:db8::1", port: 443, username: "" });
    expect(settings.proxies.map((proxy) => proxy.name)).toEqual(["Belgium", "Local"]);
    expect((await api.getSettings()).proxies.find((proxy) => proxy.name === "v6")?.username).toBeNull();
  });

  test("a scan records the proxy it ran through by name and host, per scan or as the default", async () => {
    const api = fresh();
    const id = (await api.saveProxyProfile(belgium)).proxies[0]?.id ?? "";
    const through = { kind: "profile", name: "Belgium", host: "be.proxy.example" };

    const chosen = await run(api, { ...options, proxy: { kind: "profile", id } });
    expect(chosen.detail.summary.proxy).toEqual(through);
    expect(chosen.detail.environment?.proxy).toEqual(through);
    expect(chosen.detail.options.proxy).toEqual({ kind: "profile", id, name: "Belgium", host: "be.proxy.example" });
    expect(chosen.logs.some((entry) => entry.message.includes("belgium (be.proxy.example)"))).toBe(true);

    await api.updateSettings({ defaultProxy: { kind: "profile", id } });
    expect((await run(api, options)).detail.summary.proxy).toEqual(through);
    expect((await run(api, { ...options, proxy: { kind: "none" } })).detail.summary.proxy?.kind).toBe("none");
    await rejects(api.updateSettings({ defaultProxy: { kind: "profile", id: "proxy-nowhere" } }), "invalidInput");
  });

  test("deleting the default proxy falls back to the system proxy; old scans keep its name", async () => {
    const api = fresh();
    const id = (await api.saveProxyProfile(belgium)).proxies[0]?.id ?? "";
    await api.updateSettings({ defaultProxy: { kind: "profile", id } });
    const { detail } = await run(api, options);

    const settings = await api.deleteProxyProfile(id);
    expect(settings.proxies).toEqual([]);
    expect(settings.defaultProxy).toEqual({ kind: "system" });
    expect((await api.getScan(detail.summary.id)).summary.proxy?.name).toBe("Belgium");
    await rejects(api.deleteProxyProfile(id), "notFound");
  });
});

describe("scan configurations", () => {
  const draft = {
    name: " Sportsdesk desktop ",
    options: { ...options, visitor: "browse", extraPages: 1, seed: 7 },
  } as const;

  test("are saved without a seed, listed by name, updated in place and deleted without their scans", async () => {
    const api = fresh();
    const saved = await api.saveScanConfiguration(draft);
    expect(saved).toMatchObject({ name: "Sportsdesk desktop", lastScanId: null, lastRunAt: null });
    expect(saved.options).not.toHaveProperty("seed");
    expect(saved.options).toMatchObject({ visitor: "browse", extraPages: 1 });
    await api.saveScanConfiguration({ name: "A first", options });
    expect((await api.listScanConfigurations()).map((configuration) => configuration.name)).toEqual([
      "A first",
      "Sportsdesk desktop",
    ]);

    const renamed = await api.saveScanConfiguration({
      id: saved.id,
      name: "Sportsdesk",
      options: { ...options, device: "mobile" },
    });
    expect(renamed).toMatchObject({ id: saved.id, name: "Sportsdesk", createdAt: saved.createdAt });
    expect(renamed.options.device).toBe("mobile");
    expect(await api.listScanConfigurations()).toHaveLength(2);

    const { finished } = await untilFinished(api);
    const scan = await api.runScanConfiguration(saved.id);
    await finished;
    await api.deleteScanConfiguration(saved.id);
    expect(await api.listScanConfigurations()).toHaveLength(1);
    expect((await api.getScan(scan.id)).summary.configuration).toEqual({ id: saved.id, name: "Sportsdesk" });
    await rejects(api.deleteScanConfiguration(saved.id), "notFound");
    await rejects(api.runScanConfiguration(saved.id), "notFound");
  });

  test.each([
    ["no name", { name: " " }],
    ["a name of more than 80 characters", { name: "x".repeat(81) }],
    ["a relative URL", { options: { ...options, url: "/football" } }],
    ["more than five extra pages", { options: { ...options, visitor: "browse" as const, extraPages: 9 } }],
    ["an unknown proxy profile", { options: { ...options, proxy: { kind: "profile" as const, id: "proxy-nowhere" } } }],
  ])("%s is refused", async (_name, patch) => {
    const api = fresh();
    await rejects(api.saveScanConfiguration({ ...draft, ...patch }), "invalidInput");
    expect(await api.listScanConfigurations()).toEqual([]);
  });

  test("names are unique without regard to case", async () => {
    const api = fresh();
    const saved = await api.saveScanConfiguration(draft);
    await rejects(api.saveScanConfiguration({ ...draft, name: "SPORTSDESK DESKTOP" }), "invalidInput");
    // Saving a configuration under its own name is not a clash.
    await expect(api.saveScanConfiguration({ ...draft, id: saved.id })).resolves.toMatchObject({ id: saved.id });
  });

  test("a run uses the saved options, is recorded as the last run, and can be filtered in history", async () => {
    const api = fresh();
    const saved = await api.saveScanConfiguration(draft);
    await run(api, options);

    const { finished } = await untilFinished(api);
    const summary = await api.runScanConfiguration(saved.id);
    expect(summary.configuration).toEqual({ id: saved.id, name: "Sportsdesk desktop" });
    await finished;
    const detail = await api.getScan(summary.id);
    expect(detail.options).toMatchObject({ visitor: "browse", extraPages: 1 });
    // Every run gets its own seed.
    expect(detail.options.seed).not.toBe(7);

    const [listed] = await api.listScanConfigurations();
    expect(listed).toMatchObject({ lastScanId: summary.id, lastRunAt: summary.startedAt });
    const fromIt = await api.listScans({ configurationId: saved.id });
    expect(fromIt.items.map((scan) => scan.id)).toEqual([summary.id]);
    expect((await api.listScans({})).total).toBe(2);

    // Once that scan is deleted the configuration still knows when it ran, but has no scan to link.
    await api.deleteScan(summary.id);
    expect((await api.listScanConfigurations())[0]).toMatchObject({ lastScanId: null, lastRunAt: summary.startedAt });
  });

  test("a proxy that a configuration uses cannot be deleted, and the refusal names the configuration", async () => {
    const api = fresh();
    const id =
      (
        await api.saveProxyProfile({
          name: "Belgium",
          scheme: "http",
          host: "be.proxy.example",
          port: 8080,
          username: null,
        })
      ).proxies[0]?.id ?? "";
    const saved = await api.saveScanConfiguration({
      ...draft,
      options: { ...options, proxy: { kind: "profile", id } },
    });
    expect(saved.options.proxy).toEqual({ kind: "profile", id, name: "Belgium", host: "be.proxy.example" });

    await expect(api.deleteProxyProfile(id)).rejects.toThrow(
      'The proxy "Belgium" is used by the scan configuration "Sportsdesk desktop". Change or delete it first.',
    );
    expect((await api.getSettings()).proxies).toHaveLength(1);
    await api.deleteScanConfiguration(saved.id);
    await expect(api.deleteProxyProfile(id)).resolves.toMatchObject({ proxies: [] });
  });

  test("configurations survive a restart", async () => {
    await fresh().saveScanConfiguration(draft);
    expect((await fresh().listScanConfigurations()).map((configuration) => configuration.name)).toEqual([
      "Sportsdesk desktop",
    ]);
  });
});

describe("seeded history", () => {
  test("has one multi-page visit through a proxy, started from a configuration", async () => {
    const detail = await seeded().getScan("seed-sportsdesk-browse");
    expect(detail.pages).toHaveLength(3);
    expect(detail.summary).toMatchObject({
      status: "completed",
      configuration: { id: "config-sportsdesk-belgium", name: "Sportsdesk from Belgium" },
      proxy: { kind: "profile", name: "Belgium", host: "be.proxy.example" },
    });
    expect(detail.environment).toMatchObject({ visitor: "browse", headless: false });
    expect(detail.findings.filter((finding) => finding.pages.some((page) => page > 0)).length).toBeGreaterThanOrEqual(
      2,
    );
  });

  test("keeps scans from before page views as they were stored: no pages, no environment, the old options", async () => {
    const api = seeded();
    for (const scanId of ["seed-morningledger-mobile", "seed-archive-1"]) {
      const detail = await api.getScan(scanId);
      expect(detail.summary.status).toBe("completed");
      expect(detail.pages).toEqual([]);
      expect(detail.environment).toBeNull();
      expect(detail.summary.proxy).toBeNull();
      expect(Object.keys(detail.options).sort()).toEqual(
        ["acceptConsent", "aiEnabled", "autoScroll", "device", "observeSeconds", "url"].sort(),
      );
      expect(detail.findings.every((finding) => finding.pages.length === 0)).toBe(true);
      expect(detail.scripts.every((script) => script.pages.length === 0)).toBe(true);
      expect(detail.slots.every((slot) => slot.requests.length === 0)).toBe(true);
      expect((await api.getLogs(scanId)).some((entry) => entry.kind === "visitor")).toBe(false);
    }
  });

  test("has proxies, and configurations of one site for several countries, one of which lost its proxy", async () => {
    const api = seeded();
    const settings = await api.getSettings();
    expect(settings.proxies.map((proxy) => [proxy.name, proxy.scheme, proxy.hasPassword])).toEqual([
      ["Belgium", "http", true],
      ["Netherlands", "socks5", false],
    ]);
    const configurations = await api.listScanConfigurations();
    expect(configurations.map((configuration) => configuration.name)).toEqual([
      "Morning Ledger, no proxy",
      "Sportsdesk from Belgium",
      "Sportsdesk from Germany",
      "Sportsdesk from the Netherlands, mobile",
    ]);
    expect(configurations[1]).toMatchObject({ lastScanId: "seed-sportsdesk-browse" });
    expect(
      (await api.listScans({ configurationId: "config-sportsdesk-belgium" })).items.map((scan) => scan.id),
    ).toEqual(["seed-sportsdesk-browse"]);

    // Germany's proxy is gone: the configuration cannot run until it is edited.
    await expect(api.runScanConfiguration("config-sportsdesk-germany")).rejects.toThrow(
      'The proxy of the scan configuration "Sportsdesk from Germany" no longer exists. Edit the configuration and pick another proxy.',
    );
    await rejects(api.runScanConfiguration("config-sportsdesk-germany"), "invalidInput");
  });

  test("the markdown export of a visit names the configuration, the visit and its page views", async () => {
    const api = seeded();
    const markdown = await api.exportScan("seed-sportsdesk-browse", "markdown");
    expect(markdown).toContain("- Configuration: Sportsdesk from Belgium");
    expect(markdown).toContain("## Visit\n\n- Visitor: Browse like a visitor");
    expect(markdown).toContain("- Proxy: Belgium (be.proxy.example)");
    expect(markdown).toContain("| 2 | /2026/11/council-approves-harbour-plan | Followed link |");
    expect(markdown).toContain("Seen on: page 2");
    // A scan stored before visits exports as before.
    const legacy = await api.exportScan("seed-morningledger-mobile", "markdown");
    expect(legacy).not.toContain("## Visit");
    expect(legacy).not.toContain("Seen on");
  });
});

test("state saved before visits, proxies and configurations existed loads with the new defaults", async () => {
  localStorage.setItem(
    "adscan.fake.db.fresh",
    JSON.stringify({ settings: { budgetUsd: 1, headless: false }, scans: [], deletedSeedIds: [] }),
  );
  const api = fresh();
  expect(await api.getSettings()).toMatchObject({
    budgetUsd: 1,
    headless: false,
    defaultVisitor: "scroll",
    defaultExtraPages: 2,
    persistentProfile: false,
    proxies: [],
    defaultProxy: { kind: "system" },
  });
  expect(await api.listScanConfigurations()).toEqual([]);
  expect((await api.getScanProfile()).exists).toBe(false);
});
