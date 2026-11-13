import { expect, test } from "@playwright/test";
import { openApp, reportHealth } from "./app";

const rows = (page: import("@playwright/test").Page) => page.getByRole("region", { name: "Scans" }).locator("tbody tr");

test("search, filter, sort and page through history", async ({ page }) => {
  await openApp(page, "/history");
  await expect(page).toHaveTitle("History · Ad-Tech Scanner");
  await expect(rows(page)).toHaveCount(20);
  await expect(page.getByText("Showing 20 of 27 scans")).toBeVisible();

  await page.getByRole("button", { name: "Load more" }).click();
  await expect(rows(page)).toHaveCount(27);
  await expect(page.getByRole("button", { name: "Load more" })).toHaveCount(0);

  await page.getByRole("searchbox", { name: "Search scans" }).fill("techpulse");
  await expect(rows(page)).toHaveCount(3);
  for (const row of await rows(page).all()) await expect(row).toContainText("techpulse.example");

  await page.getByRole("radio", { name: "Mobile" }).click();
  await expect(rows(page)).toHaveCount(2);
  await page.getByRole("radio", { name: "All", exact: true }).click();
  await page.getByRole("button", { name: "Clear search" }).click();

  await page.getByRole("combobox", { name: "Status" }).click();
  await page.getByRole("option", { name: "Failed" }).click();
  await expect(rows(page)).toHaveCount(1);
  await expect(rows(page).first()).toContainText("unreachable.example");

  await page.getByRole("combobox", { name: "Status" }).click();
  await page.getByRole("option", { name: "Cancelled" }).click();
  await page.getByRole("searchbox", { name: "Search scans" }).fill("sportsdesk");
  await expect(page.getByText("No scans match")).toBeVisible();
  await page.getByRole("button", { name: "Clear filters" }).click();
  // Both pages loaded earlier are still there.
  await expect(rows(page)).toHaveCount(27);

  await page.getByRole("combobox", { name: "Sort" }).click();
  await page.getByRole("option", { name: "Lowest health first" }).click();
  await expect(
    rows(page)
      .first()
      .getByRole("img", { name: /Health score 4 of 100/ }),
  ).toBeVisible();
});

test("open, re-scan and delete from a row; delete everything from the danger zone", async ({ page }) => {
  await openApp(page, "/history");

  // Delete one, with confirmation.
  const target = page.getByRole("row", { name: /unreachable\.example/ });
  await target.getByRole("button", { name: "Actions for unreachable.example" }).click();
  await page.getByRole("menuitem", { name: "Delete…" }).click();
  const confirm = page.getByRole("alertdialog");
  await expect(confirm).toContainText("Delete the scan of unreachable.example?");
  await confirm.getByRole("button", { name: "Cancel" }).click();
  await expect(target).toBeVisible();

  await target.getByRole("button", { name: "Actions for unreachable.example" }).click();
  await page.getByRole("menuitem", { name: "Delete…" }).click();
  await confirm.getByRole("button", { name: "Delete" }).click();
  await expect(target).toHaveCount(0);
  await expect(page.getByText("Showing 20 of 26 scans")).toBeVisible();
  await expect(page.getByText("Deleted the scan of unreachable.example")).toBeVisible();

  // Open from a row.
  await page.getByRole("link", { name: /www\.techpulse\.example\/reviews\/laptops/ }).click();
  await expect(page).toHaveURL(/#\/scans\/seed-techpulse-desktop/);
  await page.goBack();

  // Re-scan from the row menu.
  await page.getByRole("button", { name: "Actions for www.morningledger.example" }).first().click();
  await page.getByRole("menuitem", { name: "Re-scan" }).click();
  await expect(page).toHaveURL(/#\/scans\/scan-/);
  await expect(reportHealth(page)).toBeVisible({ timeout: 20_000 });

  // Delete all.
  await page.getByRole("link", { name: "History" }).click();
  await page.getByRole("button", { name: "Delete all…" }).click();
  await expect(confirm).toContainText("Delete all 27 scans?");
  await confirm.getByRole("button", { name: "Delete all" }).click();
  await expect(page.getByText("No scans yet")).toBeVisible();
  await expect(page.getByRole("region", { name: "Danger zone" })).toHaveCount(0);
  await expect(page.getByRole("region", { name: "Recent" })).toHaveCount(0);

  // Still empty after a reload.
  await page.reload();
  await expect(page.getByText("No scans yet")).toBeVisible();
});
