import { type InfiniteData, QueryClient } from "@tanstack/react-query";
import { describe, expect, test } from "vitest";
import type { LogEntry, ScanDetail, ScanList } from "@/bindings";
import { makeDetail, makeFinding, makeLog, makeStep, makeSummary } from "@/test/fixtures";
import { queryKeys } from "./keys";
import {
  appendLog,
  applyEventToDetail,
  applyScanEvent,
  patchScanList,
  withLogsMissedWhileLoading,
} from "./scan-events";

describe("applyEventToDetail", () => {
  const detail = makeDetail({
    summary: makeSummary({ status: "running", costUsd: 0 }),
    steps: [makeStep({ id: "s1", status: "running" }), makeStep({ id: "s2", status: "pending" })],
  });

  test("replaces the summary", () => {
    const summary = makeSummary({ status: "running", costUsd: 0.12 });
    expect(applyEventToDetail(detail, { type: "scanUpdated", summary }).summary.costUsd).toBe(0.12);
  });

  test("replaces a step in place and keeps the order", () => {
    const step = makeStep({ id: "s1", status: "completed", summary: "done" });
    const next = applyEventToDetail(detail, { type: "stepUpdated", step });
    expect(next.steps.map((item) => [item.id, item.status])).toEqual([
      ["s1", "completed"],
      ["s2", "pending"],
    ]);
  });

  test("appends a new finding once, even if the event is delivered twice", () => {
    const finding = makeFinding({ id: "f9" });
    const once = applyEventToDetail(detail, { type: "findingAdded", finding });
    const twice = applyEventToDetail(once, { type: "findingAdded", finding });
    expect(twice.findings.map((item) => item.id)).toEqual(["f9"]);
  });

  test("does not mutate the cached detail", () => {
    applyEventToDetail(detail, { type: "findingAdded", finding: makeFinding() });
    expect(detail.findings).toEqual([]);
  });
});

test("appendLog ignores a duplicate entry", () => {
  const entry = makeLog({ id: "l1" });
  expect(appendLog([entry], entry)).toHaveLength(1);
  expect(appendLog([entry], makeLog({ id: "l2" })).map((item) => item.id)).toEqual(["l1", "l2"]);
});

test("patchScanList updates a known scan and returns the same list for an unknown one", () => {
  const list: ScanList = { items: [makeSummary({ id: "a", costUsd: 0 })], total: 1 };
  expect(patchScanList(list, makeSummary({ id: "a", costUsd: 0.2 })).items[0]?.costUsd).toBe(0.2);
  expect(patchScanList(list, makeSummary({ id: "zzz" }))).toBe(list);
});

