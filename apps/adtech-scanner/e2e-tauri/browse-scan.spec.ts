import { afterAll, beforeAll, expect, test } from "vitest";
import type { LogEntry, ScanDetail, ScanSummary } from "../src/bindings";
import { type App, button, goTo, invokeOk, labelled, launchApp, reportTab, seeText, textOf } from "./support/app";
import { type MockAnthropic, startMockAnthropic } from "./support/mock-anthropic";

const MAIN = "//main";
const PAGE_VIEWS = labelled("Page views");
/** The findings that take more than one page load, or a refreshed slot, to see. */
const VISIT_FINDINGS = [
  "Later page views are sold without header bidding",
  "Ad slots are not reset when the page changes route",
  "Slots are refreshed with bids that were already used",
];

let api: MockAnthropic;
let app: App;
let detail: ScanDetail;

beforeAll(async () => {
  // The app is given no key; the mock only exists so that no request can reach the real API.
  api = await startMockAnthropic();
  app = await launchApp({ ADSCAN_ANTHROPIC_BASE_URL: api.baseUrl });
});

afterAll(async () => {
  await app?.close();
  await api?.close();
});

test("a browse visit of the demo site records page views and finds what one page load cannot show", async () => {
  const { browser } = app;
  const url = await invokeOk<string>(browser, "get_demo_url");
  const started = await invokeOk<ScanSummary>(browser, "start_scan", {
    options: {
      url,
      device: "desktop",
      observeSeconds: 8,
      autoScroll: true,
      acceptConsent: true,
      aiEnabled: false,
      visitor: "browse",
      extraPages: 2,
      // There is no desktop to show a window on; the options ask for what the settings would not.
      headless: true,
      seed: 20261112,
    },
  });
  expect(started).toMatchObject({ status: "running", proxy: { kind: "system" }, configuration: null });

  await browser.waitUntil(
    async () => {
      detail = await invokeOk<ScanDetail>(browser, "get_scan", { scanId: started.id });
      return !["queued", "running"].includes(detail.summary.status);
    },
    { timeout: 150_000, interval: 500, timeoutMsg: "the browse scan did not finish" },
  );
  expect(detail.summary).toMatchObject({ status: "completed", error: null });

  // Page views: the entry page first, then pages of the same site the visitor reached by itself.
  expect(detail.pages.length).toBeGreaterThanOrEqual(2);
  expect(detail.pages[0]).toMatchObject({ index: 0, url, reachedBy: "entry", startedAtMs: 0 });
  for (const [index, page] of detail.pages.entries()) {
    expect(page.index).toBe(index);
    expect(page.url.startsWith(url)).toBe(true);
    if (index > 0) expect(["linkClick", "routeChange"]).toContain(page.reachedBy);
  }
  expect(detail.slots.some((slot) => slot.page > 0)).toBe(true);
  expect(detail.options).toMatchObject({ visitor: "browse", extraPages: 2, headless: true, seed: 20261112 });

  // The visit is reported as what it was: automated and headless.
  expect(detail.environment).toMatchObject({
    visitor: "browse",
    seed: 20261112,
    headless: true,
    webdriver: true,
    persistentProfile: false,
    proxy: { kind: "system" },
  });

  // Every step of the visit is in the audit trail.
  const logs = await invokeOk<LogEntry[]>(browser, "get_logs", { scanId: started.id });
  const activities = logs
    .filter((entry) => entry.kind === "visitor")
    .map((entry) => (entry.data as { activity: string }).activity);
  for (const kind of ["environment", "pageView", "action", "linkChoice"]) expect(activities).toContain(kind);
  expect(activities.filter((kind) => kind === "pageView")).toHaveLength(detail.pages.length);

  // At least one finding that needs the visit, on the page view it was seen on.
  const visitFindings = detail.findings.filter((finding) => VISIT_FINDINGS.includes(finding.title));
  expect(visitFindings.map((finding) => finding.title)).not.toEqual([]);
  for (const finding of visitFindings) {
    expect(finding.pages.length).toBeGreaterThan(0);
    for (const page of finding.pages) expect(page).toBeLessThan(detail.pages.length);
  }
});

test("the report of that visit can be narrowed to one page view", async () => {
  const { browser } = app;
  await goTo(browser, `/scans/${detail.summary.id}`);
  await browser.$(reportTab("Findings")).waitForDisplayed({ timeout: 20_000 });

  const switcher = await seeText(browser, "All pages", { within: PAGE_VIEWS });
  expect(switcher).toContain(`The visit covered ${detail.pages.length} page views`);
  const second = detail.pages[1];
  await browser.$(`${PAGE_VIEWS}${button("2")}`).click();
  const facts = await seeText(browser, "into the visit", { within: PAGE_VIEWS });
  expect(facts).toContain(second.title ?? new URL(second.url).pathname);

  // The exported report names the page views too.
  expect(await textOf(browser, MAIN)).not.toBeNull();
  const markdown = await invokeOk<string>(browser, "export_scan", { scanId: detail.summary.id, format: "markdown" });
  expect(markdown).toContain("## Visit");
  expect(markdown).toContain("### Page views");
  expect(markdown).toContain("**Seen on:** page");
});
