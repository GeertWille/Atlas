import type { InfiniteData, QueryClient } from "@tanstack/react-query";
import type { LogEntry, ScanDetail, ScanEvent, ScanList, ScanSummary } from "@/bindings";
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
      // A scan never goes back to running: a late progress update must not undo the final summary.
      if (isActive(event.summary.status) && !isActive(detail.summary.status)) return detail;
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

type MissedEvents = Map<string, ScanEvent[]>;

/** Per scan: the events delivered while its detail (or log) query is being fetched. */
const detailFetches: MissedEvents = new Map();
const logFetches: MissedEvents = new Map();

/**
 * Runs one fetch and collects the events delivered meanwhile. The response may
 * predate any of them, so the caller folds them back in. A newer fetch for the
 * same scan takes over the collection; the superseded one keeps what it had.
 */
async function fetchCollectingEvents<T>(
  fetches: MissedEvents,
  scanId: string,
  fetch: () => Promise<T>,
): Promise<{ fetched: T; missed: ScanEvent[] }> {
  const missed: ScanEvent[] = [];
  fetches.set(scanId, missed);
  try {
    return { fetched: await fetch(), missed };
  } finally {
    if (fetches.get(scanId) === missed) fetches.delete(scanId);
  }
}

function isFinish(event: ScanEvent): boolean {
  return event.type === "scanUpdated" && !isActive(event.summary.status);
}

/**
 * Fetches a scan detail that is consistent with every event delivered so far:
 * step, finding and summary events that arrived during the fetch are folded
 * into the response, and a response that still says "running" although the
 * scan finished meanwhile is fetched again, because only a later response
 * carries the report.
 */
export async function fetchScanDetail(
  scanId: string,
  fetch: () => Promise<ScanDetail>,
  signal: AbortSignal,
): Promise<ScanDetail> {
  for (;;) {
    const { fetched, missed } = await fetchCollectingEvents(detailFetches, scanId, fetch);
    const predatesFinish = isActive(fetched.summary.status) && missed.some(isFinish);
    if (!predatesFinish) return missed.filter((event) => !isFinish(event)).reduce(applyEventToDetail, fetched);
    // A cancelled fetch must not take the collection over from the fetch that replaced it.
    signal.throwIfAborted();
  }
}

/** Fetches a scan's log with the entries delivered during the fetch appended, oldest first. */
export async function fetchScanLogs(scanId: string, fetch: () => Promise<LogEntry[]>): Promise<LogEntry[]> {
  const { fetched, missed } = await fetchCollectingEvents(logFetches, scanId, fetch);
  return missed
    .flatMap((event) => (event.type === "log" ? [event.entry] : []))
    .reduce(appendLog, fetched)
    .sort((a, b) => a.ts - b.ts);
}

/** Returns the same list when the scan is not on this page. */
export function patchScanList(list: ScanList, summary: ScanSummary): ScanList {
  if (!list.items.some((item) => item.id === summary.id)) return list;
  return { ...list, items: list.items.map((item) => (item.id === summary.id ? summary : item)) };
}

type ListCache = ScanList | InfiniteData<ScanList>;

const pagesOf = (cache: ListCache): ScanList[] => ("pages" in cache ? cache.pages : [cache]);

function patchListCache(cache: ListCache, summary: ScanSummary): ListCache {
  if (!("pages" in cache)) return patchScanList(cache, summary);
  return { ...cache, pages: cache.pages.map((page) => patchScanList(page, summary)) };
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

/**
 * Marks the queries stale and refetches the mounted ones. A first load that is
 * already in flight is restarted too: its response may predate the event, and
 * nothing else would correct it afterwards.
 */
async function refetchFresh(client: QueryClient, queryKey: readonly unknown[]) {
  await client.cancelQueries({ queryKey, predicate: (query) => query.state.data === undefined });
  await client.invalidateQueries({ queryKey });
}

export function applyScanEvent(client: QueryClient, event: ScanEvent) {
  const scanId = eventScanId(event);

  if (event.type === "log") {
    logFetches.get(scanId)?.push(event);
    client.setQueryData<LogEntry[]>(queryKeys.scanLogs(scanId), (logs) => (logs ? appendLog(logs, event.entry) : logs));
    return;
  }

  detailFetches.get(scanId)?.push(event);
  const finished = isFinish(event);
  // A finished scan keeps showing its live state until the complete report has been fetched.
  if (!finished) {
    client.setQueryData<ScanDetail>(queryKeys.scanDetail(scanId), (detail) =>
      detail ? applyEventToDetail(detail, event) : detail,
    );
  }
  if (event.type !== "scanUpdated") return;

  // Lists filter by status, so patching them is only enough while the status stays the same.
  let listed = false;
  let statusChanged = false;
  for (const [key, cache] of client.getQueriesData<ListCache>({ queryKey: queryKeys.scanLists })) {
    const known =
      cache &&
      pagesOf(cache)
        .flatMap((page) => page.items)
        .find((item) => item.id === scanId);
    if (!cache || !known) continue;
    listed = true;
    statusChanged ||= known.status !== event.summary.status;
    client.setQueryData(key, patchListCache(cache, event.summary));
  }

  if (finished) {
    for (const queryKey of [queryKeys.scanDetail(scanId), queryKeys.scanLogs(scanId)]) {
      // Nobody is watching the live state: drop it, so the next visit loads the report instead of flashing it.
      client.removeQueries({ queryKey, type: "inactive", fetchStatus: "idle" });
      // The final detail carries auctions, slots, scripts and the overview.
      void client.invalidateQueries({ queryKey });
    }
    void refetchFresh(client, queryKeys.stats);
  }
  // A scan no loaded list has seen yet is new.
  if (!listed || statusChanged) void refetchFresh(client, queryKeys.scanLists);
}
