import { expect, type Page, test } from "@playwright/test";
import {
  enableJev,
  expectNoAxeViolations,
  expectNoHorizontalOverflow,
  openApp,
  reportHealth,
  reportTab,
  startScan,
} from "./app";

/** Toasts fade out over the page; axe would judge their half-transparent text. */
async function toastsGone(page: Page) {
  await expect(page.locator("[data-sonner-toast]")).toHaveCount(0, { timeout: 10_000 });
}

async function scanWithJev(page: Page, options: Parameters<typeof openApp>[2] = {}) {
  await openApp(page, "/settings", options);
  await enableJev(page);
  await page.getByRole("link", { name: "Home", exact: true }).click();
  await startScan(page, "sportsdesk.example/live");
}

test("a scan with Jev shows who classified each script and how each decision was reached", async ({ page }) => {
  await openApp(page, "/settings", { speed: 8 });
  await enableJev(page);
  await page.getByRole("link", { name: "Home", exact: true }).click();
  await expect(page.getByText(/with claude-sonnet-5-5 and Jev script triage, hard cap/)).toBeVisible();
  await startScan(page, "sportsdesk.example/live");

  // Decisions stream into the live view as Jev makes them.
  const activity = page.getByRole("log", { name: "Activity" });
  await expect(activity.getByText(/hb-wrapper\.min\.js: ad orchestration 0\.\d\d .* \[jev-yes\]/)).toBeVisible({
    timeout: 15_000,
  });
  await expect(reportHealth(page)).toBeVisible({ timeout: 25_000 });
  await expect(page.getByRole("list", { name: "Scan caveats" })).toHaveCount(0);

  // Scripts: Jev rows carry the ad probability; rule rows are unchanged.
  await reportTab(page, "Scripts").click();
  const lastCell = (name: RegExp) => page.getByRole("row", { name }).getByRole("cell").last();
  await expect(lastCell(/hb-wrapper\.min\.js/)).toHaveText(/^Jev · \d+% ad$/);
  await expect(lastCell(/main\.4f2a9c\.js/)).toHaveText(/^Jev · \d% ad$/);
  await expect(lastCell(/gpt\.js/)).toHaveText("Rule · 100%");
  await page.getByRole("row", { name: /hb-wrapper\.min\.js/ }).click();
  const details = page.getByRole("region", { name: "Script details" });
  await expect(details).toContainText("Classified byJev System One");
  await expect(details).toContainText(/Ad probability\d+%/);
  await expect(details).toContainText("rule jev-yes (>= 0.50)");
  await expect(details).toContainText("handed to Phase 2");

  // Audit trail: the question set once, then one decision per script.
  await reportTab(page, "Audit trail").click();
  const steps = page.getByRole("navigation", { name: "Pipeline steps" });
  await steps.getByRole("button", { name: /Filter scripts/ }).click();
  const entries = page.getByRole("region", { name: "Log entries" });
  await expect(
    entries.getByText(/^Jev System One: 14 scripts, up to \d+ parts to judge, 2 questions each\./),
  ).toBeVisible();
  await page.getByRole("button", { name: "Filter by kind" }).click();
  await page.getByRole("menuitemcheckbox", { name: /^Classification/ }).click();
  await page.keyboard.press("Escape");
  await expect(page.getByText(/^14 of \d+ entries$/)).toBeVisible();

  // Drop, keep-when-unsure and analyse are all told apart at a glance and explained when opened.
  await expect(entries.getByRole("button", { name: /Not ad-related s2 \S+main\.4f2a9c\.js/ })).toBeVisible();
  const unsure = entries.getByRole("button", { name: /kept for analysis, ranked last \[jev-unsure\]/ }).first();
  await expect(unsure).toContainText("Ad-related");
  const wrapper = entries.getByRole("button", { name: /Ad-related s7 \S+hb-wrapper\.min\.js/ });
  await wrapper.focus();
  await page.keyboard.press("Enter");
  await expect(wrapper).toHaveAttribute("aria-expanded", "true");
  const opened = entries.locator('[data-kind="classification"]', { hasText: "hb-wrapper.min.js" });
  await expect(opened.getByRole("term")).toHaveText(["ad_orchestration", "publisher_specific"]);
  await expect(opened).toContainText("Thresholds analyse 0.50, drop 0.15, publisherSpecific 0.50");
  await expect(opened).toContainText("Rule jev-yes");
  await expect(opened).toContainText("var IS_MOBILE = window.screen.width < 1440;");

  // The reading guide Jev built for the logic analysis is on record too.
  await page.getByRole("button", { name: "Filter by kind" }).click();
  await page.getByRole("menuitemcheckbox", { name: /^Classification/ }).click();
  await page.keyboard.press("Escape");
  await steps.getByRole("button", { name: /Analyse logic/ }).click();
  await expect(entries.getByText(/^Jev System One reading guide: \d+ chunks of the selected scripts/)).toBeVisible();
});

