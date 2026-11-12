import { expect, test } from "@playwright/test";
import { openApp, reportHealth, reportTab, startScan } from "./app";

test("first run: onboarding → demo scan → live view → report with every tab", async ({ page }) => {
  await openApp(page, "/", { profile: "fresh", speed: 4 });

  // Setup checklist: browser found, API key optional.
  await expect(page.getByRole("heading", { name: "Find the revenue your ad stack is leaking" })).toBeVisible();
  await expect(page.getByRole("heading", { name: "Browser detected" })).toBeVisible();
  await expect(page.getByText("Google Chrome 141.0.7390.55")).toBeVisible();
  await expect(page.getByRole("heading", { name: "Add an Anthropic API key" })).toContainText("Optional");
  await expect(page.getByText(/Scans work without a key/)).toBeVisible();
  await expect(page).toHaveTitle("Welcome · Ad-Tech Scanner");

  // A rejected key explains itself and does not block continuing.
  await page.getByLabel("Anthropic API key").fill("wrong");
  await page.getByRole("button", { name: "Save key" }).click();
  await expect(page.getByRole("alert")).toContainText("Anthropic rejected this key");

  await page.getByRole("button", { name: "Scan the demo site" }).click();

  // Live view: pipeline, streaming activity, sidebar indicator.
  await expect(page).toHaveURL(/#\/scans\/scan-/);
  const pipeline = page.getByRole("list", { name: "Pipeline" });
  await expect(pipeline.getByRole("listitem")).toHaveCount(9);
  await expect(pipeline.locator('[aria-current="step"]')).toBeVisible();
  await expect(page.getByRole("region", { name: "Running", exact: true }).getByRole("link")).toHaveCount(1);
  await expect(
    page.getByRole("log", { name: "Activity" }).getByText(/Navigating to http:\/\/127\.0\.0\.1/),
  ).toBeVisible();
  await expect(page.getByRole("meter", { name: /LLM cost/ })).toBeVisible();
  await expect(page).toHaveTitle(/127\.0\.0\.1 \(scanning\)/);

  // The same screen becomes the report when the scan completes.
  await expect(reportHealth(page)).toBeVisible({ timeout: 20_000 });
  await expect(page.getByRole("region", { name: "Running", exact: true })).toHaveCount(0);
  await expect(page.getByText("Deterministic checks only", { exact: true })).toBeVisible();
  await expect(page.getByRole("region", { name: "Overview" })).toContainText("3 of 6 ad slots rendered a creative");
  await expect(page.getByRole("region", { name: "Overview" }).getByText("Requested slots filled")).toBeVisible();

  // Findings
  await expect(reportTab(page, "Findings")).toHaveAttribute("aria-selected", "true");
  await expect(page.getByRole("article", { name: "3 of 6 ad slots were defined but never requested" })).toBeVisible();

  // Auctions: waterfall and by-bidder table
  await reportTab(page, "Auctions").click();
  const auction = page.getByRole("region", { name: "Auction 1" });
  await expect(auction.getByText("timeout 1.50 s")).toBeVisible();
  await expect(
    auction.getByRole("list", { name: "Bids for div-gpt-top-leaderboard" }).getByRole("listitem"),
  ).toHaveCount(6);
  await auction.getByRole("listitem", { name: /^ix: Timeout/ }).hover();
  await expect(page.getByRole("tooltip")).toContainText("none within 1.50 s");
  await expect(page.getByRole("table").getByRole("rowheader", { name: "appnexus" })).toBeVisible();

  // Bidder checks: the site's own ads.txt, and only the bidder that needs attention spelled out
  const checks = page.getByRole("heading", { name: "Bidder checks" }).locator("xpath=ancestor::section[1]");
  await expect(checks.getByText(/^http:\/\/127\.0\.0\.1:47615\/ads\.txt: \d+ seller records$/)).toBeVisible();
  await expect(checks.getByText("Not looked up for this scan")).toBeVisible();
  const flagged = checks.getByRole("list", { name: "Bidders that need attention" });
  await expect(flagged.getByRole("listitem", { name: "sovrn" })).toContainText(
    "ads.txt: ads.txt has no entry for sovrn.com",
  );
  await expect(flagged.getByRole("img", { name: "Problem" })).toHaveCount(1);
  await expect(checks.getByText(/^Nothing found for /)).toContainText("appnexus");

  // Ad slots: lifecycle timeline with outcomes and flags
  await reportTab(page, "Ad slots").click();
  await expect(page.getByText("Never requested", { exact: true })).toHaveCount(3);
  await expect(page.getByText("Requested before auction end")).toBeVisible();
  await expect(page.getByText("No Prebid targeting")).toBeVisible();
  await expect(page.getByText(/First auction \d/)).toBeVisible();

  // Scripts: funnel, search, details
  await reportTab(page, "Scripts").click();
  await expect(page.getByRole("region", { name: "Script volume by stage" })).toContainText("Scripts loaded36");
  await page.getByRole("searchbox", { name: "Search scripts" }).fill("gpt.js");
  await expect(page.getByText("1 of 36 scripts")).toBeVisible();
  await page.getByRole("row", { name: /gpt\.js/ }).click();
  await expect(page.getByRole("region", { name: "Script details" })).toContainText(
    "https://securepubads.g.doubleclick.net/tag/js/gpt.js",
  );

  // Audit trail: steps with skipped AI phases, and their log entries
  await reportTab(page, "Audit trail").click();
  const stepNav = page.getByRole("navigation", { name: "Pipeline steps" });
  await expect(stepNav.getByRole("button", { name: /Analyse logic/ })).toContainText(
    "AI analysis is turned off for this scan",
  );
  await stepNav.getByRole("button", { name: /Deterministic checks/ }).click();
  await expect(
    page.getByRole("region", { name: "Log entries" }).getByText("Evaluating 14 deterministic rules"),
  ).toBeVisible();

  // Onboarding is done: Home is now the dashboard, with the scan in recent scans.
  await page.getByRole("link", { name: "Home", exact: true }).click();
  await expect(page.getByRole("heading", { name: "Scan a page for revenue leaks" })).toBeVisible();
  await expect(
    page.getByRole("region", { name: "Recent scans" }).getByRole("link", { name: /127\.0\.0\.1/ }),
  ).toBeVisible();
});

test("an AI scan streams cost, tool calls and findings, then links evidence to its script", async ({ page }) => {
  await openApp(page, "/", { speed: 8 });
  await page.getByRole("button", { name: "Options" }).click();
  await expect(page.getByRole("switch", { name: "AI logic analysis" })).toBeChecked();
  await startScan(page, "sportsdesk.example/live");

  const activity = page.getByRole("log", { name: "Activity" });
  await expect(activity.getByText("Find logic bugs in the wrapper that directly cause revenue loss")).toBeVisible({
    timeout: 15_000,
  });
  await expect(activity.getByText("read_script", { exact: true }).first()).toBeVisible();
  await expect(page.getByRole("region", { name: "Findings" }).getByRole("listitem").first()).toBeVisible();
  await expect(page.getByRole("meter", { name: /LLM cost/ })).not.toHaveAttribute("aria-valuenow", "0");

  await expect(page.getByText("AI analysis", { exact: true }).first()).toBeVisible({ timeout: 20_000 });
  const critical = page.getByRole("article", { name: /Desktop screens under 1440 px are treated as mobile/ });
  await expect(critical.getByText("Critical")).toBeVisible();
  await expect(critical.getByText("~34% revenue at risk")).toBeVisible();
  await expect(critical.getByText("var IS_MOBILE = window.screen.width < 1440;")).toBeVisible();

  // Deep link: evidence → the script's stored source at the cited lines, and Back returns to the finding.
  await critical.getByRole("button", { name: /hb-wrapper\.min\.js:208/ }).click();
  await expect(page).toHaveURL(/\/source\/s\d+\?line=208&to=215&finding=/);
  await expect(page.getByRole("heading", { level: 1, name: "hb-wrapper.min.js" })).toBeVisible();
  await page.goBack();
  await expect(critical).toBeVisible();
  // The script's row in the report says how it was triaged.
  await reportTab(page, "Scripts").click();
  await expect(page).toHaveURL(/tab=scripts/);
  await page.getByRole("row", { name: /hb-wrapper\.min\.js/ }).click();
  await expect(page.getByRole("row", { name: /hb-wrapper\.min\.js/ })).toHaveAttribute("aria-selected", "true");
  await expect(page.getByRole("region", { name: "Script details" })).toContainText("handed to Phase 2");

  // Re-scan on the other device goes straight to a new live view.
  await page.getByRole("button", { name: "Scan on mobile" }).click();
  await expect(page.getByRole("list", { name: "Pipeline" })).toBeVisible();
  await expect(page.getByText("Mobile", { exact: true }).first()).toBeVisible();
});

test("export hands the report to a save dialog and writes the file", async ({ page }) => {
  await openApp(page, "/scans/seed-sportsdesk-desktop");
  await page.getByRole("button", { name: "Export" }).click();
  const [download] = await Promise.all([
    page.waitForEvent("download"),
    page.getByRole("menuitem", { name: "Markdown report…" }).click(),
  ]);
  expect(download.suggestedFilename()).toMatch(/^adscan-www\.sportsdesk\.example-desktop-\d{4}-\d{2}-\d{2}\.md$/);
  await expect(page.getByText("Report exported")).toBeVisible();
});
