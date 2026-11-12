import type { ScanSummary } from "@/bindings";
import { totalFindings } from "@/shared/domain/vocabulary";

export type HistorySort = "newest" | "oldest" | "healthAsc" | "findingsDesc" | "costDesc";

export const HISTORY_SORT_LABEL: Record<HistorySort, string> = {
  newest: "Newest first",
  oldest: "Oldest first",
  healthAsc: "Lowest health first",
  findingsDesc: "Most findings first",
  costDesc: "Highest cost first",
};

/** Scans without a health score (failed, cancelled, running) sort after scored ones. */
const COMPARE: Record<HistorySort, (a: ScanSummary, b: ScanSummary) => number> = {
  newest: (a, b) => b.startedAt - a.startedAt,
  oldest: (a, b) => a.startedAt - b.startedAt,
  healthAsc: (a, b) => (a.healthScore ?? Number.POSITIVE_INFINITY) - (b.healthScore ?? Number.POSITIVE_INFINITY),
  findingsDesc: (a, b) => totalFindings(b.findingCounts) - totalFindings(a.findingCounts),
  costDesc: (a, b) => b.costUsd - a.costUsd,
};

export function sortScans(scans: ScanSummary[], sort: HistorySort): ScanSummary[] {
  return [...scans].sort((a, b) => COMPARE[sort](a, b) || b.startedAt - a.startedAt);
}
