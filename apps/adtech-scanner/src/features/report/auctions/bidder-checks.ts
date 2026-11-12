import type { AdsTxtReport, BidderDiagnosis, BidderDiagnostics, ConsentReport, DiagnosticCheck } from "@/bindings";
import { formatDuration } from "@/shared/lib/format";

export type FlaggedBidder = {
  bidder: string;
  requests: number;
  bids: number;
  /** Problems first, then notices, in the order the scanner ranked them. */
  flagged: DiagnosticCheck[];
  passed: number;
  notChecked: number;
};

const isFlagged = (check: DiagnosticCheck) => check.status === "problem" || check.status === "notice";
const count = (diagnosis: BidderDiagnosis, status: DiagnosticCheck["status"]) =>
  diagnosis.checks.filter((check) => check.status === status).length;

/** Splits the bidders into those with something to act on and those that are fine. */
export function groupBidders(bidders: BidderDiagnosis[]): { flagged: FlaggedBidder[]; clear: string[] } {
  const flagged: FlaggedBidder[] = [];
  const clear: string[] = [];
  for (const diagnosis of bidders) {
    const found = diagnosis.checks.filter(isFlagged);
    if (found.length === 0) {
      clear.push(diagnosis.bidder);
      continue;
    }
    flagged.push({
      bidder: diagnosis.bidder,
      requests: diagnosis.requests,
      bids: diagnosis.bids,
      flagged: found,
      passed: count(diagnosis, "pass"),
      notChecked: count(diagnosis, "notChecked"),
    });
  }
  // Bidders with a problem before bidders with only a notice.
  const hasProblem = (bidder: FlaggedBidder) => bidder.flagged.some((check) => check.status === "problem");
  flagged.sort((a, b) => Number(hasProblem(b)) - Number(hasProblem(a)));
  return { flagged, clear };
}

const plural = (value: number, noun: string) => `${value} ${noun}${value === 1 ? "" : "s"}`;

export function describeAdsTxt(adsTxt: AdsTxtReport): string {
  switch (adsTxt.status) {
    case "found": {
      const invalid = adsTxt.invalidLineCount > 0 ? `, ${plural(adsTxt.invalidLineCount, "invalid line")}` : "";
      return `${plural(adsTxt.recordCount, "seller record")}${invalid}`;
    }
    case "missing":
      return "The site has no ads.txt";
    case "unreadable":
      return `Could not be read: ${adsTxt.detail ?? "no reason given"}`;
  }
}

export function describeSellersJson(diagnostics: BidderDiagnostics): string {
  return diagnostics.sellersJsonChecked
    ? "Looked up at the exchanges that ads.txt names"
    : "Not looked up for this scan";
}

export function describeConsent(consent: ConsentReport): string {
  if (!consent.cmpPresent) return "No TCF consent API on this page";
  if (consent.gdprApplies !== true) return "A consent platform is present; GDPR did not apply to this page view";
  const known =
    consent.resolvedAtMs === null
      ? "the choice was never made during the scan"
      : `choice known after ${formatDuration(consent.resolvedAtMs)}`;
  const early =
    consent.auctionsBeforeConsent > 0
      ? `, ${consent.auctionsBeforeConsent} of ${plural(consent.auctionCount, "auction")} started before that`
      : "";
  return `GDPR applies, ${known}${early}`;
}

export function describeRemainder(bidder: FlaggedBidder): string | null {
  const parts = [
    bidder.passed > 0 ? `${plural(bidder.passed, "other check")} passed` : null,
    bidder.notChecked > 0 ? `${bidder.notChecked} could not be made` : null,
  ].filter((part) => part !== null);
  return parts.length > 0 ? parts.join(", ") : null;
}
