import type { LogEntry, LogKind, LogLevel } from "@/bindings";

export type LogFilters = {
  /** `null` shows every step. */
  stepId: string | null;
  /** Empty arrays mean "no restriction". */
  kinds: LogKind[];
  levels: LogLevel[];
  search: string;
};

export const NO_LOG_FILTERS: LogFilters = { stepId: null, kinds: [], levels: [], search: "" };

function matchesSearch(entry: LogEntry, needle: string): boolean {
  if (entry.message.toLowerCase().includes(needle)) return true;
  return entry.data != null && JSON.stringify(entry.data).toLowerCase().includes(needle);
}

export function filterLogs(logs: LogEntry[], filters: LogFilters): LogEntry[] {
  const needle = filters.search.trim().toLowerCase();
  return logs.filter(
    (entry) =>
      (filters.stepId === null || entry.stepId === filters.stepId) &&
      (filters.kinds.length === 0 || filters.kinds.includes(entry.kind)) &&
      (filters.levels.length === 0 || filters.levels.includes(entry.level)) &&
      (needle === "" || matchesSearch(entry, needle)),
  );
}

export function countLogsByStep(logs: LogEntry[]): Map<string, number> {
  const counts = new Map<string, number>();
  for (const entry of logs) counts.set(entry.stepId, (counts.get(entry.stepId) ?? 0) + 1);
  return counts;
}

export function logsAsText(logs: LogEntry[]): string {
  return logs
    .map((entry) => {
      const data = entry.data == null ? "" : ` ${JSON.stringify(entry.data)}`;
      return `${new Date(entry.ts).toISOString()} ${entry.level.toUpperCase().padEnd(5)} [${entry.node}] ${entry.kind}: ${entry.message}${data}`;
    })
    .join("\n");
}
