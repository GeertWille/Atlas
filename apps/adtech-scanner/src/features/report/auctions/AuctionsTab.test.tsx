import { screen, within } from "@testing-library/react";
import { expect, test } from "vitest";
import type { BidderDiagnostics } from "@/bindings";
import { makeAuction, makeBid } from "@/test/fixtures";
import { renderWithApp } from "@/test/render";
import { AuctionsTab } from "./AuctionsTab";

const auctions = [
  makeAuction([makeBid({ bidder: "appnexus", status: "rendered" }), makeBid({ bidder: "seedtag", status: "noBid" })]),
];

const diagnostics: BidderDiagnostics = {
  adsTxt: {
    url: "https://pub.example/ads.txt",
    status: "found",
    detail: null,
    recordCount: 42,
    invalidLineCount: 0,
    invalidLines: [],
    ownerDomain: null,
    managerDomains: [],
  },
  sellersJsonChecked: false,
  consent: {
    cmpPresent: false,
    consentManagementConfigured: null,
    gdprApplies: null,
    resolvedAtMs: null,
    auctionCount: 1,
    auctionsBeforeConsent: 0,
    consentedPurposes: [],
  },
  bidders: [
    {
      bidder: "appnexus",
      requests: 1,
      bids: 1,
      checks: [
        {
          area: "adsTxt",
          status: "pass",
          code: "ads-txt-authorised",
          summary: "ads.txt lists appnexus.com (1 entry)",
          excerpt: "appnexus.com, 958, DIRECT",
          adUnits: [],
          pages: [],
          requestIds: [],
        },
      ],
    },
    {
      bidder: "seedtag",
      requests: 4,
      bids: 0,
      checks: [
        {
          area: "adsTxt",
          status: "problem",
          code: "ads-txt-not-listed",
          summary: "ads.txt has no entry for seedtag.com",
          excerpt: null,
          adUnits: [],
          pages: [],
          requestIds: [],
        },
        {
          area: "params",
          status: "notice",
          code: "params-shared-id",
          summary: "All 3 ad units use the same `adUnitId` (27) although their sizes differ",
          excerpt: "adUnitId: 27",
          adUnits: ["div-gpt-top", "div-gpt-rail"],
          pages: [],
          requestIds: [],
        },
        {
          area: "consent",
          status: "notChecked",
          code: "consent-not-applicable",
          summary: "The page exposes no TCF consent API, so there was no consent to check",
          excerpt: null,
          adUnits: [],
          pages: [],
          requestIds: [],
        },
      ],
    },
  ],
};

test("bidder checks spell out what needs attention and keep the rest to one line", async () => {
  await renderWithApp(<AuctionsTab auctions={auctions} diagnostics={diagnostics} />);

  const panel = screen.getByRole("heading", { name: "Bidder checks" }).closest("section") as HTMLElement;
  expect(within(panel).getByText("https://pub.example/ads.txt: 42 seller records")).toBeInTheDocument();
  expect(within(panel).getByText("Not looked up for this scan")).toBeInTheDocument();
  expect(within(panel).getByText("No TCF consent API on this page")).toBeInTheDocument();

  const flagged = within(panel).getByRole("list", { name: "Bidders that need attention" });
  const seedtag = within(flagged).getByRole("listitem", { name: "seedtag" });
  expect(within(flagged).queryByRole("listitem", { name: "appnexus" })).toBeNull();
  expect(seedtag).toHaveTextContent("0 of 4 requests answered with a bid");
  expect(within(seedtag).getByRole("img", { name: "Problem" })).toBeInTheDocument();
  expect(within(seedtag).getByRole("img", { name: "Notice" })).toBeInTheDocument();
  expect(seedtag).toHaveTextContent("ads.txt: ads.txt has no entry for seedtag.com");
  expect(seedtag).toHaveTextContent("Params: All 3 ad units use the same `adUnitId` (27)");
  expect(within(seedtag).getByText("adUnitId: 27")).toBeInTheDocument();
  expect(within(seedtag).getByText("div-gpt-top, div-gpt-rail")).toBeInTheDocument();
  expect(seedtag).toHaveTextContent("1 could not be made");
  // A check that passed, or could not be made, is counted but not listed.
  expect(seedtag).not.toHaveTextContent("no TCF consent API");
  expect(panel).toHaveTextContent("Nothing found for appnexus.");
  expect(panel).not.toHaveTextContent("appnexus.com, 958, DIRECT");
});

test("a scan stored before the bidder checks existed shows the auctions without them", async () => {
  await renderWithApp(<AuctionsTab auctions={auctions} diagnostics={null} />);

  expect(screen.getByRole("heading", { name: "By bidder" })).toBeInTheDocument();
  expect(screen.queryByRole("heading", { name: "Bidder checks" })).toBeNull();
});

test("when every bidder is fine the panel is three facts and one line", async () => {
  const clean = { ...diagnostics, bidders: diagnostics.bidders.slice(0, 1) };
  await renderWithApp(<AuctionsTab auctions={auctions} diagnostics={clean} />);

  expect(screen.queryByRole("list", { name: "Bidders that need attention" })).toBeNull();
  expect(screen.getByText(/Nothing found for/)).toHaveTextContent("Nothing found for appnexus.");
});