test("when Jev fails mid-scan, the fallback to Claude Haiku is called out in the report and the audit trail", async ({
  page,
}) => {
  await scanWithJev(page, { jev: "fail" });
  await expect(reportHealth(page)).toBeVisible({ timeout: 25_000 });

  const overview = page.getByRole("region", { name: "Overview" });
  await expect(overview.getByRole("list", { name: "Scan caveats" }).getByRole("listitem")).toHaveText(
    "Jev System One failed during script triage (rate limited by the API after 3 attempts); 10 scripts fall back to claude-haiku-5-5.",
  );
  // The markdown label the backend writes is not shown raw.
  await expect(overview).not.toContainText("**");
  // The scan still produced its AI findings.
  await expect(
    page.getByRole("article", { name: /Desktop screens under 1440 px are treated as mobile/ }),
  ).toBeVisible();

  await reportTab(page, "Scripts").click();
  await expect(
    page
      .getByRole("row", { name: /main\.4f2a9c\.js/ })
      .getByRole("cell")
      .last(),
  ).toHaveText(/^Jev · \d% ad$/);
  await expect(
    page
      .getByRole("row", { name: /hb-wrapper\.min\.js/ })
      .getByRole("cell")
      .last(),
  ).toHaveText(/^AI · \d+%$/);

  await reportTab(page, "Audit trail").click();
  await page.getByRole("button", { name: "Filter by level" }).click();
  await page.getByRole("menuitemcheckbox", { name: /^Warning/ }).click();
  await page.keyboard.press("Escape");
  const warning = page
    .getByRole("region", { name: "Log entries" })
    .getByRole("button", { name: /Jev System One failed during script triage/ });
  await expect(warning).toBeVisible();
  await expect(warning.locator("xpath=ancestor::*[@data-level][1]")).toHaveAttribute("data-level", "warn");
  await warning.click();
  await expect(page.getByRole("region", { name: "Log entries" })).toContainText('"remainingScripts": 10');
});

