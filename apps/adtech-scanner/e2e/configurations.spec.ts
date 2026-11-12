import { expect, test } from "@playwright/test";
import { expectNoAxeViolations, openApp, reportHealth, startScan } from "./app";

const table = (page: import("@playwright/test").Page) => page.getByRole("region", { name: "Configurations" });
/** The row of the configuration with exactly this name ("X" is not "X copy"). */
const row = (page: import("@playwright/test").Page, name: string) =>
  table(page)
    .getByRole("row")
    .filter({ has: page.getByText(name, { exact: true }) });

test("configurations are one click from anywhere: sidebar, home and the command palette", async ({ page }) => {
  await openApp(page, "/");
  // Home lists them, most recently run first.
  await expect(table(page).getByRole("row").nth(1)).toContainText("Morning Ledger, no proxy");
  await expect(row(page, "Sportsdesk from Belgium")).toContainText("Browse like a visitor, 3 pages");

  await page.getByRole("link", { name: "Configurations", exact: true }).click();
  await expect(page).toHaveTitle("Configurations · Ad-Tech Scanner");
  await expect(table(page).locator("tbody tr")).toHaveCount(4);
  await expect(row(page, "Sportsdesk from the Netherlands, mobile")).toContainText("Never run");
  await expect(row(page, "Sportsdesk from the Netherlands, mobile")).toContainText("Mobile");
  await expect(
    row(page, "Sportsdesk from Belgium").getByRole("link", { name: /Health score \d+ of 100/ }),
  ).toHaveAttribute("href", /#\/scans\/seed-sportsdesk-browse$/);

  // The palette runs one by name.
  await page.keyboard.press("Control+k");
  const palette = page.getByRole("dialog", { name: "Command palette" });
  await palette.getByRole("combobox").fill("run ledger");
  await expect(palette.getByRole("option").first()).toHaveText(/Run configuration: Morning Ledger, no proxy/);
  await expect(palette.getByRole("option", { name: /Run configuration/ })).toHaveCount(1);
  await page.keyboard.press("Enter");
  await expect(page).toHaveURL(/#\/scans\/scan-/);
  await expect(page.getByRole("list", { name: "Pipeline" })).toBeVisible();
  await expect(reportHealth(page)).toBeVisible({ timeout: 30_000 });
  // The scan carries the configuration's name, and its proxy choice.
  await expect(page.getByRole("main").locator("header").getByText("Morning Ledger, no proxy")).toBeVisible();
  await expect(page.getByRole("region", { name: "Overview" }).getByText("No proxy", { exact: true })).toBeVisible();
});

test("run a configuration with one click; its last run and history follow", async ({ page }) => {
  await openApp(page, "/configurations");
  await row(page, "Sportsdesk from the Netherlands, mobile").getByRole("button", { name: /^Run / }).click();
  await expect(page).toHaveURL(/#\/scans\/scan-/);
  await expect(reportHealth(page)).toBeVisible({ timeout: 30_000 });
  const scanUrl = page.url();
  const header = page.getByRole("main").locator("header");
  await expect(header.getByText("Sportsdesk from the Netherlands, mobile")).toBeVisible();
  await expect(header.getByText("Mobile", { exact: true })).toBeVisible();
  await expect(
    page.getByRole("region", { name: "Overview" }).getByText("Netherlands (nl.proxy.example)"),
  ).toBeVisible();

  // Re-scan on a scan that came from a configuration runs that configuration again.
  await page.getByRole("button", { name: "Re-scan" }).hover();
  await expect(page.getByRole("tooltip")).toHaveText(
    "Run the configuration “Sportsdesk from the Netherlands, mobile” again",
  );
  await page.getByRole("button", { name: "Re-scan" }).click();
  await expect(page).not.toHaveURL(scanUrl);
  await expect(reportHealth(page)).toBeVisible({ timeout: 30_000 });
  await expect(header.getByText("Sportsdesk from the Netherlands, mobile")).toBeVisible();

  await page.getByRole("link", { name: "Configurations", exact: true }).click();
  await expect(
    row(page, "Sportsdesk from the Netherlands, mobile").getByRole("link", { name: /Health score \d+ of 100/ }),
  ).toContainText("just now");

  // History shows the configuration and the proxy on the row, and can be narrowed to the configuration.
  await page.getByRole("link", { name: "History" }).click();
  const scans = page.getByRole("region", { name: "Scans" }).locator("tbody tr");
  await expect(scans.first()).toContainText("Sportsdesk from the Netherlands, mobile");
  await expect(scans.first()).toContainText("Proxy Netherlands");
  await page.getByRole("combobox", { name: "Configuration" }).click();
  await page.getByRole("option", { name: "Sportsdesk from the Netherlands, mobile" }).click();
  await expect(scans).toHaveCount(2);
  await page.getByRole("combobox", { name: "Configuration" }).click();
  await page.getByRole("option", { name: "Sportsdesk from Germany" }).click();
  await expect(page.getByText("No scans match")).toBeVisible();
  await page.getByRole("button", { name: "Clear filters" }).click();
  await expect(scans).toHaveCount(20);
});

test("save a scan's options as a configuration, edit it, update it and save a variant", async ({ page }) => {
  await openApp(page, "/");
  await page.getByRole("button", { name: "Options" }).click();
  await page.getByRole("radio", { name: "Browse like a visitor" }).click();
  await page.getByRole("button", { name: "Proxy, profile and budget" }).click();
  await page.getByRole("combobox", { name: "Proxy" }).click();
  await page.getByRole("option", { name: "Belgium" }).click();
  await page.getByRole("textbox", { name: "Page URL" }).fill("techpulse.example/reviews");
  await page.getByRole("button", { name: "Save as configuration…" }).click();

  const dialog = page.getByRole("dialog", { name: "Save as configuration" });
  await expect(dialog.getByLabel("Name")).toBeFocused();
  await expectNoAxeViolations(page);
  await dialog.getByLabel("Name").fill("Techpulse from Belgium");
  await page.keyboard.press("Enter");
  await expect(dialog).toHaveCount(0);

  // The composer now has it open: named, unchanged, and no scan was started.
  const composer = page.getByRole("form", { name: "New scan" });
  await expect(page).toHaveURL(/#\/\?configuration=config-/);
  await expect(composer.getByText("Techpulse from Belgium")).toBeVisible();
  await expect(composer.getByText("Edited", { exact: true })).toHaveCount(0);
  await expect(page.getByRole("region", { name: "Running", exact: true })).toHaveCount(0);
  await expect(row(page, "Techpulse from Belgium")).toContainText("Never run");

  // Edit: the change is marked, and can be saved back.
  await composer.getByRole("radio", { name: "Mobile" }).click();
  await expect(composer.getByText("Edited", { exact: true })).toBeVisible();
  await composer.getByRole("button", { name: "Update configuration" }).click();
  await expect(composer.getByText("Edited", { exact: true })).toHaveCount(0);
  await expect(row(page, "Techpulse from Belgium")).toContainText("Mobile");

  // A variant for another country, saved under a new name.
  await composer.getByRole("combobox", { name: "Proxy" }).click();
  await page.getByRole("option", { name: "Netherlands" }).click();
  await composer.getByRole("button", { name: "Save as new…" }).click();
  await dialog.getByLabel("Name").fill("Techpulse from the Netherlands");
  await dialog.getByRole("button", { name: "Save configuration" }).click();
  await expect(composer.getByText("Techpulse from the Netherlands")).toBeVisible();
  await expect(row(page, "Techpulse from Belgium")).toContainText("Belgium");
  await expect(row(page, "Techpulse from the Netherlands")).toContainText("Netherlands");

  // Scanning the open, unchanged configuration is a run of it.
  await composer.getByRole("button", { name: "Scan", exact: true }).click();
  await expect(page).toHaveURL(/#\/scans\/scan-/);
  await expect(reportHealth(page)).toBeVisible({ timeout: 40_000 });
  await expect(page.getByRole("main").locator("header").getByText("Techpulse from the Netherlands")).toBeVisible();

  // It all survives a reload.
  await page.goto("/#/configurations");
  await page.reload();
  await expect(table(page).locator("tbody tr")).toHaveCount(6);
});

test("edit, duplicate, rename and delete from the list; scans are kept", async ({ page }) => {
  await openApp(page, "/configurations");
  const menu = async (name: string, item: string) => {
    await row(page, name)
      .getByRole("button", { name: `Actions for ${name}` })
      .click();
    await page.getByRole("menuitem", { name: item }).click();
  };

  await menu("Sportsdesk from Belgium", "Edit");
  await expect(page).toHaveURL(/#\/\?configuration=config-sportsdesk-belgium$/);
  const composer = page.getByRole("form", { name: "New scan" });
  await expect(composer.getByRole("textbox", { name: "Page URL" })).toHaveValue(
    "https://www.sportsdesk.example/football/transfer-news",
  );
  await expect(composer.getByRole("radio", { name: "Browse like a visitor" })).toBeChecked();
  await expect(composer.getByRole("combobox", { name: "Proxy" })).toHaveText("Belgium");
  // Closing it gives the empty composer back.
  await composer.getByRole("link", { name: "Close configuration" }).click();
  await expect(composer.getByRole("textbox", { name: "Page URL" })).toHaveValue("");

  await page.getByRole("link", { name: "Configurations", exact: true }).click();
  await menu("Sportsdesk from Belgium", "Duplicate");
  await expect(row(page, "Sportsdesk from Belgium copy")).toContainText("Never run");

  await menu("Sportsdesk from Belgium copy", "Rename…");
  const rename = page.getByRole("dialog", { name: "Rename configuration" });
  await rename.getByLabel("Name").fill("Sportsdesk from Belgium");
  await rename.getByRole("button", { name: "Rename" }).click();
  await expect(rename.getByRole("alert")).toContainText("already exists");
  await rename.getByLabel("Name").fill("Sportsdesk from Brussels");
  await page.keyboard.press("Enter");
  await expect(row(page, "Sportsdesk from Brussels")).toBeVisible();

  await menu("Sportsdesk from Belgium", "Delete…");
  const confirm = page.getByRole("alertdialog", { name: "Delete “Sportsdesk from Belgium”?" });
  await expect(confirm).toContainText("Scans that were run from it stay in history.");
  await expectNoAxeViolations(page);
  await confirm.getByRole("button", { name: "Delete" }).click();
  await expect(row(page, "Sportsdesk from Belgium")).toHaveCount(0);
  await expect(page.getByText("4 configurations")).toBeVisible();
  // Its scan is still there, under the name it was started with.
  await page.goto("/#/scans/seed-sportsdesk-browse");
  await expect(page.getByRole("main").locator("header").getByText("Sportsdesk from Belgium")).toBeVisible();
});

test("a configuration whose proxy was deleted says so, cannot run, and is fixed by choosing another proxy", async ({
  page,
}) => {
  await openApp(page, "/configurations");
  const broken = row(page, "Sportsdesk from Germany");
  await expect(broken).toContainText(
    "The proxy “Germany” was deleted. Edit the configuration and choose another proxy.",
  );
  await expect(broken.getByRole("button", { name: /^Run / })).toBeDisabled();
  // The palette does not offer what cannot run.
  await page.keyboard.press("Control+k");
  const palette = page.getByRole("dialog", { name: "Command palette" });
  await palette.getByRole("combobox").fill("run configuration");
  await expect(palette.getByRole("option", { name: /Run configuration/ })).toHaveCount(3);
  await expect(palette.getByRole("option", { name: /Germany/ })).toHaveCount(0);
  await page.keyboard.press("Escape");

  await broken.getByRole("button", { name: "Actions for Sportsdesk from Germany" }).click();
  await page.getByRole("menuitem", { name: "Edit" }).click();
  const composer = page.getByRole("form", { name: "New scan" });
  await expect(composer.getByText("This proxy was deleted. Choose another one.")).toBeVisible();
  await composer.getByRole("button", { name: "Scan", exact: true }).click();
  await expect(page).toHaveURL(/#\/\?configuration=config-sportsdesk-germany$/);

  await composer.getByRole("combobox", { name: "Proxy" }).click();
  await page.getByRole("option", { name: "Belgium" }).click();
  await composer.getByRole("button", { name: "Update configuration" }).click();
  await expect(composer.getByText("Edited", { exact: true })).toHaveCount(0);
  await expect(row(page, "Sportsdesk from Germany").getByRole("button", { name: /^Run / })).toBeEnabled();
  await expect(row(page, "Sportsdesk from Germany")).not.toContainText("was deleted");
});

test("with no configurations the screen explains them and home stays uncluttered", async ({ page }) => {
  await openApp(page, "/configurations", { profile: "fresh" });
  await expect(page.getByText("No configurations yet")).toBeVisible();
  await expect(page.getByText(/the same site from Belgium on mobile and from Germany on desktop/)).toBeVisible();
  await expectNoAxeViolations(page);
  await page.getByRole("link", { name: "Set up a scan" }).click();
  await page.getByRole("button", { name: /Continue/ }).click();
  await expect(page.getByRole("form", { name: "New scan" })).toBeVisible();
  await expect(table(page)).toHaveCount(0);
  await startScan(page, "example.com");
  await expect(page).toHaveURL(/#\/scans\/scan-/);
});

test("re-scanning a scan whose configuration was deleted scans it with its own options", async ({ page }) => {
  await openApp(page, "/configurations");
  await row(page, "Morning Ledger, no proxy")
    .getByRole("button", { name: /^Actions for/ })
    .click();
  await page.getByRole("menuitem", { name: "Delete…" }).click();
  await page.getByRole("alertdialog").getByRole("button", { name: "Delete" }).click();
  await expect(row(page, "Morning Ledger, no proxy")).toHaveCount(0);

  await page.goto("/#/scans/seed-morningledger-desktop");
  const header = page.getByRole("main").locator("header");
  // The report still says where the scan came from.
  await expect(header.getByText("Morning Ledger, no proxy")).toBeVisible();
  await page.getByRole("button", { name: "Re-scan" }).hover();
  await expect(page.getByRole("tooltip")).toHaveText("Scan this page again with the same options");
  await page.getByRole("button", { name: "Re-scan" }).click();
  await expect(page).toHaveURL(/#\/scans\/scan-/);
  await expect(reportHealth(page)).toBeVisible({ timeout: 30_000 });
  await expect(header.getByText("Morning Ledger, no proxy")).toHaveCount(0);
  await expect(page.getByRole("region", { name: "Overview" }).getByText("No proxy", { exact: true })).toBeVisible();
});
