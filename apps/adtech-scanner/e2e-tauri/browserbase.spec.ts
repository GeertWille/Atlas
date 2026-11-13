import { afterAll, beforeAll, expect, test } from "vitest";
import type {
  BrowserbaseProbe,
  BrowserbaseProfileInfo,
  BrowserbaseStatus,
  CommandError,
  LogEntry,
  ScanDetail,
  ScanList,
  Settings,
} from "../src/bindings";
import {
  type App,
  button,
  goTo,
  hashRoute,
  invoke,
  invokeOk,
  labelled,
  launchApp,
  reportTab,
  seeText,
} from "./support/app";
import { type MockAnthropic, startMockAnthropic } from "./support/mock-anthropic";
import {
  FIXTURE_DOMAIN,
  MOCK_BROWSERBASE_KEY,
  MOCK_BROWSERBASE_PROJECT,
  type MockBrowserbase,
  startMockBrowserbase,
} from "./support/mock-browserbase";
import { type PublisherPage, startPublisherPage } from "./support/publisher-page";

const MAIN = "//main";
const NEW_SCAN = labelled("New scan");
const UNKNOWN_SCAN = "00000000-0000-4000-8000-000000000000";

let api: MockAnthropic;
let cloud: MockBrowserbase;
let page: PublisherPage;
let app: App;
/** The fixture page under a name that looks public and that only the mock's browsers resolve, to this machine. */
let publicUrl = "";
let scanId = "";

beforeAll(async () => {
  // The app is given no model key; the mock only exists so that no request can reach the real API.
  api = await startMockAnthropic();
  cloud = await startMockBrowserbase();
  page = await startPublisherPage();
  publicUrl = `http://pub.${FIXTURE_DOMAIN}:${new URL(page.url).port}/`;
  app = await launchApp({
    ADSCAN_ANTHROPIC_BASE_URL: api.baseUrl,
    ADSCAN_BROWSERBASE_BASE_URL: cloud.baseUrl,
    // Any secret from the environment keeps every secret in memory: the test never touches a keychain.
    ADSCAN_PROXY_PASSWORD: "e2e-proxy-password",
  });
});

afterAll(async () => {
  await app?.close();
  await page?.close();
  await cloud?.close();
  await api?.close();
});

async function rejection(command: string, args: Record<string, unknown> = {}): Promise<CommandError> {
  const outcome = await invoke(app.browser, command, args);
  if (!("rejected" in outcome)) throw new Error(`${command} was expected to fail`);
  return outcome.rejected as CommandError;
}

test("without credentials the Browserbase commands answer in the contract's shapes and refuse to start anything", async () => {
  const { browser } = app;
  const settings = await invokeOk<Settings>(browser, "get_settings");
  expect(settings.defaultBrowser).toBe("local");
  expect(settings.browserbase).toEqual({
    region: "us-west-2",
    defaultProxy: { kind: "none" },
    persistentProfile: false,
    connected: false,
    projectName: null,
    concurrency: null,
  });
  expect(await invokeOk<BrowserbaseProfileInfo>(browser, "get_browserbase_profile")).toEqual({
    exists: false,
    lastUsedAt: null,
    inUse: false,
  });
  expect(await invokeOk<BrowserbaseProfileInfo>(browser, "clear_browserbase_profile")).toMatchObject({ exists: false });

  const notSetUp = { code: "browserbaseAuth", message: "Browserbase is not set up. Add the API key in Settings." };
  expect(await rejection("test_browserbase")).toEqual(notSetUp);
  expect(await rejection("probe_browserbase", { region: "eu-central-1" })).toEqual(notSetUp);
  const options = {
    url: publicUrl,
    device: "desktop",
    observeSeconds: 3,
    autoScroll: true,
    acceptConsent: true,
    aiEnabled: false,
    browser: { kind: "browserbase" },
  };
  expect(await rejection("start_scan", { options })).toEqual(notSetUp);
  expect(await rejection("update_settings", { patch: { defaultBrowser: "browserbase" } })).toEqual({
    code: "invalidInput",
    message: "Add a Browserbase API key before making Browserbase the default.",
  });
  expect((await rejection("open_browserbase_live_view", { scanId: UNKNOWN_SCAN })).code).toBe("notFound");
  // Nothing of this reached Browserbase.
  expect(cloud.requests).toEqual([]);
});

