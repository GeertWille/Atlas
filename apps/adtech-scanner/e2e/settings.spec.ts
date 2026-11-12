import { expect, test } from "@playwright/test";
import { openApp, reportTab, startScan } from "./app";

const VALID_KEY = "sk-ant-api03-abcdefghijklmnopqrstuv";
const TYPESAFE_KEY = "typesafe-e2e-key-0123456789";

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

test("TypeSafe key and the Jev switch: rejected key, stored key, enable, reload, remove", async ({ page }) => {
  await openApp(page, "/settings");
  const jev = page.getByRole("switch", { name: "Classify scripts with Jev" });
  const triage = page.locator("[data-slot=field]", { hasText: "Script triage" });
  await expect(jev).toBeDisabled();
  await expect(page.getByText("Add a TypeSafe key first.")).toBeVisible();
  await expect(triage).toContainText("Claude Haiku 5.5");
  // The second vendor is disclosed before anything can be switched on.
  await expect(page.getByText(/Sends excerpts of the scanned page's scripts to api\.typesafe\.ai/)).toBeVisible();

  const input = page.getByLabel("TypeSafe API key");
  await expect(input).toHaveAttribute("type", "password");
  await input.fill("invalid-key");
  await page.getByRole("button", { name: "Save TypeSafe key" }).click();
  await expect(page.getByRole("alert")).toHaveText("TypeSafe rejected this API key.");
  await expect(jev).toBeDisabled();

  await input.fill(TYPESAFE_KEY);
  await input.press("Enter");
  await expect(page.getByText("A validated TypeSafe key is stored in your system keychain.")).toBeVisible();
  await expect(page.locator("body")).not.toContainText(TYPESAFE_KEY);
  // Saving a key does not flip the switch.
  await expect(jev).toBeEnabled();
  await expect(jev).not.toBeChecked();
  await expect(triage).toContainText("Claude Haiku 5.5");

  // The switch is reachable and operable from the keyboard.
  await jev.focus();
  await page.keyboard.press("Space");
  await expect(jev).toBeChecked();
  await expect(triage).toContainText("Jev System One");
  await expect(triage).toContainText("Claude Haiku 5.5 takes over if Jev fails.");
  await expect(page.getByText("Settings saved")).toBeVisible();

  await page.reload();
  await expect(jev).toBeChecked();
  await expect(triage).toContainText("Jev System One");
  await expect(page.getByText(/api\.anthropic\.com, api\.typesafe\.ai when Jev is enabled and,/)).toBeVisible();

  await page.getByRole("button", { name: "Remove TypeSafe key" }).click();
  const dialog = page.getByRole("alertdialog", { name: "Remove the TypeSafe key?" });
  await expect(dialog).toContainText("Jev is turned off");
  await dialog.getByRole("button", { name: "Remove TypeSafe key" }).click();
  await expect(page.getByLabel("TypeSafe API key")).toBeVisible();
  await expect(jev).toBeDisabled();
  await expect(jev).not.toBeChecked();
  await expect(triage).toContainText("Claude Haiku 5.5");
  // The Anthropic key was never touched.
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

test("the sellers.json check is opt-in, survives a reload and shows in the next report", async ({ page }) => {
  await openApp(page, "/settings");
  const sellersJson = page.getByRole("switch", { name: "Check sellers.json at ad exchanges" });
  await expect(sellersJson).not.toBeChecked();
  await expect(page.getByText(/Off: only the scanned site is contacted\./)).toBeVisible();
  await expect(
    page.getByText(/when the sellers\.json check is on, the ad exchanges that ads\.txt names\./),
  ).toBeVisible();

  await sellersJson.focus();
  await page.keyboard.press("Space");
  await expect(sellersJson).toBeChecked();
  await expect(page.getByText("Settings saved")).toBeVisible();
  await page.reload();
  await expect(sellersJson).toBeChecked();

  await page.getByRole("link", { name: "Home", exact: true }).click();
  await startScan(page, "sportsdesk.example");
  await expect(reportTab(page, "Auctions")).toBeVisible({ timeout: 20_000 });
  await reportTab(page, "Auctions").click();
  const checks = page.getByRole("heading", { name: "Bidder checks" }).locator("xpath=ancestor::section[1]");
  await expect(checks.getByText("https://sportsdesk.example/ads.txt", { exact: false })).toBeVisible();
  await expect(checks.getByText("Looked up at the exchanges that ads.txt names")).toBeVisible();
  const openx = checks.getByRole("listitem", { name: "openx" });
  await expect(openx.getByRole("img", { name: "Notice" })).toBeVisible();
  await expect(openx).toContainText("sellers.json: ads.txt lists seller id 544096208 at openx.com as DIRECT");
  await expect(openx.getByText("openx.com, 544096208, DIRECT", { exact: true })).toBeVisible();

  // The audit trail names the extra requests.
  await reportTab(page, "Audit trail").click();
  await expect(
    page
      .getByRole("region", { name: "Log entries" })
      .getByText("Requesting sellers.json from the exchanges that ads.txt names"),
  ).toBeVisible();
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
