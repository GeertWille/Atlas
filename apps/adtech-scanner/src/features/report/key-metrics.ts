import type { AdSlot, Auction, ScanMetrics } from "@/bindings";
import { currenciesOf, currencyList } from "@/shared/domain/prices";
import {
  EMPTY,
  formatBytes,
  formatCpm,
  formatDuration,
  formatInteger,
  formatPercent,
  pluralize,
} from "@/shared/lib/format";

export type KeyMetric = { label: string; value: string; hint: string };

/**
 * The eight headline numbers of a report, each with one line of context. The counts cover every page
 * view of the visit; with more than one (`multiPage`) the three timings say that they are the entry page's.
 */
export function buildKeyMetrics(
  metrics: ScanMetrics,
  slots: AdSlot[],
  multiPage = false,
  /** For the currency of the winning bids; without them the average reads as dollars. */
  auctions: Auction[] = [],
): KeyMetric[] {
  const sinceStart = multiPage ? "on the entry page" : "after navigation start";
  const requestedSlots = slots.filter((slot) => slot.requestedAtMs !== null).length;
  const neverRequested = slots.length - requestedSlots;
  // The backend reports the mean and the sum of the winning CPMs; their ratio is the number of winners.
  const winningBids = metrics.avgWinningCpm ? Math.round(metrics.totalWinningCpm / metrics.avgWinningCpm) : 0;
  // The backend gives no average when the winners are in more than one currency: say which, instead of a number.
  const winningCurrencies = currenciesOf(
    auctions.flatMap((auction) =>
      auction.adUnits.flatMap((unit) => unit.bids.filter((bid) => bid.bidder === unit.winningBidder)),
    ),
  );
  return [
    {
      label: "Requested slots filled",
      value: requestedSlots > 0 ? formatPercent(metrics.fillRate) : EMPTY,
      // Slots that never made an ad request are outside this rate, and usually the bigger leak: name them first.
      hint:
        neverRequested > 0
          ? `${neverRequested} of ${slots.length} never requested`
          : `${metrics.emptySlotCount} of ${pluralize(slots.length, "slot")} empty`,
    },
    {
      label: "Bid timeout rate",
      value: metrics.bidRequestCount > 0 ? formatPercent(metrics.timeoutRate) : EMPTY,
      hint: `${metrics.bidTimeoutCount} of ${pluralize(metrics.bidRequestCount, "request")}`,
    },
    {
      label: "Avg winning CPM",
      value: formatCpm(metrics.avgWinningCpm, winningCurrencies[0] ?? "USD"),
      hint:
        winningCurrencies.length > 1
          ? `winning bids in ${currencyList(winningCurrencies)}: not averaged`
          : `across ${pluralize(winningBids, "winning bid")}`,
    },
    {
      label: "Auctions",
      value: formatInteger(metrics.auctionCount),
      hint: `${pluralize(metrics.adUnitCount, "ad unit")}, ${pluralize(metrics.bidderCount, "bidder")}`,
    },
    {
      label: "First auction",
      value: formatDuration(metrics.timeToFirstAuctionMs),
      hint:
        metrics.prebidTimeoutMs === null
          ? sinceStart
          : `${multiPage ? "entry page, " : ""}timeout ${formatDuration(metrics.prebidTimeoutMs)}`,
    },
    {
      label: "First ad rendered",
      value: formatDuration(metrics.timeToFirstAdMs),
      hint: sinceStart,
    },
    {
      label: "Page load",
      value: formatDuration(metrics.pageLoadMs),
      hint: multiPage ? "entry page load event" : "load event",
    },
    {
      label: "Scripts",
      value: formatInteger(metrics.scriptCount),
      hint: `${metrics.adScriptCount} ad-related, ${formatBytes(metrics.totalScriptBytes)}`,
    },
  ];
}

export function detectedStack(metrics: ScanMetrics, prebidVersion: string | null): string[] {
  return [
    metrics.hasPrebid ? `Prebid.js${prebidVersion ? ` ${prebidVersion}` : ""}` : null,
    metrics.hasGpt ? "Google Publisher Tag" : null,
    metrics.hasAmazonTam ? "Amazon TAM" : null,
  ].filter((name): name is string => name !== null);
}
