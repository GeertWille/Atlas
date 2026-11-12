import { expect, type Page, test } from "@playwright/test";
import {
  expectNoAxeViolations,
  expectNoHorizontalOverflow,
  expectWindowDoesNotScroll,
  openApp,
  reportTab,
} from "./app";

const SCAN = "/scans/seed-sportsdesk-desktop";
const BROWSE = "/scans/seed-sportsdesk-browse";
const EVICTED = "/scans/seed-techpulse-desktop";
const RECORDING_OFF = "/scans/seed-morningledger-desktop";
const LEGACY = "/scans/seed-morningledger-mobile";
const MOD = process.platform === "darwin" ? "Meta" : "Control";

const code = (page: Page) => page.getByTestId("code");
const grid = (page: Page) => page.getByRole("treegrid", { name: "Requests" });
const details = (page: Page) => page.getByRole("region", { name: "Request details" });
const filter = (page: Page) => page.getByRole("combobox", { name: "Filter requests" });

type State = { name: string; route: string; ready: (page: Page) => Promise<void> };

const openBid = async (page: Page, tab: string) => {
  await grid(page)
    .getByRole("row", { name: /no-bid 5/ })
    .first()
    .click();
  await details(page).getByRole("tab", { name: tab }).click();
};

/** The panel about the open file stands beside the code in a wide window and is a drawer in a narrow one. */
const showFilePanel = async (page: Page) => {
  const panel = page.getByRole("complementary", { name: "In this file" });
  if ((page.viewportSize()?.width ?? 0) < 1200) await page.getByRole("button", { name: "Show the file panel" }).click();
  await expect(panel).toBeVisible();
};

const VIEWER: State[] = [
  {
    name: "a script with findings, its markers and the side panel",
    route: `${SCAN}/source/s7?line=208&to=215&finding=seed-sportsdesk-desktop-f5`,
    ready: async (page) => {
      await expect(code(page).locator(".cm-findingMarker").first()).toBeVisible();
      await showFilePanel(page);
    },
  },
  {
    name: "find and go to line",
    route: `${SCAN}/source/s7`,
    ready: async (page) => {
      await code(page).locator(".cm-content").click();
      await page.keyboard.press(`${MOD}+f`);
      await page.getByRole("textbox", { name: "Find" }).fill("pbjs");
      await expect(page.getByRole("search", { name: "Find in this file" }).getByRole("status")).toHaveText(/^1 of /);
      await code(page).locator(".cm-content").click();
      await page.keyboard.press("Control+g");
      await expect(page.getByRole("form", { name: "Go to line" })).toBeVisible();
    },
  },
  {
    name: "quick open",
    route: `${SCAN}/source/s7`,
    ready: async (page) => {
      await expect(code(page).locator(".cm-line").first()).toBeVisible();
      await page.keyboard.press(`${MOD}+p`);
      await expect(page.getByRole("dialog", { name: "Go to file" })).toBeVisible();
    },
  },
  {
    name: "the lines the AI did not read, dimmed",
    route: `${SCAN}/source/s7`,
    ready: async (page) => {
      await expect(code(page).locator(".cm-line").first()).toBeVisible();
      await showFilePanel(page);
      await page.getByRole("switch", { name: "Dim unread" }).click();
      await expect(code(page).locator(".cm-editor")).toHaveClass(/cm-dimUnread/);
    },
  },
  {
    name: "an original with very long lines",
    route: `${SCAN}/source/s2?view=original`,
    ready: async (page) => {
      await expect(page.getByTestId("source-format")).toContainText("highlighting off");
      await expect(code(page).locator(".cm-line").first()).toBeVisible();
    },
  },
  {
    name: "a script whose source was removed",
    route: `${EVICTED}/source/s7`,
    ready: async (page) => {
      await expect(page.getByText("This script's source was removed")).toBeVisible();
      await expect(page.getByRole("region", { name: "Evidence the report kept" })).toBeVisible();
    },
  },
  {
    name: "a script that was not kept",
    route: `${RECORDING_OFF}/source/s7`,
    ready: (page) => expect(page.getByText("This script's source was not kept")).toBeVisible(),
  },
  {
    name: "a scan from before sources were stored",
    route: `${LEGACY}/source/s7`,
    ready: (page) => expect(page.getByText("This scan has no stored source")).toBeVisible(),
  },
  {
    name: "an unknown script",
    route: `${SCAN}/source/s999`,
    ready: (page) => expect(page.getByText("This scan has no such script")).toBeVisible(),
  },
];

