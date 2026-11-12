import { expect, type Page, test } from "@playwright/test";
import { expectNoHorizontalOverflow, expectWindowDoesNotScroll, openApp, reportTab } from "./app";

const SCAN = "/scans/seed-sportsdesk-desktop";
const BROWSE = "/scans/seed-sportsdesk-browse";
const MOD = process.platform === "darwin" ? "Meta" : "Control";

const grid = (page: Page) => page.getByRole("treegrid", { name: "Requests" });
const count = (page: Page) => page.getByTestId("request-count");
const details = (page: Page) => page.getByRole("region", { name: "Request details" });
const filter = (page: Page) => page.getByRole("combobox", { name: "Filter requests" });

test("the report's Network tab is a digest whose every element opens the network view filtered to it", async ({
  page,
}) => {
  await openApp(page, `${BROWSE}?tab=network`);
  const tab = page.getByRole("tabpanel", { name: /Network/ });
  await expect(reportTab(page, "Network")).toContainText("402");
  await expect(tab.getByRole("link", { name: /^Requests: 402/ })).toBeVisible();
  await expect(tab.getByRole("link", { name: /^Late bids: 1/ })).toBeVisible();
  await expect(tab.getByRole("list", { name: "Requests by category" }).getByRole("link")).toHaveCount(9);
  const notable = tab.getByRole("list", { name: "Notable requests" }).getByRole("listitem");
  expect(await notable.count()).toBeGreaterThan(3);
  expect(await notable.count()).toBeLessThanOrEqual(8);

  await tab.getByRole("link", { name: /^Late bids: 1/ }).click();
  await expect(page).toHaveURL(/\/network\?q=cat%3Abid\+is%3Alate$/);
  await expect(count(page)).toHaveText("1 of 402 requests");
  await expect(grid(page).getByRole("row", { name: /late \+84 ms/ })).toBeVisible();
  await page.goBack();
  await expect(page).toHaveURL(/tab=network$/);

  await tab
    .getByRole("list", { name: "Requests by category" })
    .getByRole("link", { name: /Ad server/ })
    .click();
  await expect(filter(page)).toHaveValue("cat:adserver");
  await expect(page.getByRole("button", { name: /^Ad server \d+/ })).toHaveAttribute("aria-pressed", "true");
  await page.goBack();

  // A notable request opens with its details; a page block opens that page view.
  await notable.filter({ hasText: "late +84 ms" }).getByRole("link").click();
  await expect(details(page).getByTestId("verdict")).toHaveText("Response arrived after the auction closed (+84 ms)");
  await page.goBack();
  await tab.getByRole("link", { name: /^Page 2: 136 requests/ }).click();
  await expect(page).toHaveURL(/\/network\?page=1$/);
  await expect(count(page)).toHaveText("136 requests");
  await page.goBack();

  // The page selection of the report travels along.
  await page.getByRole("radiogroup", { name: "Page view" }).getByRole("radio", { name: /^3/ }).click();
  await tab.getByRole("link", { name: "Open the network view" }).click();
  await expect(page).toHaveURL(/\/network\?page=2$/);
  await expect(page.getByRole("radiogroup", { name: "Page view" }).getByRole("radio", { name: /^3/ })).toBeChecked();
  // And back again through the breadcrumb.
  await page.getByRole("link", { name: "www.sportsdesk.example" }).click();
  await expect(page).toHaveURL(/tab=network&page=2$/);
});

test("filtering: the query language, chips with counts, lifecycle and recipes all write one filter in the URL", async ({
  page,
}) => {
  await openApp(page, `${SCAN}/network`);
  await expect(count(page)).toHaveText("191 requests");
  const category = page.getByRole("group", { name: "Category" });
  await expect(category.getByRole("button", { name: /^All 191/ })).toHaveAttribute("aria-pressed", "true");

  await category.getByRole("button", { name: /^Bids 8/ }).click();
  await expect(filter(page)).toHaveValue("cat:bid");
  await expect(count(page)).toHaveText("8 of 191 requests");
  // Category counts ignore the category itself, so they still say what another chip would show.
  await expect(category.getByRole("button", { name: /^All 191/ })).toBeVisible();
  const states = page.getByRole("group", { name: "State" });
  await expect(states.getByRole("button", { name: "Slow 1" })).toBeVisible();
  await states.getByRole("button", { name: "Slow 1" }).click();
  await expect(filter(page)).toHaveValue("cat:bid is:slow");
  await expect(count(page)).toHaveText("1 of 191 requests");
  await expect(page).toHaveURL(/q=cat%3Abid\+is%3Aslow$/);
  await states.getByRole("button", { name: "Slow 1" }).click();
  await expect(filter(page)).toHaveValue("cat:bid");

  // Typing: completion of keys and values, negation, and an honest note for what is not a filter.
  await filter(page).fill("cat:bid -bidder:");
  await expect(page.getByRole("option", { name: "appnexus" })).toBeVisible();
  await page.getByRole("option", { name: "ix" }).click();
  await expect(filter(page)).toHaveValue("cat:bid -bidder:ix");
  await expect(count(page)).toHaveText("7 of 191 requests");
  await filter(page).fill("foo:bar");
  await expect(page.getByText("Unknown filter “foo:”; searched as text")).toBeVisible();
  await expect(page.getByText("No request matches")).toBeVisible();
  await page.getByRole("button", { name: "Clear the filter" }).first().click();
  await expect(count(page)).toHaveText("191 requests");

  await page.getByRole("button", { name: "Lifecycle" }).click();
  await page.getByRole("menuitemcheckbox", { name: "Before the first auction" }).click();
  await page.keyboard.press("Escape");
  await expect(filter(page)).toHaveValue("is:before-auction");
  await expect(count(page)).toHaveText(/^\d+ of 191 requests$/);

  await page.getByRole("button", { name: "Recipes" }).click();
  await page.getByRole("menuitem", { name: /Ad requests without Prebid keys/ }).click();
  await expect(filter(page)).toHaveValue("cat:adserver is:no-hb-targeting");
  await expect(count(page)).toHaveText("1 of 191 requests");
  await expect(grid(page).getByRole("row", { name: /no hb keys/ })).toBeVisible();
  // The filter is remembered as a recent one.
  await page.getByRole("button", { name: "Recipes" }).click();
  await expect(page.getByRole("menuitem", { name: "cat:adserver is:no-hb-targeting", exact: true })).toBeVisible();
});