test("with only a TypeSafe key the composer offers script triage and the scan skips logic analysis", async ({
  page,
}) => {
  await openApp(page, "/settings", { profile: "fresh" });
  const triage = page.locator("[data-slot=field]", { hasText: "Script triage" });
  await expect(triage).toContainText("None");
  await enableJev(page);
  await expect(triage).toContainText("Jev System One");
  await expect(triage).toContainText("Logic analysis needs an Anthropic key.");

  // A stored TypeSafe key counts as set up: Home opens on the composer, not on the welcome screen.
  await page.getByRole("link", { name: "Home", exact: true }).click();
  await expect(page).toHaveTitle("Home · Ad-Tech Scanner");
  await expect(page.getByRole("button", { name: /Continue/ })).toHaveCount(0);

  await expect(page.getByText(/Estimated \$0\.006 for Jev script triage\./)).toBeVisible();
  await page.getByRole("button", { name: "Options" }).click();
  const triageSwitch = page.getByRole("switch", { name: "AI script triage" });
  await expect(triageSwitch).toBeChecked();
  await expect(page.getByRole("switch", { name: "AI logic analysis" })).toHaveCount(0);
  await triageSwitch.click();
  await expect(page.getByText("Deterministic checks only. No LLM cost.")).toBeVisible();
  await triageSwitch.click();

  await startScan(page, "sportsdesk.example/live");
  await expect(reportHealth(page)).toBeVisible({ timeout: 25_000 });
  // The report does not claim an analysis that never ran.
  await expect(page.getByText("AI script triage", { exact: true })).toBeVisible();
  await expect(page.getByText("AI analysis", { exact: true })).toHaveCount(0);
  await reportTab(page, "Scripts").click();
  await expect(
    page
      .getByRole("row", { name: /hb-wrapper\.min\.js/ })
      .getByRole("cell")
      .last(),
  ).toHaveText(/^Jev · \d+% ad$/);
  await expect(page.getByText("Phase 2", { exact: true })).toHaveCount(0);

  await reportTab(page, "Audit trail").click();
  const steps = page.getByRole("navigation", { name: "Pipeline steps" });
  await expect(steps.getByRole("button", { name: /Filter scripts/ })).toContainText(
    "Classified 14 of 14 undecided scripts",
  );
  await expect(steps.getByRole("button", { name: /Analyse logic/ })).toContainText(
    "No Anthropic API key is configured",
  );
});

for (const colorScheme of ["light", "dark"] as const) {
  test.describe(`Jev screens at the minimum window, ${colorScheme}`, () => {
    test.use({ viewport: { width: 1024, height: 680 }, colorScheme });

    test("settings, scripts and an opened classification entry fit and have no axe violations", async ({ page }) => {
      await scanWithJev(page);
      await expect(reportHealth(page)).toBeVisible({ timeout: 25_000 });

      await reportTab(page, "Scripts").click();
      await page.getByRole("row", { name: /hb-wrapper\.min\.js/ }).click();
      await expect(page.getByRole("region", { name: "Script details" })).toBeVisible();
      await toastsGone(page);
      await expectNoHorizontalOverflow(page);
      await expectNoAxeViolations(page);

      await reportTab(page, "Audit trail").click();
      await page.getByRole("button", { name: /Filter scripts/ }).click();
      await page.getByRole("button", { name: /Ad-related s7 \S+hb-wrapper\.min\.js/ }).click();
      await expect(page.getByText(/^Thresholds analyse 0\.50, drop 0\.15/)).toBeVisible();
      await expectNoHorizontalOverflow(page);
      await expectNoAxeViolations(page);

      await page.getByRole("link", { name: "Settings" }).click();
      await expect(page.getByRole("switch", { name: "Classify scripts with Jev" })).toBeChecked();
      await expectNoHorizontalOverflow(page);
      await expectNoAxeViolations(page);
    });

    test("the fallback caveat and the Jev-only composer have no axe violations", async ({ page }) => {
      await scanWithJev(page, { jev: "fail" });
      await expect(page.getByRole("list", { name: "Scan caveats" })).toBeVisible({ timeout: 25_000 });
      await toastsGone(page);
      await expectNoHorizontalOverflow(page);
      await expectNoAxeViolations(page);

      await page.getByRole("link", { name: "Settings" }).click();
      await page.getByRole("button", { name: "Remove key", exact: true }).click();
      await page.getByRole("alertdialog").getByRole("button", { name: "Remove key" }).click();
      await page.getByRole("link", { name: "Home", exact: true }).click();
      await page.getByRole("button", { name: "Options" }).click();
      await expect(page.getByRole("switch", { name: "AI script triage" })).toBeChecked();
      await toastsGone(page);
      await expectNoHorizontalOverflow(page);
      await expectNoAxeViolations(page);
    });
  });
}
