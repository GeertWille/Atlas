import { afterAll, beforeAll, expect, test } from "vitest";
import type { ApiKeyStatus, BrowserInfo, DashboardStats, ScanList, ScanSummary, Settings } from "../src/bindings";
import {
  type App,
  button,
  closeCommandPalette,
  invoke,
  invokeOk,
  labelled,
  launchApp,
  openCommandPalette,
  seeText,
} from "./support/app";
import { cspViolations, watchCspViolations } from "./support/csp";
import { type MockAnthropic, startMockAnthropic } from "./support/mock-anthropic";

const UNKNOWN_SCAN = "00000000-0000-4000-8000-000000000000";

let api: MockAnthropic;
let app: App;

beforeAll(async () => {
  api = await startMockAnthropic();
  app = await launchApp({ ADSCAN_ANTHROPIC_BASE_URL: api.baseUrl });
});

afterAll(async () => {
  await app?.close();
  await api?.close();
});

test("the built frontend runs under the production CSP without a violation", async () => {
  const { browser } = app;
  await watchCspViolations(browser);

  // Everything index.html loads was fetched before the listener existed, so fetch it all again.
  const reloaded = await browser.execute(async () => {
    const again = (source: Element) =>
      new Promise<string>((done) => {
        // A cloned script counts as already run and would never load, so build the element anew.
        const copy = document.createElement(source.localName) as HTMLScriptElement | HTMLLinkElement;
        for (const { name, value } of source.attributes) copy.setAttribute(name, value);
        const url = copy instanceof HTMLLinkElement ? copy.href : copy.src;
        copy.addEventListener("load", () => done(`loaded ${new URL(url).pathname}`));
        copy.addEventListener("error", () => done(`failed ${new URL(url).pathname}`));
        document.head.append(copy);
      });
    const sources = [...document.querySelectorAll('script[src], link[rel="stylesheet"]')];
    return Promise.all(sources.map(again));
  });
  expect(reloaded.length).toBeGreaterThanOrEqual(3);
  expect(reloaded.filter((outcome) => outcome.startsWith("failed"))).toEqual([]);
  expect(reloaded).toContain("loaded /theme-boot.js");

  const fonts = await browser.execute(async () => {
    await document.fonts.ready;
    return {
      body: getComputedStyle(document.body).fontFamily,
      inter: document.fonts.check('13px "Inter Variable"'),
      mono: document.fonts.check('13px "JetBrains Mono Variable"'),
      failed: [...document.fonts].filter((face) => face.status === "error").map((face) => face.family),
    };
  });
  expect(fonts.body).toContain("Inter Variable");
  expect(fonts).toMatchObject({ inter: true, mono: true, failed: [] });

  for (const screen of ["History", "Log search", "Settings", "Home"]) {
    await browser.$(`//nav//a[normalize-space(.)="${screen}"]`).click();
    await browser.waitUntil(async () => (await browser.getTitle()).startsWith(screen === "Home" ? "Welcome" : screen));
  }
  await openCommandPalette(browser);
  await closeCommandPalette(browser);
  expect(await cspViolations(browser)).toEqual([]);

  // The policy is really enforced: what it forbids is blocked and reported.
  await browser.execute(async () => {
    const inline = document.createElement("script");
    inline.textContent = "window.__inlineScriptRan = true";
    document.head.append(inline);
    await fetch("https://example.invalid/").catch(() => undefined);
  });
  expect(await browser.execute(() => "__inlineScriptRan" in window)).toBe(false);
  expect(await cspViolations(browser)).toEqual([
    { directive: "script-src-elem", blocked: "inline" },
    { directive: "connect-src", blocked: "https://example.invalid/" },
  ]);
});

test("read commands answer with the contract's shapes", async () => {
  const { browser } = app;
  const settings = await invokeOk<Settings>(browser, "get_settings");
  expect(settings).toEqual({
    filterModel: "claude-haiku-5-5",
    analysisModel: "claude-sonnet-5-5",
    analysisEffort: "medium",
    budgetUsd: 0.5,
    chromePath: null,
    headless: true,
    defaultDevice: "desktop",
    defaultObserveSeconds: 20,
    theme: "system",
    hasApiKey: false,
    jevEnabled: false,
    hasTypesafeKey: false,
  });

  const detected = await invokeOk<BrowserInfo | null>(browser, "detect_browser");
  expect(detected).toEqual({ path: expect.any(String), version: expect.any(String), name: expect.any(String) });
  expect(await invokeOk<string>(browser, "get_demo_url")).toMatch(/^http:\/\/127\.0\.0\.1:\d+\/$/);
  expect(await invokeOk<ScanList>(browser, "list_scans", { query: {} })).toEqual({ items: [], total: 0 });
  expect(
    await invokeOk<ScanList>(browser, "list_scans", {
      query: { search: "x", status: "completed", device: "mobile", limit: 5, offset: 0 },
    }),
  ).toEqual({ items: [], total: 0 });
  expect(await invokeOk<DashboardStats>(browser, "get_dashboard_stats")).toEqual({
    totalScans: 0,
    sitesScanned: 0,
    openCriticalFindings: 0,
    totalFindings: 0,
    totalCostUsd: 0,
    avgCostUsd: 0,
    avgHealthScore: null,
  });
  // Optional arguments may be left out or sent as `undefined`, which is what the adapter does.
  expect(await invokeOk<unknown[]>(browser, "search_logs", { query: "anything" })).toEqual([]);
  expect(await invokeOk<unknown[]>(browser, "search_logs", { query: "anything", limit: 3 })).toEqual([]);
  expect(await invokeOk<null>(browser, "delete_all_scans")).toBeNull();
});

