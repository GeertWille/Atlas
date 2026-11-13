import type { BodySide, ScanListQuery, SourceView } from "@/bindings";

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
  /** Everything read from one scan's archive: its network record and its script sources. */
  scanArchive: (scanId: string) => ["scans", "archive", scanId] as const,
  networkSummary: (scanId: string) => ["scans", "archive", scanId, "summary"] as const,
  networkRows: (scanId: string) => ["scans", "archive", scanId, "rows"] as const,
  networkRequest: (scanId: string, requestId: string) => ["scans", "archive", scanId, "request", requestId] as const,
  networkBody: (scanId: string, requestId: string, side: BodySide) =>
    ["scans", "archive", scanId, "body", requestId, side] as const,
  scriptSource: (scanId: string, scriptId: string, view: SourceView) =>
    ["scans", "archive", scanId, "source", scriptId, view] as const,
  archiveStorage: ["archive-storage"] as const,
  sellersJsonCache: ["sellers-json-cache"] as const,
  scanProfile: ["scan-profile"] as const,
  browserbaseProfile: ["browserbase-profile"] as const,
  configurations: ["configurations"] as const,
  logSearches: ["log-search"] as const,
  logSearch: (query: string) => ["log-search", query] as const,
};
