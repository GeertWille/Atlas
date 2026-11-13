import { expect, test } from "@playwright/test";
import { MOD, openApp } from "./app";

test("keyboard shortcuts: palette, new scan, settings", async ({ page }) => {
  await openApp(page, "/history");
  const palette = page.getByRole("dialog", { name: "Command palette" });

  const search = page.getByRole("searchbox", { name: "Search scans" });
  await search.focus();
  await page.keyboard.press(`${MOD}+k`);
  await expect(palette).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(palette).toBeHidden();
  // Closing the palette puts the keyboard back where it was.
  await expect(search).toBeFocused();

  await page.keyboard.press(`${MOD}+Comma`);
  await expect(page).toHaveURL(/#\/settings/);

  await page.keyboard.press(`${MOD}+n`);
  await expect(page).toHaveURL(/#\/$/);
  await expect(page.getByRole("textbox", { name: "Page URL" })).toBeFocused();

  // The palette toggles with the same shortcut, and every opening starts empty.
  await page.keyboard.press(`${MOD}+k`);
  await expect(palette).toBeVisible();
  await page.keyboard.type("hist");
  await page.keyboard.press(`${MOD}+k`);
  await expect(palette).toBeHidden();
  await page.keyboard.press(`${MOD}+k`);
  await expect(palette.getByRole("combobox")).toHaveValue("");
});

test("deep links keep their search params when the page URL has its own query string", async ({ page }) => {
  await page.goto("/?fakeProfile=seeded&fakeSpeed=40&ref=mail#/scans/seed-techpulse-desktop?tab=auctions");
  await expect(page.getByRole("tab", { name: /^Auctions/ })).toHaveAttribute("aria-selected", "true");

  await page.getByRole("tab", { name: /^Ad slots/ }).click();
  await expect(page).toHaveURL(/\?ref=mail#\/scans\/seed-techpulse-desktop\?tab=slots$/);
  await page.reload();
  await expect(page.getByRole("tab", { name: /^Ad slots/ })).toHaveAttribute("aria-selected", "true");
});

test("searching the audit trails from the palette replaces the query already on screen", async ({ page }) => {
  await openApp(page, "/logs?q=failsafe");
  const input = page.getByRole("searchbox", { name: "Search all audit trails" });
  await expect(input).toHaveValue("failsafe");

  await page.keyboard.press(`${MOD}+k`);
  await page.keyboard.type("IS_MOBILE");
  await page.getByRole("option", { name: "Search audit trails for “IS_MOBILE”" }).click();

  await expect(input).toHaveValue("IS_MOBILE");
  await expect(page).toHaveURL(/#\/logs\?q=IS_MOBILE/);
  // The old draft must not win the URL back once its debounce fires.
  await page.waitForTimeout(600);
  await expect(page).toHaveURL(/#\/logs\?q=IS_MOBILE/);
  await expect(input).toHaveValue("IS_MOBILE");
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
  await expect(page).toHaveURL(/#\/scans\/scan-/);
  await expect(page.getByRole("heading", { level: 1, name: "127.0.0.1" })).toBeVisible();
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
