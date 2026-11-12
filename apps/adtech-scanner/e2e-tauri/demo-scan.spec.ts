import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterAll, beforeAll, expect, test } from "vitest";
import type { BrowserInfo, ScanDetail, ScanList, Settings } from "../src/bindings";
import {
  type App,
  button,
  field,
  goTo,
  hashRoute,
  invokeOk,
  labelled,
  launchApp,
  reportTab,
  seeText,
  textOf,
  withRole,
} from "./support/app";
import { type MockAnthropic, startMockAnthropic } from "./support/mock-anthropic";
import { canDriveNativeDialogs, nativeDialog } from "./support/native-dialog";
import { installScanProbe, readScanProbe } from "./support/scan-probe";

const PIPELINE = labelled("Pipeline");
const ACTIVITY = withRole("log", "Activity");
const MAIN = "//main";
const SIDEBAR = "//aside";

let api: MockAnthropic;
let app: App;
let scanId = "";

beforeAll(async () => {
  // The app is given no key; the mock only exists so that no request can reach the real API.
  api = await startMockAnthropic();
  app = await launchApp({ ADSCAN_ANTHROPIC_BASE_URL: api.baseUrl });
});

afterAll(async () => {
  await app?.close();
  await api?.close();
});

async function reloadApp() {
  await app.browser.refresh();
  await app.browser.$(button("New scan")).waitForDisplayed({ timeout: 20_000 });
}

test("onboarding finds the browser and does not need an API key", async () => {
  const { browser } = app;
  const welcome = await seeText(browser, "Browser detected", { within: MAIN });
  expect(welcome).toContain("Find the revenue your ad stack is leaking");
  expect(welcome).toContain("Scans work without a key");
  expect(await browser.getTitle()).toBe("Welcome · Ad-Tech Scanner");

  await browser.$(labelled("Anthropic API key")).setValue("sk-ant-not-a-real-key");
  await browser.$(button("Save key")).click();
  await seeText(browser, "Anthropic rejected this API key.", { within: `${MAIN}//*[@role="alert"]` });
  expect((await invokeOk<Settings>(browser, "get_settings")).hasApiKey).toBe(false);
});

test("a demo-site scan streams through the pipeline and becomes the report", async () => {
  const { browser } = app;
  await installScanProbe(browser);
  await browser.$(button("Scan the demo site")).click();

  await browser.$(PIPELINE).waitForDisplayed();
  expect(await browser.$$(`${PIPELINE}/li`).length).toBe(9);
  scanId = (await hashRoute(browser)).replace("/scans/", "");
  expect(scanId).toMatch(/^[0-9a-f-]{36}$/);

  // Everything below reaches the screen only through `scan-event`.
  await seeText(browser, /Step 2 of 9: Capture page/, { within: MAIN, timeout: 15_000 });
  await seeText(browser, "RUNNING", { within: SIDEBAR });
  await seeText(browser, "Prebid.js v10.14.0 detected", { within: ACTIVITY, timeout: 15_000 });
  await seeText(browser, "Page scrolled for the first time", { within: ACTIVITY, timeout: 30_000 });
  expect(await browser.getTitle()).toContain("127.0.0.1 (scanning)");
  expect(await browser.$(labelled("LLM cost against the per-scan budget")).isDisplayed()).toBe(true);

  await browser.$(reportTab("Findings")).waitForDisplayed({ timeout: 90_000 });
  const header = await seeText(browser, "Deterministic checks only", { within: MAIN });
  expect(header).toMatch(/127\.0\.0\.1 \(desktop\) runs Prebid\.js with Google Publisher Tag/);
  expect(await textOf(browser, SIDEBAR)).not.toContain("RUNNING");

  const probe = await readScanProbe(browser);
  const kinds = new Set(probe.events.map((event) => event.type));
  expect([...kinds].sort()).toEqual(["findingAdded", "log", "scanUpdated", "stepUpdated"]);
  // The live view swaps to the report on a refetch triggered by the final event,
  // so the detail has to be complete the moment that event is out.
  const atFinalEvent = probe.detailAtFinalEvent;
  expect(atFinalEvent?.summary.status).toBe("completed");
  expect(atFinalEvent?.steps.map((step) => step.status)).toEqual([
    "completed",
    "completed",
    "completed",
    "completed",
    "completed",
    "skipped",
    "skipped",
    "skipped",
    "completed",
  ]);
  // The bidder checks read the demo site's own ads.txt over loopback, and nothing else.
  const checked = atFinalEvent?.bidderDiagnostics;
  expect(checked?.adsTxt).toMatchObject({
    url: expect.stringMatching(/^http:\/\/127\.0\.0\.1:\d+\/ads\.txt$/),
    status: "found",
    recordCount: 6,
    invalidLineCount: 0,
    ownerDomain: "meridian-media.example",
  });
  expect(checked?.sellersJsonChecked).toBe(false);
  const problemsOf = (bidder: string) =>
    checked?.bidders
      .find((diagnosis) => diagnosis.bidder === bidder)
      ?.checks.filter((check) => check.status === "problem")
      .map((check) => check.code);
  expect(problemsOf("ix")).toEqual(["ads-txt-not-listed"]);
  expect(problemsOf("openx")).toEqual(["response-reason"]);
  expect(problemsOf("triplelift")).toEqual(["params-missing"]);
  expect(problemsOf("appnexus")).toEqual([]);
  expect(atFinalEvent).toEqual(await invokeOk<ScanDetail>(browser, "get_scan", { scanId }));
  expect(atFinalEvent?.overview).toBeTruthy();
  expect(atFinalEvent?.summary.healthScore).toBeLessThan(100);
});

