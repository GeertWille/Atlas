import { expect, test } from "@playwright/test";
import { expectNoAxeViolations, openApp, reportHealth, startScan } from "./app";

test("the scan profile is kept between scans once switched on, and cleared after confirming", async ({ page }) => {
  await openApp(page, "/settings", { profile: "fresh" });
  const browser = page.getByRole("region", { name: "Browser" });
  const keep = browser.getByRole("switch", { name: "Keep a scan profile between scans" });
  await expect(keep).not.toBeChecked();
  await expect(browser.getByText(/Stored only on this Mac, and separate from your own Chrome\./)).toBeVisible();
  await expect(browser.getByText("No scan profile is stored yet.")).toBeVisible();
  await keep.click();
  await expect(page.getByText("Settings saved")).toBeVisible();

  // The composer says what a scan will do with it.
  await page.getByRole("link", { name: "Home", exact: true }).click();
  await page.getByRole("button", { name: /Continue/ }).click();
  await page.getByRole("button", { name: "Options" }).click();
  await expect(
    page.getByText(/Kept between scans: cookies and consent persist, like a returning visitor's\./),
  ).toBeVisible();
  await startScan(page, "morningledger.example");
  await expect(reportHealth(page)).toBeVisible({ timeout: 30_000 });
  await expect(
    page.getByRole("region", { name: "Overview" }).getByText("Kept scan profile", { exact: true }),
  ).toBeVisible();

  await page.getByRole("link", { name: "Settings", exact: true }).click();
  await expect(browser.getByText(/20\.4 MB on this Mac, last used (just now|1 min ago)/)).toBeVisible();
  await browser.getByRole("button", { name: "Clear scan profile…" }).click();
  const confirm = page.getByRole("alertdialog", { name: "Clear the scan profile?" });
  await expect(confirm).toContainText("The next scan starts like a first-time visitor.");
  await expectNoAxeViolations(page);
  await confirm.getByRole("button", { name: "Cancel" }).click();
  await expect(browser.getByText(/20\.4 MB/)).toBeVisible();
  await browser.getByRole("button", { name: "Clear scan profile…" }).click();
  await confirm.getByRole("button", { name: "Clear profile" }).click();
  await expect(browser.getByText("No scan profile is stored yet.")).toBeVisible();
  await page.reload();
  await expect(browser.getByText("No scan profile is stored yet.")).toBeVisible();
  await expect(keep).toBeChecked();
});

test("the scan profile cannot be cleared while a scan is using it", async ({ page }) => {
  await openApp(page, "/settings", { profile: "fresh", speed: 20 });
  await page.getByRole("switch", { name: "Keep a scan profile between scans" }).click();
  await expect(page.getByText("Settings saved")).toBeVisible();
  await page.getByRole("link", { name: "Home", exact: true }).click();
  await page.getByRole("button", { name: /Continue/ }).click();
  await page.getByRole("button", { name: "Options" }).click();
  await page.getByRole("radio", { name: "I'll browse myself" }).click();
  await startScan(page, "morningledger.example");
  await expect(page.getByRole("region", { name: "Manual session" })).toBeVisible({ timeout: 15_000 });

  await page.getByRole("link", { name: "Settings", exact: true }).click();
  const browser = page.getByRole("region", { name: "Browser" });
  await expect(browser.getByText(/A running scan is using it\./)).toBeVisible();
  await expect(browser.getByRole("button", { name: "Clear scan profile…" })).toBeDisabled();

  // Once the session is finished and the scan done, it can be cleared.
  await page.getByRole("region", { name: "Running", exact: true }).getByRole("link").click();
  await page.getByRole("button", { name: "Finish capture" }).click();
  await expect(reportHealth(page)).toBeVisible({ timeout: 40_000 });
  await page.getByRole("link", { name: "Settings", exact: true }).click();
  await expect(browser.getByRole("button", { name: "Clear scan profile…" })).toBeEnabled();
});

test("add a proxy with credentials, use it for a scan, and see it in the report and history", async ({ page }) => {
  await openApp(page, "/settings", { profile: "fresh" });
  const proxies = page.getByRole("region", { name: "Proxies" });
  await expect(proxies.getByText(/Behind a proxy the app cannot fully verify where a request lands/)).toBeVisible();
  await proxies.getByRole("button", { name: "Add proxy…" }).click();
  const dialog = page.getByRole("dialog", { name: "Add a proxy" });
  await expect(dialog.getByLabel("Name", { exact: true })).toBeFocused();
  await expectNoAxeViolations(page);

  // Problems are explained on their fields before anything is saved.
  await dialog.getByLabel("Name", { exact: true }).fill("France");
  await dialog.getByLabel("Host").fill("https://fr.proxy.example:3128/");
  await dialog.getByLabel("Password (optional)").fill("s3cret");
  await dialog.getByRole("button", { name: "Add proxy" }).click();
  await expect(dialog.getByLabel("Host")).toHaveAccessibleDescription(/no http:\/\/, port, path or password/);
  await expect(dialog.getByLabel("User name (optional)")).toHaveAccessibleDescription(
    "Enter the user name this password belongs to.",
  );
  await expectNoAxeViolations(page);

  await dialog.getByLabel("Host").fill("fr.proxy.example");
  await dialog.getByLabel("Port").fill("3128");
  await dialog.getByLabel("User name (optional)").fill("scanner");
  await page.keyboard.press("Enter");
  await expect(dialog).toHaveCount(0);
  const saved = proxies.getByRole("list", { name: "Saved proxies" }).getByRole("listitem");
  await expect(saved).toHaveCount(1);
  await expect(saved).toContainText("France");
  await expect(saved).toContainText("http://fr.proxy.example:3128");
  await expect(saved).toContainText("Password stored");
  // The password is nowhere on the page, nor in what the fake backend stores.
  await expect(page.locator("body")).not.toContainText("s3cret");
  expect(await page.evaluate(() => JSON.stringify(localStorage))).not.toContain("s3cret");

  // SOCKS5 cannot sign in: refused with the reason.
  await proxies.getByRole("button", { name: "Add proxy…" }).click();
  await dialog.getByLabel("Name", { exact: true }).fill("Tunnel");
  await dialog.getByLabel("Host").fill("socks.proxy.example");
  await dialog.getByLabel("User name (optional)").fill("me");
  await dialog.getByRole("combobox", { name: "Type" }).click();
  await page.getByRole("option", { name: "SOCKS5" }).click();
  await expect(dialog.getByLabel("Port")).toHaveValue("1080");
  await dialog.getByRole("button", { name: "Add proxy" }).click();
  await expect(dialog.getByLabel("User name (optional)")).toHaveAccessibleDescription(
    "Chrome cannot sign in to a SOCKS5 proxy. Remove the user name, or use an HTTP or HTTPS proxy.",
  );
  await dialog.getByRole("button", { name: "Cancel" }).click();
  await expect(saved).toHaveCount(1);

  // Make it the default, and scan through it.
  await proxies.getByRole("combobox", { name: "Default proxy" }).click();
  await page.getByRole("option", { name: "France" }).click();
  await expect(page.getByText("Settings saved")).toBeVisible();
  await page.getByRole("link", { name: "Home", exact: true }).click();
  await page.getByRole("button", { name: /Continue/ }).click();
  await page.getByRole("button", { name: "Options" }).click();
  await page.getByRole("button", { name: "Proxy, profile and budget" }).click();
  await expect(page.getByRole("combobox", { name: "Proxy" })).toHaveText("Settings default (France)");
  await startScan(page, "morningledger.example");
  await expect(reportHealth(page)).toBeVisible({ timeout: 30_000 });
  await expect(page.getByRole("region", { name: "Overview" }).getByText("France (fr.proxy.example)")).toBeVisible();
  await page.getByRole("link", { name: "History" }).click();
  await expect(page.getByRole("region", { name: "Scans" }).locator("tbody tr").first()).toContainText("Proxy France");

  // Deleting the proxy needs confirming, resets the default, and the old report keeps its name.
  await page.getByRole("link", { name: "Settings", exact: true }).click();
  await saved.getByRole("button", { name: "Delete France" }).click();
  await page
    .getByRole("alertdialog", { name: "Delete the proxy “France”?" })
    .getByRole("button", { name: "Delete" })
    .click();
  await expect(saved).toHaveCount(0);
  await expect(proxies.getByRole("combobox", { name: "Default proxy" })).toHaveText("System proxy");
  await page.getByRole("link", { name: "History" }).click();
  await expect(page.getByRole("region", { name: "Scans" }).locator("tbody tr").first()).toContainText("Proxy France");
});

test("a proxy in use by a configuration cannot be deleted; the stored password can be removed", async ({ page }) => {
  await openApp(page, "/settings");
  const proxies = page.getByRole("region", { name: "Proxies" });
  const belgium = proxies.getByRole("listitem").filter({ hasText: "Belgium" });
  await expect(belgium).toContainText("Password stored");

  await belgium.getByRole("button", { name: "Delete Belgium" }).click();
  await page.getByRole("alertdialog").getByRole("button", { name: "Delete" }).click();
  const toast = page.getByRole("listitem").filter({ hasText: "Could not delete the proxy" });
  await expect(toast).toContainText("This proxy is used by “Sportsdesk from Belgium”");
  await expect(belgium).toBeVisible();

  await belgium.getByRole("button", { name: "Edit Belgium" }).click();
  const dialog = page.getByRole("dialog", { name: "Edit proxy" });
  await expect(dialog.getByLabel("Password (optional)")).toHaveValue("");
  await expect(dialog.getByLabel("User name (optional)")).toHaveValue("scanner");
  await dialog.getByRole("button", { name: "Remove password" }).click();
  await expect(dialog.getByRole("button", { name: "Remove password" })).toHaveCount(0);
  await dialog.getByRole("button", { name: "Cancel" }).click();
  await expect(belgium).not.toContainText("Password stored");
});

test("scan defaults for the visitor reach the composer", async ({ page }) => {
  await openApp(page, "/settings");
  const defaults = page.getByRole("region", { name: "Scan defaults" });
  await defaults.getByRole("combobox", { name: "Default visitor" }).click();
  await page.getByRole("option", { name: "Browse like a visitor" }).click();
  await defaults.getByRole("button", { name: "More pages" }).click();
  await expect(defaults.getByText("4 pages")).toBeVisible();
  await expect(page.getByText("Settings saved")).toBeVisible();

  await page.getByRole("link", { name: "Home", exact: true }).click();
  await expect(page.getByRole("form", { name: "New scan" }).getByText("Browse like a visitor, 4 pages")).toBeVisible();
  await page.reload();
  await page.getByRole("button", { name: "Options" }).click();
  await expect(page.getByRole("radio", { name: "Browse like a visitor" })).toBeChecked();
  await expect(page.getByRole("switch", { name: "Show the browser" })).toBeChecked();
});
