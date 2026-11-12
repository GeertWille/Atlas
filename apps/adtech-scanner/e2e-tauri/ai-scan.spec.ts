import { afterAll, beforeAll, expect, test } from "vitest";
import type { LogSearchHit, ScanDetail } from "../src/bindings";
import {
  type App,
  button,
  field,
  hashRoute,
  invokeOk,
  labelled,
  launchApp,
  reportTab,
  seeText,
  textOf,
  withRole,
} from "./support/app";
import { cspViolations, watchCspViolations } from "./support/csp";
import {
  AI_CLASSIFICATION_REASON,
  AI_FINDING_TITLE,
  AI_OVERVIEW,
  MOCK_API_KEY,
  type MockAnthropic,
  startMockAnthropic,
} from "./support/mock-anthropic";
import { type PublisherPage, startPublisherPage, UNDECIDED_SCRIPT } from "./support/publisher-page";

const ACTIVITY = withRole("log", "Activity");
const MAIN = "//main";

let api: MockAnthropic;
let page: PublisherPage;
let app: App;
let scanId = "";

beforeAll(async () => {
  api = await startMockAnthropic();
  page = await startPublisherPage();
  app = await launchApp({ ADSCAN_ANTHROPIC_BASE_URL: api.baseUrl, ADSCAN_API_KEY: MOCK_API_KEY });
});

afterAll(async () => {
  await app?.close();
  await page?.close();
  await api?.close();
});

test("with a key the composer offers AI analysis and the scan runs both model phases", async () => {
  const { browser } = app;
  await watchCspViolations(browser);
  // A browser and a key are both there, so there is nothing to set up.
  const composer = await seeText(browser, "hard cap $0.50 per scan", { within: labelled("New scan"), timeout: 15_000 });
  expect(composer).toContain("claude-sonnet-5-5");

  const demoUrl = await invokeOk<string>(browser, "get_demo_url");
  await browser.$(labelled("Page URL")).setValue(demoUrl);
  await browser.$(`${labelled("New scan")}//button[@type="submit"]`).click();
  await browser.$(labelled("Pipeline")).waitForDisplayed();
  scanId = (await hashRoute(browser)).replace("/scans/", "");

  // Phase 2 arrives as it happens: the tool call, the finding it reported, and what the calls cost.
  await seeText(browser, "search_scripts", { within: ACTIVITY, timeout: 90_000 });
  await seeText(browser, AI_FINDING_TITLE, { within: labelled("Findings so far"), timeout: 30_000 });
  await browser.waitUntil(async () => {
    const spent = await browser.$(labelled("LLM cost against the per-scan budget")).getAttribute("aria-valuenow");
    return Number(spent) > 0;
  });

  await browser.$(reportTab("Findings")).waitForDisplayed({ timeout: 90_000 });
  const report = await seeText(browser, AI_FINDING_TITLE, { within: MAIN });
  expect(report).toContain(AI_OVERVIEW);
  expect(report).not.toContain("Deterministic checks only");
});

test("the AI finding carries verified code evidence that links to its script", async () => {
  const { browser } = app;
  const card = `//article[@aria-label=${JSON.stringify(AI_FINDING_TITLE)}]`;
  const finding = await seeText(browser, "min-width", { within: card });
  expect(finding).toContain("Critical");
  expect(finding).toContain("AI analysis");
  expect(finding).toContain("~25% revenue at risk");
  expect(finding).toContain("Inverted media query");

  await browser.$(`${card}//button[contains(., "hb-wrapper.min.js:")]`).click();
  await browser.$(`${reportTab("Scripts")}[@aria-selected="true"]`).waitForDisplayed();
  expect(await hashRoute(browser)).toContain("tab=scripts&script=");
  const details = await seeText(browser, "hb-wrapper.min.js", { within: labelled("Script details") });
  expect(details).toMatch(/Phase 2/);
  expect(await textOf(browser, labelled("Script volume by stage"))).toMatch(/Analysed in Phase 2\s+[1-9]/);
});

