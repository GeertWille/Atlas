import type { ScriptCategory, ScriptRecord } from "@/bindings";

export type ScriptScope = "all" | "adRelated" | "analyzed";
export type ScriptSort = "loadOrder" | "sizeDesc";

export type ScriptFilters = {
  search: string;
  /** Empty means every category. */
  categories: ScriptCategory[];
  scope: ScriptScope;
};

export const NO_SCRIPT_FILTERS: ScriptFilters = { search: "", categories: [], scope: "all" };

function matchesSearch(script: ScriptRecord, needle: string): boolean {
  return [script.url, script.vendor, script.classificationReason].some((field) =>
    field?.toLowerCase().includes(needle),
  );
}

export function filterScripts(scripts: ScriptRecord[], filters: ScriptFilters): ScriptRecord[] {
  const needle = filters.search.trim().toLowerCase();
  return scripts.filter(
    (script) =>
      (needle === "" || matchesSearch(script, needle)) &&
      (filters.categories.length === 0 || filters.categories.includes(script.category)) &&
      (filters.scope === "all" || (filters.scope === "adRelated" ? script.adRelated : script.analyzed)),
  );
}

export function sortScripts(scripts: ScriptRecord[], sort: ScriptSort): ScriptRecord[] {
  return [...scripts].sort((a, b) =>
    sort === "sizeDesc"
      ? b.sizeBytes - a.sizeBytes
      : (a.loadedAtMs ?? Number.POSITIVE_INFINITY) - (b.loadedAtMs ?? Number.POSITIVE_INFINITY),
  );
}

export type FunnelStage = { key: ScriptScope; label: string; count: number; bytes: number; shareOfBytes: number };

/** How Phase 1 narrows the volume: everything loaded → ad-related → handed to Phase 2. */
export function scriptFunnel(scripts: ScriptRecord[]): FunnelStage[] {
  const bytesOf = (subset: ScriptRecord[]) => subset.reduce((sum, script) => sum + script.sizeBytes, 0);
  const totalBytes = bytesOf(scripts);
  const stage = (key: ScriptScope, label: string, subset: ScriptRecord[]): FunnelStage => {
    const bytes = bytesOf(subset);
    return { key, label, count: subset.length, bytes, shareOfBytes: totalBytes === 0 ? 0 : bytes / totalBytes };
  };
  return [
    stage("all", "Scripts loaded", scripts),
    stage(
      "adRelated",
      "Ad-related",
      scripts.filter((script) => script.adRelated),
    ),
    stage(
      "analyzed",
      "Analysed in Phase 2",
      scripts.filter((script) => script.analyzed),
    ),
  ];
}
