import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { afterAll, beforeAll, describe, test } from "vitest";
import type { ScanSummary } from "../src/bindings";
import { type App, type Browser, button, goTo, invokeOk, labelled, launchApp, reportTab, seeText } from "./support/app";
import { AI_FINDING_TITLE, MOCK_API_KEY, type MockAnthropic, startMockAnthropic } from "./support/mock-anthropic";

/**
 * Not a test: saves a picture of every main screen of the real window, light
 * and dark, for review by eye. Runs only when told where to put them:
 * `TAURI_E2E_SCREENSHOTS=/some/dir pnpm test:tauri screens`.
 */
const outDir = process.env.TAURI_E2E_SCREENSHOTS;
const THEMES = ["light", "dark"] as const;

async function shot(browser: Browser, name: string) {
  // Let entrance animations and the compositor settle.
  await browser.pause(700);
  writeFileSync(join(outDir ?? "", `${name}.png`), Buffer.from(await browser.takeScreenshot(), "base64"));
}

async function switchTheme(browser: Browser, theme: (typeof THEMES)[number]) {
  await invokeOk(browser, "update_settings", { patch: { theme } });
  await browser.refresh();
  await browser.$(button("New scan")).waitForDisplayed({ timeout: 20_000 });
}

const scrollMain = (browser: Browser, top: number) =>
  browser.execute((y: number) => document.querySelector("main")?.scrollTo(0, y), top);

describe.skipIf(!outDir)("screenshots of the real window", () => {
  let api: MockAnthropic;
  let app: App;

  beforeAll(async () => {
    mkdirSync(outDir ?? "", { recursive: true });
    api = await startMockAnthropic();
  });

  afterAll(async () => {
    await app?.close();
    await api?.close();
  });

  test("first run without a key", async () => {
    app = await launchApp({ ADSCAN_ANTHROPIC_BASE_URL: api.baseUrl });
    const { browser } = app;
    for (const theme of THEMES) {
      await switchTheme(browser, theme);
      await seeText(browser, "Browser detected");
      await shot(browser, `onboarding-${theme}`);
    }
    await app.close();
  });

  test("every screen with an AI scan of the demo site", async () => {
    app = await launchApp({ ADSCAN_ANTHROPIC_BASE_URL: api.baseUrl, ADSCAN_API_KEY: MOCK_API_KEY });
    const { browser } = app;
    const url = await invokeOk<string>(browser, "get_demo_url");
    await browser.$(labelled("Page URL")).setValue(url);
    await browser.$(button("Options")).click();
    await shot(browser, "home-composer-light");
    await browser.$(`${labelled("New scan")}//button[@type="submit"]`).click();
    await seeText(browser, "Page scrolled for the first time", { timeout: 60_000 });
    await shot(browser, "live-capture-light");
    await seeText(browser, AI_FINDING_TITLE, { within: labelled("Findings so far"), timeout: 60_000 });
    await shot(browser, "live-analysis-light");
    await browser.$(reportTab("Findings")).waitForDisplayed({ timeout: 60_000 });
    const { items } = await invokeOk<{ items: ScanSummary[] }>(browser, "list_scans", { query: {} });
    const scan = `/scans/${items[0]?.id}`;

    for (const theme of THEMES) {
      await switchTheme(browser, theme);
      await goTo(browser, "/");
      await seeText(browser, "Recent scans");
      await shot(browser, `home-${theme}`);

      await goTo(browser, scan);
      await browser.$(reportTab("Findings")).waitForDisplayed();
      await shot(browser, `report-findings-${theme}`);
      await scrollMain(browser, 520);
      await shot(browser, `report-findings-evidence-${theme}`);
      for (const [tab, name] of [
        ["Auctions", "auctions"],
        ["Ad slots", "slots"],
        ["Scripts", "scripts"],
        ["Audit trail", "audit"],
      ] as const) {
        await browser.$(reportTab(tab)).click();
        await scrollMain(browser, 500);
        await shot(browser, `report-${name}-${theme}`);
      }
      await browser.$(`${labelled("Pipeline steps")}//button[contains(., "Analyse logic")]`).click();
      await browser.$(`${labelled("Log entries")}//button[contains(., "report_finding")]`).click();
      await shot(browser, `report-audit-tool-call-${theme}`);

      await goTo(browser, "/history");
      await seeText(browser, "Showing 1 of 1 scan");
      await shot(browser, `history-${theme}`);
      await goTo(browser, "/logs?q=min-width");
      await seeText(browser, /match(es)? in 1 scan/);
      await shot(browser, `log-search-${theme}`);
      await goTo(browser, "/settings");
      await seeText(browser, "Budget per scan");
      await shot(browser, `settings-${theme}`);
      await scrollMain(browser, 10_000);
      await shot(browser, `settings-bottom-${theme}`);
      await browser.keys(["Control", "k"]);
      await browser.$('//*[@role="dialog"]//input').waitForDisplayed();
      await shot(browser, `command-palette-${theme}`);
      await browser.keys("Escape");
    }

    // A second scan, for the live view in the dark theme.
    await goTo(browser, "/");
    await browser.$(labelled("Page URL")).setValue(url);
    await browser.$(`${labelled("New scan")}//button[@type="submit"]`).click();
    await seeText(browser, "Page scrolled for the first time", { timeout: 60_000 });
    await shot(browser, "live-capture-dark");
  });
});