test("every report tab shows what the scan captured, including the planted bugs", async () => {
  const { browser } = app;

  const findings = await seeText(browser, "findings", { within: `${MAIN}//*[@role="tabpanel"]` });
  for (const planted of [
    "Bidding waits for the first scroll on desktop",
    "sent before the Prebid auction finished",
    "Auction timeout of 250 ms",
    "have no matching GPT slot",
    "loaded more than once",
    "2 bidder(s) never returned a bid",
    "1 bidder(s) have adapter params the adapter rejects",
  ]) {
    expect(findings).toContain(planted);
  }
  expect(findings).toContain("Deterministic rule");
  // Only the focused finding is expanded; it carries the evidence.
  expect(findings).toContain("ad-leaderboard");

  await browser.$(reportTab("Auctions")).click();
  const auctions = await seeText(browser, "By bidder", { within: MAIN });
  // WebKit rings every element focused from script, which is how a clicked tab gets its focus.
  const clickedTab = await browser.execute(() => {
    const tab = document.activeElement;
    return tab && { name: tab.textContent, outline: getComputedStyle(tab).outlineStyle };
  });
  expect(clickedTab).toEqual({ name: "Auctions3", outline: "none" });
  expect(auctions).toMatch(/Auction 1[\s\S]*ad-sticky-footer[\s\S]*Auction 2[\s\S]*ad-leaderboard/);
  expect(auctions).toContain("timeout 250 ms");
  expect(auctions).toMatch(/pubmatic\s+5\s+0\s+0\s+5/);
  expect(await browser.$$(`${labelled("Bids for ad-leaderboard")}/li`).length).toBe(6);

  // Bidder checks name the cause of each planted bidder bug, and nothing for the healthy bidders.
  const checks = await textOf(browser, labelled("Bidders that need attention"));
  expect(checks).toMatch(
    /ix\s+0 of 3 requests answered with a bid[\s\S]*ads\.txt: ads\.txt has no entry for indexexchange\.com/,
  );
  expect(checks).toMatch(
    /openx\s+0 of 3 requests answered with a bid[\s\S]*Endpoint: The bidder's endpoint explained why it did not bid on its 1 bid request\s+no-bid reason 7 \(blocked publisher or site\)/,
  );
  expect(checks).toMatch(
    /triplelift[\s\S]*Params: `inventoryCode` missing on 2 of 3 ad units\s+ad-sidebar-1, ad-inarticle-1/,
  );
  expect(checks).not.toContain("appnexus");
  const facts = await seeText(browser, "Bidder checks", { within: MAIN });
  expect(facts).toMatch(/ads\.txt\s+http:\/\/127\.0\.0\.1:\d+\/ads\.txt: 6 seller records/);
  expect(facts).toContain("Not looked up for this scan");
  expect(facts).toContain("No TCF consent API on this page");
  expect(facts).toMatch(/Nothing found for appnexus, criteo, pubmatic, rubicon\./);

  await browser.$(reportTab("Ad slots")).click();
  const slots = await seeText(browser, "ad-sticky-footer", { within: MAIN });
  expect(slots).toMatch(/ad-sticky-footer\s+Empty[\s\S]*Requested before auction end\s+No Prebid targeting/);
  expect(slots).toMatch(/ad-leaderboard\s+Filled\s+\/21804213\/meridian-post\/top/);

  await browser.$(reportTab("Scripts")).click();
  const scripts = await seeText(browser, "9 scripts", { within: MAIN });
  expect(scripts).toMatch(/Ad-related\s+5/);
  expect(scripts).toContain("gpt.js?v=adstack-4.2.1");
  await browser.$(`${labelled("Scripts")}//*[@role="row"][contains(., "hb-wrapper.min.js")]`).click();
  const details = await seeText(browser, "/static/ads/hb-wrapper.min.js", { within: labelled("Script details") });
  expect(details).toContain("Wrapper");

  await browser.$(reportTab("Audit trail")).click();
  const steps = await seeText(browser, "34 rules evaluated, 10 findings", { within: labelled("Pipeline steps") });
  expect(steps).toContain("7 bidders checked: 3 problems, 0 notices; ads.txt has 6 records; sellers.json not checked");
  expect(steps).toContain("AI analysis is turned off for this scan");
  await browser.$(`${labelled("Pipeline steps")}//button[contains(., "Deterministic checks")]`).click();
  // How many entries the trail has depends on how many gestures the visitor made; the step's share does not.
  await seeText(browser, /10 of \d+ entries[\s\S]*\[High\] Bidding waits for the first scroll on desktop/, {
    within: MAIN,
  });
});