test("settings patches round-trip, including clearing the browser path", async () => {
  const { browser } = app;
  const { path } = (await invokeOk<BrowserInfo>(browser, "detect_browser")) ?? {};
  const patched = await invokeOk<Settings>(browser, "update_settings", {
    patch: { chromePath: path, analysisEffort: "high", headless: false, defaultObserveSeconds: 30 },
  });
  expect(patched).toMatchObject({
    chromePath: path,
    analysisEffort: "high",
    headless: false,
    defaultObserveSeconds: 30,
    budgetUsd: 0.5,
  });
  // An omitted field keeps its value; an explicit `null` returns to auto-detection.
  expect((await invokeOk<Settings>(browser, "update_settings", { patch: { headless: true } })).chromePath).toBe(path);
  expect((await invokeOk<Settings>(browser, "update_settings", { patch: { chromePath: null } })).chromePath).toBeNull();

  expect(await invoke(browser, "update_settings", { patch: { budgetUsd: 500 } })).toEqual({
    rejected: { code: "invalidInput", message: expect.any(String) },
  });
  const rejected = await invokeOk<ApiKeyStatus>(browser, "set_api_key", { key: "sk-ant-not-a-real-key" });
  expect(rejected).toEqual({ valid: false, message: "Anthropic rejected this API key." });
});

test("failures reject with { code, message }", async () => {
  const { browser } = app;
  const notFound = { rejected: { code: "notFound", message: expect.stringContaining(UNKNOWN_SCAN) } };
  expect(await invoke(browser, "get_scan", { scanId: UNKNOWN_SCAN })).toEqual(notFound);
  expect(await invoke(browser, "get_logs", { scanId: UNKNOWN_SCAN })).toEqual(notFound);
  expect(await invoke(browser, "get_logs", { scanId: UNKNOWN_SCAN, stepId: "step" })).toEqual(notFound);
  expect(await invoke(browser, "cancel_scan", { scanId: UNKNOWN_SCAN })).toEqual(notFound);
  expect(await invoke(browser, "delete_scan", { scanId: UNKNOWN_SCAN })).toEqual(notFound);
  expect(await invoke(browser, "export_scan", { scanId: UNKNOWN_SCAN, format: "json" })).toEqual(notFound);

  const options = { device: "desktop", observeSeconds: 5, autoScroll: true, acceptConsent: true, aiEnabled: false };
  expect(await invoke(browser, "start_scan", { options: { ...options, url: "file:///etc/passwd" } })).toEqual({
    rejected: { code: "invalidInput", message: expect.any(String) },
  });
  // A request that does not match the command's signature never reaches it; Tauri rejects with a plain string.
  expect(await invoke(browser, "get_scan", {})).toEqual({ rejected: expect.stringContaining("scanId") });
});

test("a scan can be cancelled and exported, and export only writes where a save dialog pointed", async () => {
  const { browser } = app;
  const url = await invokeOk<string>(browser, "get_demo_url");
  const started = await invokeOk<ScanSummary>(browser, "start_scan", {
    options: { url, device: "mobile", observeSeconds: 30, autoScroll: false, acceptConsent: false, aiEnabled: false },
  });
  expect(started).toMatchObject({ url, hostname: "127.0.0.1", device: "mobile", status: "running", aiUsed: false });

  expect(await invokeOk<null>(browser, "cancel_scan", { scanId: started.id })).toBeNull();
  await browser.waitUntil(
    async () => {
      const list = await invokeOk<ScanList>(browser, "list_scans", { query: { status: "cancelled" } });
      return list.items.some((scan) => scan.id === started.id);
    },
    { timeout: 20_000 },
  );

  const json = JSON.parse(await invokeOk<string>(browser, "export_scan", { scanId: started.id, format: "json" }));
  expect(json.summary).toMatchObject({ id: started.id, status: "cancelled" });
  const markdown = await invokeOk<string>(browser, "export_scan", { scanId: started.id, format: "markdown" });
  expect(markdown).toContain("127.0.0.1");

  const refused = { rejected: { code: "invalidInput", message: expect.stringContaining("save dialog") } };
  expect(
    await invoke(browser, "export_scan", { scanId: started.id, format: "markdown", path: "/tmp/adscan-planted.md" }),
  ).toEqual(refused);
  expect(
    await invoke(browser, "export_scan", { scanId: started.id, format: "json", path: "/tmp/adscan-planted.sh" }),
  ).toEqual({ rejected: { code: "invalidInput", message: expect.stringContaining(".json") } });
  expect(await invokeOk<null>(browser, "delete_scan", { scanId: started.id })).toBeNull();
});

