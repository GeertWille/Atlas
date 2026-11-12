import { screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";
import { makeAuction, makeBid, makeDetail, makeEnvironment, makePage, makeScript, makeSlot } from "@/test/fixtures";
import { renderWithApp } from "@/test/render";
import { AuctionsTab } from "../auctions/AuctionsTab";
import { buildKeyMetrics } from "../key-metrics";
import { ReportOverview } from "../ReportOverview";
import { ScriptsTab } from "../scripts/ScriptsTab";
import { SlotsTab } from "../slots/SlotsTab";
import { layoutSlotTimeline, slotFlags } from "../slots/slot-timeline";
import { visitFacts } from "../visit-summary";
import { PageSwitcher } from "./PageSwitcher";

const pages = [
  makePage(),
  makePage({
    index: 1,
    url: "https://www.example.com/climate/kelp-carbon",
    title: "Kelp and carbon",
    reachedBy: "linkClick",
    link: { text: "Kelp and carbon", href: "https://www.example.com/climate/kelp-carbon", reason: "content link" },
    startedAtMs: 23_000,
    metrics: { ...makePage().metrics, auctionCount: 0, slotCount: 2, fillRate: 0.5 },
  }),
  makePage({
    index: 2,
    url: "https://www.example.com/sport/final",
    reachedBy: "routeChange",
    loadMs: null,
    startedAtMs: 47_500,
  }),
];

test("the page switcher is not rendered for one page", async () => {
  await renderWithApp(<PageSwitcher pages={[makePage()]} selected={null} onSelect={() => {}} />);
  expect(screen.queryByRole("region", { name: "Page views" })).toBeNull();
});

test("the page switcher lists every page view, reports the choice and describes the selected page", async () => {
  const user = userEvent.setup();
  const onSelect = vi.fn();
  await renderWithApp(<PageSwitcher pages={pages} selected={null} onSelect={onSelect} />);
  const switcher = screen.getByRole("radiogroup", { name: "Page view" });
  expect(
    within(switcher)
      .getAllByRole("radio")
      .map((item) => item.textContent),
  ).toEqual(["All pages", "1/news", "2/climate/kelp-carbon", "3/sport/final"]);
  expect(within(switcher).getByRole("radio", { name: "All pages" })).toBeChecked();
  expect(screen.getByText(/The visit covered 3 page views/)).toBeInTheDocument();

  await user.click(within(switcher).getByRole("radio", { name: /kelp-carbon/ }));
  expect(onSelect).toHaveBeenLastCalledWith(1);
});

test("a selected page is described by how it was reached and what happened on it", async () => {
  const onSelect = vi.fn();
  const user = userEvent.setup();
  await renderWithApp(<PageSwitcher pages={pages} selected={pages[1] ?? null} onSelect={onSelect} />);
  expect(screen.getByRole("radio", { name: /kelp-carbon/ })).toBeChecked();
  expect(screen.getByRole("region", { name: "Page views" })).toHaveTextContent(
    "Kelp and carbon · Followed link “Kelp and carbon” · opened 23.0 s into the visit · watched for 20.0 s · 0 auctions · 2 slots, 50% of requested filled",
  );
  await user.click(screen.getByRole("radio", { name: "All pages" }));
  expect(onSelect).toHaveBeenLastCalledWith(null);
});

test("auctions are grouped per page view with one numbering, and a page without auctions says so", async () => {
  const auctions = [makeAuction([makeBid()], { id: "a1", page: 0 }), makeAuction([makeBid()], { id: "a2", page: 2 })];
  await renderWithApp(<AuctionsTab auctions={auctions} diagnostics={null} pages={pages} />);

  const first = screen.getByRole("region", { name: "Auctions on page 1" });
  expect(within(first).getByRole("heading", { name: /Page 1/ })).toHaveTextContent("Page 1/news");
  expect(within(first).getByText(/Entry page · 1 auction/)).toBeInTheDocument();
  expect(within(first).getByRole("region", { name: "Auction 1" })).toBeInTheDocument();

  const second = screen.getByRole("region", { name: "Auctions on page 2" });
  expect(within(second).getByText(/Followed link · opened 23.0 s into the visit · 0 auctions/)).toBeInTheDocument();
  expect(within(second).getByText("No Prebid auction started on this page view.")).toBeInTheDocument();

  const third = screen.getByRole("region", { name: "Auctions on page 3" });
  expect(within(third).getByText(/In-page navigation/)).toBeInTheDocument();
  expect(within(third).getByRole("region", { name: "Auction 2" })).toBeInTheDocument();
});

test("slots are grouped per page view, each on its own timeline with that page's lifecycle marks", async () => {
  const slots = [
    makeSlot({ page: 0 }),
    makeSlot({ page: 1, elementId: "div-gpt-article" }),
    makeSlot({ page: 1, elementId: "div-gpt-side" }),
  ];
  await renderWithApp(<SlotsTab slots={slots} metrics={makeDetail().metrics} pages={pages} />);

  const first = screen.getByRole("region", { name: "Ad slots on page 1" });
  expect(within(first).getByText("div-gpt-top")).toBeInTheDocument();
  expect(within(first).getByText(/Page load 2\.10 s/)).toBeInTheDocument();
  const second = screen.getByRole("region", { name: "Ad slots on page 2" });
  expect(within(second).getByText(/2 slots/)).toBeInTheDocument();
  expect(within(second).getByText("div-gpt-article")).toBeInTheDocument();
  expect(within(second).queryByText("div-gpt-top")).toBeNull();
  // A route change loads no document, so its page has no slots and no load mark.
  const third = screen.getByRole("region", { name: "Ad slots on page 3" });
  expect(within(third).getByText("GPT defined no ad slot on this page view.")).toBeInTheDocument();
});

test("refreshes of a slot are extra request marks, and one that re-used a bid is flagged", () => {
  const refreshed = makeSlot({
    requests: [
      { atMs: 2000, hadPrebidTargeting: true, prebidAdId: "1a2b", requestId: null },
      { atMs: 9000, hadPrebidTargeting: true, prebidAdId: "1a2b", requestId: null },
      { atMs: 14_000, hadPrebidTargeting: false, prebidAdId: null, requestId: null },
    ],
  });
  const [row] = layoutSlotTimeline([refreshed], makeDetail().metrics).rows;
  expect(row?.refreshes.map(({ atMs, staleBid, hadPrebidTargeting }) => [atMs, staleBid, hadPrebidTargeting])).toEqual([
    [9000, true, true],
    [14_000, false, false],
  ]);
  // The axis reaches the last refresh, well past the viewable mark at 3.5 s.
  expect(row?.refreshes.every((refresh) => refresh.percent > 50 && refresh.percent <= 100)).toBe(true);
  expect(slotFlags(refreshed).find((flag) => flag.key === "refreshed")).toMatchObject({
    label: "Refreshed with a used bid",
    warning: true,
  });

  const fresh = makeSlot({
    requests: [
      { atMs: 2000, hadPrebidTargeting: true, prebidAdId: "1a2b", requestId: null },
      { atMs: 32_000, hadPrebidTargeting: true, prebidAdId: "9f9f", requestId: null },
    ],
  });
  expect(slotFlags(fresh).find((flag) => flag.key === "refreshed")).toMatchObject({
    label: "Refreshed once",
    warning: false,
  });
  expect(
    slotFlags(makeSlot({ requests: [{ atMs: 2000, hadPrebidTargeting: true, prebidAdId: "1a2b", requestId: null }] })),
  ).toEqual([]);
  expect(layoutSlotTimeline([makeSlot()], makeDetail().metrics).rows[0]?.refreshes).toEqual([]);
});

test("the scripts table gains a Pages column only for a multi-page visit", async () => {
  const scripts = [
    makeScript({ id: "s1", url: "https://www.example.com/app.js", pages: [0, 1] }),
    makeScript({ id: "s2", url: "https://www.example.com/boot.js", pages: [0] }),
  ];
  const { unmount } = await renderWithApp(<ScriptsTab scripts={scripts} showPages />);
  expect(screen.getByRole("columnheader", { name: "Pages" })).toBeInTheDocument();
  const cells = (name: RegExp) => within(screen.getByRole("row", { name })).getAllByRole("cell");
  // Script, category, vendor, then the pages.
  expect(cells(/app\.js/)[3]).toHaveTextContent("1, 2");
  expect(cells(/boot\.js/)[3]).toHaveTextContent(/^1$/);
  unmount();

  await renderWithApp(<ScriptsTab scripts={scripts} />);
  expect(screen.queryByRole("columnheader", { name: "Pages" })).toBeNull();
});

test("the overview says how the site was visited, once the scan recorded it", async () => {
  const environment = makeEnvironment({
    visitor: "browse",
    headless: false,
    persistentProfile: true,
    proxy: { kind: "profile", name: "Belgium", host: "be.proxy.example" },
  });
  expect(visitFacts({ environment, pages })).toEqual([
    "Browse like a visitor",
    "3 pages",
    "Browser shown",
    "Kept scan profile",
    "Belgium (be.proxy.example)",
  ]);
  expect(visitFacts({ environment: makeEnvironment({ emulatedMobile: true }), pages: [] })).toEqual([
    "Scroll",
    "1 page",
    "Headless",
    "Fresh profile",
    "System proxy",
    "Mobile emulation",
  ]);
  expect(visitFacts({ environment: null, pages })).toBeNull();

  await renderWithApp(<ReportOverview detail={makeDetail({ environment, pages })} />);
  const overview = screen.getByRole("region", { name: "Overview" });
  expect(within(overview).getByText("Visit")).toBeInTheDocument();
  expect(within(overview).getByText("Belgium (be.proxy.example)")).toBeInTheDocument();
});

test("with several pages the three timings say that they are the entry page's", () => {
  const { metrics, slots } = makeDetail();
  const hints = (multiPage: boolean) =>
    Object.fromEntries(buildKeyMetrics(metrics, slots, multiPage).map((metric) => [metric.label, metric.hint]));
  expect(hints(true)).toMatchObject({
    "First auction": "entry page, timeout 1.50 s",
    "First ad rendered": "on the entry page",
    "Page load": "entry page load event",
  });
  expect(hints(false)).toEqual(Object.fromEntries(buildKeyMetrics(metrics, slots).map((m) => [m.label, m.hint])));
  expect(hints(false)["First ad rendered"]).toBe("after navigation start");
  expect(buildKeyMetrics({ ...metrics, prebidTimeoutMs: null }, slots, true)[4]?.hint).toBe("on the entry page");
});