test("a report deep link with search parameters survives a reload", async () => {
  const { browser } = app;
  await goTo(browser, `/scans/${scanId}?tab=slots`);
  await reloadApp();
  await browser.$(`${reportTab("Ad slots")}[@aria-selected="true"]`).waitForDisplayed();
  expect(await browser.getUrl()).toBe(`tauri://localhost/#/scans/${scanId}?tab=slots`);
  await seeText(browser, "ad-sticky-footer", { within: MAIN });
});

test("history lists the scan", async () => {
  const { browser } = app;
  await browser.$('//nav//a[normalize-space(.)="History"]').click();
  const table = await seeText(browser, "Showing 1 of 1 scan", { within: MAIN });
  expect(table).toMatch(/28\s+127\.0\.0\.1\s+Desktop\s+High: 3\s+Medium: 4\s+Low: 2/);

  await browser.$(labelled("Search scans")).setValue("nothing-like-this");
  await seeText(browser, "No scans match", { within: MAIN });
  await browser.$(labelled("Search scans")).setValue("127.0");
  await seeText(browser, "Showing 1 of 1 scan", { within: MAIN });
});

test("log search finds an audit-trail entry and opens it in its report", async () => {
  const { browser } = app;
  await browser.$('//nav//a[normalize-space(.)="Log search"]').click();
  await browser.$(labelled("Search all audit trails")).setValue("scrolled");
  await seeText(browser, "1 match in 1 scan", { within: MAIN });
  expect(await hashRoute(browser)).toBe("/logs?q=scrolled");
  const group = labelled("Matches in 127.0.0.1");
  expect(await textOf(browser, `${group}//mark`)).toBe("scrolled");

  await browser.$(`${group}//a[.//mark]`).click();
  await browser.$(`${reportTab("Audit trail")}[@aria-selected="true"]`).waitForDisplayed();
  const route = await hashRoute(browser);
  expect(route).toMatch(new RegExp(`^/scans/${scanId}\\?tab=audit&log=[0-9a-f-]{36}$`));
  const entry = `//*[@id="log-${route.split("log=")[1]}"]`;
  await seeText(browser, "Page scrolled for the first time", { within: entry });
  expect(await browser.$(entry).isDisplayed({ withinViewport: true })).toBe(true);

  await browser.$('//nav//a[normalize-space(.)="Log search"]').click();
  await browser.$(labelled("Search all audit trails")).setValue("zzzz-nothing-matches");
  await seeText(browser, "No matches for “zzzz-nothing-matches”", { within: MAIN });
});

