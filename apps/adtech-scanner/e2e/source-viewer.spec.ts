import { expect, type Page, test } from "@playwright/test";
import { expectNoHorizontalOverflow, expectWindowDoesNotScroll, openApp, reportTab } from "./app";

const SCAN = "/scans/seed-sportsdesk-desktop";
const WRAPPER = `${SCAN}/source/s7`;
const MOD = process.platform === "darwin" ? "Meta" : "Control";

const code = (page: Page) => page.getByTestId("code");
const caret = (page: Page) => page.getByTestId("caret-position");
const lineWith = (page: Page, text: string) => code(page).locator(".cm-line", { hasText: text });

test("an evidence chip opens the script at the cited lines, flashes them, and Back returns to the finding", async ({
  page,
}) => {
  await openApp(page, `${SCAN}?tab=findings`);
  const finding = page.getByRole("article", { name: /Desktop screens under 1440 px/ });
  await finding.getByRole("button", { name: "hb-wrapper.min.js:208" }).click();

  await expect(page).toHaveURL(/\/source\/s7\?line=208&to=215&finding=seed-sportsdesk-desktop-f5$/);
  await expect(page.getByRole("heading", { level: 1, name: "hb-wrapper.min.js" })).toBeVisible();
  const cited = lineWith(page, "var IS_MOBILE = window.screen.width < 1440;");
  await expect(cited).toBeInViewport();
  // The cited range is tinted by severity and flashes once on arrival.
  await expect(cited).toHaveClass(/cm-findingLine-critical/);
  await expect(cited).toHaveClass(/cm-flash-[ab]/);
  await expect(cited).not.toHaveClass(/cm-flash-[ab]/);
  await expect(caret(page)).toHaveText("Ln 208, Col 1");
  // The gutter marks where the finding starts, with the severity's shape as well as its colour.
  await expect(code(page).locator(".cm-findingMarker.cm-severity-critical").first()).toHaveAttribute(
    "title",
    /^Critical: Desktop screens under 1440 px/,
  );
  // The finding the link came from has its one-line summary open under the range.
  const lens = code(page).getByRole("note", { name: /Finding: Desktop screens under 1440 px/ });
  await expect(lens).toBeVisible();
  // Arriving does not rewrite the link: the range and the finding it names stay in the URL.
  await page.waitForTimeout(500);
  await expect(page).toHaveURL(/\/source\/s7\?line=208&to=215&finding=seed-sportsdesk-desktop-f5$/);
  await expect(page.getByTestId("source-format")).toHaveText("Formatted by the app (parser)");

  await page.goBack();
  await expect(page).toHaveURL(/tab=findings$/);
  await expect(finding).toBeVisible();
  await page.goForward();
  await expect(caret(page)).toHaveText("Ln 208, Col 1");

  await lens.getByRole("button", { name: "Open finding" }).click();
  await expect(page).toHaveURL(/tab=findings&finding=seed-sportsdesk-desktop-f5$/);
});