test("on a page view that never auctioned, everything is before the first auction", async ({ page }) => {
  await openApp(page, `${BROWSE}/network?page=1&q=is%3Abefore-auction`);
  await expect(count(page)).toHaveText("136 requests");
  await page.getByRole("button", { name: "Lifecycle" }).click();
  await expect(page.getByText(/No auction ran here/)).toBeVisible();
});

test("a reload restores filter, grouping, sorting, selection, tab and window", async ({ page }) => {
  await openApp(page, `${SCAN}/network`);
  await filter(page).fill("cat:bid");
  await expect(count(page)).toHaveText("8 of 191 requests");
  await grid(page).getByRole("columnheader", { name: "Time" }).getByRole("button").click();
  await grid(page).getByRole("columnheader", { name: "Time" }).getByRole("button").click();
  await expect(grid(page).getByRole("columnheader", { name: "Time" })).toHaveAttribute("aria-sort", "descending");
  await page.getByRole("button", { name: /^Group / }).click();
  await page.getByRole("menuitem", { name: "Vendor" }).click();
  await grid(page)
    .getByRole("row", { name: /late \+84 ms/ })
    .click();
  await details(page).getByRole("tab", { name: "Headers" }).click();
  for (const part of [
    /[?&]q=cat%3Abid(&|$)/,
    /[?&]sort=-time(&|$)/,
    /[?&]group=vendor(&|$)/,
    /[?&]req=r\d+(&|$)/,
    /[?&]dt=headers(&|$)/,
  ]) {
    await expect(page).toHaveURL(part);
  }

  await page.reload();
  await expect(filter(page)).toHaveValue("cat:bid");
  await expect(count(page)).toHaveText("8 of 191 requests");
  await expect(grid(page).getByRole("columnheader", { name: "Time" })).toHaveAttribute("aria-sort", "descending");
  await expect(grid(page).getByRole("row", { name: /^Xandr/ })).toHaveAttribute("aria-expanded", "true");
  await expect(grid(page).getByRole("row", { name: /late \+84 ms/ })).toHaveAttribute("aria-selected", "true");
  await expect(details(page).getByRole("tab", { name: "Headers" })).toHaveAttribute("aria-selected", "true");
  await expect(details(page).getByText("Response headers (")).toBeVisible();

  // The time window: picked from the events of the timeline, kept in the URL, cleared with Escape.
  await details(page).getByRole("button", { name: "Close request details" }).click();
  await page.getByRole("button", { name: "Events on the timeline" }).click();
  await page.getByRole("menuitem", { name: /^Auction 1:/ }).click();
  await expect(page).toHaveURL(/from=685&to=2785$/);
  await expect(grid(page).getByRole("columnheader", { name: /Waterfall/ })).toContainText("685 ms to 2.79 s");
  await page.reload();
  await expect(page.getByRole("img", { name: /Window from 685 ms to 2\.79 s/ })).toBeVisible();
  await expect(page.getByRole("slider", { name: "Window start" })).toHaveAttribute("aria-valuetext", "685 ms");
  await grid(page).focus();
  await page.keyboard.press("Escape");
  await expect(page).not.toHaveURL(/from=/);
});

test("dragging on the overview selects a window that narrows the list", async ({ page }) => {
  await openApp(page, `${SCAN}/network`);
  await expect(count(page)).toHaveText("191 requests");
  const strip = page.getByRole("img", { name: /requests over/ });
  const box = await strip.boundingBox();
  if (!box) throw new Error("the strip has no box");
  await page.mouse.move(box.x + box.width * 0.5, box.y + 10);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width * 0.8, box.y + 10, { steps: 5 });
  await page.mouse.up();
  await expect(page).toHaveURL(/from=\d+&to=\d+$/);
  await expect(count(page)).toHaveText(/^\d+ of 191 requests$/);
  await page.getByRole("button", { name: "Clear the time window" }).click();
  await expect(count(page)).toHaveText("191 requests");
  // The same from the keyboard: the slider's thumbs.
  await page.getByRole("slider", { name: "Window start" }).focus();
  await page.keyboard.press("End");
  await page.keyboard.press("ArrowLeft");
  await expect(page).toHaveURL(/from=\d+&to=\d+$/);
});

