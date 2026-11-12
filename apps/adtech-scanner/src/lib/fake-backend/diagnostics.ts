import type { BidderDiagnosis, BidderDiagnostics, DiagnosticCheck } from "@/bindings";
import type { CapturedSite } from "./site";

/** The ads.txt entry each simulated bidder needs: ad system domain and the site's account there. */
const AD_SYSTEMS: Record<string, { domain: string; account: string }> = {
  appnexus: { domain: "appnexus.com", account: "11842" },
  rubicon: { domain: "rubiconproject.com", account: "26014" },
  pubmatic: { domain: "pubmatic.com", account: "164120" },
  ix: { domain: "indexexchange.com", account: "194730" },
  criteo: { domain: "criteo.com", account: "B-071554" },
  openx: { domain: "openx.com", account: "544096208" },
  triplelift: { domain: "triplelift.com", account: "11582" },
  sovrn: { domain: "sovrn.com", account: "330912" },
  sharethrough: { domain: "sharethrough.com", account: "a7Kc21Xp" },
};

/** The simulated publishers forgot this partner when they last edited ads.txt. */
const MISSING_FROM_ADS_TXT = "sovrn";
/** With the sellers.json lookup on, this exchange lists the site's account as an intermediary. */
const SELLER_TYPE_MISMATCH = "openx";

function check(
  area: DiagnosticCheck["area"],
  status: DiagnosticCheck["status"],
  code: string,
  summary: string,
  excerpt: string | null = null,
): DiagnosticCheck {
  return { area, status, code, summary, excerpt, adUnits: [], pages: [], requestIds: [] };
}

function checksFor(bidder: string, adUnits: number, sellersJson: boolean): DiagnosticCheck[] {
  const system = AD_SYSTEMS[bidder];
  const checks = [
    check("params", "pass", "params-ok", `Required params are set on all ${adUnits} ad units`),
    check("consent", "pass", "consent-ok", "A consent string was passed and it covers this vendor"),
    check("response", "notChecked", "response-not-seen", "No request to a known endpoint of this bidder was observed"),
  ];
  if (!system) return checks;
  const line = `${system.domain}, ${system.account}, DIRECT`;
  if (bidder === MISSING_FROM_ADS_TXT) {
    checks.unshift(check("adsTxt", "problem", "ads-txt-not-listed", `ads.txt has no entry for ${system.domain}`));
    return checks;
  }
  checks.push(check("adsTxt", "pass", "ads-txt-authorised", `ads.txt lists ${system.domain} (1 entry)`, line));
  if (sellersJson && bidder === SELLER_TYPE_MISMATCH) {
    checks.unshift(
      check(
        "sellersJson",
        "notice",
        "sellers-json-type-mismatch",
        `ads.txt lists seller id ${system.account} at ${system.domain} as DIRECT, but its sellers.json lists it as INTERMEDIARY`,
        line,
      ),
    );
  } else if (sellersJson) {
    checks.push(
      check(
        "sellersJson",
        "pass",
        "sellers-json-ok",
        `Seller id ${system.account} is listed in the sellers.json of ${system.domain}`,
        line,
      ),
    );
  }
  return checks;
}

/** Where the backend reads ads.txt: the root domain over https, or a local page's own origin. */
function adsTxtUrl(pageUrl: string): string {
  const { hostname, origin } = new URL(pageUrl);
  const isLocal = hostname === "localhost" || /^[\d.]+$/.test(hostname);
  return isLocal ? `${origin}/ads.txt` : `https://${hostname.replace(/^www\./, "")}/ads.txt`;
}

/** Bidder checks as the backend would report them for a simulated site. */
export function buildDiagnostics(site: CapturedSite, pageUrl: string, sellersJson: boolean): BidderDiagnostics {
  const tallies = new Map<string, { requests: number; bids: number; adUnits: Set<string> }>();
  for (const auction of site.auctions) {
    for (const unit of auction.adUnits) {
      for (const bid of unit.bids) {
        const tally = tallies.get(bid.bidder) ?? { requests: 0, bids: 0, adUnits: new Set<string>() };
        tally.requests += 1;
        if (bid.status === "bid" || bid.status === "won" || bid.status === "rendered") tally.bids += 1;
        tally.adUnits.add(unit.code);
        tallies.set(bid.bidder, tally);
      }
    }
  }
  const bidders: BidderDiagnosis[] = [...tallies.entries()]
    .sort(([a], [b]) => a.localeCompare(b))
    .map(([bidder, tally]) => ({
      bidder,
      requests: tally.requests,
      bids: tally.bids,
      checks: checksFor(bidder, tally.adUnits.size, sellersJson),
    }));
  const listed = bidders.filter(({ bidder }) => bidder in AD_SYSTEMS && bidder !== MISSING_FROM_ADS_TXT).length;
  return {
    adsTxt: {
      url: adsTxtUrl(pageUrl),
      status: "found",
      detail: null,
      // The partners on the page plus the resellers every real file carries.
      recordCount: listed + 37,
      invalidLineCount: 0,
      invalidLines: [],
      ownerDomain: null,
      managerDomains: [],
    },
    sellersJsonChecked: sellersJson,
    consent: {
      cmpPresent: true,
      consentManagementConfigured: true,
      gdprApplies: true,
      resolvedAtMs: Math.round(site.pageLoadMs * 0.2),
      auctionCount: site.auctions.length,
      auctionsBeforeConsent: 0,
      consentedPurposes: [1, 2, 3, 4, 7, 9, 10],
    },
    bidders,
  };
}
