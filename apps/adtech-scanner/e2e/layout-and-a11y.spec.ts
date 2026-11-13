import { expect, type Page, test } from "@playwright/test";
import { expectNoAxeViolations, expectNoHorizontalOverflow, expectWindowDoesNotScroll, MOD, openApp } from "./app";

const REPORT = "/scans/seed-sportsdesk-desktop";
const MULTI_PAGE_REPORT = "/scans/seed-sportsdesk-browse";

const SCREENS: { name: string; route: string; ready: (page: Page) => Promise<void> }[] = [
  {
    name: "home",
    route: "/",
    ready: (page) => expect(page.getByRole("region", { name: "Recent scans" }).getByRole("link").first()).toBeVisible(),
  },
  { name: "report findings", route: REPORT, ready: (page) => expect(page.getByRole("article").first()).toBeVisible() },
  {
    name: "report auctions",
    route: `${REPORT}?tab=auctions`,
    ready: (page) => expect(page.getByRole("region", { name: "Auction 1" })).toBeVisible(),
  },
  {
    name: "report slots",
    route: `${REPORT}?tab=slots`,
    ready: (page) => expect(page.getByText("Never requested").first()).toBeVisible(),
  },
  {
    name: "report scripts",
    route: `${REPORT}?tab=scripts`,
    ready: (page) => expect(page.getByRole("row", { name: /gpt\.js/ })).toBeVisible(),
  },
  {
    name: "report audit trail",
    route: `${REPORT}?tab=audit`,
    ready: (page) => expect(page.getByRole("region", { name: "Log entries" }).getByRole("time").first()).toBeVisible(),
  },
  {
    name: "failed scan",
    route: "/scans/seed-unreachable",
    ready: (page) => expect(page.getByText("This scan failed")).toBeVisible(),
  },
  {
    name: "history",
    route: "/history",
    ready: (page) => expect(page.getByRole("region", { name: "Scans" }).locator("tbody tr").first()).toBeVisible(),
  },
  {
    name: "log search",
    route: "/logs?q=timeout",
    ready: (page) => expect(page.getByRole("region", { name: /Matches in/ }).first()).toBeVisible(),
  },
  { name: "settings", route: "/settings", ready: (page) => expect(page.getByLabel("Budget per scan")).toBeVisible() },
  {
    name: "configurations",
    route: "/configurations",
    ready: (page) =>
      expect(page.getByRole("region", { name: "Configurations" }).getByText("Never run").first()).toBeVisible(),
  },
  {
    name: "composer with a configuration that needs fixing",
    route: "/?configuration=config-sportsdesk-germany",
    ready: (page) => expect(page.getByText("This proxy was deleted. Choose another one.")).toBeVisible(),
  },
  {
    name: "multi-page report findings",
    route: MULTI_PAGE_REPORT,
    ready: (page) => expect(page.getByRole("radiogroup", { name: "Page view" })).toBeVisible(),
  },
  {
    name: "multi-page report auctions",
    route: `${MULTI_PAGE_REPORT}?tab=auctions`,
    ready: (page) => expect(page.getByRole("region", { name: "Auctions on page 3" })).toBeVisible(),
  },
  {
    name: "multi-page report slots",
    route: `${MULTI_PAGE_REPORT}?tab=slots`,
    ready: (page) => expect(page.getByRole("region", { name: "Ad slots on page 2" })).toBeVisible(),
  },
  {
    name: "multi-page report, one page, scripts",
    route: `${MULTI_PAGE_REPORT}?tab=scripts&page=1`,
    ready: (page) => expect(page.getByRole("columnheader", { name: "Pages" })).toBeVisible(),
  },
  {
    name: "report with the automated-traffic finding",
    route: "/scans/seed-techpulse-desktop",
    ready: (page) => expect(page.getByRole("note", { name: "Scan environment" })).toBeVisible(),
  },
];

const VIEWPORTS = [
  { label: "minimum window", width: 1024, height: 680 },
  { label: "large display", width: 1920, height: 1200 },
];

