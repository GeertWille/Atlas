import { expect, test } from "@playwright/test";
import { expectNoAxeViolations, openApp, reportHealth, reportTab, startScan } from "./app";

const MULTI_PAGE = "/scans/seed-sportsdesk-browse";
const LEGACY = "/scans/seed-morningledger-mobile";

test("the composer offers four visitor behaviours and only the options each one has", async ({ page }) => {
  await openApp(page, "/");
  await page.getByRole("button", { name: "Options" }).click();
  const visitor = page.getByRole("radiogroup", { name: "Visitor" });
  await expect(visitor.getByRole("radio")).toHaveText([
    "Observe only",
    "Scroll",
    "Browse like a visitor",
    "I'll browse myself",
  ]);
  await expect(visitor.getByRole("radio", { name: "Scroll" })).toBeChecked();
  await expect(page.getByRole("switch", { name: "Auto-scroll" })).toHaveCount(0);
  await expect(page.getByText(/This visit is automated and says so to the page/)).toBeVisible();
  await expect(page.getByRole("button", { name: "More pages" })).toHaveCount(0);

  // The whole group is one tab stop; arrows move the choice.
  await visitor.getByRole("radio", { name: "Scroll" }).focus();
  await page.keyboard.press("ArrowRight");
  await expect(visitor.getByRole("radio", { name: "Browse like a visitor" })).toBeFocused();
  await page.keyboard.press("Space");
  await expect(visitor.getByRole("radio", { name: "Browse like a visitor" })).toBeChecked();
  await expect(page.getByText("3 pages", { exact: true })).toBeVisible();
  await expect(page.getByRole("switch", { name: "Show the browser" })).toBeChecked();
  await expect(page.getByText(/Estimated .* for 3 pages/)).toBeVisible();
  await page.getByRole("button", { name: "Fewer pages" }).click();
  await page.getByRole("button", { name: "Fewer pages" }).click();
  await expect(page.getByText("The entry page only; no link is followed.")).toBeVisible();
  await expect(page.getByRole("button", { name: "Fewer pages" })).toBeDisabled();

  await visitor.getByRole("radio", { name: "I'll browse myself" }).click();
  await expect(
    page.getByText(/Chrome opens; browse the site, then press Finish capture \(up to 15 minutes\)/),
  ).toBeVisible();
  await expect(page.getByRole("slider")).toHaveCount(0);
  await expect(page.getByRole("switch", { name: "Accept consent banner" })).toHaveCount(0);
  await expect(page.getByRole("switch", { name: "Show the browser" })).toHaveCount(0);

  // Proxy, profile and budget are one click further, not in the way.
  await expect(page.getByRole("combobox", { name: "Proxy" })).toHaveCount(0);
  await page.getByRole("button", { name: "Proxy, profile and budget" }).click();
  await page.getByRole("combobox", { name: "Proxy" }).click();
  await expect(page.getByRole("option")).toHaveText([
    "Settings default (System proxy)",
    "No proxy",
    "System proxy",
    "Belgium",
    "Netherlands",
  ]);
  await page.keyboard.press("Escape");
});

test("a browse visit shows where it is while it runs, and ends in a report with a page per page view", async ({
  page,
}) => {
  await openApp(page, "/", { speed: 6 });
  await page.getByRole("button", { name: "Options" }).click();
  await page.getByRole("radio", { name: "Browse like a visitor" }).click();
  await startScan(page, "sportsdesk.example/football");

  const visit = page.getByRole("region", { name: "Visit" });
  await expect(visit.getByText(/Now on page 1/)).toBeVisible({ timeout: 15_000 });
  await expect(visit.getByRole("list", { name: "Latest actions" }).getByText(/Scrolled to \d+%/)).toBeVisible({
    timeout: 15_000,
  });
  await expect(visit.getByRole("list", { name: "Pages visited" }).getByRole("listitem")).toHaveCount(2, {
    timeout: 20_000,
  });
  await expect(visit.getByText("Now on page 2: /2026/11/council-approves-harbour-plan")).toBeVisible();
  await expect(visit.getByRole("img", { name: "Followed link" })).toBeVisible();
  await expect(visit.getByText(/^Skipped 15 links: 9 off-site, 3 inside ads/)).toBeVisible();
  await expect(page.getByRole("log", { name: "Activity" })).toBeVisible();
  // A scripted visit has nothing for the user to finish.
  await expect(page.getByRole("region", { name: "Manual session" })).toHaveCount(0);

  await expect(reportHealth(page)).toBeVisible({ timeout: 40_000 });
  await expect(page.getByRole("region", { name: "Visit" })).toHaveCount(0);
  await expect(page.getByRole("radiogroup", { name: "Page view" }).getByRole("radio")).toHaveCount(4);
  const overview = page.getByRole("region", { name: "Overview" });
  for (const fact of ["Browse like a visitor", "3 pages", "Browser shown", "Fresh profile", "System proxy"]) {
    await expect(overview.getByText(fact, { exact: true })).toBeVisible();
  }
});