test("the webview can only do what its capability grants", async () => {
  const { browser } = app;
  const denied = (command: string) => ({ rejected: expect.stringContaining(`${command} not allowed`) });
  // Scan events come from the backend only; the page cannot forge one or touch the filesystem.
  expect(await invoke(browser, "plugin:event|emit", { event: "scan-event", payload: {} })).toEqual(
    denied("event.emit"),
  );
  expect(await invoke(browser, "plugin:fs|read_text_file", { path: "/etc/hostname" })).toEqual(
    denied("fs.read_text_file"),
  );
  expect(await invoke(browser, "plugin:fs|write_text_file", { path: "/tmp/adscan-planted.md" })).toEqual(
    denied("fs.write_text_file"),
  );
  expect(await invoke(browser, "plugin:opener|open_url", { url: "file:///etc/hostname" })).toEqual({
    rejected: expect.stringContaining("Not allowed to open url"),
  });
  // A web link passes the scope. Whether a browser then opens depends on the machine, so only the scope is asserted.
  const webLink = await invoke(browser, "plugin:opener|open_url", { url: "https://example.invalid/" });
  expect(JSON.stringify(webLink)).not.toContain("Not allowed");
  expect(await browser.$(button("New scan")).isDisplayed()).toBe(true);
});

test("the app keeps working while the computer is offline", async () => {
  const { browser } = app;
  await browser.execute(() => window.dispatchEvent(new Event("offline")));
  try {
    await browser.$('//nav//a[normalize-space(.)="Settings"]').click();
    const dark = browser.$(`${labelled("Theme")}//button[normalize-space(.)="Dark"]`);
    await dark.waitForDisplayed();
    await dark.click();
    await browser.waitUntil(async () => (await invokeOk<Settings>(browser, "get_settings")).theme === "dark");
    await browser.$('//nav//a[normalize-space(.)="History"]').click();
    await seeText(browser, "No scans yet", { within: "//main" });
  } finally {
    await browser.execute(() => window.dispatchEvent(new Event("online")));
    await invokeOk(browser, "update_settings", { patch: { theme: "system" } });
  }
});

test("the custom title bar drags the window and zooms it on a double click", async () => {
  const { browser } = app;
  // Both commands are within the capability.
  expect(await invokeOk<null>(browser, "plugin:window|internal_toggle_maximize")).toBeNull();
  expect(await invokeOk<null>(browser, "plugin:window|internal_toggle_maximize")).toBeNull();

  // No window manager runs here to move the window, so answer the title bar's requests in the page and
  // record them. They travel as `fetch("ipc://localhost/<command>")`; `invoke` itself cannot be replaced.
  await browser.execute(() => {
    const page = window as unknown as { __windowCommands: string[] };
    page.__windowCommands = [];
    const send = window.fetch.bind(window);
    window.fetch = (input, init) => {
      const command = decodeURIComponent(String(input).replace("ipc://localhost/", ""));
      if (!command.startsWith("plugin:window|")) return send(input, init);
      page.__windowCommands.push(command);
      const headers = { "content-type": "application/json", "Tauri-Response": "ok" };
      return Promise.resolve(new Response("null", { headers }));
    };
  });
  const windowCommands = () =>
    browser.execute(() => (window as unknown as { __windowCommands: string[] }).__windowCommands.splice(0));

  // The page title is text inside the bar, not the bar itself.
  await browser.$("//header//h1").click();
  expect(await windowCommands()).toEqual(["plugin:window|start_dragging"]);
  await browser.$("//aside//span[normalize-space(.)='Ad-Tech Scanner']").doubleClick();
  expect(await windowCommands()).toEqual(["plugin:window|start_dragging", "plugin:window|internal_toggle_maximize"]);
  await browser.$(button("New scan")).click();
  expect(await windowCommands()).toEqual([]);
});