test("settings are saved by the backend and survive a reload", async () => {
  const { browser } = app;
  await browser.$('//nav//a[normalize-space(.)="Settings"]').click();
  const budget = browser.$(field("Budget per scan"));
  await budget.waitForDisplayed();
  await budget.setValue("0.75");
  await browser.keys("Enter");
  await browser.$(`${labelled("Default device")}//button[normalize-space(.)="Mobile"]`).click();
  await browser.$(`${labelled("Theme")}//button[normalize-space(.)="Dark"]`).click();
  await browser.waitUntil(() => browser.execute(() => document.documentElement.classList.contains("dark")));
  const sellersJson = `//button[@role="switch"][@id=//label[normalize-space(.)="Check sellers.json at ad exchanges"]/@for]`;
  expect(await browser.$(sellersJson).getAttribute("aria-checked")).toBe("false");
  await browser.$(sellersJson).click();
  await expect
    .poll(() => invokeOk<Settings>(browser, "get_settings"), { timeout: 10_000 })
    .toMatchObject({ budgetUsd: 0.75, defaultDevice: "mobile", theme: "dark", checkSellersJson: true });

  await reloadApp();
  expect(await browser.execute(() => document.documentElement.classList.contains("dark"))).toBe(true);
  await browser.$(field("Budget per scan")).waitForDisplayed();
  expect(await browser.$(field("Budget per scan")).getValue()).toBe("0.75");

  await browser.$(`${labelled("Theme")}//button[normalize-space(.)="Light"]`).click();
  await browser.waitUntil(() => browser.execute(() => !document.documentElement.classList.contains("dark")));
});

test.skipIf(!canDriveNativeDialogs)("export writes the report where the native save dialog says", async () => {
  const { browser } = app;
  const exportDir = mkdtempSync(join(tmpdir(), "adscan-export-"));
  const target = join(exportDir, "report.md");
  const openSaveDialog = async () => {
    const markdown = browser.$(withRole("menuitem", "Markdown report"));
    // GTK can still hold the pointer for a dialog that has just closed and swallow the click.
    await browser.waitUntil(
      async () => {
        if (!(await markdown.isExisting())) await browser.$(button("Export")).click();
        return markdown.isExisting();
      },
      { interval: 500 },
    );
    await markdown.click();
  };
  try {
    await goTo(browser, `/scans/${scanId}`);
    await browser.$(button("Export")).waitForDisplayed();

    // Cancelling the dialog is not an error and writes nothing.
    await (await nativeDialog(browser, openSaveDialog)).cancel();
    await browser.waitUntil(() => browser.$(button("Export")).isEnabled());
    expect(await textOf(browser, "//body")).not.toMatch(/Report exported|Export failed/);

    await (await nativeDialog(browser, openSaveDialog)).submitPath(target);
    await seeText(browser, "Report exported", { timeout: 15_000 });
    const document = readFileSync(target, "utf8");
    expect(document).toContain("Bidding waits for the first scroll on desktop");
    expect(document).toBe(await invokeOk<string>(browser, "export_scan", { scanId, format: "markdown" }));
  } finally {
    rmSync(exportDir, { recursive: true, force: true });
  }
});

test.skipIf(!canDriveNativeDialogs)(
  "a browser picked in the native open dialog is used until auto-detect is restored",
  async () => {
    const { browser } = app;
    const { path } = await invokeOk<BrowserInfo>(browser, "detect_browser");
    await browser.$('//nav//a[normalize-space(.)="Settings"]').click();
    await seeText(browser, "(detected automatically)", { within: MAIN });

    const picker = await nativeDialog(browser, () => browser.$(button("Choose executable")).click());
    await picker.submitPath(path);
    await seeText(browser, "(chosen manually)", { within: MAIN, timeout: 15_000 });
    expect((await invokeOk<Settings>(browser, "get_settings")).chromePath).toBe(path);

    await browser.$(button("Use auto-detect")).click();
    await seeText(browser, "(detected automatically)", { within: MAIN });
    expect((await invokeOk<Settings>(browser, "get_settings")).chromePath).toBeNull();
  },
);

test("deleting the scan removes its report and its audit trail", async () => {
  const { browser } = app;
  await browser.$('//nav//a[normalize-space(.)="History"]').click();
  await browser.$(button("Actions for 127.0.0.1")).click();
  await browser.$(withRole("menuitem", "Delete")).click();
  await seeText(browser, "Delete the scan of 127.0.0.1?", { within: '//*[@role="alertdialog"]' });
  await browser.$('//*[@role="alertdialog"]//button[normalize-space(.)="Delete"]').click();
  await seeText(browser, "Deleted the scan of 127.0.0.1");
  await seeText(browser, "No scans yet", { within: MAIN });

  expect((await invokeOk<ScanList>(browser, "list_scans", { query: {} })).total).toBe(0);
  expect(await invokeOk<unknown[]>(browser, "search_logs", { query: "scrolled" })).toEqual([]);
  await goTo(browser, `/scans/${scanId}`);
  await seeText(browser, "This scan no longer exists", { within: MAIN });
});
