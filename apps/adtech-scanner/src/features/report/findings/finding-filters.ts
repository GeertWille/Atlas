import type { Finding, FindingCategory, FindingSource, Severity } from "@/bindings";
import { SEVERITIES } from "@/shared/domain/vocabulary";

export type FindingFilters = {
  /** Empty arrays mean "no restriction". */
  severities: Severity[];
  categories: FindingCategory[];
  source: FindingSource | "all";
};

export const NO_FINDING_FILTERS: FindingFilters = { severities: [], categories: [], source: "all" };

export function hasActiveFilters(filters: FindingFilters): boolean {
  return filters.severities.length > 0 || filters.categories.length > 0 || filters.source !== "all";
}

export function filterFindings(findings: Finding[], filters: FindingFilters): Finding[] {
  return findings.filter(
    (finding) =>
      (filters.severities.length === 0 || filters.severities.includes(finding.severity)) &&
      (filters.categories.length === 0 || filters.categories.includes(finding.category)) &&
      (filters.source === "all" || finding.source === filters.source),
  );
}

/** Most severe first; ties broken by estimated revenue at risk, then confidence. */
export function sortFindings(findings: Finding[]): Finding[] {
  return [...findings].sort(
    (a, b) =>
      SEVERITIES.indexOf(a.severity) - SEVERITIES.indexOf(b.severity) ||
      (b.estimatedRevenueImpactPct ?? -1) - (a.estimatedRevenueImpactPct ?? -1) ||
      b.confidence - a.confidence,
  );
}

export function countBy<K extends string>(findings: Finding[], key: (finding: Finding) => K): Partial<Record<K, number>> {
  const counts: Partial<Record<K, number>> = {};
  for (const finding of findings) counts[key(finding)] = (counts[key(finding)] ?? 0) + 1;
  return counts;
}
