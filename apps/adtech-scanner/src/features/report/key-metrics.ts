import type { ScanMetrics } from "@/bindings";
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

/** The eight headline numbers of a report, each with one line of context. */
export function buildKeyMetrics(metrics: ScanMetrics): KeyMetric[] {
  const requestedSlots = metrics.slotCount > 0;
  return [
    {
      label: "Fill rate",
      value: requestedSlots ? formatPercent(metrics.fillRate) : EMPTY,
      hint: `${metrics.emptySlotCount} of ${pluralize(metrics.slotCount, "slot")} not filled`,
    },
    {
      label: "Bid timeout rate",
      value: metrics.bidRequestCount > 0 ? formatPercent(metrics.timeoutRate) : EMPTY,
      hint: `${metrics.bidTimeoutCount} of ${pluralize(metrics.bidRequestCount, "request")}`,
    },
    {
      label: "Avg winning CPM",
      value: formatCpm(metrics.avgWinningCpm),
      hint: `${formatCpm(metrics.totalWinningCpm)} across all winners`,
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
          ? "after navigation start"
          : `timeout ${formatDuration(metrics.prebidTimeoutMs)}`,
    },
    {
      label: "First ad rendered",
      value: formatDuration(metrics.timeToFirstAdMs),
      hint: "after navigation start",
    },
    {
      label: "Page load",
      value: formatDuration(metrics.pageLoadMs),
      hint: "load event",
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
