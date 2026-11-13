import { expect, type Page, test } from "@playwright/test";
import {
  expectNoAxeViolations,
  expectNoHorizontalOverflow,
  expectWindowDoesNotScroll,
  MOD,
  openApp,
  reportHealth,
  reportTab,
} from "./app";

const DISCLOSURE =
  "With Browserbase, the browser runs in Browserbase's cloud, not on this Mac. The pages you scan, their cookies, and everything the scanner reads from them (scripts, requests, bid data) travel through Browserbase, a third party. Browserbase keeps a video of each session and a log that includes the content the scanner reads and the cookies it reads or sets. CAPTCHAs on scanned pages are solved automatically, and the browser does not tell the page it is automated. To do this, Browserbase runs its own scripts inside every page you scan; the scanner leaves them out of its results, but they can change how a page behaves. Results are also stored on this Mac. Browserbase bills browser time per minute, and proxy traffic per megabyte.";
const SEEDED_REPORT = "/scans/seed-morningledger-browserbase";
const SEEDED_CONFIGURATION = "Garden Weekly from Belgium, Browserbase";

const region = (page: Page, name: string) => page.getByRole("region", { name });
const scanButton = (page: Page) => page.getByRole("button", { name: "Scan", exact: true });

async function choose(page: Page, within: ReturnType<typeof region>, combobox: string, option: string | RegExp) {
  await within.getByRole("combobox", { name: combobox }).click();
  await page.getByRole("option", { name: option }).click();
}

/** The composer with a public address typed, its options open and Browserbase chosen. */
async function composeOnBrowserbase(page: Page, url = "morningledger.example") {
  await page.getByRole("textbox", { name: "Page URL" }).fill(url);
  await page.getByRole("button", { name: "Options" }).click();
  await region(page, "Origin").getByRole("radio", { name: "Browserbase" }).click();
}

async function removeBrowserbaseKey(page: Page) {
  await region(page, "Browserbase").getByRole("button", { name: "Remove Browserbase key" }).click();
  await page.getByRole("alertdialog").getByRole("button", { name: "Remove Browserbase key" }).click();
  await expect(page.getByText("Browserbase key removed")).toBeVisible();
}

test("Browserbase is set up in Settings: the disclosure first, then the key, the free test and the page check", async ({
  page,
}) => {
  await openApp(page, "/settings", { profile: "fresh" });
  const browserbase = region(page, "Browserbase");
  await expect(browserbase.getByRole("note", { name: "What using Browserbase means" })).toHaveText(DISCLOSURE);
  await expect(browserbase.getByRole("button", { name: "Test connection" })).toHaveCount(0);
  const defaults = region(page, "Scan defaults");
  await expect(defaults.getByRole("radio", { name: "Browserbase" })).toBeDisabled();

  const key = browserbase.getByLabel("Browserbase API key");
  await key.fill("bb_live_invalid");
  await browserbase.getByRole("button", { name: "Save Browserbase key" }).click();
  await expect(browserbase.getByRole("alert")).toHaveText("Browserbase did not accept the API key.");

  await key.fill("bb_live_0123456789abcdef");
  await key.press("Enter");
  await expect(
    browserbase.getByText("Connected to the project “Local”. The key is in your system keychain."),
  ).toBeVisible();
  // The disclosure stays where it was.
  await expect(browserbase.getByRole("note", { name: "What using Browserbase means" })).toBeVisible();

  await browserbase.getByRole("button", { name: "Test connection" }).click();
  await expect(browserbase.getByText(/Project “Local”: up to 3 browsers at once, 1 running now\./)).toBeVisible();

  const check = browserbase.getByRole("button", { name: "Check what a page sees" });
  await expect(check).toHaveAccessibleDescription(
    /Starts a Browserbase browser for under a minute .* bills it as one browser minute/,
  );
  await check.click();
  const seen = browserbase.getByRole("group", { name: "What a page sees" });
  await expect(seen).toContainText("false: the page is not told the visit is automated");
  await expect(seen).toContainText("America/Los_Angeles");
  await expect(seen).toContainText("billed as one minute");

  // What a Browserbase scan starts from.
  await choose(page, browserbase, "Default region", /Frankfurt/);
  await expect(page.getByText("Settings saved")).toBeVisible();
  const keep = browserbase.getByRole("switch", { name: "Keep a browser profile on Browserbase" });
  await expect(keep).not.toBeChecked();
  await expect(browserbase.getByText("No browser profile is kept at Browserbase.")).toBeVisible();
  await defaults.getByRole("radio", { name: "Browserbase" }).click();
  await expect(defaults.getByRole("radio", { name: "Browserbase" })).toBeChecked();

  // It is all still there after a restart, and the composer starts from it.
  await page.reload();
  await expect(browserbase.getByText(/Connected to the project “Local”/)).toBeVisible();
  await expect(browserbase.getByRole("combobox", { name: "Default region" })).toHaveText(/Frankfurt/);
  await page.getByRole("link", { name: "Home", exact: true }).click();
  await page.getByRole("button", { name: /Continue/ }).click();
  await page.getByRole("textbox", { name: "Page URL" }).fill("morningledger.example");
  await expect(scanButton(page)).toHaveAccessibleDescription(
    /^Scroll · Browserbase, Frankfurt \(eu-central-1\) · no proxy · /,
  );
});