describe("applyScanEvent", () => {
  function seededClient() {
    const client = new QueryClient();
    const running = makeSummary({ id: "scan-1", status: "running", costUsd: 0 });
    client.setQueryData<ScanDetail>(queryKeys.scanDetail("scan-1"), makeDetail({ summary: running }));
    client.setQueryData<LogEntry[]>(queryKeys.scanLogs("scan-1"), []);
    client.setQueryData<ScanList>(queryKeys.scanList({ limit: 8 }), { items: [running], total: 1 });
    client.setQueryData<InfiniteData<ScanList>>(queryKeys.scanHistory({}), {
      pages: [{ items: [running], total: 1 }],
      pageParams: [0],
    });
    return client;
  }
  const isStale = (client: QueryClient, key: readonly unknown[]) => client.getQueryState(key)?.isInvalidated;

  test("a log event lands in that scan's log cache only", () => {
    const client = seededClient();
    applyScanEvent(client, { type: "log", entry: makeLog({ id: "l1", scanId: "scan-1" }) });
    applyScanEvent(client, { type: "log", entry: makeLog({ id: "l2", scanId: "other" }) });
    expect(client.getQueryData<LogEntry[]>(queryKeys.scanLogs("scan-1"))?.map((entry) => entry.id)).toEqual(["l1"]);
    expect(client.getQueryData(queryKeys.scanLogs("other"))).toBeUndefined();
  });

  test("a running update patches detail, plain lists and infinite lists without refetching", () => {
    const client = seededClient();
    applyScanEvent(client, {
      type: "scanUpdated",
      summary: makeSummary({ id: "scan-1", status: "running", costUsd: 0.2 }),
    });
    expect(client.getQueryData<ScanDetail>(queryKeys.scanDetail("scan-1"))?.summary.costUsd).toBe(0.2);
    expect(client.getQueryData<ScanList>(queryKeys.scanList({ limit: 8 }))?.items[0]?.costUsd).toBe(0.2);
    expect(client.getQueryData<InfiniteData<ScanList>>(queryKeys.scanHistory({}))?.pages[0]?.items[0]?.costUsd).toBe(
      0.2,
    );
    expect(isStale(client, queryKeys.scanList({ limit: 8 }))).toBe(false);
  });

  test("a scan no list knows about invalidates the lists", () => {
    const client = seededClient();
    applyScanEvent(client, { type: "scanUpdated", summary: makeSummary({ id: "brand-new", status: "running" }) });
    expect(isStale(client, queryKeys.scanList({ limit: 8 }))).toBe(true);
    expect(isStale(client, queryKeys.scanDetail("scan-1"))).toBe(false);
  });

  test("a finished scan invalidates its detail, logs, the lists and the stats", () => {
    const client = seededClient();
    client.setQueryData(queryKeys.stats, { totalScans: 1 });
    applyScanEvent(client, { type: "scanUpdated", summary: makeSummary({ id: "scan-1", status: "completed" }) });
    expect(isStale(client, queryKeys.scanDetail("scan-1"))).toBe(true);
    // The cached detail still says "running": the live view stays up until the full report arrives.
    expect(client.getQueryData<ScanDetail>(queryKeys.scanDetail("scan-1"))?.summary.status).toBe("running");
    expect(isStale(client, queryKeys.scanLogs("scan-1"))).toBe(true);
    expect(isStale(client, queryKeys.scanList({ limit: 8 }))).toBe(true);
    expect(isStale(client, queryKeys.stats)).toBe(true);
  });

  test("step and finding events update a cached detail and ignore an uncached one", () => {
    const client = seededClient();
    applyScanEvent(client, { type: "findingAdded", finding: makeFinding({ id: "f1", scanId: "scan-1" }) });
    applyScanEvent(client, { type: "stepUpdated", step: makeStep({ id: "x", scanId: "not-cached" }) });
    expect(client.getQueryData<ScanDetail>(queryKeys.scanDetail("scan-1"))?.findings).toHaveLength(1);
    expect(client.getQueryData(queryKeys.scanDetail("not-cached"))).toBeUndefined();
  });

  test("log entries pushed while the log query is still loading are merged into its response", async () => {
    const client = new QueryClient();
    let respond: (logs: LogEntry[]) => void = () => {};
    const response = new Promise<LogEntry[]>((resolve) => {
      respond = resolve;
    });
    const loading = client.fetchQuery({
      queryKey: queryKeys.scanLogs("scan-9"),
      queryFn: async () => withLogsMissedWhileLoading("scan-9", await response),
    });

    // Pushed after the backend built its response, before the response reached the cache.
    applyScanEvent(client, { type: "log", entry: makeLog({ id: "l3", scanId: "scan-9", ts: 300 }) });
    applyScanEvent(client, { type: "log", entry: makeLog({ id: "l2", scanId: "scan-9", ts: 200 }) });
    respond([makeLog({ id: "l1", scanId: "scan-9", ts: 100 }), makeLog({ id: "l2", scanId: "scan-9", ts: 200 })]);

    expect((await loading).map((entry) => entry.id)).toEqual(["l1", "l2", "l3"]);
    // Nothing is left over to leak into a later load.
    expect(withLogsMissedWhileLoading("scan-9", [])).toEqual([]);
  });

  test("log entries for a scan nobody is loading are not buffered", () => {
    const client = new QueryClient();
    applyScanEvent(client, { type: "log", entry: makeLog({ id: "l1", scanId: "unwatched" }) });
    expect(withLogsMissedWhileLoading("unwatched", [])).toEqual([]);
  });
});