test("a manual session records until Finish capture, and the report says a person drove it", async ({ page }) => {
  await openApp(page, "/", { speed: 20 });
  await page.getByRole("button", { name: "Options" }).click();
  await page.getByRole("radio", { name: "I'll browse myself" }).click();
  await startScan(page, "morningledger.example");

  const session = page.getByRole("region", { name: "Manual session" });
  await expect(session.getByRole("heading", { name: "You are browsing morningledger.example" })).toBeVisible({
    timeout: 15_000,
  });
  await expect(session.getByText(/^00:0\d$/)).toBeVisible();
  await expect(session.getByText(/^14:5\d$/)).toBeVisible();
  // The capture waits for the user however long the scripted session takes.
  await expect(session.getByText("3 pages")).toBeVisible({ timeout: 15_000 });
  await expect(page.getByText("Step 2 of 9: Capture page")).toBeVisible();
  await expect(session.getByRole("button", { name: "Cancel scan" })).toBeVisible();
  await expectNoAxeViolations(page);

  await session.getByRole("button", { name: "Finish capture" }).click();
  await expect(session).toHaveCount(0, { timeout: 15_000 });
  await expect(page.getByRole("button", { name: "Cancel scan" })).toBeVisible();

  await expect(reportHealth(page)).toBeVisible({ timeout: 40_000 });
  const overview = page.getByRole("region", { name: "Overview" });
  await expect(overview.getByText("I'll browse myself", { exact: true })).toBeVisible();
  await expect(overview.getByText("Browser shown", { exact: true })).toBeVisible();
  await expect(page.getByRole("radiogroup", { name: "Page view" }).getByRole("radio")).toHaveCount(4);
  // A person's visit is not reported as automated.
  await expect(page.getByRole("note", { name: "Scan environment" })).toHaveCount(0);
  await reportTab(page, "Audit trail").click();
  await expect(
    page.getByRole("region", { name: "Log entries" }).getByText("Capture finished by you after 3 pages"),
  ).toBeVisible();
});

test("a manual session can be cancelled instead of finished", async ({ page }) => {
  await openApp(page, "/", { speed: 20 });
  await page.getByRole("button", { name: "Options" }).click();
  await page.getByRole("radio", { name: "I'll browse myself" }).click();
  await startScan(page, "morningledger.example");

  const session = page.getByRole("region", { name: "Manual session" });
  await session.getByRole("button", { name: "Cancel scan" }).click();
  await page.getByRole("alertdialog").getByRole("button", { name: "Cancel scan" }).click();
  await expect(page.getByRole("alert").filter({ hasText: "This scan was cancelled" })).toBeVisible();
  await expect(session).toHaveCount(0);
});