test("a scan on Browserbase with a region and a Browserbase location: composed, run, and reported with its late usage", async ({
  page,
}) => {
  await openApp(page, "/", { speed: 8 });
  await composeOnBrowserbase(page);
  const origin = region(page, "Origin");
  await choose(page, origin, "Region", /Frankfurt/);
  await choose(page, origin, "Location", "Browserbase location…");
  await origin.getByLabel("Country code").fill("be");
  await expect(
    origin.getByText(
      "A residential connection; the country is best effort. About 18 MB go through it, an estimated $0.22 at list price.",
    ),
  ).toBeVisible();
  // The browser's cost stands apart from the model's.
  const analysis = region(page, "Analysis");
  await analysis.getByRole("switch", { name: "AI logic analysis" }).click();
  await expect(analysis.getByRole("note", { name: "Browser cost" })).toHaveText(
    "Browserbase bills apart from the LLM budget: about 1 browser minute and 18 MB of proxy traffic. An estimated $0.22 at Browserbase's list price of $0.12 per browser hour and $12 per GB (2026-11-12); your plan's included usage is not counted.",
  );
  // The heading above the composer does not say where the browser runs: the composer does.
  await expect(page.getByText(/every script in the browser, then checks the wrapper logic\./)).toBeVisible();
  await expect(page.getByText(/your local browser/)).toHaveCount(0);
  await expect(scanButton(page)).toHaveAccessibleDescription(
    "Scroll · Browserbase, Frankfurt (eu-central-1) · Belgium (Browserbase proxy) · AI off",
  );
  await expect(region(page, "Visit").getByText("No window", { exact: true })).toBeVisible();
  await scanButton(page).click();

  // No Chrome is started on this Mac, and the visit panel follows the cloud browser.
  const visit = region(page, "Visit");
  await expect(visit.getByText("Browserbase, Frankfurt (eu-central-1)")).toBeVisible({ timeout: 15_000 });
  await expect(visit.getByText(/· running 00:\d\d/)).toBeVisible();
  await expect(page.getByRole("list", { name: "Pipeline" })).toContainText("Browserbase, eu-central-1");

  await expect(reportHealth(page)).toBeVisible({ timeout: 45_000 });
  const overview = region(page, "Overview");
  for (const fact of [
    "Browserbase, Frankfurt (eu-central-1)",
    "Belgium (Browserbase proxy)",
    "Fresh browser profile",
  ]) {
    await expect(overview.getByText(fact, { exact: true }).first()).toBeVisible();
  }
  // The region is said once in the overview, on the visit line.
  await expect(overview.getByText(/Frankfurt/)).toHaveCount(1);
  // The proxy traffic is read again half a minute after the end; until then it is still being counted.
  const usage = overview.getByRole("group", { name: "Browserbase visit" });
  await expect(usage).toContainText("proxy traffic still being counted");
  await expect(usage).toContainText("18 MB through the proxy", { timeout: 15_000 });
  await expect(usage).toContainText(/about \$0\.2\d at list price/);
  // The LLM cost is its own figure and has nothing of it.
  await expect(page.getByText("$0.00 LLM cost")).toBeVisible();

  await usage.getByRole("button", { name: "Browser details" }).click();
  for (const fact of [
    "Browserbase, Frankfurt, Germany (eu-central-1)",
    "Browserbase proxy: a residential connection, Belgium requested (best effort)",
    "Europe/Brussels, set by Browserbase for its proxy",
    "false: the pages were not told the visit is automated",
    "Chrome/157.0.8103.79, headless, which the user agent does not show",
    "5 scripts ran in the pages; left out of this scan's scripts and results",
    "Solved by Browserbase automatically",
    "A video and a log",
  ]) {
    await expect(usage.getByText(fact, { exact: true })).toBeVisible();
  }
  await expect(usage.getByText(/an estimate at Browserbase's list price of \$0\.12 per browser hour/)).toContainText(
    "your plan's included usage is not counted",
  );
  await expect(overview.getByText(/Developer|Startup|overage/)).toHaveCount(0);
  // The note says where it ran and the one thing that can change the result; the facts are in the details.
  const note = overview.getByRole("note", { name: "Scan environment" });
  await expect(note).toHaveText(
    "This visit ran in a Browserbase browser; what the pages could read there is under Browser details. Browserbase ran its own scripts inside every page (5 scripts in this visit); the scanner leaves them out of its results, but they can change how a page behaves. To compare, repeat the scan on this Mac.",
  );
  await expect(note).not.toContainText("the pages could tell");

  // The capture's own line about those scripts is in the audit trail, and none of them among the scripts.
  await reportTab(page, "Audit trail").click();
  await expect(
    region(page, "Log entries")
      .getByText(
        "Browserbase ran 5 scripts of its own in the page (271667 characters); they are not among the scan's scripts",
      )
      .first(),
  ).toBeVisible();
  await expect(
    region(page, "Log entries")
      .getByText("1 response body was too large for the connection to the remote browser and was not read")
      .first(),
  ).toBeVisible();

  // History marks the scan, and nothing else in it.
  await page.getByRole("link", { name: "History", exact: true }).click();
  const rows = region(page, "Scans").locator("tbody tr");
  await expect(rows.first().getByTitle("Ran in a Browserbase browser in Frankfurt (eu-central-1)")).toBeVisible();
  await expect(region(page, "Scans").getByTitle(/Ran in a Browserbase browser/)).toHaveCount(2);
});

test("“I'll browse myself” on Browserbase: the live view is opened by scan, and Finish keeps what was recorded", async ({
  page,
}) => {
  await openApp(page, "/", { speed: 20 });
  await composeOnBrowserbase(page);
  const visitOptions = region(page, "Visit");
  await visitOptions.getByRole("radio", { name: "I'll browse myself" }).click();
  await expect(visitOptions.getByText("In the live view")).toBeVisible();
  await expect(visitOptions.getByText("Browserbase bills the session for as long as it is open.")).toBeVisible();
  await scanButton(page).click();

  const session = region(page, "Manual session");
  await expect(session.getByRole("heading")).toHaveText("You are browsing morningledger.example", { timeout: 15_000 });
  await expect(session).toContainText("Use Browserbase's live view, which opened in your own browser.");
  await expect(session).toContainText("Browserbase bills the session until you finish.");
  await session.getByRole("button", { name: "Open live view" }).click();
  await expect(page.getByText("Live view opened in your browser")).toBeVisible();
  // The address of the live view is a credential: nothing like it is anywhere on the page.
  expect(await page.content()).not.toMatch(
    /debuggerFullscreenUrl|devtools\/inspector|connect\.[a-z0-9-]+\.browserbase\.com\//,
  );

  await expect(session.getByRole("button", { name: "Cancel scan" })).toBeVisible();
  await session.getByRole("button", { name: "Finish capture" }).click();
  await expect(reportHealth(page)).toBeVisible({ timeout: 45_000 });
  await expect(region(page, "Overview").getByRole("note", { name: "Scan environment" })).toContainText(
    "This visit ran in a Browserbase browser, which you drove through its live view; what the pages could read there is under Browser details.",
  );
});

test("a configuration with Browserbase is saved, shown, run, and broken once the credentials are cleared", async ({
  page,
}) => {
  await openApp(page, "/");
  await composeOnBrowserbase(page, "techpulse.example/reviews");
  await choose(page, region(page, "Origin"), "Region", /Singapore/);
  await page.getByRole("button", { name: "Save as configuration…" }).click();
  const dialog = page.getByRole("dialog");
  await dialog.getByRole("textbox").first().fill("Techpulse from Singapore");
  await dialog.getByRole("button", { name: /^Save/ }).click();
  await expect(page.getByText("Scan runs this configuration as saved and records it as its last run.")).toBeVisible();
  // Opened again it is what was saved.
  await expect(region(page, "Origin").getByRole("radio", { name: "Browserbase" })).toBeChecked();
  await expect(region(page, "Origin").getByRole("combobox", { name: "Region" })).toHaveText(/Singapore/);

  await page.getByRole("link", { name: "Configurations", exact: true }).click();
  const list = region(page, "Configurations");
  const row = list.getByRole("row", { name: /^Techpulse from Singapore/ });
  await expect(row.getByTitle("Runs in a Browserbase browser in Singapore (ap-southeast-1)")).toBeVisible();
  // Beside the Browserbase mark the proxy is the country alone; its title has the whole of it.
  const seeded = list.getByRole("row", { name: new RegExp(`^${SEEDED_CONFIGURATION}`) });
  await expect(seeded.getByTitle("Belgium (Browserbase proxy)")).toHaveText("Belgium");
  await expect(seeded.getByTitle("Runs in a Browserbase browser in Frankfurt (eu-central-1)")).toContainText(
    "Frankfurt",
  );
  await row.getByRole("button", { name: "Run Techpulse from Singapore" }).click();
  await expect(page.getByRole("list", { name: "Pipeline" })).toContainText("Browserbase, ap-southeast-1");
  await expect(reportHealth(page)).toBeVisible({ timeout: 45_000 });
  await expect(
    region(page, "Overview").getByText("Browserbase, Singapore (ap-southeast-1)", { exact: true }).first(),
  ).toBeVisible();

  await page.getByRole("link", { name: "Settings", exact: true }).click();
  await removeBrowserbaseKey(page);
  await page.getByRole("link", { name: "Configurations", exact: true }).click();
  for (const name of ["Techpulse from Singapore", SEEDED_CONFIGURATION]) {
    const broken = list.getByRole("row", { name: new RegExp(`^${name}`) });
    await expect(broken).toContainText("Browserbase is not set up any more.");
    await expect(broken.getByRole("button", { name: `Run ${name}` })).toBeDisabled();
  }

  // Opened, it says what is wrong and how to go on.
  await list.getByRole("link", { name: "Techpulse from Singapore" }).click();
  await expect(
    page.getByText(
      "This configuration cannot run until Browserbase is set up again, or This Mac is chosen under Origin.",
    ),
  ).toBeVisible();
  const origin = region(page, "Origin");
  await expect(origin.getByRole("alert")).toHaveText(
    "Browserbase is not set up any more. Set it up in Settings, or choose This Mac.",
  );
  await expect(scanButton(page)).toHaveAccessibleDescription(/· Browserbase not set up ·/);
  await origin.getByRole("radio", { name: "This Mac" }).click();
  await expect(origin.getByRole("alert")).toHaveCount(0);
  await expect(page.getByText("Edited", { exact: true })).toBeVisible();
});

test("the demo always runs on this Mac, and says so when Browserbase is the default", async ({ page }) => {
  await openApp(page, "/settings");
  await region(page, "Scan defaults").getByRole("radio", { name: "Browserbase" }).click();
  await expect(page.getByText("Settings saved")).toBeVisible();

  await page.keyboard.press(`${MOD}+k`);
  await page
    .getByRole("dialog", { name: "Command palette" })
    .getByRole("option", { name: "Scan the demo site" })
    .click();
  await expect(
    page.getByText("The demo site is on this Mac, so this scan runs in this Mac's Chrome, not on Browserbase."),
  ).toBeVisible();
  await expect(page.getByRole("list", { name: "Pipeline" })).toContainText("Browser ready", { timeout: 15_000 });
  await expect(page.getByText(/Starting a Browserbase browser/)).toHaveCount(0);
  await expect(reportHealth(page)).toBeVisible({ timeout: 45_000 });
  const overview = region(page, "Overview");
  await expect(overview.getByText("Headless", { exact: true })).toBeVisible();
  await expect(overview.getByText(/Browserbase/)).toHaveCount(0);
});

test("an address on this Mac or the network disables the choice with a plain reason", async ({ page }) => {
  await openApp(page, "/");
  await composeOnBrowserbase(page);
  const origin = region(page, "Origin");
  const browserbase = origin.getByRole("radio", { name: "Browserbase" });
  await expect(browserbase).toBeChecked();
  const url = page.getByRole("textbox", { name: "Page URL" });

  for (const address of ["localhost:3000/article", "http://192.168.1.20/", "printer.local"]) {
    await url.fill(address);
    await expect(browserbase).toBeDisabled();
    await expect(origin.getByRole("radio", { name: "This Mac" })).toBeChecked();
    await expect(
      origin.getByText("This address is on this Mac or your network; a cloud browser cannot reach it."),
    ).toBeVisible();
    await expect(scanButton(page)).toHaveAccessibleDescription(/^Scroll · browser hidden · system proxy · /);
  }

  // A public address brings the choice back.
  await url.fill("morningledger.example");
  await expect(browserbase).toBeEnabled();
  await expect(browserbase).toBeChecked();
});

test("at Browserbase's limit of browsers the scan waits, then fails with the backend's message", async ({ page }) => {
  await openApp(page, "/", { speed: 6, browserbase: "limit" });
  await composeOnBrowserbase(page);
  await scanButton(page).click();
  const visit = region(page, "Visit");
  await expect(visit.getByText("Browserbase is at its limit of browsers; trying again in 5 s")).toBeVisible({
    timeout: 15_000,
  });
  // Until Browserbase has a browser for it, the scan says what it is waiting for.
  await expect(visit.getByText("Starting a Browserbase browser in Oregon (us-west-2)…")).toBeVisible();
  await expect(page.getByText("Step 2 of 9: Starting a Browserbase browser…")).toBeVisible();

  await expect(page.getByText("This scan failed")).toBeVisible({ timeout: 30_000 });
  await expect(
    page
      .getByText(
        "Browserbase is at its limit of browsers running at once (3 of 3, shared by everything that uses this project). Try again when one has finished.",
      )
      .first(),
  ).toBeVisible();
  // Settings say who else may be using them.
  await page.getByRole("link", { name: "Settings", exact: true }).click();
  await expect(
    region(page, "Browserbase").getByText(/Another application that uses the same project shares that limit\./),
  ).toBeVisible();
});

test("a lost connection ends the scan with the backend's message", async ({ page }) => {
  await openApp(page, "/", { speed: 30, browserbase: "lost" });
  await composeOnBrowserbase(page);
  await scanButton(page).click();
  await expect(page.getByText("This scan failed")).toBeVisible({ timeout: 30_000 });
  await expect(
    page
      .getByText("The connection to the Browserbase browser was lost. The browser has ended. Nothing was kept.")
      .first(),
  ).toBeVisible();
  // The audit trail of the failed scan has the moment it happened.
  await expect(
    page.getByText("The connection to the Browserbase browser was lost", { exact: true }).first(),
  ).toBeVisible();
});

test("an end that Browserbase does not confirm: the scan completes, and its report warns about the bill instead of a cost", async ({
  page,
}) => {
  await openApp(page, "/", { speed: 30, browserbase: "unconfirmed" });
  await composeOnBrowserbase(page);
  const origin = region(page, "Origin");
  await choose(page, origin, "Location", "Browserbase location…");
  await origin.getByLabel("Country code").fill("be");
  await scanButton(page).click();
  await expect(reportHealth(page)).toBeVisible({ timeout: 45_000 });

  const usage = region(page, "Overview").getByRole("group", { name: "Browserbase visit" });
  await expect(usage).toContainText(
    "Browserbase did not confirm that this browser ended. It may have kept running, and been billed, until its time limit.",
  );
  // Nothing was reported, so no time and no dollar figure is shown as if it had been.
  await expect(usage).not.toContainText(/of browser time|about \$/);
  await usage.getByRole("button", { name: "Browser details" }).click();
  await expect(usage.getByText("Not reported by Browserbase", { exact: true }).first()).toBeVisible();
  await expect(usage.getByText("Known once the proxy traffic is counted", { exact: true })).toBeVisible();

  // A count that never comes is not "still being counted" for ever: a minute after the scan it is "not reported".
  await expect(usage).toContainText("proxy traffic still being counted");
  await page.clock.install();
  await page.clock.fastForward(61_000);
  await expect(usage).toContainText("proxy traffic not reported", { timeout: 10_000 });
  await expect(usage).not.toContainText("still being counted");
  await expect(usage.getByText("Not reported by Browserbase", { exact: true })).toHaveCount(2);
  await expect(
    usage.getByText("Not known: Browserbase did not report the browser time", { exact: true }),
  ).toBeVisible();

  // The capture's own warning is in the audit trail, as a warning.
  await reportTab(page, "Audit trail").click();
  await expect(
    region(page, "Log entries")
      .getByText(
        "Browserbase did not confirm that the browser has ended; it ends by itself at its time limit at the latest",
      )
      .first(),
  ).toBeVisible();
});

test("a body too large for the connection to a Browserbase browser is listed, and says why it was not read", async ({
  page,
}) => {
  await openApp(page, `${SEEDED_REPORT}/network?q=path%3A%2Ffeed%2F`);
  await page
    .getByRole("treegrid", { name: "Requests" })
    .getByRole("row", { name: /all-seasons\.json/ })
    .click();
  const details = page.getByRole("region", { name: "Request details" });
  await details.getByRole("tab", { name: "Response" }).click();
  await expect(details.getByRole("note")).toContainText("Body not read (was 11.6 MB)");
  await expect(details.getByRole("note")).toContainText(
    "It was too large to bring over the connection to the Browserbase browser in one piece, so the scanner left it there.",
  );
});

test("a scan stored before the browser could run elsewhere is scanned again on this Mac, also with Browserbase as the default", async ({
  page,
}) => {
  await openApp(page, "/settings");
  await region(page, "Scan defaults").getByRole("radio", { name: "Browserbase" }).click();
  await expect(page.getByText("Settings saved")).toBeVisible();

  // This scan's options do not say where its browser ran: it ran here, and that is where it runs again.
  await page.goto("/#/scans/seed-sportsdesk-mobile");
  await page.getByRole("button", { name: "Re-scan" }).click();
  await expect(page).toHaveURL(/#\/scans\/scan-/);
  const pipeline = page.getByRole("list", { name: "Pipeline" });
  await expect(pipeline).toContainText("Browser ready", { timeout: 15_000 });
  await expect(pipeline).not.toContainText("Browserbase");
  await expect(page.getByText(/Starting a Browserbase browser/)).toHaveCount(0);
  await expect(reportHealth(page)).toBeVisible({ timeout: 45_000 });
  await expect(region(page, "Overview").getByText(/Browserbase/)).toHaveCount(0);

  // A new scan from the composer does follow the default.
  await page.getByRole("link", { name: "Home", exact: true }).click();
  await page.getByRole("textbox", { name: "Page URL" }).fill("morningledger.example");
  await expect(scanButton(page)).toHaveAccessibleDescription(/^Scroll · Browserbase, Oregon \(us-west-2\) · /);
});

test("without a local Chrome, Browserbase is offered as another way to run scans", async ({ page }) => {
  await openApp(page, "/", { profile: "no-browser" });
  await expect(page.getByRole("heading", { name: "No Chrome or Chromium found" })).toBeVisible();
  await page.getByRole("button", { name: "Set up Browserbase instead" }).click();
  const browserbase = region(page, "Browserbase");
  await browserbase.getByLabel("Browserbase API key").fill("bb_live_0123456789abcdef");
  await browserbase.getByRole("button", { name: "Save Browserbase key" }).click();
  await expect(browserbase.getByText(/Connected to the project “Local”/)).toBeVisible();

  // A scan runs although this Mac has no browser.
  await page.getByRole("link", { name: "Home", exact: true }).click();
  await expect(page.getByText(/Scans can run on Browserbase\./)).toBeVisible();
  await composeOnBrowserbase(page);
  await scanButton(page).click();
  await expect(reportHealth(page)).toBeVisible({ timeout: 45_000 });
});

/** Every new or changed state, with how to get there. Each leaves the page on the state itself. */
const STATES: { name: string; arrive: (page: Page) => Promise<void> }[] = [
  {
    name: "settings before Browserbase is set up",
    arrive: async (page) => {
      await openApp(page, "/settings", { profile: "fresh" });
      await region(page, "Browserbase").getByLabel("Browserbase API key").scrollIntoViewIfNeeded();
    },
  },
  {
    name: "settings with Browserbase set up, tested and probed, and a default location",
    arrive: async (page) => {
      await openApp(page, "/settings");
      const browserbase = region(page, "Browserbase");
      await browserbase.getByRole("button", { name: "Test connection" }).click();
      await expect(browserbase.getByText(/Project “Local”/)).toBeVisible();
      await browserbase.getByRole("button", { name: "Check what a page sees" }).click();
      await expect(browserbase.getByRole("group", { name: "What a page sees" })).toBeVisible();
      await choose(page, browserbase, "Default location", "Browserbase location…");
      await browserbase.getByRole("button", { name: "Save location" }).click();
      await expect(browserbase.getByRole("alert")).toBeVisible();
    },
  },
  {
    name: "the dialog that removes the Browserbase key",
    arrive: async (page) => {
      await openApp(page, "/settings");
      await region(page, "Browserbase").getByRole("button", { name: "Remove Browserbase key" }).click();
      await expect(page.getByRole("alertdialog")).toBeVisible();
    },
  },
  {
    name: "composer on Browserbase with a location",
    arrive: async (page) => {
      await openApp(page, "/");
      await composeOnBrowserbase(page);
      const origin = region(page, "Origin");
      await choose(page, origin, "Location", "Browserbase location…");
      await origin.getByLabel("Country code").fill("be");
    },
  },
  {
    name: "composer on Browserbase with a location that lacks its country",
    arrive: async (page) => {
      await openApp(page, "/");
      await composeOnBrowserbase(page);
      await choose(page, region(page, "Origin"), "Location", "Browserbase location…");
      await scanButton(page).click();
      await expect(region(page, "Origin").getByRole("alert")).toBeVisible();
    },
  },
  {
    name: "composer on Browserbase, browsing myself",
    arrive: async (page) => {
      await openApp(page, "/");
      await composeOnBrowserbase(page);
      await region(page, "Visit").getByRole("radio", { name: "I'll browse myself" }).click();
    },
  },
  {
    name: "composer with a local address",
    arrive: async (page) => {
      await openApp(page, "/");
      await composeOnBrowserbase(page);
      await page.getByRole("textbox", { name: "Page URL" }).fill("localhost:3000");
      await expect(region(page, "Origin").getByRole("radio", { name: "Browserbase" })).toBeDisabled();
    },
  },
  {
    name: "composer without Browserbase set up",
    arrive: async (page) => {
      await openApp(page, "/", { profile: "fresh" });
      await page.getByRole("button", { name: /Continue/ }).click();
      await page.getByRole("button", { name: "Options" }).click();
      await expect(region(page, "Origin").getByRole("radio", { name: "Browserbase" })).toBeDisabled();
    },
  },
  {
    name: "composer with a configuration whose Browserbase credentials are gone",
    arrive: async (page) => {
      await openApp(page, "/settings");
      await removeBrowserbaseKey(page);
      await page.getByRole("link", { name: "Configurations", exact: true }).click();
      await region(page, "Configurations").getByRole("link", { name: SEEDED_CONFIGURATION }).click();
      await expect(region(page, "Origin").getByRole("alert")).toBeVisible();
      // The toast of the removal has nothing to do with this state.
      await expect(page.getByText("Browserbase key removed")).toHaveCount(0, { timeout: 10_000 });
    },
  },
  {
    name: "live manual session on Browserbase",
    arrive: async (page) => {
      await openApp(page, "/", { speed: 20 });
      await composeOnBrowserbase(page);
      await region(page, "Visit").getByRole("radio", { name: "I'll browse myself" }).click();
      await scanButton(page).click();
      await expect(region(page, "Manual session").getByRole("button", { name: "Open live view" })).toBeEnabled({
        timeout: 15_000,
      });
    },
  },
  {
    name: "live view waiting at Browserbase's limit",
    arrive: async (page) => {
      await openApp(page, "/", { speed: 2, browserbase: "limit" });
      await composeOnBrowserbase(page);
      await scanButton(page).click();
      await expect(region(page, "Visit").getByText(/trying again in 5 s/)).toBeVisible({ timeout: 15_000 });
    },
  },
  {
    name: "scan that failed at Browserbase's limit",
    arrive: async (page) => {
      await openApp(page, "/", { speed: 60, browserbase: "limit" });
      await composeOnBrowserbase(page);
      await scanButton(page).click();
      await expect(page.getByText("This scan failed")).toBeVisible({ timeout: 30_000 });
    },
  },
  {
    name: "report of a Browserbase scan with its browser details open",
    arrive: async (page) => {
      await openApp(page, SEEDED_REPORT);
      await page.getByRole("button", { name: "Browser details" }).click();
      await expect(page.getByText("A video and a log", { exact: true })).toBeVisible();
    },
  },
  {
    name: "configurations with one on Browserbase",
    arrive: async (page) => {
      await openApp(page, "/configurations");
      await expect(page.getByTitle("Runs in a Browserbase browser in Frankfurt (eu-central-1)")).toBeVisible();
    },
  },
  {
    name: "history with a Browserbase scan",
    arrive: async (page) => {
      await openApp(page, "/history");
      await expect(page.getByTitle("Ran in a Browserbase browser in Frankfurt (eu-central-1)")).toBeVisible();
    },
  },
  {
    name: "onboarding without a local browser",
    arrive: async (page) => {
      await openApp(page, "/", { profile: "no-browser" });
      await expect(page.getByRole("button", { name: "Set up Browserbase instead" })).toBeVisible();
    },
  },
];

for (const colorScheme of ["light", "dark"] as const) {
  test.describe(`every Browserbase state at the minimum window, ${colorScheme}`, () => {
    test.use({ viewport: { width: 1024, height: 680 }, colorScheme });

    for (const state of STATES) {
      test(`${state.name}: fits and has no axe violations`, async ({ page }) => {
        await state.arrive(page);
        await expect(page.locator("html")).toHaveClass(colorScheme === "dark" ? /dark/ : /^(?!.*dark)/);
        await expectNoHorizontalOverflow(page);
        await expectWindowDoesNotScroll(page);
        // Found by its text: behind a modal dialog the page is hidden from assistive technology.
        await expect(page.locator("button", { hasText: "New scan" })).toBeInViewport({ ratio: 1 });
        // That hiding is what axe cannot see.
        const modal = (await page.getByRole("alertdialog").count()) > 0;
        await expectNoAxeViolations(page, modal ? ["aria-hidden-focus"] : []);
      });
    }
  });
}

test("the Browserbase controls of the composer are reachable and operable by keyboard", async ({ page }) => {
  await openApp(page, "/");
  await page.getByRole("textbox", { name: "Page URL" }).fill("morningledger.example");
  await page.getByRole("button", { name: "Options" }).click();
  const origin = region(page, "Origin");
  await origin.getByRole("radio", { name: "This Mac" }).focus();
  await page.keyboard.press("ArrowRight");
  await expect(origin.getByRole("radio", { name: "Browserbase" })).toBeFocused();
  await page.keyboard.press("Space");
  await expect(origin.getByRole("radio", { name: "Browserbase" })).toBeChecked();

  await page.keyboard.press("Tab");
  await expect(origin.getByRole("combobox", { name: "Region" })).toBeFocused();
  await page.keyboard.press("Enter");
  await page.getByRole("option", { name: /Frankfurt/ }).press("Enter");
  await expect(origin.getByRole("combobox", { name: "Region" })).toHaveText(/Frankfurt/);
  await expect(origin.getByRole("combobox", { name: "Region" })).toBeFocused();

  await page.keyboard.press("Tab");
  await expect(origin.getByRole("combobox", { name: "Location" })).toBeFocused();
  await page.keyboard.press("Enter");
  await page.getByRole("option", { name: "Browserbase location…" }).press("Enter");
  // Focus is back on the control that opened the list before it moves on.
  await expect(origin.getByRole("combobox", { name: "Location" })).toBeFocused();
  await page.keyboard.press("Tab");
  await expect(origin.getByLabel("Country code")).toBeFocused();
  await page.keyboard.type("be");
  // Enter in a field of the composer scans.
  await page.keyboard.press("Enter");
  await expect(page).toHaveURL(/#\/scans\//);
  await expect(page.getByRole("list", { name: "Pipeline" })).toContainText("Browserbase, eu-central-1");
});

for (const viewport of [
  { width: 1024, height: 680 },
  { width: 1280, height: 820 },
]) {
  test.describe(`${viewport.width}×${viewport.height}`, () => {
    test.use({ viewport });

    test("switching between This Mac and Browserbase moves nothing in the composer: the region takes its empty cell, and the browser's cost is added below everything else", async ({
      page,
    }) => {
      await openApp(page, "/");
      await page.getByRole("textbox", { name: "Page URL" }).fill("morningledger.example");
      await page.getByRole("button", { name: "Options" }).click();
      const composer = page.getByRole("form", { name: "New scan" });
      const parts = [
        region(page, "Origin"),
        region(page, "Origin").getByRole("combobox", { name: "Browser profile" }),
        region(page, "Analysis"),
        region(page, "Analysis").getByRole("textbox", { name: "LLM budget" }),
      ];
      // Each top is read against the composer's in one step: the page may be scrolling as the composer grows.
      const layout = async () => ({
        tops: await Promise.all(
          parts.map((part) =>
            part.evaluate(
              (element) =>
                element.getBoundingClientRect().top - (element.closest("form")?.getBoundingClientRect().top ?? 0),
            ),
          ),
        ),
        height: (await composer.boundingBox())?.height,
      });
      const browser = (name: string) => region(page, "Origin").getByRole("radio", { name });
      const visitor = (name: string) => region(page, "Visit").getByRole("radio", { name });
      const cost = region(page, "Analysis").getByRole("note", { name: "Browser cost" });
      /** The space between two things in the Analysis section. */
      const GAP = 16;

      for (const behaviour of ["Scroll", "Browse like a visitor", "I'll browse myself"]) {
        await visitor(behaviour).click();
        await browser("This Mac").click();
        // No placeholder stands in for what only a Browserbase browser has.
        await expect(region(page, "Origin").getByText("Region")).toHaveCount(0);
        await expect(cost).toHaveCount(0);
        const onThisMac = await layout();
        await browser("Browserbase").click();
        await expect(region(page, "Origin").getByRole("combobox", { name: "Region" })).toBeVisible();
        await expect(cost).toBeVisible();
        const onBrowserbase = await layout();
        // Nothing that was there has moved.
        expect(onBrowserbase.tops, behaviour).toEqual(onThisMac.tops);
        // The composer is taller by the line about Browserbase's cost, its last, and by nothing else.
        const line = await cost.boundingBox();
        expect((onBrowserbase.height ?? 0) - (onThisMac.height ?? 0), behaviour).toBeCloseTo(
          (line?.height ?? 0) + GAP,
          0,
        );
        const composerBox = await composer.boundingBox();
        expect((line?.y ?? 0) + (line?.height ?? 0), behaviour).toBeGreaterThan(
          (composerBox?.y ?? 0) + (composerBox?.height ?? 0) - 2 * GAP - 1,
        );
      }
    });
  });
}
