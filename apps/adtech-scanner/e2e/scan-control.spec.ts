import { expect, test } from "@playwright/test";
import { openApp, reportHealth, reportTab, startScan } from "./app";

test("cancel a running scan, after confirming", async ({ page }) => {
  await openApp(page, "/", { speed: 1 });
  await startScan(page, "morningledger.example");
  await expect(page.getByRole("list", { name: "Pipeline" })).toBeVisible();

  await page.getByRole("button", { name: "Cancel scan" }).click();
  const dialog = page.getByRole("alertdialog", { name: "Cancel this scan?" });
  await dialog.getByRole("button", { name: "Keep scanning" }).click();
  await expect(dialog).toBeHidden();
  await expect(page.getByRole("list", { name: "Pipeline" })).toBeVisible();

  await page.getByRole("button", { name: "Cancel scan" }).click();
  await dialog.getByRole("button", { name: "Cancel scan" }).click();

  await expect(page.getByRole("alert").filter({ hasText: "This scan was cancelled" })).toBeVisible();
  await expect(page.getByRole("region", { name: "Running", exact: true })).toHaveCount(0);
  await expect(reportTab(page, "Audit trail")).toHaveAttribute("aria-selected", "true");
  await expect(page.getByRole("region", { name: "Log entries" }).getByText("Scan cancelled by user")).toBeVisible();
  // The step that was interrupted is marked as cancelled, not as failed.
  const steps = page.getByRole("navigation", { name: "Pipeline steps" });
  await expect(steps.getByRole("img", { name: "Cancelled" })).toHaveCount(1);
  await expect(steps.getByRole("img", { name: "Failed" })).toHaveCount(0);
  await expect(reportTab(page, "Auctions")).toHaveCount(0);
});

test("a failed scan explains the failure and keeps its audit trail", async ({ page }) => {
  await openApp(page, "/");
  await startScan(page, "unreachable.example");

  const notice = page.getByRole("alert").filter({ hasText: "This scan failed" });
  await expect(notice).toContainText("net::ERR_NAME_NOT_RESOLVED for unreachable.example");
  await expect(page.getByText("Failed", { exact: true }).first()).toBeVisible();
  await expect(reportHealth(page, "No health score")).toBeVisible();
  await expect(
    page.getByRole("navigation", { name: "Pipeline steps" }).getByRole("img", { name: "Failed" }),
  ).toHaveCount(1);
  await expect(page.getByRole("button", { name: "Export" })).toHaveCount(0);

  // It is in history as failed, and can be retried from there.
  await page.getByRole("link", { name: "History" }).click();
  await expect(page.getByRole("row", { name: /unreachable\.example/ }).first()).toContainText("Failed");
});

test("the outcome of a scan the user navigated away from arrives as a toast", async ({ page }) => {
  await openApp(page, "/", { speed: 6 });
  await startScan(page, "morningledger.example/markets");
  await expect(page.getByRole("list", { name: "Pipeline" })).toBeVisible();
  await page.getByRole("link", { name: "History" }).click();

  // History shows it running, live, then completed without a reload.
  await expect(page.getByRole("row", { name: /morningledger\.example\/markets/ })).toContainText("Running");
  const toast = page.getByRole("listitem").filter({ hasText: "Scan of morningledger.example finished" });
  await expect(toast).toBeVisible({ timeout: 20_000 });
  await expect(page.getByRole("row", { name: /morningledger\.example\/markets/ })).not.toContainText("Running");
  await toast.getByRole("button", { name: "Open" }).click();
  await expect(page).toHaveURL(/#\/scans\/scan-/);
  await expect(reportHealth(page)).toBeVisible();
});

test("without a browser, the dashboard warns and starting a scan points to settings", async ({ page }) => {
  await openApp(page, "/", { profile: "no-browser" });
  await expect(page.getByRole("heading", { name: "No Chrome or Chromium found" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Scan the demo site" })).toBeDisabled();
  await page.getByRole("button", { name: /Continue/ }).click();

  await expect(page.getByRole("alert").filter({ hasText: "No browser found" })).toBeVisible();
  await startScan(page, "example.com");
  const toast = page.getByRole("listitem").filter({ hasText: "No browser found" });
  await expect(toast).toContainText("No Chrome or Chromium installation was found");
  await toast.getByRole("button", { name: "Open settings" }).click();
  await expect(page).toHaveURL(/#\/settings/);

  // Choosing an executable resolves it.
  await page.getByRole("button", { name: /Choose executable/ }).click();
  await expect(page.getByText("Chromium 141.0.7390.37")).toBeVisible();
  await page.getByRole("link", { name: "Home", exact: true }).click();
  await expect(page.getByRole("alert").filter({ hasText: "No browser found" })).toHaveCount(0);
});

test("a deleted or unknown scan has a not-found state", async ({ page }) => {
  await openApp(page, "/scans/does-not-exist");
  await expect(page.getByText("This scan no longer exists")).toBeVisible();
  await page.getByRole("link", { name: "Go to history" }).click();
  await expect(page).toHaveURL(/#\/history/);
});