test("F8 and Shift+F8 step through the findings of a file and wrap round", async ({ page }) => {
  await openApp(page, WRAPPER);
  await expect(
    page.getByRole("complementary", { name: "In this file" }).getByRole("button", { name: /L208/ }),
  ).toBeVisible();
  // The file's facts arrive before the editor does; the key belongs to the editor.
  await expect(code(page).locator(".cm-line").first()).toBeVisible();
  await page.keyboard.press("F8");
  await expect(caret(page)).toHaveText("Ln 208, Col 1");
  await expect(page).toHaveURL(/line=208&to=215$/);
  await page.keyboard.press("F8");
  await expect(caret(page)).toHaveText("Ln 231, Col 1");
  await expect(lineWith(page, "function scheduleAuctions(units) {")).toBeInViewport();
  await page.keyboard.press("F8");
  await expect(caret(page)).toHaveText("Ln 318, Col 1");
  await page.keyboard.press("F8");
  await expect(caret(page)).toHaveText("Ln 208, Col 1");
  await page.keyboard.press("Shift+F8");
  await expect(caret(page)).toHaveText("Ln 318, Col 1");
  // Each stop opens that finding's summary; the selection is in the URL.
  await expect(code(page).getByRole("note", { name: /800 ms failsafe/ })).toBeVisible();
  await expect(page).toHaveURL(/line=318&to=332$/);

  // F8 is a media key on Mac laptops: Alt with the vertical arrows does the same.
  await page.keyboard.press("Alt+ArrowUp");
  await expect(caret(page)).toHaveText("Ln 231, Col 1");
  await page.keyboard.press("Alt+ArrowDown");
  await expect(caret(page)).toHaveText("Ln 318, Col 1");
  await page.keyboard.press("Alt+ArrowDown");
  await expect(caret(page)).toHaveText("Ln 208, Col 1");
  await expect(page).toHaveURL(/line=208&to=215$/);
  await expect(code(page).getByRole("button", { name: "Next (F8)" }).first()).toHaveAttribute(
    "title",
    "Next finding: F8 or Alt+↓. Previous: Shift+F8 or Alt+↑",
  );
  // With the caret in the find field the arrows are the field's.
  await page.keyboard.press(`${MOD}+f`);
  await expect(page.getByRole("textbox", { name: "Find" })).toBeFocused();
  await page.keyboard.press("Alt+ArrowDown");
  await page.keyboard.press("Alt+ArrowDown");
  await expect(caret(page)).toHaveText("Ln 208, Col 1");
});

test("find shows how many matches there are and which one the caret is on", async ({ page }) => {
  await openApp(page, WRAPPER);
  await code(page).locator(".cm-content").click();
  await page.keyboard.press(`${MOD}+f`);
  const find = page.getByRole("search", { name: "Find in this file" });
  await find.getByRole("textbox", { name: "Find" }).fill("requestBids");
  await expect(find.getByRole("status")).toHaveText("1 of 5");
  await page.keyboard.press("Enter");
  await expect(find.getByRole("status")).toHaveText("2 of 5");
  await page.keyboard.press("Shift+Enter");
  await expect(find.getByRole("status")).toHaveText("1 of 5");
  // The status bar says the same, and whole-word and case narrow it.
  await expect(page.getByText("Find: 1 of 5")).toBeVisible();
  await find.getByRole("button", { name: "Match case" }).click();
  await find.getByRole("textbox", { name: "Find" }).fill("REQUESTBIDS");
  await expect(find.getByRole("status")).toHaveText("No results");
  await page.keyboard.press("Escape");
  await expect(find).toBeHidden();
  await expect(page.getByText(/^Find:/)).toBeHidden();

  // Typed key by key, the query grows: jumping to the first match must not select what is typed so far.
  await code(page).locator(".cm-content").click();
  await page.keyboard.press(`${MOD}+f`);
  // The option chosen before is still on.
  await expect(find.getByRole("button", { name: "Match case" })).toHaveAttribute("aria-pressed", "true");
  await find.getByRole("textbox", { name: "Find" }).fill("");
  await page.keyboard.type("requestBids");
  await expect(find.getByRole("textbox", { name: "Find" })).toHaveValue("requestBids");
  await expect(find.getByRole("status")).toHaveText(/of 5$/);
  await page.keyboard.press("Escape");
  // Opened with a word selected (the match the caret was left on), the bar searches for that word.
  await page.keyboard.press(`${MOD}+f`);
  await expect(find.getByRole("textbox", { name: "Find" })).toHaveValue("requestBids");
  await expect(find.getByRole("status")).toHaveText(/of 5$/);
  await page.keyboard.press("Escape");

  // Find also opens when the focus is somewhere else on the screen.
  await page.getByRole("searchbox", { name: "Filter scripts" }).focus();
  await page.keyboard.press(`${MOD}+f`);
  await expect(page.getByRole("search", { name: "Find in this file" })).toBeVisible();
});

