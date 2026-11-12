import type { ScanListQuery } from "@/bindings";

export const queryKeys = {
  settings: ["settings"] as const,
  browser: ["browser"] as const,
  demoUrl: ["demo-url"] as const,
  stats: ["stats"] as const,
  scans: ["scans"] as const,
  scanLists: ["scans", "list"] as const,
  scanList: (query: ScanListQuery) => ["scans", "list", "page", query] as const,
  scanHistory: (filters: Omit<ScanListQuery, "limit" | "offset">) => ["scans", "list", "infinite", filters] as const,
  scanDetail: (scanId: string) => ["scans", "detail", scanId] as const,
  scanLogs: (scanId: string) => ["scans", "logs", scanId] as const,
  logSearch: (query: string) => ["log-search", query] as const,
};
