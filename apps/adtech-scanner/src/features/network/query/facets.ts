import type { NetworkCategory } from "@/bindings";
import { STATE_TOKENS, type StateChip, withoutCategory, withoutToken } from "./edit";
import { compileQuery, type IndexedRow, type MatchContext } from "./match";
import { parseQuery } from "./parse";

export type CategoryCounts = { all: number; byCategory: Partial<Record<NetworkCategory, number>> };

/**
 * How many requests each category chip would show: counted with every other part of the filter
 * applied, and the category itself left out, so the counts say what a click on another chip gives.
 */
export function categoryCounts(rows: IndexedRow[], query: string, context: MatchContext): CategoryCounts {
  const matches = compileQuery(parseQuery(withoutCategory(query)), context);
  const byCategory: Partial<Record<NetworkCategory, number>> = {};
  let all = 0;
  for (const indexed of rows) {
    if (!matches(indexed)) continue;
    all += 1;
    byCategory[indexed.row.category] = (byCategory[indexed.row.category] ?? 0) + 1;
  }
  return { all, byCategory };
}

/** The same for the state chips: each counted with its own word left out of the filter. */
export function stateCounts(rows: IndexedRow[], query: string, context: MatchContext): Record<StateChip, number> {
  const count = (chip: StateChip) => {
    const token = STATE_TOKENS[chip];
    const others = compileQuery(parseQuery(withoutToken(query, token)), context);
    const own = compileQuery(parseQuery(token), context);
    return rows.reduce((sum, indexed) => sum + (others(indexed) && own(indexed) ? 1 : 0), 0);
  };
  return { failed: count("failed"), slow: count("slow"), blocked: count("blocked") };
}

export type ListSummary = {
  shown: number;
  total: number;
  shownBytes: number;
  totalBytes: number;
  failed: number;
  lateBids: number;
  /** Share of the shown requests that did not come from the network, 0 to 1. */
  cachedShare: number;
  /** The vendors with most of the shown requests, largest first. */
  vendors: { vendor: string; requests: number }[];
};

const MAX_VENDORS = 4;

/** The footer of the list: what is shown, out of what. */
export function summariseList(shown: IndexedRow[], all: IndexedRow[]): ListSummary {
  const vendors = new Map<string, number>();
  let shownBytes = 0;
  let failed = 0;
  let lateBids = 0;
  let cached = 0;
  for (const { row } of shown) {
    shownBytes += row.transferredBytes;
    if (row.outcome === "failed" || row.outcome === "httpError") failed += 1;
    if (row.category === "bid" && row.ad?.verdict === "lateOnNetwork") lateBids += 1;
    if (row.servedFrom !== "network") cached += 1;
    if (row.vendor) vendors.set(row.vendor, (vendors.get(row.vendor) ?? 0) + 1);
  }
  return {
    shown: shown.length,
    total: all.length,
    shownBytes,
    totalBytes: all.reduce((sum, { row }) => sum + row.transferredBytes, 0),
    failed,
    lateBids,
    cachedShare: shown.length === 0 ? 0 : cached / shown.length,
    vendors: [...vendors.entries()]
      .map(([vendor, requests]) => ({ vendor, requests }))
      .sort((a, b) => b.requests - a.requests || a.vendor.localeCompare(b.vendor))
      .slice(0, MAX_VENDORS),
  };
}