test("go to line, and a reload restores the line and the view", async ({ page }) => {
  await openApp(page, WRAPPER);
  await code(page).locator(".cm-content").click();
  await page.keyboard.press("Control+g");
  await page.getByRole("form", { name: "Go to line" }).getByRole("textbox").fill("402");
  await page.keyboard.press("Enter");
  await expect(caret(page)).toHaveText("Ln 402, Col 1");
  await expect(page).toHaveURL(/line=402$/);

  await page.reload();
  await expect(caret(page)).toHaveText("Ln 402, Col 1");
  await expect(code(page).locator(".cm-activeLineGutter", { hasText: "402" })).toBeInViewport();

  await page.getByRole("radio", { name: "Original" }).click();
  await expect(page).toHaveURL(/view=original$/);
  await expect(page.getByTestId("source-format")).toContainText("Original, as captured");
  await expect(page.getByText("Findings cite lines of the formatted view and are not marked here.")).toBeVisible();
  await page.reload();
  await expect(page.getByRole("radio", { name: "Original" })).toBeChecked();
  await expect(page.getByTestId("source-format")).toContainText("Original, as captured");
  await expect(code(page).getByRole("img", { name: /finding/ })).toHaveCount(0);
});

test("selecting lines writes them to the URL without adding history", async ({ page }) => {
  await openApp(page, `${SCAN}?tab=scripts`);
  await page.goto(`/#${WRAPPER}?line=231`);
  await expect(caret(page)).toHaveText("Ln 231, Col 1");
  await code(page).locator(".cm-content").focus();
  await page.keyboard.press("Shift+ArrowDown");
  await page.keyboard.press("Shift+ArrowDown");
  await page.keyboard.press("Shift+ArrowDown");
  await expect(page).toHaveURL(/line=231&to=233$/);
  await expect(caret(page)).toContainText("(3 lines selected)");
  await page.goBack();
  await expect(page).toHaveURL(/tab=scripts$/);
});

test("the code never traps the keyboard, and the panels have keys", async ({ page }) => {
  await openApp(page, WRAPPER);
  const content = code(page).locator(".cm-content");
  await content.click();
  await expect(content).toBeFocused();
  await expect(content).toHaveAttribute("aria-readonly", "true");
  await page.keyboard.type("x");
  await expect(lineWith(page, "var IS_MOBILE")).toHaveCount(0);
  await expect(code(page).locator(".cm-line").first()).not.toHaveText(/^x/);
  await page.keyboard.press("Tab");
  await expect(content).not.toBeFocused();

  const files = page.getByRole("navigation", { name: "Scripts" });
  await expect(files.getByRole("searchbox", { name: "Filter scripts" })).toBeVisible();
  await page.keyboard.press(`${MOD}+b`);
  await expect(files.getByRole("searchbox", { name: "Filter scripts" })).toBeHidden();
  await expect(files.getByRole("button", { name: "Show the file list" })).toBeVisible();
  await page.keyboard.press(`${MOD}+b`);
  await expect(files.getByRole("searchbox", { name: "Filter scripts" })).toBeVisible();

  const panel = page.getByRole("complementary", { name: "In this file" });
  await expect(panel).toBeVisible();
  await page.keyboard.press(`${MOD}+Alt+b`);
  await expect(panel).toBeHidden();
  // The brackets belong to the network view's page views; they do nothing here.
  await page.keyboard.press("]");
  await expect(panel).toBeHidden();
  await page.keyboard.press(`${MOD}+Alt+b`);
  await expect(panel).toBeVisible();
});

test("the file list groups, filters and opens scripts; quick-open goes to one by name", async ({ page }) => {
  await openApp(page, WRAPPER);
  const files = page.getByRole("navigation", { name: "Scripts" });
  await expect(files.getByRole("button", { name: /^Analysed 4/ })).toHaveAttribute("aria-expanded", "true");
  await expect(files.getByRole("button", { name: /^Other \d+/ })).toHaveAttribute("aria-expanded", "false");
  await expect(files.getByRole("link", { name: /hb-wrapper\.min\.js/ })).toHaveAttribute("aria-current", "page");
  await expect(
    files.getByRole("link", { name: /hb-wrapper\.min\.js/ }).getByRole("img", { name: "2 findings, worst Critical" }),
  ).toBeVisible();

  await files.getByRole("searchbox", { name: "Filter scripts" }).fill("google");
  await expect(files.getByRole("link")).toHaveCount(4);
  await files.getByRole("link", { name: /gpt\.js/ }).click();
  await expect(page).toHaveURL(/\/source\/s12$/);
  await expect(page.getByRole("heading", { level: 1, name: "gpt.js" })).toBeVisible();
  await expect(page.getByRole("complementary", { name: "In this file" })).toContainText(
    "This file was not handed to the AI, so it has no code findings.",
  );

  await page.keyboard.press(`${MOD}+p`);
  const quickOpen = page.getByRole("dialog", { name: "Go to file" });
  await quickOpen.getByRole("combobox").fill("ad-config");
  await page.keyboard.press("Enter");
  await expect(page).toHaveURL(/\/source\/s8$/);
  await expect(quickOpen).toBeHidden();
  await page.goBack();
  await expect(page).toHaveURL(/\/source\/s12$/);
});