test("the key is saved in Settings after the disclosure, and testing the connection starts no browser", async () => {
  const { browser } = app;
  await browser.$(button("Continue without AI")).click();
  await goTo(browser, "/settings");
  await seeText(browser, "With Browserbase, the browser runs in Browserbase's cloud, not on this Mac.", {
    within: MAIN,
    timeout: 15_000,
  });
  await seeText(browser, "Browserbase keeps a video of each session and a log", { within: MAIN });

  await browser.$(labelled("Browserbase API key")).setValue("bb_live_not_the_key");
  await browser.$(button("Save Browserbase key")).click();
  await seeText(browser, "Browserbase did not accept the API key.", { within: `${MAIN}//*[@role="alert"]` });
  expect((await invokeOk<Settings>(browser, "get_settings")).browserbase.connected).toBe(false);

  await browser.$(labelled("Browserbase API key")).setValue(MOCK_BROWSERBASE_KEY);
  await browser.$(button("Save Browserbase key")).click();
  await seeText(browser, `Connected to the project “${MOCK_BROWSERBASE_PROJECT.name}”`, {
    within: MAIN,
    timeout: 15_000,
  });
  const settings = await invokeOk<Settings>(browser, "get_settings");
  expect(settings.browserbase).toMatchObject({
    connected: true,
    projectName: MOCK_BROWSERBASE_PROJECT.name,
    concurrency: MOCK_BROWSERBASE_PROJECT.concurrency,
  });
  // Neither the key nor the project id comes back to the webview.
  expect(JSON.stringify(settings)).not.toContain(MOCK_BROWSERBASE_KEY);
  expect(JSON.stringify(settings)).not.toContain(MOCK_BROWSERBASE_PROJECT.id);

  await browser.$(button("Test connection")).click();
  await seeText(browser, `Project “${MOCK_BROWSERBASE_PROJECT.name}”: up to 3 browsers at once, 0 running now`, {
    within: MAIN,
    timeout: 15_000,
  });
  expect(await invokeOk<BrowserbaseStatus>(browser, "test_browserbase")).toEqual({
    projectName: MOCK_BROWSERBASE_PROJECT.name,
    concurrency: 3,
    browserMinutes: 12,
    proxyBytes: 0,
    runningSessions: 0,
  });
  expect(cloud.created).toEqual([]);
  expect(cloud.requests.every((request) => request.startsWith("GET "))).toBe(true);
});

test("an address only this machine can reach never goes to Browserbase", async () => {
  const options = {
    url: page.url,
    device: "desktop",
    observeSeconds: 3,
    autoScroll: true,
    acceptConsent: true,
    aiEnabled: false,
    browser: { kind: "browserbase" },
  };
  expect(await rejection("start_scan", { options })).toEqual({
    code: "invalidInput",
    message:
      "This address is on this Mac or your network; a cloud browser cannot reach it. Scan it with this Mac's Chrome.",
  });
  expect(cloud.created).toEqual([]);
});