const NETWORK: State[] = [
  {
    name: "the digest tab of the report",
    route: `${BROWSE}?tab=network`,
    ready: async (page) => {
      await expect(reportTab(page, "Network")).toHaveAttribute("aria-selected", "true");
      await expect(page.getByRole("list", { name: "Notable requests" })).toBeVisible();
    },
  },
  {
    name: "the request list of three page views",
    route: `${BROWSE}/network`,
    ready: (page) => expect(page.getByTestId("request-count")).toHaveText("402 requests"),
  },
  {
    name: "a filtered list with a time window",
    route: `${BROWSE}/network?q=cat%3Abid+is%3Alate+nonsense%3Akey&page=0&from=600&to=2800&sort=-size`,
    ready: (page) => expect(page.getByTestId("request-count")).toHaveText(/ of \d+ requests$/),
  },
  {
    name: "the suggestions of the filter",
    route: `${BROWSE}/network`,
    ready: async (page) => {
      await expect(page.getByTestId("request-count")).toHaveText("402 requests");
      await filter(page).click();
      await page.keyboard.type("is:");
      await expect(page.getByRole("listbox")).toBeVisible();
    },
  },
  {
    name: "a bid exchange: summary with its verdict",
    route: `${BROWSE}/network?q=cat%3Abid`,
    ready: async (page) => {
      await openBid(page, "Summary");
      await expect(details(page).getByTestId("verdict")).toBeVisible();
    },
  },
  {
    name: "a bid exchange: headers with redacted values",
    route: `${BROWSE}/network?q=cat%3Abid`,
    ready: async (page) => {
      await openBid(page, "Headers");
      await expect(details(page).getByLabel("Filter headers by name")).toBeVisible();
    },
  },
  {
    name: "a bid exchange: the decoded request",
    route: `${BROWSE}/network?q=cat%3Abid`,
    ready: async (page) => {
      await openBid(page, "Payload");
      await expect(details(page).getByRole("region", { name: "Decoded bid request" })).toBeVisible();
    },
  },
  {
    name: "a bid exchange: the stored response as text",
    route: `${BROWSE}/network?q=cat%3Abid`,
    ready: async (page) => {
      await openBid(page, "Response");
      await details(page).getByRole("radio", { name: "Raw" }).click();
      await expect(details(page).getByTestId("body-editor").locator(".cm-line").first()).toBeVisible();
    },
  },
  {
    name: "a bid exchange: timing",
    route: `${BROWSE}/network?q=cat%3Abid`,
    ready: async (page) => {
      await openBid(page, "Timing");
      await expect(details(page).getByRole("tab", { name: "Timing" })).toHaveAttribute("aria-selected", "true");
    },
  },
  {
    name: "a bid exchange: initiator",
    route: `${BROWSE}/network?q=cat%3Abid`,
    ready: async (page) => {
      await openBid(page, "Initiator");
      await expect(details(page).getByRole("tab", { name: "Initiator" })).toHaveAttribute("aria-selected", "true");
    },
  },
  {
    name: "the bid exchanges of an auction",
    route: `${BROWSE}/network?view=bids`,
    ready: (page) => expect(page.getByRole("table", { name: /^Bid exchanges of auction/ })).toBeVisible(),
  },
  {
    name: "an evicted scan: the list without its bodies",
    route: `${EVICTED}/network?q=cat%3Abid`,
    ready: async (page) => {
      await grid(page)
        .getByRole("row", { name: /prebidjs/ })
        .first()
        .click();
      await details(page).getByRole("tab", { name: "Response" }).click();
      await expect(details(page)).toContainText("Not stored any more");
    },
  },
  {
    name: "a scan made with recording off",
    route: `${RECORDING_OFF}/network`,
    ready: (page) => expect(page.getByText("Network recording was off for this scan")).toBeVisible(),
  },
  {
    name: "a scan from before the network was recorded",
    route: `${LEGACY}/network`,
    ready: (page) => expect(page.getByText("No network record for this scan")).toBeVisible(),
  },
  {
    name: "the digest of a scan made with recording off",
    route: `${RECORDING_OFF}?tab=network`,
    ready: (page) => expect(page.getByText("Network recording was off for this scan")).toBeVisible(),
  },
];

for (const colorScheme of ["light", "dark"] as const) {
  test.describe(`accessibility, ${colorScheme}`, () => {
    test.use({ colorScheme });

    for (const state of [...VIEWER, ...NETWORK]) {
      test(`${state.name}: no axe violations`, async ({ page }) => {
        await openApp(page, state.route);
        await state.ready(page);
        await expectNoAxeViolations(page);
      });
    }

    test("the menus of the network view: no axe violations", async ({ page }) => {
      await openApp(page, `${BROWSE}/network`);
      await expect(page.getByTestId("request-count")).toHaveText("402 requests");
      // An open menu is modal: the page behind it is aria-hidden and unreachable, which axe cannot see.
      for (const name of [/^Lifecycle/, /^Recipes/, /^Group /, "Columns"]) {
        await page.getByRole("button", { name }).click();
        await expect(page.getByRole("menu")).toBeVisible();
        await expectNoAxeViolations(page, ["aria-hidden-focus"]);
        await page.keyboard.press("Escape");
        await expect(page.getByRole("menu")).toBeHidden();
      }
    });

    test("stored scan data in Settings, and the confirmation to clear it: no axe violations", async ({ page }) => {
      await openApp(page, "/settings");
      const stored = page.getByRole("region", { name: "Stored scan data" });
      await stored.scrollIntoViewIfNeeded();
      await expect(stored.getByRole("switch", { name: "Store full headers" })).toBeVisible();
      await expectNoAxeViolations(page);
      await stored.getByRole("button", { name: "Clear stored scan data…" }).click();
      await expect(page.getByRole("alertdialog")).toBeVisible();
      await expectNoAxeViolations(page);
    });
  });
}

test.describe("at the minimum window size", () => {
  test.use({ viewport: { width: 1024, height: 680 } });

  for (const state of [...VIEWER, ...NETWORK]) {
    test(`${state.name}: fits 1024×680`, async ({ page }) => {
      await openApp(page, state.route);
      await state.ready(page);
      await expectNoHorizontalOverflow(page);
      await expectWindowDoesNotScroll(page);
    });
  }
});
