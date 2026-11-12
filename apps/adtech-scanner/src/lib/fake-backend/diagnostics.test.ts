import { expect, test } from "vitest";
import { buildDiagnostics } from "./diagnostics";
import { captureSite } from "./site";

const site = captureSite("www.sportsdesk.example", "desktop");
const PAGE = "https://www.sportsdesk.example/football/transfer-news";

const flagged = (diagnostics: ReturnType<typeof buildDiagnostics>) =>
  diagnostics.bidders.flatMap((bidder) =>
    bidder.checks
      .filter((check) => check.status === "problem" || check.status === "notice")
      .map((check) => `${bidder.bidder}: ${check.code}`),
  );

test("every bidder of the simulated auctions is checked against the site's ads.txt", () => {
  const diagnostics = buildDiagnostics(site, PAGE, false);
  const bidders = new Set(
    site.auctions.flatMap((auction) => auction.adUnits.flatMap((unit) => unit.bids.map((bid) => bid.bidder))),
  );

  expect(diagnostics.adsTxt.url).toBe("https://sportsdesk.example/ads.txt");
  expect(diagnostics.bidders.map((bidder) => bidder.bidder)).toEqual([...bidders].sort());
  expect(diagnostics.sellersJsonChecked).toBe(false);
  expect(diagnostics.bidders.flatMap((bidder) => bidder.checks).some((check) => check.area === "sellersJson")).toBe(
    false,
  );
  expect(flagged(diagnostics)).toEqual(["sovrn: ads-txt-not-listed"]);
  for (const bidder of diagnostics.bidders) {
    expect(bidder.bids).toBeLessThanOrEqual(bidder.requests);
    expect(bidder.requests).toBeGreaterThan(0);
  }
});

test("the sellers.json lookup adds its own checks only when it is on", () => {
  const diagnostics = buildDiagnostics(site, PAGE, true);

  expect(diagnostics.sellersJsonChecked).toBe(true);
  expect(flagged(diagnostics)).toEqual(["openx: sellers-json-type-mismatch", "sovrn: ads-txt-not-listed"]);
});

test("a page on this machine is asked for ads.txt at its own origin", () => {
  expect(buildDiagnostics(site, "http://127.0.0.1:47615/demo-publisher/", false).adsTxt.url).toBe(
    "http://127.0.0.1:47615/ads.txt",
  );
});
