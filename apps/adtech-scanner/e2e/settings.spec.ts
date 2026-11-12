import { expect, test } from "@playwright/test";
import { openApp } from "./app";

const VALID_KEY = "sk-ant-api03-abcdefghijklmnopqrstuv";

test("settings changes are saved and survive a reload", async ({ page }) => {
  await openApp(page, "/settings");
  await expect(page).toHaveTitle("Settings · Ad-Tech Scanner");

  const budget = page.getByLabel("Budget per scan");
  await budget.fill("0.75");
  await budget.press("Enter");
  await expect(page.getByText("Settings saved")).toBeVisible();

  const analysisModel = page.getByLabel("Logic analysis model");
  await analysisModel.fill("claude-opus-5-5");
  await analysisModel.blur();
  await page.getByRole("radiogroup", { name: "Analysis effort" }).getByRole("radio", { name: "High" }).click();
  await page.getByRole("switch", { name: "Run headless" }).click();
  await page.getByRole("radiogroup", { name: "Default device" }).getByRole("radio", { name: "Mobile" }).click();
  const observe = page.getByLabel("Observe after load");
  await observe.fill("35");
  await observe.blur();
  await expect(
    page.getByRole("radiogroup", { name: "Default device" }).getByRole("radio", { name: "Mobile" }),
  ).toBeChecked();

  // The composer picks up the new defaults and the new cost estimate.
  await page.getByRole("link", { name: "Home", exact: true }).click();
  await expect(page.getByRole("radiogroup", { name: "Device" }).getByRole("radio", { name: "Mobile" })).toBeChecked();
  await expect(page.getByText(/hard cap \$0\.75 per scan/)).toBeVisible();
  await expect(page.getByText(/with claude-opus-5-5/)).toBeVisible();
  await page.getByRole("button", { name: "Options" }).click();
  await expect(page.getByText("35 s", { exact: true })).toBeVisible();

  await page.getByRole("link", { name: "Settings" }).click();
  await page.reload();
  await expect(page.getByLabel("Budget per scan")).toHaveValue("0.75");
  await expect(page.getByLabel("Logic analysis model")).toHaveValue("claude-opus-5-5");
  await expect(
    page.getByRole("radiogroup", { name: "Analysis effort" }).getByRole("radio", { name: "High" }),
  ).toBeChecked();
  await expect(page.getByRole("switch", { name: "Run headless" })).not.toBeChecked();
  await expect(page.getByLabel("Observe after load")).toHaveValue("35");
});

test("invalid values are explained and not saved", async ({ page }) => {
  await openApp(page, "/settings");
  const budget = page.getByLabel("Budget per scan");
  await budget.fill("500");
  await budget.blur();
  await expect(page.getByText("The budget cannot exceed $20.00.")).toBeVisible();
  await budget.press("Escape");
  await expect(budget).toHaveValue("0.50");

  await page.reload();
  await expect(page.getByLabel("Budget per scan")).toHaveValue("0.50");
});

test("API key: remove, reject an invalid key, validate and save a good one", async ({ page }) => {
  await openApp(page, "/settings");
  await expect(page.getByText("A validated key is stored in your system keychain.")).toBeVisible();

  await page.getByRole("button", { name: "Remove key" }).click();
  await page.getByRole("alertdialog").getByRole("button", { name: "Remove key" }).click();
  const input = page.getByLabel("Anthropic API key");
  await expect(input).toBeVisible();
  await expect(input).toHaveAttribute("type", "password");

  await input.fill("sk-wrong");
  await page.getByRole("button", { name: "Validate and save" }).click();
  await expect(page.getByRole("alert")).toContainText("Anthropic rejected this key");

  await input.fill(VALID_KEY);
  await page.getByRole("button", { name: "Validate and save" }).click();
  await expect(page.getByText("A validated key is stored in your system keychain.")).toBeVisible();
  // The key is never echoed back anywhere on the page.
  await expect(page.locator("body")).not.toContainText(VALID_KEY);

  await page.reload();
  await expect(page.getByText("A validated key is stored in your system keychain.")).toBeVisible();
});

test("theme: light, dark and system, applied before first paint after a reload", async ({ page }) => {
  await page.emulateMedia({ colorScheme: "light" });
  await openApp(page, "/settings");
  const html = page.locator("html");
  const theme = page.getByRole("radiogroup", { name: "Theme" });
  await expect(theme.getByRole("radio", { name: "System" })).toBeChecked();
  await expect(html).not.toHaveClass(/dark/);

  await theme.getByRole("radio", { name: "Dark" }).click();
  await expect(html).toHaveClass(/dark/);
  await expect(page.locator("body")).toHaveCSS("color-scheme", "dark");
  await expect(page.getByText("Settings saved")).toBeVisible();

  // No flash: the class is already there when the document has parsed, before the app loads.
  await page.reload({ waitUntil: "commit" });
  await expect(html).toHaveClass(/dark/);
  await expect(theme.getByRole("radio", { name: "Dark" })).toBeChecked();

  await theme.getByRole("radio", { name: "Light" }).click();
  await expect(html).not.toHaveClass(/dark/);

  // System follows the OS setting live.
  await theme.getByRole("radio", { name: "System" }).click();
  await page.emulateMedia({ colorScheme: "dark" });
  await expect(html).toHaveClass(/dark/);
  await page.emulateMedia({ colorScheme: "light" });
  await expect(html).not.toHaveClass(/dark/);
});

test("delete all scans from settings", async ({ page }) => {
  await openApp(page, "/settings");
  await page.getByRole("button", { name: "Delete all scans…" }).click();
  await page.getByRole("alertdialog").getByRole("button", { name: "Delete all" }).click();
  await expect(page.getByText("All scans deleted")).toBeVisible();
  await page.getByRole("link", { name: "Home", exact: true }).click();
  await expect(page.getByText("No scans yet")).toBeVisible();
  await expect(page.getByRole("region", { name: "Recent scans" })).toBeVisible();
});