for (const viewport of VIEWPORTS) {
  for (const colorScheme of ["light", "dark"] as const) {
    test.describe(`${viewport.label} (${viewport.width}×${viewport.height}), ${colorScheme}`, () => {
      test.use({ viewport: { width: viewport.width, height: viewport.height }, colorScheme });

      test("every screen fits without horizontal scrolling and keeps its primary controls in view", async ({
        page,
      }) => {
        for (const screen of SCREENS) {
          await test.step(screen.name, async () => {
            await openApp(page, screen.route);
            await screen.ready(page);
            await expect(page.locator("html")).toHaveClass(colorScheme === "dark" ? /dark/ : /^(?!.*dark)/);
            await expectNoHorizontalOverflow(page);
            await expectWindowDoesNotScroll(page);
            await expect(page.getByRole("button", { name: /New scan/ })).toBeInViewport({ ratio: 1 });
            await expect(page.getByRole("link", { name: "Settings" })).toBeInViewport({ ratio: 1 });
            await expect(page.getByRole("heading", { level: 1 })).toBeInViewport({ ratio: 1 });
          });
        }
      });

      test("the composer and report actions stay fully visible", async ({ page }) => {
        await openApp(page, "/");
        await expect(page.getByRole("button", { name: "Scan", exact: true })).toBeInViewport({ ratio: 1 });
        await expect(page.getByRole("radiogroup", { name: "Device" })).toBeInViewport({ ratio: 1 });

        await openApp(page, REPORT);
        for (const name of ["Re-scan", "Scan on mobile", "Export"]) {
          await expect(page.getByRole("button", { name })).toBeInViewport({ ratio: 1 });
        }
        for (const tab of ["Findings", "Auctions", "Ad slots", "Scripts", "Audit trail"]) {
          await expect(page.getByRole("tab", { name: new RegExp(`^${tab}`) })).toBeInViewport({ ratio: 1 });
        }
      });

      test("revealing a control far down the composer scrolls the page area, never the whole window", async ({
        page,
      }) => {
        await openApp(page, "/");
        await page.getByRole("button", { name: "Options" }).click();
        await page.getByRole("radio", { name: "Browse like a visitor" }).click();
        await expect(page.getByRole("textbox", { name: "LLM budget" })).toBeAttached();
        await expectWindowDoesNotScroll(page);
        await page.getByRole("region", { name: "Recent scans" }).evaluate((element) => element.scrollIntoView());
        await page.getByRole("form", { name: "New scan" }).evaluate((element) => element.scrollIntoView());
        expect(await page.evaluate(() => document.documentElement.scrollTop)).toBe(0);
        await expect(page.getByRole("button", { name: /New scan/ })).toBeInViewport({ ratio: 1 });
        await expect(page.getByRole("link", { name: "Settings" })).toBeInViewport({ ratio: 1 });
      });

      test("the live view fits the window with its own scroll regions", async ({ page }) => {
        await openApp(page, "/", { speed: 4 });
        await page.getByRole("textbox", { name: "Page URL" }).fill("sportsdesk.example");
        await page.keyboard.press("Enter");
        await expect(page.getByRole("log", { name: "Activity" }).getByRole("time").nth(12)).toBeVisible({
          timeout: 20_000,
        });
        await expectNoHorizontalOverflow(page);
        await expect(page.getByRole("button", { name: "Cancel scan" })).toBeInViewport({ ratio: 1 });
        await expect(page.getByRole("region", { name: "Findings" })).toBeInViewport({ ratio: 1 });
        // The page itself does not scroll; the feed does.
        expect(
          await page.evaluate(
            () => document.querySelector("main")?.scrollHeight === document.querySelector("main")?.clientHeight,
          ),
        ).toBe(true);
      });
    });
  }
}

for (const colorScheme of ["light", "dark"] as const) {
  test.describe(`accessibility, ${colorScheme}`, () => {
    test.use({ colorScheme });

    for (const screen of SCREENS) {
      test(`${screen.name} has no axe violations`, async ({ page }) => {
        await openApp(page, screen.route);
        await screen.ready(page);
        await expectNoAxeViolations(page);
      });
    }

    test("onboarding has no axe violations", async ({ page }) => {
      await openApp(page, "/", { profile: "fresh" });
      await expect(page.getByRole("heading", { name: "Browser detected" })).toBeVisible();
      await expectNoAxeViolations(page);
    });

    test("the live view has no axe violations", async ({ page }) => {
      await openApp(page, "/", { speed: 3 });
      await page.getByRole("textbox", { name: "Page URL" }).fill("sportsdesk.example");
      await page.keyboard.press("Enter");
      await expect(page.getByRole("region", { name: "Findings" }).getByRole("listitem").first()).toBeVisible({
        timeout: 20_000,
      });
      await expectNoAxeViolations(page);
    });

    test("overlays have no axe violations: palette, menus, dialogs, expanded finding", async ({ page }) => {
      await openApp(page, REPORT);
      await page.keyboard.press(`${MOD}+k`);
      await expect(page.getByRole("dialog", { name: "Command palette" })).toBeVisible();
      await expectNoAxeViolations(page);
      await page.keyboard.press("Escape");

      // An open menu is modal: the page behind it is aria-hidden and its controls are unreachable,
      // which axe cannot see. Prove the focus trap, then audit without that one rule.
      await page.getByRole("button", { name: "Filter by severity" }).click();
      const menu = page.getByRole("menu");
      await expect(menu).toBeVisible();
      for (let press = 0; press < 6; press += 1) {
        await page.keyboard.press("Tab");
        expect(await menu.evaluate((element) => element.contains(document.activeElement))).toBe(true);
      }
      await expectNoAxeViolations(page, ["aria-hidden-focus"]);
      await page.keyboard.press("Escape");

      await openApp(page, "/history");
      await page.getByRole("button", { name: "Delete all…" }).click();
      await expect(page.getByRole("alertdialog")).toBeVisible();
      await expectNoAxeViolations(page);
    });
  });
}

test("focus is visible and follows a sensible order from the top of the page", async ({ page }) => {
  await openApp(page, "/");
  await expect(page.getByRole("region", { name: "Recent scans" })).toBeVisible();
  const focused = async () =>
    page.evaluate(() => {
      const element = document.activeElement as HTMLElement;
      const style = getComputedStyle(element);
      return {
        name: element.getAttribute("aria-label") ?? element.textContent?.trim().slice(0, 24),
        outline: style.outlineStyle !== "none" || style.boxShadow !== "none",
      };
    });

  const order: string[] = [];
  for (let index = 0; index < 7; index += 1) {
    await page.keyboard.press("Tab");
    const current = await focused();
    expect(current.outline, `focus ring on ${current.name}`).toBe(true);
    order.push(current.name ?? "");
  }
  expect(order).toEqual([
    expect.stringMatching(/^New scan/),
    expect.stringMatching(/^Search/),
    "Home",
    "Configurations",
    "History",
    "Log search",
    expect.stringMatching(/sportsdesk/),
  ]);
});
