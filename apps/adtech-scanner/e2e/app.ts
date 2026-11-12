import AxeBuilder from "@axe-core/playwright";
import { expect, type Page } from "@playwright/test";

type FakeOptions = {
  /** Starting state of the in-browser fake backend. */
  profile?: "seeded" | "fresh" | "no-browser";
  /** How many times faster than real time simulated scans run. */
  speed?: number;
  /** `fail`: Jev System One stops answering part-way through script triage. */
  jev?: "ok" | "fail";
  /** Every scan's request list is padded to this many rows. */
  rows?: number;
};

/** Opens the app at a hash route with the fake backend configured through its query parameters. */
export async function openApp(
  page: Page,
  route = "/",
  { profile = "seeded", speed = 40, jev = "ok", rows = 0 }: FakeOptions = {},
) {
  await page.goto(`/?fakeProfile=${profile}&fakeSpeed=${speed}&fakeJev=${jev}&fakeRows=${rows}#${route}`);
  // The shell (and with it the global shortcuts) is mounted once the sidebar renders.
  await expect(page.getByRole("button", { name: /New scan/ })).toBeVisible();
}

/** On the settings screen: stores a TypeSafe key and turns on "Classify scripts with Jev". */
export async function enableJev(page: Page) {
  await page.getByLabel("TypeSafe API key").fill("typesafe-e2e-key");
  await page.getByRole("button", { name: "Save TypeSafe key" }).click();
  const jev = page.getByRole("switch", { name: "Classify scripts with Jev" });
  await expect(jev).toBeEnabled();
  await jev.click();
  await expect(jev).toBeChecked();
  // The switch flips optimistically; the toast confirms the backend has it.
  await expect(page.getByText("Settings saved")).toBeVisible();
}

export async function startScan(page: Page, url: string) {
  await page.getByRole("textbox", { name: "Page URL" }).fill(url);
  await page.getByRole("button", { name: "Scan", exact: true }).click();
}

export function reportTab(page: Page, name: string) {
  return page.getByRole("tab", { name: new RegExp(`^${name}`) });
}

/** Nothing on the page forces a horizontal scrollbar. */
export async function expectNoHorizontalOverflow(page: Page) {
  const overflow = await page.evaluate(() => {
    const main = document.querySelector("main");
    return {
      document: document.documentElement.scrollWidth - window.innerWidth,
      main: main ? main.scrollWidth - main.clientWidth : 0,
    };
  });
  expect(overflow).toEqual({ document: 0, main: 0 });
}

/** The health ring in the report header (sidebar links and history rows carry small rings too). */
export function reportHealth(page: Page, name: string | RegExp = /Health score \d+ of 100/) {
  return page.getByRole("main").locator("header").getByRole("img", { name });
}

/** The page as it is now passes axe's WCAG 2.1 A and AA rules. */
/**
 * The window itself has nothing to scroll: only the regions inside it do. A document taller than the
 * window can be shifted by anything that scrolls an element into view, leaving a blank band below the app.
 */
export async function expectWindowDoesNotScroll(page: Page) {
  const overflow = await page.evaluate(() => document.documentElement.scrollHeight - window.innerHeight);
  expect(overflow, "document height beyond the window").toBeLessThanOrEqual(0);
}

export async function expectNoAxeViolations(page: Page, disabledRules: string[] = []) {
  const results = await new AxeBuilder({ page })
    .withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa"])
    .disableRules(disabledRules)
    .analyze();
  expect(
    results.violations.map(
      (violation) => `${violation.id}: ${violation.nodes.map((node) => node.target.join(" ")).join(" | ")}`,
    ),
  ).toEqual([]);
}
