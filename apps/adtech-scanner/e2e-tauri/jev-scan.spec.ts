import { afterAll, beforeAll, expect, test } from "vitest";
import type { ApiKeyStatus, LogEntry, ScanDetail, Settings } from "../src/bindings";
import { type App, field, hashRoute, invokeOk, labelled, launchApp, reportTab, seeText } from "./support/app";
import { MOCK_API_KEY, type MockAnthropic, startMockAnthropic } from "./support/mock-anthropic";
import {
  JEV_AD_PROBABILITY,
  JEV_MODEL,
  MOCK_TYPESAFE_KEY,
  type MockTypesafe,
  startMockTypesafe,
} from "./support/mock-typesafe";
import { type PublisherPage, startPublisherPage, UNDECIDED_SCRIPT } from "./support/publisher-page";

const MAIN = "//main";
const HAIKU = "claude-haiku-5-5";
const JEV_SWITCH = '//button[@role="switch"][@id=//label[normalize-space(.)="Classify scripts with Jev"]/@for]';
const SCRIPT_ROW = `${labelled("Scripts")}//*[@role="row"][contains(., ${JSON.stringify(UNDECIDED_SCRIPT)})]`;

let api: MockAnthropic;
let jev: MockTypesafe;
let page: PublisherPage;
let app: App;

beforeAll(async () => {
  api = await startMockAnthropic();
  jev = await startMockTypesafe();
  page = await startPublisherPage();
  app = await launchApp({
    ADSCAN_ANTHROPIC_BASE_URL: api.baseUrl,
    ADSCAN_API_KEY: MOCK_API_KEY,
    ADSCAN_TYPESAFE_BASE_URL: jev.baseUrl,
    ADSCAN_TYPESAFE_API_KEY: MOCK_TYPESAFE_KEY,
  });
});

afterAll(async () => {
  await app?.close();
  await page?.close();
  await jev?.close();
  await api?.close();
});

/** Scans the fixture page from the composer and returns the scan's id once its report is up. */
async function scanFixturePage(): Promise<string> {
  const { browser } = app;
  await browser.$('//nav//a[normalize-space(.)="Home"]').click();
  await browser.$(labelled("Page URL")).setValue(page.url);
  await browser.$(`${labelled("New scan")}//button[@type="submit"]`).click();
  await browser.$(labelled("Pipeline")).waitForDisplayed();
  const scanId = (await hashRoute(browser)).replace("/scans/", "");
  await browser.$(reportTab("Scripts")).waitForDisplayed({ timeout: 90_000 });
  return scanId;
}

const scriptOf = (detail: ScanDetail) => detail.scripts.find((script) => script.url.includes(UNDECIDED_SCRIPT));
const stepOf = (detail: ScanDetail, node: string) => detail.steps.find((step) => step.node === node);

test("a stored TypeSafe key leaves Jev off until it is switched on in Settings", async () => {
  const { browser } = app;
  const before = await invokeOk<Settings>(browser, "get_settings");
  expect(before).toMatchObject({ hasApiKey: true, hasTypesafeKey: true, jevEnabled: false });

  // Keys are checked against the provider they are for, and only that provider's key is replaced.
  const rejected = await invokeOk<ApiKeyStatus>(browser, "set_api_key", { key: "not-the-key", provider: "typesafe" });
  expect(rejected).toEqual({ valid: false, message: "TypeSafe rejected this API key." });
  expect(await invokeOk<Settings>(browser, "get_settings")).toMatchObject({ hasApiKey: true, hasTypesafeKey: true });

  await browser.$('//nav//a[normalize-space(.)="Settings"]').click();
  const settings = await seeText(browser, "Classify scripts with Jev", { within: MAIN });
  expect(settings).toContain("api.typesafe.ai");
  await browser.$(JEV_SWITCH).click();
  await browser.waitUntil(async () => (await invokeOk<Settings>(browser, "get_settings")).jevEnabled);
  await seeText(browser, "Jev System One", { within: MAIN });

  const observe = browser.$(field("Observe after load"));
  await observe.setValue("5");
  await browser.keys("Enter");
  await browser.waitUntil(async () => (await invokeOk<Settings>(browser, "get_settings")).defaultObserveSeconds === 5);
  expect(jev.requests).toEqual([]);
});

