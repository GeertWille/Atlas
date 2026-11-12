import { expect, type Page, test } from "@playwright/test";
import { PRODUCTION_PORT } from "../playwright.config";
import { openApp } from "./app";

const SCAN = "/scans/seed-sportsdesk-browse";
const OTHER_SCAN = "/scans/seed-sportsdesk-desktop";

/**
 * Milliseconds per action, from the network plan. They are asserted on Chromium against the production build
 * and time the interface alone: the fake backend has built its request lists before the clock starts and
 * answers without its simulated delay. The spec is a project of its own that runs after every other spec has
 * finished (see playwright.config.ts), so nothing else is using the processor.
 */
type Budget = {
  firstRows: number;
  allRows: number;
  keystroke: number;
  sort: number;
  select: number;
  headers: number;
  decoded: number;
  body: number;
};

const SIZES: { rows: number; label: string; budget: Budget }[] = [
  {
    rows: 3000,
    label: "3,000",
    budget: {
      firstRows: 250,
      allRows: 600,
      keystroke: 50,
      sort: 60,
      select: 50,
      headers: 150,
      decoded: 150,
      body: 300,
    },
  },
  {
    rows: 10_000,
    label: "10,000",
    budget: {
      firstRows: 250,
      allRows: 1500,
      keystroke: 100,
      sort: 150,
      select: 50,
      headers: 150,
      decoded: 150,
      body: 300,
    },
  },
];

/** Runs `start` in the page and returns the milliseconds until `ready` holds, checked once per frame. */
function timeInPage(page: Page, start: string, ready: string) {
  return page.evaluate(
    async ([startSource, readySource]) => {
      const isReady = new Function(`return (${readySource})`) as () => boolean;
      const began = performance.now();
      new Function(startSource as string)();
      while (!isReady()) await new Promise((resolve) => requestAnimationFrame(resolve));
      return performance.now() - began;
    },
    [start, ready],
  );
}

const COUNT = `document.querySelector('[data-testid="request-count"]')?.textContent`;
const DETAILS = `document.querySelector('section[aria-label="Request details"]')`;
const dataRows = `document.querySelectorAll('[role="treegrid"] [role="row"][aria-rowindex]:not([aria-rowindex="1"])')`;

/** Each step is typed over the one before it and changes how many requests match. */
const KEYSTROKES = [
  "p",
  "pr",
  "pre",
  "preb",
  "prebi",
  "prebid",
  "prebi",
  "preb",
  "pre",
  "pr",
  "p",
  "",
  "c",
  "ca",
  "cat",
  "cat:",
  "cat:b",
  "cat:bi",
  "cat:bid",
  "cat:bid ",
  "cat:bid i",
  "cat:bid is",
  "cat:bid is:",
  "cat:bid is:l",
  "cat:bid is:la",
  "cat:bid is:lat",
  "cat:bid is:late",
];

