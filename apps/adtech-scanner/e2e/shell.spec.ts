import { expect, test } from "@playwright/test";
import { openApp } from "./app";

test("keyboard shortcuts: palette, new scan, settings", async ({ page }) => {
  await openApp(page, "/history");
  const palette = page.getByRole("dialog", { name: "Command palette" });

  await page.keyboard.press("Control+k");
  await expect(palette).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(palette).toBeHidden();

  await page.keyboard.press("Control+Comma");
  await expect(page).toHaveURL(/#\/settings/);

  await page.keyboard.press("Control+n");
  await expect(page).toHaveURL(/#\/$/);
  await expect(page.getByRole("textbox", { name: "Page URL" })).toBeFocused();

  // The palette toggles with the same shortcut.
  await page.keyboard.press("Control+k");
  await expect(palette).toBeVisible();
  await page.keyboard.press("Control+k");
  await expect(palette).toBeHidden();
});

test("command palette: navigate, open a scan, search logs, switch theme, start the demo", async ({ page }) => {
  await page.emulateMedia({ colorScheme: "light" });
  await openApp(page, "/");
  const palette = page.getByRole("dialog", { name: "Command palette" });
  const open = async () => {
    await page.getByRole("button", { name: /^Search/ }).click();
    await expect(palette).toBeVisible();
  };

  await open();
  await page.keyboard.type("hist");
  await expect(palette.getByRole("option").first()).toHaveText("History");
  await page.keyboard.press("Enter");
  await expect(page).toHaveURL(/#\/history/);
  await expect(palette).toBeHidden();

  await open();
  await page.keyboard.type("techpulse reviews");
  await palette.getByRole("option", { name: /techpulse\.example\/reviews\/laptops/ }).click();
  await expect(page).toHaveURL(/#\/scans\/seed-techpulse-desktop/);

  await open();
  await page.keyboard.type("failsafe");
  await palette.getByRole("option", { name: "Search audit trails for “failsafe”" }).click();
  await expect(page).toHaveURL(/#\/logs\?q=failsafe/);
  await expect(page.getByRole("searchbox", { name: "Search all audit trails" })).toHaveValue("failsafe");

  await open();
  await page.keyboard.type("dark");
  await page.keyboard.press("Enter");
  await expect(page.locator("html")).toHaveClass(/dark/);

  await open();
  await page.keyboard.type("no such command");
  await expect(palette.getByText("No matching commands.")).toBeVisible();
  await page.keyboard.press("Escape");

  await open();
  await palette.getByRole("option", { name: "Scan the demo site" }).click();
  await expect(page.getByRole("list", { name: "Pipeline" })).toBeVisible();
});

test("sidebar: navigation state, recent scans and the running indicator", async ({ page }) => {
  await openApp(page, "/", { speed: 1 });
  const main = page.getByRole("navigation", { name: "Main" });
  await expect(main.getByRole("link", { name: "Home" })).toHaveAttribute("data-status", "active");

  await main.getByRole("link", { name: "Log search" }).click();
  await expect(main.getByRole("link", { name: "Log search" })).toHaveAttribute("data-status", "active");
  await expect(main.getByRole("link", { name: "Home" })).not.toHaveAttribute("data-status", "active");

  await page.getByRole("region", { name: "Recent" }).getByRole("link").first().click();
  await expect(page).toHaveURL(/#\/scans\/seed-sportsdesk-mobile/);

  await page.getByRole("button", { name: /New scan/ }).click();
  await page.getByRole("textbox", { name: "Page URL" }).fill("citywire-daily.example");
  await page.keyboard.press("Enter");
  const running = page.getByRole("region", { name: "Running", exact: true });
  await expect(running.getByRole("link", { name: /citywire-daily\.example/ })).toBeVisible();

  // The dashboard shows it live as well.
  await main.getByRole("link", { name: "Home" }).click();
  await expect(page.getByRole("region", { name: "Running scans" }).getByRole("link")).toContainText("Scanning now");
  await running.getByRole("link").click();
  await expect(page.getByRole("list", { name: "Pipeline" })).toBeVisible();
});

test("unknown routes have a way home", async ({ page }) => {
  await openApp(page, "/nowhere");
  await expect(page.getByText("There is nothing here")).toBeVisible();
  await page.getByRole("link", { name: "Go home" }).click();
  await expect(page.getByRole("heading", { name: "Scan a page for revenue leaks" })).toBeVisible();
});