test("the side panel lists findings, ad API calls and what the AI read, and dims the rest on request", async ({
  page,
}) => {
  await openApp(page, WRAPPER);
  const panel = page.getByRole("complementary", { name: "In this file" });
  await expect(panel.getByRole("region", { name: "Findings" }).getByRole("button")).toHaveCount(3);
  await panel.getByRole("button", { name: /800 ms failsafe.*L318/ }).click();
  await expect(caret(page)).toHaveText("Ln 318, Col 1");

  await panel.getByRole("button", { name: /^requestBids\s*L326$/ }).click();
  await expect(caret(page)).toHaveText("Ln 326, Col 1");
  await expect(lineWith(page, "pbjs.requestBids({")).toBeInViewport();

  const read = panel.getByRole("region", { name: "Read by the AI" });
  await expect(read).toContainText(/\d+% of the lines/);
  await read.getByRole("switch", { name: "Dim unread" }).click();
  await expect(code(page).locator(".cm-editor")).toHaveClass(/cm-dimUnread/);
  await expect(lineWith(page, "pbjs.requestBids({")).toHaveClass(/cm-readLine/);

  // The overview ruler shows the findings and what was read; a click on it goes there.
  const ruler = page.getByRole("img", { name: /Overview of the file: 3 findings/ });
  const box = await ruler.boundingBox();
  if (!box) throw new Error("the ruler has no box");
  await page.mouse.click(box.x + box.width / 2, box.y + box.height - 3);
  await expect(caret(page)).toHaveText(/Ln 1,29\d, Col 1|Ln 129\d, Col 1/);
});

test("a script with markup and bidirectional controls in it is shown as inert text", async ({ page }) => {
  await openApp(page, `${SCAN}/source/s10`);
  await expect(lineWith(page, "var fallbackPixel")).toContainText(`'<img src=x onerror="document.title=\\'owned\\'">'`);
  await expect(code(page).locator("img:not(.cm-widgetBuffer), a, script, iframe")).toHaveCount(0);
  await expect(page).not.toHaveTitle(/owned/);
  // The override that would reorder the line is a visible, named placeholder instead.
  const hidden = lineWith(page, "var accessLevel").locator(".cm-hiddenChar");
  await expect(hidden).toHaveText(["U+202E", "U+2066", "U+2069", "U+2066"]);
  await expect(hidden.first()).toHaveAttribute("title", "U+202E right-to-left override");
  await expect(code(page).locator(".cm-content")).toHaveCSS("direction", "ltr");
});

test("how the text was formatted is said honestly", async ({ page }) => {
  await openApp(page, `${SCAN}/source/s5`);
  await expect(page.getByTestId("source-format")).toHaveText("Formatted approximately; the file did not parse");
  await page.goto(`/#${SCAN}/source/s13`);
  await expect(page.getByTestId("source-format")).toHaveText(
    "Shown as captured; too large to format · highlighting off (very long lines)",
  );
});