test("the audit trail keeps the model conversation and full-text search finds it", async () => {
  const { browser } = app;
  await browser.$(reportTab("Audit trail")).click();
  const steps = await seeText(browser, "Analyse logic", { within: labelled("Pipeline steps") });
  expect(steps).toMatch(/Analyse logic[\s\S]*tokens · \$0\.\d+/);
  await browser.$(`${labelled("Pipeline steps")}//button[contains(., "Analyse logic")]`).click();
  const entries = await seeText(browser, "report_finding", { within: labelled("Log entries") });
  expect(entries).toContain("search_scripts");
  expect(entries).toContain("Mock: looking for the device check behind the scroll gate.");

  await browser.$('//nav//a[normalize-space(.)="Log search"]').click();
  await browser.$(button("report_finding")).click();
  await seeText(browser, /\d+ match(es)? in 1 scan/, { within: MAIN });
  // The index splits identifiers at the underscore, so each half is a match of its own.
  const group = labelled("Matches in 127.0.0.1");
  expect(await textOf(browser, group)).toContain("report_finding");
  expect(await textOf(browser, `${group}//mark[1]`)).toBe("report");
  expect(await textOf(browser, `${group}//mark[2]`)).toBe("finding");

  const hits = await invokeOk<LogSearchHit[]>(browser, "search_logs", { query: "min-width", limit: 5 });
  expect(hits.length).toBeGreaterThan(0);
  expect(hits.every((hit) => hit.entry.scanId === scanId && hit.snippet.includes("<mark>"))).toBe(true);
});

test("the stored scan matches what the model was asked and what it cost", async () => {
  const detail = await invokeOk<ScanDetail>(app.browser, "get_scan", { scanId });
  expect(detail.summary.aiUsed).toBe(true);
  expect(detail.summary.costUsd).toBeGreaterThan(0);
  const statuses = Object.fromEntries(detail.steps.map((step) => [step.node, step.status]));
  // The knowledge base decides every demo-site script, so Phase 1 has nothing to ask.
  expect(statuses).toMatchObject({ filterScripts: "skipped", beautify: "completed", analyzeLogic: "completed" });
  const aiFindings = detail.findings.filter((finding) => finding.source === "ai");
  expect(aiFindings.map((finding) => finding.title)).toEqual([AI_FINDING_TITLE]);
  expect(aiFindings[0]?.evidence[0]?.scriptUrl).toContain("/static/ads/hb-wrapper.min.js");

  const analysisTurns = api.requests.filter((request) => request.model === "claude-sonnet-5-5");
  expect(analysisTurns.map((request) => request.messages.length)).toEqual([1, 3, 5]);
  expect(api.requests.some((request) => request.model === "claude-haiku-5-5")).toBe(false);
});

test("a script the knowledge base cannot decide is classified by Phase 1 and analysed by Phase 2", async () => {
  const { browser } = app;
  await browser.$('//nav//a[normalize-space(.)="Settings"]').click();
  const observe = browser.$(field("Observe after load"));
  await observe.waitForDisplayed();
  await observe.setValue("5");
  await browser.keys("Enter");
  await seeText(browser, "Settings saved");

  await browser.$('//nav//a[normalize-space(.)="Home"]').click();
  await browser.$(labelled("Page URL")).setValue(page.url);
  await browser.$(`${labelled("New scan")}//button[@type="submit"]`).click();
  await browser.$(labelled("Pipeline")).waitForDisplayed();
  await seeText(browser, /Classified 1 of 1 undecided scripts; 1 scripts are ad-related/, {
    within: labelled("Pipeline"),
    timeout: 60_000,
  });
  await browser.$(reportTab("Scripts")).waitForDisplayed({ timeout: 60_000 });

  await browser.$(reportTab("Scripts")).click();
  const row = `${labelled("Scripts")}//*[@role="row"][contains(., ${JSON.stringify(UNDECIDED_SCRIPT)})]`;
  const classified = await seeText(browser, AI_CLASSIFICATION_REASON, { within: row });
  expect(classified).toMatch(/Wrapper[\s\S]*AI · 90%/);
  await browser.$(reportTab("Findings")).click();
  await seeText(browser, AI_FINDING_TITLE, { within: MAIN });

  for (const tab of ["Auctions", "Ad slots", "Audit trail"]) {
    await browser.$(reportTab(tab)).click();
    await browser.$(`${reportTab(tab)}[@aria-selected="true"]`).waitForDisplayed();
  }
  // Nothing any screen rendered in this session, live or report, went against the production CSP.
  expect(await cspViolations(browser)).toEqual([]);

  const classification = api.requests.filter((request) => request.model === "claude-haiku-5-5");
  expect(classification).toHaveLength(1);
  expect(classification[0]?.messages[0]?.content[0]?.text).toContain(`${page.url}${UNDECIDED_SCRIPT}`);
});
