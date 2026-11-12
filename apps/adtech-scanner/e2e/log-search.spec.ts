import { expect, test } from "@playwright/test";
import { openApp } from "./app";

test("search all audit trails and click through to the exact entry", async ({ page }) => {
  await openApp(page, "/logs");
  await expect(page.getByText("Search how every conclusion was reached")).toBeVisible();
  const input = page.getByRole("searchbox", { name: "Search all audit trails" });
  await expect(input).toBeFocused();

  await input.fill("MOBILE_LAZY_OFFSET");
  await expect(page.getByText(/\d+ matches in \d+ scans/)).toBeVisible();
  await expect(page).toHaveURL(/#\/logs\?q=MOBILE_LAZY_OFFSET/);
  await expect(page).toHaveTitle(/“MOBILE_LAZY_OFFSET” in audit trails/);

  const group = page.getByRole("region", { name: /Matches in www\.sportsdesk\.example/ }).first();
  await expect(group.locator("mark").first()).toHaveText("MOBILE_LAZY_OFFSET");
  // The snippet is rendered as text: escaped markup from the log never becomes elements.
  expect(await group.locator("li script, li img, li iframe").count()).toBe(0);

  const hit = group.getByRole("link").filter({ hasText: "Tool result" }).first();
  await hit.click();

  await expect(page).toHaveURL(/#\/scans\/seed-sportsdesk-[a-z]+\?tab=audit&log=/);
  await expect(page.getByRole("tab", { name: /^Audit trail/ })).toHaveAttribute("aria-selected", "true");
  const logId = new URL(page.url().replace("/#", "")).searchParams.get("log");
  const entry = page.locator(`[id="log-${logId}"]`);
  await expect(entry).toBeInViewport();
  await expect(entry.getByRole("button", { expanded: true })).toBeVisible();
  await expect(entry).toContainText("MOBILE_LAZY_OFFSET");

  // Back returns to the same results.
  await page.goBack();
  await expect(input).toHaveValue("MOBILE_LAZY_OFFSET");
  await expect(page.getByText(/\d+ matches in \d+ scans/)).toBeVisible();
});

test("short queries, no results and example chips", async ({ page }) => {
  await openApp(page, "/logs");
  const input = page.getByRole("searchbox", { name: "Search all audit trails" });

  await input.fill("x");
  await expect(page.getByText("Search how every conclusion was reached")).toBeVisible();

  await input.fill("zzzz-nothing-matches");
  await expect(page.getByText("No matches for “zzzz-nothing-matches”")).toBeVisible();

  await page.getByRole("button", { name: "Clear search" }).click();
  await page.getByRole("button", { name: "report_finding" }).click();
  await expect(input).toHaveValue("report_finding");
  await expect(page.getByRole("region", { name: /Matches in/ }).first()).toBeVisible();
});

test("a finding in the audit trail links to its finding card", async ({ page }) => {
  await openApp(page, "/scans/seed-sportsdesk-desktop?tab=audit");
  const entries = page.getByRole("region", { name: "Log entries" });
  await entries.getByRole("button", { name: /Finding, Warning.*800 ms failsafe fires the ad server/ }).click();
  await entries.getByRole("button", { name: "Open finding" }).click();

  await expect(page).toHaveURL(/tab=findings&finding=seed-sportsdesk-desktop-f/);
  const card = page.getByRole("article", { name: /800 ms failsafe fires the ad server/ });
  await expect(card).toBeInViewport();
  await expect(card.getByText("Recommendation")).toBeVisible();
});