test("a scan without stored sources says why, shows the evidence it kept and offers a new scan", async ({ page }) => {
  await openApp(page, "/scans/seed-techpulse-desktop/source/s7");
  await expect(page.getByText("This script's source was removed")).toBeVisible();
  const kept = page.getByRole("region", { name: "Evidence the report kept" });
  await expect(kept.getByRole("figure")).toHaveCount(1);
  await expect(kept).toContainText("L509");
  await expect(kept).toContainText("_pbjsPartner.requestBids({ timeout: 3000 });");
  await expect(page.getByRole("link", { name: "Storage settings" })).toBeVisible();

  await page.goto("/#/scans/seed-morningledger-desktop/source/s7");
  await expect(page.getByText("This script's source was not kept")).toBeVisible();
  await expect(page.getByText(/“Keep network data” was off when this scan ran/)).toBeVisible();

  await page.goto("/#/scans/seed-morningledger-mobile/source/s7");
  await expect(page.getByText("This scan has no stored source")).toBeVisible();
  await expect(page.getByText(/A new scan captures the page as it is now: its code may have changed\./)).toBeVisible();
  // This one ran without AI: there is no "without AI" variant to offer.
  await expect(page.getByRole("button", { name: "Scan again without AI (free)" })).toBeHidden();
  await page.getByRole("button", { name: "Scan again", exact: true }).click();
  await expect(page).toHaveURL(/#\/scans\/(?!seed-)[\w-]+$/);
});

test("a script larger than a reply may be says so and keeps the file's facts and its findings in reach", async ({
  page,
}) => {
  await openApp(page, WRAPPER, { sourceLimit: 300_000 });
  // A file under the limit is shown as ever.
  await expect(code(page).locator(".cm-line").first()).toBeVisible();
  await page.goto(`/#${SCAN}/source/s2`);
  await expect(page.getByRole("heading", { level: 1, name: /^main\./ })).toBeVisible();
  await expect(page.getByText("This script is too large to show")).toBeVisible();
  await expect(page.getByText(/Text over 8 MB is not loaded into the viewer\./)).toBeVisible();
  await expect(page.getByText(/This file is [\d.]+ (KB|MB) as transferred\./)).toBeVisible();
  // There is nothing a new scan would change about it, so none is offered.
  await expect(page.getByRole("button", { name: "Scan again", exact: true })).toBeHidden();
  // The original text is as large: the other view says the same instead of failing.
  await page.getByRole("radio", { name: "Original" }).click();
  await expect(page.getByText("This script is too large to show")).toBeVisible();
});

test("an unknown script and a deleted scan have their own screens", async ({ page }) => {
  await openApp(page, `${SCAN}/source/s999`);
  await expect(page.getByText("This scan has no such script")).toBeVisible();
  await page.getByRole("link", { name: "All scripts of this scan" }).click();
  await expect(reportTab(page, "Scripts")).toHaveAttribute("aria-selected", "true");
  await page.goto("/#/scans/no-such-scan/source/s1");
  await expect(page.getByText("This scan no longer exists")).toBeVisible();
});

test("the Scripts tab opens a script's source from its details and by double click", async ({ page }) => {
  await openApp(page, `${SCAN}?tab=scripts`);
  await page.getByRole("row", { name: /ad-config\.js/ }).click();
  await page.getByRole("region", { name: "Script details" }).getByRole("button", { name: "View source" }).click();
  await expect(page).toHaveURL(/\/source\/s8$/);
  await expect(code(page).locator(".cm-line").first()).toBeVisible();
  // From the viewer back to its row in the Scripts tab.
  await page.getByRole("button", { name: "More actions for this script" }).click();
  await page.getByRole("menuitem", { name: "Show in Scripts tab" }).click();
  await expect(page.getByRole("row", { name: /ad-config\.js/ })).toHaveAttribute("aria-selected", "true");
  await page.getByRole("row", { name: /gpt\.js/ }).dblclick();
  await expect(page).toHaveURL(/\/source\/s12$/);
});

test.describe("at the minimum window size", () => {
  test.use({ viewport: { width: 1024, height: 680 } });

  test("the side panel is a drawer behind a button with the finding count, and nothing overflows", async ({ page }) => {
    await openApp(page, `${WRAPPER}?line=208`);
    await expect(caret(page)).toHaveText("Ln 208, Col 1");
    const panel = page.getByRole("complementary", { name: "In this file" });
    await expect(panel).toBeHidden();
    await expect(page.getByRole("navigation", { name: "Scripts" }).getByRole("searchbox")).toBeVisible();
    await expectNoHorizontalOverflow(page);
    await expectWindowDoesNotScroll(page);
    const codeWidth = (await code(page).boundingBox())?.width ?? 0;
    expect(codeWidth).toBeGreaterThanOrEqual(520);

    const open = page.getByRole("button", { name: "Show the file panel" });
    await expect(open).toHaveText("3");
    await open.click();
    await expect(panel).toBeVisible();
    // It floats over the code instead of squeezing it.
    expect((await code(page).boundingBox())?.width).toBe(codeWidth);
    await panel.getByRole("button", { name: "Hide the file panel" }).click();
    await expect(panel).toBeHidden();
    await expect(page.getByTestId("source-format")).toBeInViewport();
    await expect(page.getByRole("img", { name: /Overview of the file/ })).toBeInViewport();
  });
});

test.describe("performance", () => {
  /** Waits in the page for `ready`, and returns how long that took from the moment `start` ran. */
  async function timeInPage(page: Page, start: string, ready: string): Promise<number> {
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

  test("a 20,000-line file is interactive within 500 ms of its data, and a jump takes under 100 ms", async ({
    page,
  }) => {
    const large = `${SCAN}/source/s2`;
    await openApp(page, large);
    await expect(page.getByText("20,000 lines")).toBeVisible();
    // The text is now in the query cache: opening the file again measures the editor, not the backend.
    await page.goto(`/#${WRAPPER}`);
    await expect(page.getByText("1,299 lines")).toBeVisible();
    const opened = await timeInPage(
      page,
      `location.hash = ${JSON.stringify(`#${large}`)}`,
      `document.querySelectorAll('[data-testid="code"] .cm-line').length > 20 && document.querySelector('[data-testid="source-format"]')?.textContent?.includes('parser') === true && document.body.textContent.includes('20,000 lines')`,
    );
    console.log(`20,000-line file interactive after ${opened.toFixed(0)} ms`);
    expect(opened).toBeLessThan(500);

    await code(page).locator(".cm-content").click();
    await page.keyboard.press("Control+g");
    await page.getByRole("form", { name: "Go to line" }).getByRole("textbox").fill("15000");
    const jumped = await timeInPage(
      page,
      `document.querySelector('form[aria-label="Go to line"]').requestSubmit()`,
      `[...document.querySelectorAll('[data-testid="code"] .cm-lineNumbers .cm-gutterElement')].some((element) => element.textContent === '15000')`,
    );
    console.log(`jump across 15,000 lines took ${jumped.toFixed(0)} ms`);
    expect(jumped).toBeLessThan(100);
    await expect(caret(page)).toHaveText("Ln 15,000, Col 1");
  });

  test("the original of that file, one line of 2 MB, opens without freezing the window", async ({ page }) => {
    await openApp(page, `${SCAN}/source/s2`);
    await expect(page.getByText("20,000 lines")).toBeVisible();
    const began = Date.now();
    await page.getByRole("radio", { name: "Original" }).click();
    await expect(page.getByTestId("source-format")).toHaveText(
      "Original, as captured · highlighting off (very long lines)",
    );
    await expect(code(page).locator(".cm-line").first()).toBeVisible();
    console.log(`2 MB single-line original shown after ${Date.now() - began} ms`);
    expect(Date.now() - began).toBeLessThan(3000);
    // The window still answers: a frame comes round quickly, scrolling works, find works.
    const frame = await page.evaluate(
      () =>
        new Promise<number>((resolve) => {
          const start = performance.now();
          requestAnimationFrame(() => requestAnimationFrame(() => resolve(performance.now() - start)));
        }),
    );
    expect(frame).toBeLessThan(200);
    await code(page)
      .locator(".cm-scroller")
      .evaluate((scroller) => scroller.scrollBy(0, 4000));
    await code(page).locator(".cm-content").click();
    await page.keyboard.press(`${MOD}+f`);
    await page.getByRole("textbox", { name: "Find" }).fill("function");
    await expect(page.getByRole("search", { name: "Find in this file" }).getByRole("status")).toHaveText(
      /of [\d,]+\+?$/,
    );
  });
});
