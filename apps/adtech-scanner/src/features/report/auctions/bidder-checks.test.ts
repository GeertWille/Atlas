import { expect, test } from "vitest";
import type { AdsTxtReport, BidderDiagnosis, ConsentReport, DiagnosticCheck } from "@/bindings";
import {
  describeAdsTxt,
  describeBids,
  describeConsent,
  describeRemainder,
  describeSellersJson,
  groupBidders,
} from "./bidder-checks";

const check = (status: DiagnosticCheck["status"], code: string): DiagnosticCheck => ({
  area: "adsTxt",
  status,
  code,
  summary: code,
  excerpt: null,
  adUnits: [],
  pages: [],
});

const bidder = (name: string, checks: DiagnosticCheck[]): BidderDiagnosis => ({
  bidder: name,
  requests: 4,
  bids: 1,
  checks,
});

test("bidders with something to act on are separated from the rest, problems first", () => {
  const { flagged, clear } = groupBidders([
    bidder("appnexus", [check("pass", "params-ok"), check("notChecked", "response-not-seen")]),
    bidder("openx", [check("notice", "sellers-json-type-mismatch"), check("pass", "params-ok")]),
    bidder("rubicon", []),
    bidder("seedtag", [
      check("problem", "ads-txt-not-listed"),
      check("notice", "params-shared-id"),
      check("pass", "params-ok"),
      check("pass", "consent-ok"),
      check("notChecked", "response-not-seen"),
    ]),
  ]);

  expect(clear).toEqual(["appnexus", "rubicon"]);
  expect(flagged.map((entry) => [entry.bidder, entry.flagged.map((found) => found.code)])).toEqual([
    ["seedtag", ["ads-txt-not-listed", "params-shared-id"]],
    ["openx", ["sellers-json-type-mismatch"]],
  ]);
  expect(flagged.map(describeRemainder)).toEqual([
    "2 other checks passed, 1 could not be made",
    "1 other check passed",
  ]);
  expect(describeRemainder({ ...(flagged[0] as (typeof flagged)[0]), passed: 0, notChecked: 0 })).toBeNull();
});

const adsTxt = (overrides: Partial<AdsTxtReport>): AdsTxtReport => ({
  url: "https://pub.example/ads.txt",
  status: "found",
  detail: null,
  recordCount: 42,
  invalidLineCount: 0,
  invalidLines: [],
  ownerDomain: null,
  managerDomains: [],
  ...overrides,
});

test.each<[Partial<AdsTxtReport>, string]>([
  [{}, "42 seller records"],
  [{ recordCount: 1, invalidLineCount: 2 }, "1 seller record, 2 invalid lines"],
  [{ status: "missing", recordCount: 0 }, "The site has no ads.txt"],
  [{ status: "unreadable", detail: "the site answered HTTP 403" }, "Could not be read: the site answered HTTP 403"],
  [{ status: "unreadable" }, "Could not be read: no reason given"],
])("ads.txt %j reads as %s", (overrides, expected) => {
  expect(describeAdsTxt(adsTxt(overrides))).toBe(expected);
});

const consent = (overrides: Partial<ConsentReport>): ConsentReport => ({
  cmpPresent: true,
  consentManagementConfigured: true,
  gdprApplies: true,
  resolvedAtMs: 812,
  auctionCount: 3,
  auctionsBeforeConsent: 0,
  consentedPurposes: [1],
  ...overrides,
});

test.each<[Partial<ConsentReport>, string]>([
  [{ cmpPresent: false, gdprApplies: null }, "No TCF consent API on this page"],
  [{ gdprApplies: false }, "A consent platform is present; GDPR did not apply to this page view"],
  [{ gdprApplies: null }, "A consent platform is present; it did not report whether GDPR applies"],
  [{}, "GDPR applies, choice known after 812 ms"],
  [{ auctionsBeforeConsent: 2 }, "GDPR applies, choice known after 812 ms, 2 of 3 auctions started before that"],
  [
    { resolvedAtMs: null, auctionCount: 1, auctionsBeforeConsent: 1 },
    "GDPR applies, the choice was never made during the scan, 1 of 1 auction started before that",
  ],
])("consent %j reads as %s", (overrides, expected) => {
  expect(describeConsent(consent(overrides))).toBe(expected);
});

test("sellers.json says whether it was looked up", () => {
  const diagnostics = { adsTxt: adsTxt({}), consent: consent({}), bidders: [] };
  expect(describeSellersJson({ ...diagnostics, sellersJsonChecked: true })).toBe(
    "Looked up at the exchanges that ads.txt names",
  );
  expect(describeSellersJson({ ...diagnostics, sellersJsonChecked: false })).toBe("Not looked up for this scan");
});

test.each([
  [{ bids: 0, requests: 1 }, "0 of 1 request answered with a bid"],
  [{ bids: 1, requests: 1 }, "1 of 1 request answered with a bid"],
  [{ bids: 2, requests: 4 }, "2 of 4 requests answered with a bid"],
  [{ bids: 0, requests: 0 }, "0 of 0 requests answered with a bid"],
])("bids %j read as %s", (counts, expected) => {
  expect(describeBids(counts)).toBe(expected);
});
