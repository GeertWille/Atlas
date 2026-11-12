import type { Finding, ScriptRecord, Severity } from "@/bindings";
import { fileNameOf } from "@/shared/lib/url";
import { worstSeverity } from "./anchors";

export type FileGroupKey = "analysed" | "adRelated" | "other";

export const FILE_GROUP_LABEL: Record<FileGroupKey, string> = {
  analysed: "Analysed",
  adRelated: "Ad-related",
  other: "Other",
};

export type FileEntry = {
  script: ScriptRecord;
  name: string;
  /** Who it belongs to: the vendor when known, else where it was loaded from. */
  origin: string;
  findingCount: number;
  worst: Severity | null;
};

export type FileGroup = { key: FileGroupKey; files: FileEntry[] };

function groupOf(script: ScriptRecord): FileGroupKey {
  if (script.analyzed) return "analysed";
  return script.adRelated ? "adRelated" : "other";
}

/** Findings that cite code in a script: by its id, or for reports stored before ids, by its URL. */
export function findingsCiting(script: ScriptRecord, findings: Finding[]): Finding[] {
  return findings.filter((finding) =>
    finding.evidence.some(
      (evidence) =>
        evidence.kind === "code" &&
        (evidence.scriptId === script.id || (evidence.scriptId === null && evidence.scriptUrl === script.url)),
    ),
  );
}

function entry(script: ScriptRecord, findings: Finding[]): FileEntry {
  const citing = findingsCiting(script, findings);
  return {
    script,
    name: fileNameOf(script.url),
    origin: script.vendor ?? (script.inline ? "inline" : script.hostname),
    findingCount: citing.length,
    worst: worstSeverity(citing.map((finding) => finding.severity)),
  };
}

/**
 * The scripts of a scan as the viewer lists them: what the AI analysed first, then other ad code, then
 * the rest. Inside a group, files with findings lead; otherwise load order is kept. `filter` narrows by
 * name, URL or vendor; a group without matches is left out.
 */
export function groupFiles(scripts: ScriptRecord[], findings: Finding[], filter = ""): FileGroup[] {
  const needle = filter.trim().toLowerCase();
  const matches = (file: FileEntry) =>
    needle === "" ||
    [file.script.url, file.name, file.script.vendor].some((field) => field?.toLowerCase().includes(needle));
  const groups: Record<FileGroupKey, FileEntry[]> = { analysed: [], adRelated: [], other: [] };
  for (const script of scripts) {
    const file = entry(script, findings);
    if (matches(file)) groups[groupOf(script)].push(file);
  }
  return (["analysed", "adRelated", "other"] as const)
    .map((key) => ({ key, files: [...groups[key]].sort((a, b) => b.findingCount - a.findingCount) }))
    .filter((group) => group.files.length > 0);
}

export type FileListItem =
  | { kind: "group"; key: FileGroupKey; count: number; collapsed: boolean }
  | { kind: "file"; file: FileEntry };

/** The groups flattened for a virtual list; a collapsed group keeps its heading and hides its files. */
export function flattenGroups(groups: FileGroup[], collapsed: ReadonlySet<FileGroupKey>): FileListItem[] {
  return groups.flatMap((group): FileListItem[] => {
    const isCollapsed = collapsed.has(group.key);
    return [
      { kind: "group", key: group.key, count: group.files.length, collapsed: isCollapsed },
      ...(isCollapsed ? [] : group.files.map((file): FileListItem => ({ kind: "file", file }))),
    ];
  });
}
