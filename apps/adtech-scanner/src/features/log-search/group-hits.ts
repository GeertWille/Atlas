import type { LogSearchHit } from "@/bindings";

export type HitGroup = {
  scanId: string;
  scanUrl: string;
  /** Timestamp of the earliest matching entry: tells apart several scans of the same URL. */
  firstEntryAt: number;
  hits: LogSearchHit[];
};

/** Groups hits by scan, keeping the backend's relevance order within and between groups. */
export function groupHitsByScan(hits: LogSearchHit[]): HitGroup[] {
  const groups = new Map<string, HitGroup>();
  for (const hit of hits) {
    const { scanId } = hit.entry;
    const group = groups.get(scanId) ?? { scanId, scanUrl: hit.scanUrl, firstEntryAt: hit.entry.ts, hits: [] };
    group.hits.push(hit);
    group.firstEntryAt = Math.min(group.firstEntryAt, hit.entry.ts);
    groups.set(scanId, group);
  }
  return [...groups.values()];
}
