import type { InfiniteData, QueryClient } from "@tanstack/react-query";
import type { LogEntry, ScanDetail, ScanEvent, ScanList, ScanSummary } from "@/bindings";
import type { ApiPort } from "@/lib/api";
import { isActive } from "@/shared/domain/vocabulary";
import { queryKeys } from "./keys";

function replaceById<T extends { id: string }>(items: T[], next: T): T[] {
  return items.some((item) => item.id === next.id)
    ? items.map((item) => (item.id === next.id ? next : item))
    : [...items, next];
}

/** Folds one backend event into a cached scan detail. */
export function applyEventToDetail(detail: ScanDetail, event: ScanEvent): ScanDetail {
  switch (event.type) {
    case "scanUpdated":
      return { ...detail, summary: event.summary };
    case "stepUpdated":
      return { ...detail, steps: replaceById(detail.steps, event.step) };
    case "findingAdded":
      return { ...detail, findings: replaceById(detail.findings, event.finding) };
    case "log":
      return detail;
  }
}

export function appendLog(logs: LogEntry[], entry: LogEntry): LogEntry[] {
  return logs.some((existing) => existing.id === entry.id) ? logs : [...logs, entry];
}

/** Returns the same list when the scan is not on this page, so callers can detect a miss. */
export function patchScanList(list: ScanList, summary: ScanSummary): ScanList {
  if (!list.items.some((item) => item.id === summary.id)) return list;
  return { ...list, items: list.items.map((item) => (item.id === summary.id ? summary : item)) };
}

type ListCache = ScanList | InfiniteData<ScanList>;

function patchListCache(cache: ListCache, summary: ScanSummary): ListCache {
  if (!("pages" in cache)) return patchScanList(cache, summary);
  const pages = cache.pages.map((page) => patchScanList(page, summary));
  return pages.every((page, index) => page === cache.pages[index]) ? cache : { ...cache, pages };
}

function eventScanId(event: ScanEvent): string {
  switch (event.type) {
    case "scanUpdated":
      return event.summary.id;
    case "stepUpdated":
      return event.step.scanId;
    case "findingAdded":
      return event.finding.scanId;
    case "log":
      return event.entry.scanId;
  }
}

export function applyScanEvent(client: QueryClient, event: ScanEvent) {
  const scanId = eventScanId(event);
  if (event.type === "log") {
    client.setQueryData<LogEntry[]>(queryKeys.scanLogs(scanId), (logs) =>
      logs ? appendLog(logs, event.entry) : logs,
    );
    return;
  }

  client.setQueryData<ScanDetail>(queryKeys.scanDetail(scanId), (detail) =>
    detail ? applyEventToDetail(detail, event) : detail,
  );
  if (event.type !== "scanUpdated") return;

  let knownEverywhere = true;
  for (const [key, cache] of client.getQueriesData<ListCache>({ queryKey: queryKeys.scanLists })) {
    if (!cache) continue;
    const patched = patchListCache(cache, event.summary);
    if (patched === cache) knownEverywhere = false;
    else client.setQueryData(key, patched);
  }

  const finished = !isActive(event.summary.status);
  if (finished) {
    // The final detail carries auctions, slots, scripts and the overview.
    void client.invalidateQueries({ queryKey: queryKeys.scanDetail(scanId) });
    void client.invalidateQueries({ queryKey: queryKeys.scanLogs(scanId) });
    void client.invalidateQueries({ queryKey: queryKeys.stats });
  }
  if (finished || !knownEverywhere) {
    void client.invalidateQueries({ queryKey: queryKeys.scanLists });
  }
}

/** The app's single live subscription: every screen reacts through the query cache. */
export function subscribeQueryClientToScanEvents(api: ApiPort, client: QueryClient) {
  return api.subscribeScanEvents((event) => applyScanEvent(client, event));
}