test("the page switcher narrows every tab to one page view and keeps the choice in the address", async ({ page }) => {
  await openApp(page, MULTI_PAGE);
  const switcher = page.getByRole("radiogroup", { name: "Page view" });
  await expect(switcher.getByRole("radio")).toHaveText([
    "All pages",
    "1/football/transfer-news",
    "2/2026/11/council-approves-harbour-plan",
    "3/2026/11/weekend-weather-outlook",
  ]);
  await expect(switcher.getByRole("radio", { name: "All pages" })).toBeChecked();
  await expect(reportTab(page, "Findings")).toContainText("10");
  await expect(reportTab(page, "Ad slots")).toContainText("12");

  // All pages: auctions and slots are grouped per page view.
  await reportTab(page, "Auctions").click();
  await expect(
    page.getByRole("region", { name: "Auctions on page 1" }).getByRole("region", { name: "Auction 1" }),
  ).toBeVisible();
  await expect(
    page.getByRole("region", { name: "Auctions on page 2" }).getByText("No Prebid auction started on this page view."),
  ).toBeVisible();
  await expect(
    page.getByRole("region", { name: "Auctions on page 3" }).getByRole("region", { name: "Auction 2" }),
  ).toBeVisible();
  await reportTab(page, "Ad slots").click();
  await expect(page.getByRole("region", { name: "Ad slots on page 2" }).getByText("Followed link")).toBeVisible();
  await expect(
    page.getByRole("region", { name: "Ad slots on page 3" }).getByText("GPT defined no ad slot on this page view."),
  ).toBeVisible();
  await expect(page.getByText("Refreshed with a used bid")).toBeVisible();

  // Page 2: its own slots, no auctions, the findings seen there.
  await switcher.getByRole("radio", { name: /council-approves-harbour-plan/ }).click();
  await expect(page).toHaveURL(/tab=slots&page=1$/);
  await expect(page.getByRole("region", { name: "Page views" })).toContainText(
    /Council approves harbour plan · Followed link “Council approves harbour plan” · opened .* · 0 auctions · 6 slots/,
  );
  await expect(reportTab(page, "Ad slots")).toContainText("6");
  await expect(reportTab(page, "Auctions")).toContainText("0");
  await expect(page.getByRole("region", { name: /Ad slots on page/ })).toHaveCount(0);
  await expect(page.getByText("No Prebid targeting").first()).toBeVisible();
  await reportTab(page, "Auctions").click();
  await expect(page.getByText("No Prebid auctions captured")).toBeVisible();
  await expect(page).toHaveURL(/tab=auctions&page=1$/);

  await reportTab(page, "Findings").click();
  await expect(
    page.getByRole("article", { name: "Later page views request their ads without header bids" }),
  ).toBeVisible();
  await expect(page.getByRole("article", { name: /Desktop screens under 1440 px/ })).toHaveCount(0);

  // The selection survives a reload, and "All pages" clears it.
  await page.reload();
  await expect(switcher.getByRole("radio", { name: /council-approves-harbour-plan/ })).toBeChecked();
  await switcher.getByRole("radio", { name: "All pages" }).click();
  await expect(page).not.toHaveURL(/page=/);
  await expect(page.getByRole("article", { name: /Desktop screens under 1440 px/ })).toBeVisible();
});

test("findings of a multi-page visit say where they were seen, on the card and in the copied markdown", async ({
  page,
}) => {
  await openApp(page, MULTI_PAGE);
  const later = page.getByRole("article", { name: "Later page views request their ads without header bids" });
  await expect(later.getByText("Page 2", { exact: true })).toBeVisible();
  await later.getByRole("button", { name: /Later page views/ }).click();
  await expect(later.getByText("Seen on")).toBeVisible();
  await expect(later.getByText("2 /2026/11/council-approves-harbour-plan")).toBeVisible();

  await later.getByRole("button", { name: "Copy finding as Markdown" }).click();
  const copied = await page.evaluate(() => navigator.clipboard.readText());
  expect(copied).toContain("Seen on: page 2 (/2026/11/council-approves-harbour-plan)");

  await expect(
    page.getByRole("article", { name: /Desktop screens under 1440 px/ }).getByText("Page 1", { exact: true }),
  ).toBeVisible();

  await reportTab(page, "Scripts").click();
  await expect(page.getByRole("columnheader", { name: "Pages" })).toBeVisible();
  await expect(page.getByRole("row", { name: /gpt\.js/ })).toContainText("1, 2");
});