test.describe("performance", () => {
  test.describe.configure({ mode: "serial" });
  test.use({ baseURL: `http://localhost:${PRODUCTION_PORT}` });

  for (const { rows, label, budget } of SIZES) {
    test(`${label} requests: opening, filtering, sorting and the details stay inside their budgets`, async ({
      page,
    }) => {
      // Another scan's network view first: the code of the route is loaded, the rows of this scan are not.
      await openApp(page, `${OTHER_SCAN}/network`, { rows, latency: "off" });
      await expect(page.getByTestId("request-count")).toHaveText(`${label} requests`);
      const measured: Record<string, number> = {};

      const opened = await page.evaluate(
        async ([hash, rowsSource, countSource, total]) => {
          const mounted = new Function(`return (${rowsSource}).length`) as () => number;
          const count = new Function(`return (${countSource})`) as () => string | undefined;
          const frame = () => new Promise((resolve) => requestAnimationFrame(resolve));
          const began = performance.now();
          location.hash = hash as string;
          // The other scan's rows leave with its route.
          while (mounted() > 0) await frame();
          while (mounted() === 0) await frame();
          const firstRows = performance.now() - began;
          while (count() !== `${total} requests`) await frame();
          return { firstRows, allRows: performance.now() - began };
        },
        [`#${SCAN}/network`, dataRows, COUNT, label],
      );
      measured.firstRows = opened.firstRows;
      measured.allRows = opened.allRows;

      // However long the list, the grid mounts only what the window shows and a little either side.
      const mounted = await page.evaluate(`(${dataRows}).length`);
      expect(mounted).toBeGreaterThan(10);
      expect(mounted).toBeLessThanOrEqual(45);

      const keystrokes = await page.evaluate(
        async ([steps, countSource]) => {
          const count = new Function(`return (${countSource})`) as () => string | undefined;
          const input = document.querySelector<HTMLInputElement>('input[aria-label="Filter requests"]');
          if (!input) throw new Error("no filter field");
          const setValue = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
          const frame = () => new Promise((resolve) => requestAnimationFrame(resolve));
          const times: number[] = [];
          for (const step of steps as string[]) {
            const before = count();
            const began = performance.now();
            setValue?.call(input, step);
            input.dispatchEvent(new Event("input", { bubbles: true }));
            // The repaint is the frame after the count has changed; a step that leaves it alone is not a sample.
            let frames = 0;
            while (count() === before && frames < 30) {
              await frame();
              frames += 1;
            }
            if (count() === before) continue;
            await frame();
            times.push(performance.now() - began);
          }
          return times;
        },
        [KEYSTROKES, COUNT] as const,
      );
      expect(keystrokes.length).toBeGreaterThanOrEqual(12);
      const ordered = [...keystrokes].sort((a, b) => a - b);
      measured.keystroke = ordered[Math.min(ordered.length - 1, Math.ceil(ordered.length * 0.95) - 1)] ?? 0;
      await expect(page.getByTestId("request-count")).toHaveText(new RegExp(`of ${label} requests$`));
      await page.getByRole("combobox", { name: "Filter requests" }).fill("");
      await expect(page.getByTestId("request-count")).toHaveText(`${label} requests`);

      measured.sort = await timeInPage(
        page,
        `document.querySelector('[role="columnheader"][data-column="size"] button').click()`,
        `document.querySelector('[role="columnheader"][data-column="size"]').getAttribute('aria-sort') !== 'none'`,
      );
      await page.locator('[role="columnheader"][data-column="size"] button').click();

      // The facts of a row come from the list itself: nothing is asked of the backend to show them.
      measured.select = await timeInPage(
        page,
        `(${dataRows})[4].click()`,
        `${DETAILS}?.querySelector('h2')?.textContent?.length > 0`,
      );
      await expect(
        page.getByRole("region", { name: "Request details" }).getByRole("heading", { level: 2 }),
      ).toBeVisible();

      measured.headers = await timeInPage(
        page,
        `[...${DETAILS}.querySelectorAll('[role="tab"]')].find((tab) => tab.textContent.startsWith('Headers')).dispatchEvent(new MouseEvent('mousedown', { bubbles: true, button: 0 }))`,
        `${DETAILS}.querySelector('input[aria-label="Filter headers by name"]') !== null`,
      );

      // A stored body in the editor, whose code an earlier request has already loaded.
      const details = page.getByRole("region", { name: "Request details" });
      const okRows = page.getByRole("treegrid", { name: "Requests" }).getByRole("row", { name: /^200 / });
      const openResponse = `[...${DETAILS}.querySelectorAll('[role="tab"]')].find((tab) => tab.textContent.startsWith('Response')).dispatchEvent(new MouseEvent('mousedown', { bubbles: true, button: 0 }))`;
      await page.getByRole("combobox", { name: "Filter requests" }).fill("type:script");
      await okRows.first().click();
      await details.getByRole("tab", { name: "Response" }).click();
      await expect(details.getByTestId("body-editor").locator(".cm-line").first()).toBeVisible();
      await details.getByRole("tab", { name: "Summary" }).click();
      await okRows.nth(1).click();
      await expect(details.getByRole("tab", { name: "Summary" })).toHaveAttribute("aria-selected", "true");
      measured.body = await timeInPage(
        page,
        openResponse,
        `${DETAILS}.querySelector('[data-testid="body-editor"] .cm-line') !== null`,
      );

      await details.getByRole("tab", { name: "Summary" }).click();
      await page.getByRole("combobox", { name: "Filter requests" }).fill("bidder:sovrn method:POST");
      await okRows.first().click();
      await expect(details.getByRole("heading", { level: 2 })).toContainText("ap.lijit.com");
      measured.decoded = await timeInPage(
        page,
        openResponse,
        `${DETAILS}.querySelector('[aria-label="Decoded bid response"]') !== null`,
      );

      console.log(
        `${label} rows: ${Object.entries(measured)
          .map(([name, ms]) => `${name} ${ms.toFixed(0)} ms (budget ${budget[name as keyof Budget]})`)
          .join(", ")}; ${mounted} rows mounted; ${keystrokes.length} keystrokes sampled`,
      );
      for (const [name, ms] of Object.entries(measured)) {
        expect(ms, `${name} at ${label} rows`).toBeLessThan(budget[name as keyof Budget]);
      }
    });
  }
});