test("a scan started in the composer runs in a Browserbase browser and records where it ran and what it used", async () => {
  const { browser } = app;
  await invokeOk<Settings>(browser, "update_settings", {
    patch: { defaultObserveSeconds: 3, browserbase: { region: "eu-central-1" } },
  });
  await goTo(browser, "/");
  await browser.$(labelled("Page URL")).waitForDisplayed({ timeout: 15_000 });
  await browser.refresh();
  await browser.$(labelled("Page URL")).waitForDisplayed({ timeout: 20_000 });
  await browser.$(labelled("Page URL")).setValue(publicUrl);
  await browser.$(`${NEW_SCAN}//button[starts-with(normalize-space(.), "Options")]`).click();
  const onBrowserbase = `${NEW_SCAN}//button[normalize-space(.)="Browserbase"]`;
  await browser.$(onBrowserbase).waitForDisplayed({ timeout: 15_000 });
  await browser.$(onBrowserbase).click();
  await seeText(browser, "The pages pass through Browserbase, which keeps a video and a log of the session.", {
    within: NEW_SCAN,
  });
  await browser.$(`${NEW_SCAN}//button[@type="submit"]`).click();

  await browser.$(labelled("Pipeline")).waitForDisplayed({ timeout: 15_000 });
  scanId = (await hashRoute(browser)).replace("/scans/", "");
  expect(scanId).toMatch(/^[0-9a-f-]{36}$/);
  await browser.$(reportTab("Findings")).waitForDisplayed({ timeout: 120_000 });
  await seeText(browser, "Browserbase", { within: MAIN });

  const detail = await invokeOk<ScanDetail>(browser, "get_scan", { scanId });
  expect(detail.summary.status).toBe("completed");
  expect(detail.summary.url).toBe(publicUrl);
  // No browser on this machine was looked for.
  expect(detail.steps[0]?.summary).toBe("Browserbase, eu-central-1");
  expect(detail.options.browser).toEqual({ kind: "browserbase", region: "eu-central-1" });
  expect(detail.options.headless).toBe(true);
  expect(detail.options.proxy).toEqual({ kind: "none" });
  expect(detail.pages.length).toBe(1);
  expect(detail.scripts.some((script) => script.url.endsWith("/lazy-ads.js"))).toBe(true);

  // What Browserbase was asked for: its own defaults, stated; this installation's tag; no proxy.
  expect(cloud.created.length).toBe(1);
  const asked = cloud.created[0];
  expect(asked?.projectId).toBe(MOCK_BROWSERBASE_PROJECT.id);
  expect(asked?.region).toBe("eu-central-1");
  expect(asked?.keepAlive).toBe(false);
  expect(asked?.browserSettings).toEqual({
    solveCaptchas: true,
    recordSession: true,
    logSession: true,
    blockAds: false,
  });
  expect(asked?.proxies).toBeUndefined();
  expect(asked?.userMetadata).toEqual({
    app: "adscan",
    install: expect.stringMatching(/^[0-9a-f]{32}$/),
    scan: scanId,
  });

  // The environment record: the visit as Browserbase ran it, next to what the page itself read.
  const environment = detail.environment;
  expect(environment?.webdriver).toBe(false);
  expect(environment?.headless).toBe(true);
  const visit = environment?.browser;
  if (visit?.kind !== "browserbase") throw new Error("the scan was not recorded as a Browserbase visit");
  expect(visit.region).toBe("eu-central-1");
  expect(visit.sessionId).toMatch(/^[0-9a-f-]{36}$/);
  expect(visit.proxyKind).toBe("none");
  expect(visit.settings).toEqual({
    solveCaptchas: true,
    recordSession: true,
    logSession: true,
    blockAds: false,
    keepAlive: false,
  });
  expect(visit.browserVersion).toMatch(/Chrome\/\d+/);
  expect(visit.userAgent).toContain("Mozilla/5.0");
  // Without a Browserbase proxy the scanner sets the region's timezone and says what the browser had.
  expect(visit.appliedTimezone).toBe("Europe/Berlin");
  expect(visit.originalTimezone).toEqual(expect.any(String));
  expect(visit.roundTripMs).toBeGreaterThanOrEqual(0);

  // Usage, apart from the LLM cost.
  expect(detail.summary.remote?.region).toBe("eu-central-1");
  expect(detail.summary.remote?.sessionSeconds).toBeGreaterThan(0);
  expect(visit.sessionSeconds).toBe(detail.summary.remote?.sessionSeconds);
  expect(detail.summary.costUsd).toBe(0);

  // The browser was released, and nothing that could reach it again is in the audit trail or the scan.
  expect(cloud.running()).toEqual([]);
  const logs = await invokeOk<LogEntry[]>(browser, "get_logs", { scanId });
  const readable = JSON.stringify(logs) + JSON.stringify(detail);
  for (const secret of ["devtools/browser", "ws://127.0.0.1", MOCK_BROWSERBASE_KEY, MOCK_BROWSERBASE_PROJECT.id]) {
    expect(readable).not.toContain(secret);
  }
  const service = logs.filter(
    (entry) => entry.kind === "visitor" && (entry.data as { activity?: string } | null)?.activity === "browser",
  );
  expect(service.map((entry) => (entry.data as { detail: { event: string } }).detail.event)).toEqual(
    expect.arrayContaining(["sessionCreated", "connected", "timezone", "released"]),
  );
  // A scripted scan has no live view to open.
  expect(await rejection("open_browserbase_live_view", { scanId })).toEqual({
    code: "invalidInput",
    message: "This scan has no Browserbase browser to show right now.",
  });
});

test("the probe starts one browser, reads what a blank page sees and releases it", async () => {
  const { browser } = app;
  const probe = await invokeOk<BrowserbaseProbe>(browser, "probe_browserbase", { region: "ap-southeast-1" });
  expect(probe.region).toBe("ap-southeast-1");
  expect(probe.webdriver).toBe(false);
  expect(probe.userAgent).toContain("Mozilla/5.0");
  expect(probe.browserVersion).toMatch(/Chrome\/\d+/);
  expect(probe.timezone).toEqual(expect.any(String));
  expect(probe.language).toEqual(expect.any(String));
  expect(probe.roundTripMs).toBeGreaterThanOrEqual(0);

  expect(cloud.created.length).toBe(2);
  const asked = cloud.created[1];
  expect(asked?.region).toBe("ap-southeast-1");
  expect(asked?.timeout).toBe(60);
  expect(asked?.userMetadata.scan).toBe("probe");
  expect(asked?.userMetadata.install).toBe(cloud.created[0]?.userMetadata.install);
  expect(cloud.running()).toEqual([]);
});

test("removing the key puts the default back and Browserbase scans are refused again", async () => {
  const { browser } = app;
  const byDefault = await invokeOk<Settings>(browser, "update_settings", { patch: { defaultBrowser: "browserbase" } });
  expect(byDefault.defaultBrowser).toBe("browserbase");

  const cleared = await invokeOk<Settings>(browser, "clear_browserbase_credentials");
  expect(cleared.browserbase.connected).toBe(false);
  expect(cleared.browserbase.projectName).toBeNull();
  expect(cleared.defaultBrowser).toBe("local");
  expect((await rejection("test_browserbase")).code).toBe("browserbaseAuth");

  // The stored scan still reads as it was.
  const stored = await invokeOk<ScanList>(browser, "list_scans", { query: {} });
  expect(stored.items.find((scan) => scan.id === scanId)?.remote?.region).toBe("eu-central-1");
});