test("the environment notice states what the page could see and how to check the result", async ({ page }) => {
  await openApp(page, "/scans/seed-techpulse-desktop");
  const notice = page.getByRole("note", { name: "Scan environment" });
  await expect(notice).toContainText("Bidders may have treated this scan as automated traffic.");
  await expect(notice).toContainText(
    "navigator.webdriver was true, Chrome ran headless, DoubleVerify (invalid-traffic measurement) was on the page.",
  );
  await expect(notice).not.toContainText(/undetect|indistinguishable/i);
  await expect(page.getByRole("article", { name: "This scan was identifiable as automated traffic" })).toBeVisible();
  // It belongs to the bids and fill, not to the scripts or the audit trail.
  await reportTab(page, "Scripts").click();
  await expect(notice).toHaveCount(0);
  await reportTab(page, "Auctions").click();
  await expect(notice).toBeVisible();

  // What to try is one click away: the same scan with the browser shown.
  await notice.getByRole("button", { name: "with the browser shown" }).click();
  await expect(page).toHaveURL(/#\/scans\/scan-/);
  await expect(reportHealth(page)).toBeVisible({ timeout: 30_000 });
  const overview = page.getByRole("region", { name: "Overview" });
  await expect(overview.getByText("Browser shown", { exact: true })).toBeVisible();
  await expect(page.getByRole("note", { name: "Scan environment" })).toContainText(
    "navigator.webdriver was true, DoubleVerify",
  );
  await expect(page.getByRole("note", { name: "Scan environment" }).getByRole("button")).toHaveText([
    "as “I'll browse myself”",
  ]);
});

test("a scan stored before page views has no switcher, no page chips, no visit line and no notice", async ({
  page,
}) => {
  await openApp(page, LEGACY);
  await expect(page.getByRole("article").first()).toBeVisible();
  await expect(page.getByRole("radiogroup", { name: "Page view" })).toHaveCount(0);
  await expect(page.getByRole("note", { name: "Scan environment" })).toHaveCount(0);
  await expect(page.getByRole("region", { name: "Overview" }).getByText("Visit", { exact: true })).toHaveCount(0);
  await expect(page.getByText(/^Pages? \d/)).toHaveCount(0);
  await expect(page.getByRole("button", { name: "More scan actions" })).toHaveCount(0);
  await reportTab(page, "Ad slots").click();
  await expect(page.getByRole("region", { name: /Ad slots on page/ })).toHaveCount(0);
  await reportTab(page, "Scripts").click();
  await expect(page.getByRole("columnheader", { name: "Pages" })).toHaveCount(0);
  // It can still be scanned again, with today's defaults filled in by the backend.
  await page.getByRole("button", { name: "Re-scan" }).click();
  await expect(page).toHaveURL(/#\/scans\/scan-/);
});

test("repeat this visit starts the same scripted visit again, seed and all", async ({ page }) => {
  await openApp(page, "/scans/seed-sportsdesk-desktop");
  await page.getByRole("button", { name: "More scan actions" }).click();
  const repeat = page.getByRole("menuitem", { name: /Repeat this visit/ });
  const seed = (await repeat.textContent())?.match(/seed (\d+)/)?.[1];
  expect(seed).toBeTruthy();
  await repeat.click();

  await expect(page).toHaveURL(/#\/scans\/scan-/);
  await expect(reportHealth(page)).toBeVisible({ timeout: 30_000 });
  await page.getByRole("button", { name: "More scan actions" }).click();
  await expect(page.getByRole("menuitem", { name: /Repeat this visit/ })).toContainText(`seed ${seed}`);
  await page.keyboard.press("Escape");
  // A plain re-scan moves and picks links afresh.
  const repeated = page.url();
  await page.getByRole("button", { name: "Re-scan" }).click();
  await expect(page).not.toHaveURL(repeated);
  await expect(reportHealth(page)).toBeVisible({ timeout: 30_000 });
  await page.getByRole("button", { name: "More scan actions" }).click();
  await expect(page.getByRole("menuitem", { name: /Repeat this visit/ })).not.toContainText(`seed ${seed})`);
});

test("the audit trail explains each link the visitor followed", async ({ page }) => {
  await openApp(page, `${MULTI_PAGE}?tab=audit`);
  const entries = page.getByRole("region", { name: "Log entries" });
  await page.getByRole("button", { name: "Filter by kind" }).click();
  await page.getByRole("menuitemcheckbox", { name: /^Visitor/ }).click();
  await page.keyboard.press("Escape");
  await expect(entries.getByText("This visit is identifiable as automated: navigator.webdriver is true")).toBeVisible();
  await expect(entries.getByText("Closed a pop-up window (partner-offers.example): not recorded")).toBeVisible();

  await entries.getByRole("button", { name: /Following link: \/2026\/11\/council-approves-harbour-plan/ }).click();
  const considered = entries.getByRole("list", { name: "Links considered" }).getByRole("listitem");
  await expect(considered).toHaveCount(5);
  await expect(considered.first()).toContainText("Followed");
  await expect(considered.first()).toContainText("Council approves harbour plan");
  await expect(considered.first()).toContainText("score 6");
  await expect(
    entries.getByText("Chosen for: content link, article-like path, text of 25-120 characters"),
  ).toBeVisible();
  await expect(
    entries.getByText(/Not considered: 9 off-site, 3 inside ads, 2 sign-in, checkout and the like, 1 file downloads/),
  ).toBeVisible();
  // Audited without the rule the kind filter's counter trips on its own button: that predates this entry kind.
  await expectNoAxeViolations(page, ["label-content-name-mismatch"]);
});
