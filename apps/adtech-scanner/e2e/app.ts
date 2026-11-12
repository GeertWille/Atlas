import { expect, type Page } from "@playwright/test";

type FakeOptions = {
  /** Starting state of the in-browser fake backend. */
  profile?: "seeded" | "fresh" | "no-browser";
  /** How many times faster than real time simulated scans run. */
  speed?: number;
};

/** Opens the app at a hash route with the fake backend configured through its query parameters. */
export async function openApp(page: Page, route = "/", { profile = "seeded", speed = 40 }: FakeOptions = {}) {
  await page.goto(`/?fakeProfile=${profile}&fakeSpeed=${speed}#${route}`);
  // The shell (and with it the global shortcuts) is mounted once the sidebar renders.
  await expect(page.getByRole("button", { name: /New scan/ })).toBeVisible();
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