test("the whole path works from the keyboard: filter, rows, details, tabs, close, page views", async ({ page }) => {
  await openApp(page, `${BROWSE}/network`);
  await expect(count(page)).toHaveText("402 requests");
  await page.keyboard.press("/");
  await expect(filter(page)).toBeFocused();
  await page.keyboard.type("cat:bid");
  await expect(count(page)).toHaveText("17 of 402 requests");

  // Tab reaches the grid, which is one stop; arrows move inside it.
  for (
    let presses = 0;
    presses < 40 && !(await grid(page).evaluate((element) => element === document.activeElement));
    presses += 1
  ) {
    await page.keyboard.press("Tab");
  }
  await expect(grid(page)).toBeFocused();
  const active = () =>
    grid(page).evaluate(
      (element) => document.getElementById(element.getAttribute("aria-activedescendant") ?? "")?.textContent ?? "",
    );
  expect(await active()).toContain("Page 1");
  // Left folds the group, Right opens it again.
  await page.keyboard.press("ArrowLeft");
  await expect(grid(page).getByRole("row", { name: /^Page 1/ })).toHaveAttribute("aria-expanded", "false");
  await page.keyboard.press("ArrowRight");
  await expect(grid(page).getByRole("row", { name: /^Page 1/ })).toHaveAttribute("aria-expanded", "true");
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("ArrowDown");
  const second = await active();
  await page.keyboard.press("Enter");
  await expect(details(page)).toBeVisible();
  await expect(page).toHaveURL(/req=r\d+/);
  // With the details open, the arrows take the details along.
  await page.keyboard.press("ArrowDown");
  expect(await active()).not.toBe(second);
  await page.keyboard.press("End");
  await expect(grid(page).getByRole("row").last()).toHaveAttribute("aria-selected", "true");
  await page.keyboard.press("Home");
  expect(await active()).toContain("Page 1");

  // Tab moves into the details; the tabs answer to the arrow keys.
  await page.keyboard.press("Tab");
  await expect(page.getByRole("separator", { name: "Resize the request details" })).toBeFocused();
  await page.keyboard.press("Tab");
  await expect(details(page).getByRole("button", { name: "Copy" })).toBeFocused();
  await details(page).getByRole("tab", { name: "Summary" }).focus();
  await page.keyboard.press("ArrowRight");
  await expect(details(page).getByRole("tab", { name: "Headers" })).toHaveAttribute("aria-selected", "true");
  await page.keyboard.press("Escape");
  await expect(details(page)).toBeHidden();
  await expect(grid(page)).toBeFocused();

  // The brackets step through the page views, and round to the whole visit.
  const pages = page.getByRole("radiogroup", { name: "Page view" });
  await page.keyboard.press("]");
  await expect(pages.getByRole("radio", { name: /^1/ })).toBeChecked();
  await expect(page).toHaveURL(/page=0/);
  await page.keyboard.press("]");
  await page.keyboard.press("]");
  await expect(pages.getByRole("radio", { name: /^3/ })).toBeChecked();
  await page.keyboard.press("]");
  await expect(pages.getByRole("radio", { name: "All pages" })).toBeChecked();
  await page.keyboard.press("[");
  await expect(pages.getByRole("radio", { name: /^3/ })).toBeChecked();
  // Find focuses the filter too; the copy key copies the active row's address.
  await page.keyboard.press(`${MOD}+f`);
  await expect(filter(page)).toBeFocused();
  await grid(page).focus();
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press(`${MOD}+c`);
  await expect.poll(() => page.evaluate(() => navigator.clipboard.readText())).toMatch(/^https:\/\//);
});

test("an auction's bid opens its exchange with the verdict, and Back returns to the auction", async ({ page }) => {
  await openApp(page, `${SCAN}?tab=auctions`);
  const auction = page.getByRole("region", { name: "Auction 1" });
  const late = auction.getByRole("listitem", { name: /^ix: Timeout/ });
  await late.hover();
  await expect(page.getByRole("tooltip")).toContainText(
    "On the network: Response arrived after the auction closed (+84 ms).",
  );
  await late.click();
  await expect(page).toHaveURL(/\/network\?req=r\d+$/);
  await expect(details(page).getByTestId("verdict")).toHaveText("Response arrived after the auction closed (+84 ms)");
  await expect(details(page)).toContainText("matched by an id in the request");
  await expect(grid(page).getByRole("row", { name: /late \+84 ms/ })).toHaveAttribute("aria-selected", "true");
  // The row the link names is in sight, wherever in the list it is.
  await expect(grid(page).getByRole("row", { name: /late \+84 ms/ })).toBeInViewport();
  // And from the request back to its auction.
  await details(page).getByRole("link", { name: "Auction 1" }).click();
  await expect(page).toHaveURL(/tab=auctions/);
  await page.goBack();
  await page.goBack();
  await expect(page).toHaveURL(/tab=auctions$/);

  // The bid Prebid never sent says so, and has nothing to open.
  await auction.getByRole("listitem", { name: /^triplelift: No bid/ }).hover();
  await expect(page.getByRole("tooltip")).toContainText("Prebid announced this bid but no request left the page");
  await expect(page.getByRole("tooltip")).not.toContainText("Click to open");
});

test("Inspect exchanges opens the auction as bidders by ad units; a cell opens the same details", async ({ page }) => {
  await openApp(page, `${SCAN}?tab=auctions`);
  await page.getByRole("region", { name: "Auction 1" }).getByRole("button", { name: "Inspect exchanges" }).click();
  await expect(page).toHaveURL(/\/network\?view=bids&auction=a1$/);
  const matrix = page.getByRole("table", { name: "Bid exchanges of auction 1" });
  await expect(matrix.getByRole("columnheader")).toHaveCount(5);
  await expect(matrix.getByRole("row", { name: /^appnexus/ })).toContainText(
    "1 request for 3 ad units, 326 ms, preflight +45 ms",
  );
  await expect(matrix.getByRole("row", { name: /^triplelift/ })).toContainText("not sent");
  await expect(matrix.getByRole("row", { name: /^triplelift/ })).toContainText("no request");
  await expect(matrix.getByRole("row", { name: /^sovrn/ })).toContainText("no-bid 5");
  // The bidders of the one Prebid Server request stand together, with their server-side times.
  await expect(matrix.getByRole("rowheader", { name: /Prebid Server: 1 request for 2 bidders/ })).toBeVisible();
  await expect(matrix.getByRole("row", { name: /^criteo/ })).toContainText(/server \d+ ms/);
  await expect(matrix.getByRole("row", { name: /^criteo/ })).toContainText("shared");

  const cells = matrix.getByRole("button", { name: /^appnexus for / });
  await expect(cells).toHaveCount(3);
  await cells.first().click();
  await expect(details(page).getByRole("heading", { level: 2 })).toContainText("POST ib.adnxs.com/ut/v3/prebid");
  // Cells that one request answers open the same exchange.
  await expect(cells.nth(1)).toHaveAttribute("aria-pressed", "true");
  await expect(cells.nth(2)).toHaveAttribute("aria-pressed", "true");
  await matrix.getByRole("button", { name: /^criteo for div-gpt-top-leaderboard/ }).click();
  await expect(details(page).getByRole("heading", { level: 2 })).toContainText(
    "prebid.adnxs.com/pbs/v1/openrtb2/auction",
  );
  await details(page).getByRole("tab", { name: "Response" }).click();
  await expect(details(page).getByRole("table", { name: "Prebid Server, per bidder" })).toContainText("criteo");
  await page.goBack();
  await expect(page).toHaveURL(/tab=auctions$/);
});

test("a bidder check opens the response it was read from, decoded and raw", async ({ page }) => {
  await openApp(page, `${SCAN}?tab=auctions`);
  const sovrn = page.getByRole("listitem", { name: "sovrn", exact: true });
  await expect(sovrn).toContainText(
    "Endpoint: The endpoint answered with no-bid reason 5 (Cloud, Data Center, or Proxy IP)",
  );
  await sovrn.getByRole("button", { name: "View the response" }).click();
  await expect(page).toHaveURL(/\/network\?req=r\d+&dt=response$/);
  const decoded = details(page).getByRole("region", { name: "Decoded bid response" });
  await expect(decoded).toContainText("No bid, reason 5: Cloud, Data Center, or Proxy IP");
  await details(page).getByRole("radio", { name: "Raw" }).click();
  await expect(details(page).getByTestId("body-editor")).toContainText('"nbr": 5');

  // The request side: impressions and floors decoded, consent and ids as presence only.
  await details(page).getByRole("tab", { name: "Payload" }).click();
  const request = details(page).getByRole("region", { name: "Decoded bid request" });
  await expect(request.getByRole("table", { name: "Impressions" })).toContainText("div-gpt-incontent-2");
  await expect(request).toContainText("Consent string");
  await expect(request).toContainText("present");
  await expect(request).toContainText("id5-sync.com, criteo.com");
  await expect(request).not.toContainText("ID5*");
  await details(page).getByRole("radio", { name: "Raw" }).click();
  // The raw body has them; it is long, so find is the way to see that.
  await details(page).getByTestId("body-editor").locator(".cm-content").click();
  await page.keyboard.press(`${MOD}+f`);
  await page.getByRole("textbox", { name: "Find" }).fill("ID5*");
  await expect(page.getByRole("search", { name: "Find in this file" }).getByRole("status")).toHaveText("1 of 1");
  await page.keyboard.press("Escape");
  // An adapter with its own format says that there is no decoder, and shows the body as sent.
  await page.goBack();
  await page.goto(`/#${SCAN}/network?q=bidder%3Arubicon`);
  await grid(page)
    .getByRole("row", { name: /fastlane\.json/ })
    .click();
  await details(page).getByRole("tab", { name: "Response" }).click();
  await expect(details(page).getByText("No decoder for this adapter yet")).toBeVisible();
  await expect(details(page).getByTestId("body-editor")).toContainText('"nobid"');
});

test("slots, scripts and evidence link to their requests, and requests link back", async ({ page }) => {
  await openApp(page, `${SCAN}?tab=slots`);
  // A slot's ad request opens the ad-server call, which says whether it carried Prebid's keys.
  await page.getByRole("button", { name: "Ad request" }).first().click();
  await expect(details(page).getByRole("heading", { level: 2 })).toContainText(
    "securepubads.g.doubleclick.net/gampad/ads",
  );
  await expect(details(page)).toContainText("None: this ad request carried no hb_ targeting");
  await expect(details(page)).toContainText("the auction this slot's ad unit was in");
  await details(page).getByRole("link", { name: "div-gpt-top-leaderboard" }).click();
  await expect(page).toHaveURL(/tab=slots/);
  await page.goBack();
  // The query string is a table; the key-values Google packs into one parameter are opened.
  await details(page).getByRole("tab", { name: "Payload" }).click();
  await expect(details(page).getByText("prev_iu_szs")).toBeVisible();
  await expect(details(page).getByText("article_type", { exact: true })).toBeVisible();
  await page.goBack();
  await expect(page).toHaveURL(/tab=slots$/);

  // Evidence of a rule finding that is about a request.
  await page.goto(`/#${SCAN}?tab=findings`);
  const early = page.getByRole("article", { name: /Ad server request left before the Prebid auction ended/ });
  await early.getByRole("button", { expanded: false }).click();
  await early.getByRole("button", { name: "View request" }).click();
  await expect(details(page).getByRole("heading", { level: 2 })).toContainText("gampad/ads");
  await page.goBack();
  await expect(page).toHaveURL(/tab=findings$/);

  // A script: from its row to its request, from the request's body to the viewer, from the viewer back.
  await page.goto(`/#${SCAN}?tab=scripts`);
  await page.getByRole("row", { name: /hb-wrapper\.min\.js/ }).click();
  await page.getByRole("region", { name: "Script details" }).getByRole("button", { name: "View request" }).click();
  await expect(details(page).getByRole("heading", { level: 2 })).toContainText(
    "GET www.sportsdesk.example/assets/ads/hb-wrapper.min.js",
  );
  await details(page).getByRole("tab", { name: "Response" }).click();
  await details(page).getByRole("link", { name: "Open in script viewer" }).click();
  await expect(page).toHaveURL(/\/source\/s7$/);
  await page.getByRole("button", { name: "More actions for this script" }).click();
  await page.getByRole("menuitem", { name: "View its request" }).click();
  await expect(details(page).getByRole("heading", { level: 2 })).toContainText("hb-wrapper.min.js");
  await page.goBack();
  await page.goBack();
  await page.goBack();
  await expect(page).toHaveURL(/tab=scripts$/);
});

test("a request's initiator opens the script at the line that made the call", async ({ page }) => {
  await openApp(page, `${SCAN}/network?q=bidder%3Aopenx`);
  await grid(page)
    .getByRole("row", { name: /prebidjs/ })
    .click();
  await details(page).getByRole("tab", { name: "Initiator" }).click();
  const stack = details(page).getByRole("list");
  await expect(stack.getByRole("listitem")).toHaveCount(3);
  await expect(stack.getByRole("listitem").first()).toContainText("ajax");
  // The wrapper's frame is a position in the minified file; the viewer shows the formatted line it maps to.
  await stack.getByRole("link", { name: /^hb-wrapper\.min\.js:1:\d+$/ }).click();
  await expect(page).toHaveURL(/\/source\/s7\?oline=1&ocol=\d+$/);
  await expect(page.getByTestId("caret-position")).toHaveText("Ln 326, Col 1");
  await expect(page.getByTestId("code").locator(".cm-line", { hasText: "pbjs.requestBids({" })).toBeInViewport();
  await page.goBack();
  await expect(details(page).getByRole("tab", { name: "Initiator" })).toHaveAttribute("aria-selected", "true");

  await page.goto(`/#${SCAN}/network?q=type%3Acss`);
  await grid(page)
    .getByRole("row", { name: /base\.3f1c9a\.css/ })
    .click();
  await details(page).getByRole("tab", { name: "Initiator" }).click();
  await expect(details(page)).toContainText("The HTML parser made this request");
});

test("whatever a body contains is shown as text: no markup is rendered, no reordering goes unseen", async ({
  page,
}) => {
  await openApp(page, `${SCAN}/network?q=comments%2Flatest`);
  await grid(page)
    .getByRole("row", { name: /latest\.json/ })
    .click();
  await details(page).getByRole("tab", { name: "Response" }).click();
  const body = details(page).getByTestId("body-editor");
  await expect(body).toContainText(`<img src=x onerror=\\"document.title='owned'\\">nice try`);
  await expect(body.locator("img:not(.cm-widgetBuffer), a, script, iframe")).toHaveCount(0);
  await expect(page).not.toHaveTitle(/owned/);
  await expect(body.locator(".cm-hiddenChar")).toHaveText(["U+202E", "U+202C", "U+200B"]);
  // A URL inside a body is characters, not a link.
  await expect(body.getByRole("link")).toHaveCount(0);

  // The path of the JSON value under the caret, ready to copy.
  await body.getByText("terrace_tom").click();
  await expect(details(page).getByTestId("json-path")).toHaveText("comments[0].author");
  await details(page).getByRole("button", { name: "Copy path" }).click();
  expect(await page.evaluate(() => navigator.clipboard.readText())).toBe("comments[0].author");
});

test("headers show what was redacted and why; cURL carries placeholders and says so", async ({ page }) => {
  await openApp(page, `${SCAN}/network?q=path%3A%2Fapi%2Fsession`);
  await grid(page)
    .getByRole("row", { name: /session/ })
    .click();
  await expect(details(page)).toContainText("401");
  await details(page).getByRole("tab", { name: "Headers" }).click();
  await expect(details(page).getByRole("note")).toContainText("What stands in their place below is not what was sent.");
  await expect(details(page).getByText(/^Bearer \[\d+ bytes\]/)).toContainText("redacted: credentials are not stored");
  await expect(details(page).getByText(/^_ga=\[\d+ bytes\]/)).toContainText("redacted: cookie values are not stored");
  await details(page).getByRole("searchbox", { name: "Filter headers by name" }).fill("auth");
  await expect(details(page).getByText("Request headers (1)")).toBeVisible();

  await details(page).getByRole("button", { name: "Copy" }).click();
  await page.getByRole("menuitem", { name: "Copy as cURL" }).click();
  await expect(page.getByText("Copied, with placeholders")).toBeVisible();
  await expect(
    page.getByText(/Authorization and Cookie were not stored, so the command will not reproduce a signed-in request\./),
  ).toBeVisible();
  const command = await page.evaluate(() => navigator.clipboard.readText());
  expect(command).toContain("curl 'https://www.sportsdesk.example/api/session'");
  expect(command).toContain("-H 'Authorization: <redacted: not stored>'");
  expect(command).not.toContain("bytes]");
});

test("copying a bid request warns once that it identifies the visit", async ({ page }) => {
  await openApp(page, `${SCAN}/network?q=bidder%3Aopenx`);
  await grid(page)
    .getByRole("row", { name: /prebidjs/ })
    .click();
  await details(page).getByRole("button", { name: "Copy" }).click();
  await page.getByRole("menuitem", { name: "Copy as cURL" }).click();
  const warning = page.getByRole("alertdialog", { name: "This copy identifies the visit" });
  await expect(warning).toContainText("sends a real request to the exchange");
  await page.evaluate(() => navigator.clipboard.writeText("untouched"));
  await warning.getByRole("button", { name: "Cancel" }).click();
  expect(await page.evaluate(() => navigator.clipboard.readText())).toBe("untouched");

  await details(page).getByRole("button", { name: "Copy" }).click();
  await page.getByRole("menuitem", { name: "Copy as cURL" }).click();
  await warning.getByRole("button", { name: "Copy anyway" }).click();
  await expect.poll(() => page.evaluate(() => navigator.clipboard.readText())).toContain('--data-raw \'{"id":');
  // Not asked a second time in the same run.
  await details(page).getByRole("button", { name: "Copy" }).click();
  await page.getByRole("menuitem", { name: "Copy payload" }).click();
  await expect(warning).toBeHidden();
  await expect.poll(() => page.evaluate(() => navigator.clipboard.readText())).toMatch(/^\{"id":/);
});

test("every way a body can be missing has its own sentence, and a large one loads in stretches", async ({ page }) => {
  await openApp(page, `${SCAN}/network?q=path%3A%2Ffeed%2F`);
  const open = async (name: RegExp) => {
    await grid(page).getByRole("row", { name }).click();
    await details(page).getByRole("tab", { name: "Response" }).click();
  };
  await open(/full-season\.json/);
  await expect(details(page).getByRole("note")).toContainText("First 5.0 MB of 7.4 MB kept");
  await expect(details(page).getByText(/showing the first 2\.0 MB of 5\.0 MB/)).toBeVisible();
  await details(page).getByRole("button", { name: "Load more" }).click();
  await expect(details(page).getByText(/showing the first 4\.0 MB of 5\.0 MB/)).toBeVisible();
  await open(/standings\.json/);
  await expect(details(page).getByRole("note")).toContainText(
    "The archive could not keep up with the page at that moment",
  );
  await open(/archive\.json/);
  await expect(details(page).getByRole("button", { name: "Load more" })).toBeVisible();
  await expect(details(page).getByTestId("body-editor")).toContainText('{"fixtures":[');

  const sentences: [query: string, row: RegExp, text: string][] = [
    ["type:font", /inter-var\.woff2/, "Body not kept (font, 47 KB)"],
    ["is:pending cat:site", /poll/, "No response before the visit ended"],
    ["status:301", /transfer-news/, "The browser could not provide this body"],
    ["path:/metrics/vitals", /vitals/, "No content"],
  ];
  for (const [query, row, text] of sentences) {
    await filter(page).fill(query);
    await open(row);
    await expect(details(page).getByRole("note")).toContainText(text);
  }
  // A WebSocket is one entry for the connection, and says that its frames are not recorded.
  await filter(page).fill("type:ws");
  await grid(page)
    .getByRole("row", { name: /updates/ })
    .click();
  await details(page).getByRole("tab", { name: "Summary" }).click();
  await expect(details(page)).toContainText("Frames are not recorded");
});

test("summary, timing and the redirect chain of a request", async ({ page }) => {
  await openApp(page, `${SCAN}/network?q=host%3Acm.g.doubleclick.net`);
  await grid(page).getByRole("row", { name: /pixel/ }).click();
  const chain = details(page).getByRole("list").filter({ hasText: "crwdcntrl" });
  await expect(chain.getByRole("listitem")).toHaveCount(3);
  await chain.getByRole("button", { name: /ib\.adnxs\.com\/setuid/ }).click();
  await expect(details(page).getByRole("heading", { level: 2 })).toContainText("ib.adnxs.com/setuid");
  await expect(details(page)).toContainText("302");

  await page.goto(`/#${SCAN}/network?q=bidder%3Aappnexus+method%3Apost`);
  await grid(page)
    .getByRole("row", { name: /prebid/ })
    .click();
  await expect(details(page).getByTestId("verdict")).toHaveText(
    /^Answered in 326 ms, 1\.\d\d s before the auction closed$/,
  );
  await details(page).getByRole("tab", { name: "Timing" }).click();
  await expect(details(page)).toContainText("Waiting for the server");
  await expect(details(page)).toContainText("CORS preflight): +45 ms before this request could start");
  await expect(details(page)).toContainText("closes at 2.38 s");
});

test("the list can be grouped, sorted and given other columns; the choice of columns is kept", async ({ page }) => {
  await openApp(page, `${BROWSE}/network`);
  // All pages of a visit are grouped by page view, each group with its totals.
  const first = grid(page).getByRole("row", { name: /^Page 1/ });
  await expect(first).toContainText("(entry page)");
  await expect(first).toContainText("193 requests");
  await expect(first).toContainText("3 failed");
  await first.click();
  await expect(first).toHaveAttribute("aria-expanded", "false");
  await grid(page)
    .getByRole("row", { name: /^Page 2/ })
    .click();
  await expect(grid(page).getByRole("row", { name: /^Page 3/ })).toContainText(
    "in-page navigation, no document request",
  );
  await first.click();

  await page.getByRole("button", { name: /^Group / }).click();
  await page.getByRole("menuitem", { name: "None" }).click();
  await expect(page).toHaveURL(/group=none$/);
  await expect(grid(page).getByRole("row", { name: /^Page 1/ })).toHaveCount(0);

  await page.getByRole("button", { name: "Columns" }).click();
  await page.getByRole("menuitemcheckbox", { name: "Host" }).click();
  await page.getByRole("menuitemcheckbox", { name: "Vendor" }).click();
  await page.keyboard.press("Escape");
  await expect(grid(page).getByRole("columnheader", { name: "Host" })).toBeVisible();
  await expect(grid(page).getByRole("columnheader", { name: "Vendor" })).toBeHidden();
  await page.reload();
  await expect(grid(page).getByRole("columnheader", { name: "Host" })).toBeVisible();
  await expect(grid(page).getByRole("columnheader", { name: "Vendor" })).toBeHidden();

  // Sorting by size puts the largest transfer first.
  await grid(page).getByRole("columnheader", { name: "Size" }).getByRole("button").click();
  await grid(page).getByRole("columnheader", { name: "Size" }).getByRole("button").click();
  await expect(grid(page).getByRole("row").nth(1)).toContainText("full-season.json");
  // Cache and service-worker answers say so where the size would be.
  await filter(page).fill("is:cached,sw page:2");
  await expect(grid(page).getByRole("row", { name: /offline\.json/ })).toContainText("(service worker)");
  await expect(grid(page).getByRole("row", { name: /base\.3f1c9a\.css/ })).toContainText("(disk cache)");
});

test("the details can dock below, be resized, take the whole view, and toggle with a key", async ({ page }) => {
  await openApp(page, `${SCAN}/network?q=cat%3Abid`);
  await grid(page)
    .getByRole("row", { name: /late \+84 ms/ })
    .click();
  const before = await details(page).boundingBox();
  // Beside the details the list keeps its essential columns.
  await expect(grid(page).getByRole("columnheader")).toHaveText(["Status", "Name", "Ad outcome", "Time"]);
  const divider = page.getByRole("separator", { name: "Resize the request details" });
  await divider.focus();
  await page.keyboard.press("ArrowLeft");
  await page.keyboard.press("ArrowLeft");
  expect((await details(page).boundingBox())?.width).toBe((before?.width ?? 0) + 48);

  await details(page).getByRole("button", { name: "Dock below the list" }).click();
  const below = await details(page).boundingBox();
  expect(below?.width).toBeGreaterThan(900);
  await expect(grid(page).getByRole("columnheader", { name: /Waterfall/ })).toBeVisible();
  await page.reload();
  await expect(details(page).getByRole("button", { name: "Dock beside the list" })).toBeVisible();

  await details(page).getByRole("button", { name: "Hide the list" }).click();
  await expect(grid(page)).toBeHidden();
  await page.keyboard.press("Escape");
  await expect(grid(page)).toBeVisible();
  await expect(details(page)).toBeVisible();
  await page.keyboard.press(`${MOD}+Alt+b`);
  await expect(details(page)).toBeHidden();
  await page.keyboard.press(`${MOD}+Alt+b`);
  await expect(details(page)).toBeVisible();
});

test("scans without a network record say why, in the report and in the network view", async ({ page }) => {
  await openApp(page, "/scans/seed-morningledger-desktop?tab=network");
  await expect(page.getByText("Network recording was off for this scan")).toBeVisible();
  await expect(page.getByRole("link", { name: "Storage settings" })).toBeVisible();
  // Its bids still carry the verdict of the network, with nothing to open.
  await reportTab(page, "Auctions").click();
  const auction = page.getByRole("region", { name: "Auction 1" });
  await expect(auction.getByRole("button", { name: "Inspect exchanges" })).toBeHidden();
  await auction.getByRole("listitem").first().hover();
  await expect(page.getByRole("tooltip")).toContainText("On the network: Answered in");
  await expect(page.getByRole("tooltip")).not.toContainText("Click to open");
  await page.goto("/#/scans/seed-morningledger-desktop/network");
  await expect(page.getByText("Network recording was off for this scan")).toBeVisible();

  await page.goto("/#/scans/seed-morningledger-mobile?tab=network");
  await expect(page.getByText("No network record for this scan")).toBeVisible();
  await expect(page.getByRole("button", { name: "Scan again", exact: true })).toBeVisible();
  // A report stored before the archive shows no links into it.
  await reportTab(page, "Scripts").click();
  await page.getByRole("row", { name: /hb-wrapper\.min\.js/ }).click();
  await expect(
    page.getByRole("region", { name: "Script details" }).getByRole("button", { name: "View request" }),
  ).toBeHidden();
  await page.goto("/#/scans/seed-morningledger-mobile/network");
  await expect(page.getByText("No network record for this scan")).toBeVisible();
});

test("an evicted scan keeps its request list and says what is gone", async ({ page }) => {
  await openApp(page, "/scans/seed-techpulse-desktop/network?q=cat%3Abid");
  await expect(page.getByRole("note").first()).toContainText(
    "The list remains, without query strings, headers and bodies.",
  );
  await expect(count(page)).toHaveText(/^\d+ of \d+ requests$/);
  await grid(page)
    .getByRole("row", { name: /prebidjs/ })
    .first()
    .click();
  await expect(details(page).getByRole("note")).toContainText("Headers and bodies are gone.");
  await details(page).getByRole("tab", { name: "Response" }).click();
  await expect(details(page)).toContainText("Not stored any more");
  await details(page).getByRole("button", { name: "Copy" }).click();
  await expect(page.getByRole("menuitem", { name: "Copy as cURL" })).toBeDisabled();

  // Clearing the stored data in Settings leaves the report and empties its network view.
  await page.goto("/#/settings");
  const stored = page.getByRole("region", { name: "Stored scan data" });
  await stored.getByRole("button", { name: "Clear stored scan data…" }).click();
  await page.getByRole("alertdialog").getByRole("button", { name: "Clear scan data" }).click();
  await expect(page.getByText("Stored scan data cleared")).toBeVisible();
  await page.goto(`/#${SCAN}?tab=network`);
  await expect(page.getByText("The network record of this scan was removed")).toBeVisible();
  await page.goto(`/#${SCAN}/source/s7`);
  await expect(page.getByText("This script's source was removed")).toBeVisible();
});

test("a scan made with recording off has findings and no network record; with full headers on, nothing is redacted", async ({
  page,
}) => {
  test.setTimeout(60_000);
  await openApp(page, "/settings");
  const stored = page.getByRole("region", { name: "Stored scan data" });
  await stored.getByRole("switch", { name: "Store full headers" }).click();
  await expect(page.getByText("Settings saved")).toBeVisible();
  await page.getByRole("link", { name: "Home", exact: true }).click();
  await page.getByRole("textbox", { name: "Page URL" }).fill("sportsdesk.example");
  await page.getByRole("button", { name: "Scan", exact: true }).click();
  await expect(reportTab(page, "Network")).toBeVisible({ timeout: 25_000 });
  const scanUrl = page.url().split("?")[0];
  await page.goto(`${scanUrl}/network?q=path%3A%2Fapi%2Fsession`);
  await grid(page)
    .getByRole("row", { name: /session/ })
    .click();
  await details(page).getByRole("tab", { name: "Headers" }).click();
  await expect(details(page).getByText(/^Bearer eyJ/)).toBeVisible();
  await expect(details(page).getByText(/redacted:/)).toHaveCount(0);

  await page.goto("/#/settings");
  await stored.getByRole("switch", { name: "Keep network data with reports" }).click();
  await expect(stored.getByRole("switch", { name: "Store full headers" })).toBeDisabled();
  await page.getByRole("link", { name: "Home", exact: true }).click();
  await page.getByRole("textbox", { name: "Page URL" }).fill("sportsdesk.example");
  await page.getByRole("button", { name: "Scan", exact: true }).click();
  await expect(reportTab(page, "Network")).toBeVisible({ timeout: 25_000 });
  await reportTab(page, "Network").click();
  await expect(page.getByText("Network recording was off for this scan")).toBeVisible();
  await expect(reportTab(page, "Findings")).toContainText(/\d+/);
});

test.describe("at the minimum window size", () => {
  test.use({ viewport: { width: 1024, height: 680 } });

  test("the details dock below, categories fold into a menu, and nothing overflows", async ({ page }) => {
    await openApp(page, `${BROWSE}/network`);
    await expect(count(page)).toHaveText("402 requests");
    await expect(grid(page).getByRole("columnheader")).toHaveText([
      "Status",
      "Name",
      "Vendor",
      "Ad outcome",
      "Time",
      /^Waterfall/,
    ]);
    const waterfall = await grid(page)
      .getByRole("columnheader", { name: /Waterfall/ })
      .boundingBox();
    expect(waterfall?.width).toBeGreaterThanOrEqual(120);
    await expectNoHorizontalOverflow(page);
    await expectWindowDoesNotScroll(page);

    await page.getByRole("button", { name: /^All categories/ }).click();
    await page.getByRole("menuitem", { name: /Bid requests/ }).click();
    await expect(count(page)).toHaveText("17 of 402 requests");
    await expect(page.getByRole("group", { name: "State" }).getByRole("button")).toHaveCount(3);

    await grid(page)
      .getByRole("row", { name: /late \+84 ms/ })
      .click();
    const panel = await details(page).boundingBox();
    const list = await grid(page).boundingBox();
    expect(panel?.y).toBeGreaterThan((list?.y ?? 0) + 50);
    expect(panel?.width).toBeGreaterThan(700);
    await expect(details(page).getByRole("button", { name: "Hide the list" })).toBeVisible();
    await expectNoHorizontalOverflow(page);
    await expectWindowDoesNotScroll(page);

    await page.getByRole("radio", { name: "Bid exchanges" }).click();
    const matrix = page.getByRole("table", { name: /Bid exchanges of auction/ });
    await expect(matrix.getByRole("rowheader", { name: "appnexus" })).toBeInViewport();
    await expectWindowDoesNotScroll(page);
  });
});