test("Jev classifies the undecided script, the filter model is never asked, and the analysis gets a reading guide", async () => {
  const { browser } = app;
  const scanId = await scanFixturePage();

  await browser.$(reportTab("Scripts")).click();
  const percent = `${Math.round(JEV_AD_PROBABILITY * 100)}%`;
  const row = await seeText(browser, "Jev", { within: SCRIPT_ROW });
  expect(row).toMatch(/Wrapper/);
  expect(row).toContain(percent);
  await browser.$(SCRIPT_ROW).click();
  const details = await seeText(browser, "Jev System One", { within: labelled("Script details") });
  expect(details).toMatch(new RegExp(`Ad probability\\s+${percent}`));

  const detail = await invokeOk<ScanDetail>(browser, "get_scan", { scanId });
  expect(scriptOf(detail)).toMatchObject({
    category: "wrapper",
    adRelated: true,
    classifiedBy: "ai",
    classifierModel: JEV_MODEL,
    adProbability: JEV_AD_PROBABILITY,
    analyzed: true,
  });
  expect(stepOf(detail, "filterScripts")).toMatchObject({ status: "completed", model: JEV_MODEL });
  expect(stepOf(detail, "analyzeLogic")?.model).toBe(`${JEV_MODEL}, claude-sonnet-5-5`);
  expect(detail.summary.costUsd).toBeGreaterThan(0);

  // Triage asked the two triage questions about the script's source; the guide asked all eight.
  const triage = jev.requests.filter((request) => Object.keys(request.questions).length === 2);
  expect(triage).toHaveLength(1);
  expect(triage[0]?.model).toBe(JEV_MODEL);
  expect(triage[0]?.state.script.url).toBe(`${page.url}${UNDECIDED_SCRIPT}`);
  expect(triage[0]?.state.code).toContain("loadAds(window.adUnits)");
  expect(Object.keys(triage[0]?.questions ?? {})).toEqual(["ad_orchestration", "publisher_specific"]);
  const guide = jev.requests.filter((request) => Object.keys(request.questions).length === 8);
  expect(guide.length).toBeGreaterThan(0);
  expect(guide.every((request) => / of \d+$/.test(request.state.script.lines ?? ""))).toBe(true);

  expect(api.requests.some((request) => request.model === HAIKU)).toBe(false);
  const opening = api.requests.find((request) => request.model === "claude-sonnet-5-5")?.messages[0]?.content[0]?.text;
  expect(opening).toContain("# Reading guide");
  expect(opening).toContain("# Source to start with");

  const logs = await invokeOk<LogEntry[]>(browser, "get_logs", { scanId });
  const decision = logs.find((entry) => entry.kind === "classification");
  expect(decision?.message).toContain("[jev-yes]");
  expect(decision?.data).toMatchObject({
    classifier: "jev",
    model: JEV_MODEL,
    decision: { adRelated: true, category: "wrapper", route: "jev-yes" },
  });
  expect(JSON.stringify(logs)).not.toContain(MOCK_TYPESAFE_KEY);
});

test("when Jev fails mid-scan the filter model classifies the script and the audit trail says why", async () => {
  const { browser } = app;
  jev.failWith(429);
  const asked = jev.requests.length;
  const scanId = await scanFixturePage();

  const detail = await invokeOk<ScanDetail>(browser, "get_scan", { scanId });
  expect(detail.summary.status).toBe("completed");
  expect(scriptOf(detail)).toMatchObject({ category: "wrapper", classifierModel: HAIKU, adProbability: null });
  expect(stepOf(detail, "filterScripts")).toMatchObject({ status: "completed", model: HAIKU });
  // The first failure ends Jev for the scan: its three attempts at one request, and no reading guide.
  expect(jev.requests.length - asked).toBe(3);
  const classification = api.requests.filter((request) => request.model === HAIKU);
  expect(classification).toHaveLength(1);
  expect(classification[0]?.messages[0]?.content[0]?.text).toContain(`${page.url}${UNDECIDED_SCRIPT}`);

  await browser.$(reportTab("Audit trail")).click();
  await browser.$(`${labelled("Pipeline steps")}//button[contains(., "Filter scripts")]`).click();
  const entries = await seeText(browser, "Jev System One failed during script triage", {
    within: labelled("Log entries"),
  });
  expect(entries).toContain(`1 scripts fall back to ${HAIKU}`);
});
