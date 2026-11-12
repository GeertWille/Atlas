import { type InfiniteData, QueryClient, QueryObserver } from "@tanstack/react-query";
import { describe, expect, test, vi } from "vitest";
import type { LogEntry, ScanDetail, ScanList } from "@/bindings";
import { makeDetail, makeFinding, makeLog, makeStep, makeSummary } from "@/test/fixtures";
import { queryKeys } from "./keys";
import {
  appendLog,
  applyEventToDetail,
  applyScanEvent,
  fetchScanDetail,
  fetchScanLogs,
  patchScanList,
} from "./scan-events";

/** A backend call the test answers by hand, to place events before or after the response. */
function deferred<T>() {
  let resolve: (value: T) => void = () => {};
  let reject: (reason: unknown) => void = () => {};
  const promise = new Promise<T>((onResolve, onReject) => {
    resolve = onResolve;
    reject = onReject;
  });
  return { promise, resolve, reject };
}

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

  test("a late progress update does not put a finished scan back to running", () => {
    const finished = makeDetail({ summary: makeSummary({ status: "completed", healthScore: 71 }) });
    const late = makeSummary({ status: "running", healthScore: null });
    expect(applyEventToDetail(finished, { type: "scanUpdated", summary: late })).toBe(finished);
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

  test("a running update does not refetch lists that legitimately leave the scan out", async () => {
    const client = seededClient();
    const completedOnly = queryKeys.scanList({ status: "completed" });
    client.setQueryData<ScanList>(completedOnly, { items: [], total: 0 });
    applyScanEvent(client, {
      type: "scanUpdated",
      summary: makeSummary({ id: "scan-1", status: "running", costUsd: 0.2 }),
    });
    await Promise.resolve();
    expect(isStale(client, completedOnly)).toBe(false);
    expect(isStale(client, queryKeys.scanList({ limit: 8 }))).toBe(false);
  });

  test("a queued scan that starts running refetches the lists, which filter by status", async () => {
    const client = new QueryClient();
    const queued = makeSummary({ id: "scan-2", status: "queued" });
    client.setQueryData<ScanList>(queryKeys.scanList({ limit: 8 }), { items: [queued], total: 1 });
    client.setQueryData<ScanList>(queryKeys.scanList({ status: "running" }), { items: [], total: 0 });
    applyScanEvent(client, { type: "scanUpdated", summary: { ...queued, status: "running" } });
    await vi.waitFor(() => expect(isStale(client, queryKeys.scanList({ status: "running" }))).toBe(true));
    expect(client.getQueryData<ScanList>(queryKeys.scanList({ limit: 8 }))?.items[0]?.status).toBe("running");
  });

  test("a scan no list knows about invalidates the lists", async () => {
    const client = seededClient();
    applyScanEvent(client, { type: "scanUpdated", summary: makeSummary({ id: "brand-new", status: "running" }) });
    await vi.waitFor(() => expect(isStale(client, queryKeys.scanList({ limit: 8 }))).toBe(true));
    expect(isStale(client, queryKeys.scanDetail("scan-1"))).toBe(false);
  });

  test("a finished scan invalidates its detail, logs, the lists and the stats", async () => {
    const client = seededClient();
    client.setQueryData(queryKeys.stats, { totalScans: 1 });
    const unwatch = [queryKeys.scanDetail("scan-1"), queryKeys.scanLogs("scan-1")].map((queryKey) =>
      new QueryObserver(client, { queryKey, queryFn: () => new Promise<never>(() => {}), staleTime: 60_000 }).subscribe(
        () => {},
      ),
    );
    applyScanEvent(client, { type: "scanUpdated", summary: makeSummary({ id: "scan-1", status: "completed" }) });
    expect(isStale(client, queryKeys.scanDetail("scan-1"))).toBe(true);
    // The cached detail still says "running": the live view stays up until the full report arrives.
    expect(client.getQueryData<ScanDetail>(queryKeys.scanDetail("scan-1"))?.summary.status).toBe("running");
    expect(isStale(client, queryKeys.scanLogs("scan-1"))).toBe(true);
    await vi.waitFor(() => expect(isStale(client, queryKeys.scanList({ limit: 8 }))).toBe(true));
    expect(isStale(client, queryKeys.stats)).toBe(true);
    for (const stop of unwatch) stop();
  });

  test("a finished scan nobody is watching drops its live detail and log instead of keeping them", () => {
    const client = seededClient();
    applyScanEvent(client, { type: "scanUpdated", summary: makeSummary({ id: "scan-1", status: "failed" }) });
    expect(client.getQueryData(queryKeys.scanDetail("scan-1"))).toBeUndefined();
    expect(client.getQueryData(queryKeys.scanLogs("scan-1"))).toBeUndefined();
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
    const response = deferred<LogEntry[]>();
    const loading = client.fetchQuery({
      queryKey: queryKeys.scanLogs("scan-9"),
      queryFn: () => fetchScanLogs("scan-9", () => response.promise),
    });
    // Pushed after the backend built its response, before the response reached the cache.
    applyScanEvent(client, { type: "log", entry: makeLog({ id: "l3", scanId: "scan-9", ts: 300 }) });
    applyScanEvent(client, { type: "log", entry: makeLog({ id: "l2", scanId: "scan-9", ts: 200 }) });
    response.resolve([
      makeLog({ id: "l1", scanId: "scan-9", ts: 100 }),
      makeLog({ id: "l2", scanId: "scan-9", ts: 200 }),
    ]);
    expect((await loading).map((entry) => entry.id)).toEqual(["l1", "l2", "l3"]);
    // Nothing is left over to leak into a later load.
    expect(await fetchScanLogs("scan-9", async () => [])).toEqual([]);
  });

  test("log entries for a scan nobody is loading are not buffered", async () => {
    const client = new QueryClient();
    applyScanEvent(client, { type: "log", entry: makeLog({ id: "l1", scanId: "unwatched" }) });
    expect(await fetchScanLogs("unwatched", async () => [])).toEqual([]);
  });

  test("a log entry pushed during a refetch survives the older response", async () => {
    const client = new QueryClient();
    const key = queryKeys.scanLogs("scan-1");
    client.setQueryData<LogEntry[]>(key, [makeLog({ id: "l1", scanId: "scan-1", ts: 100 })]);
    const response = deferred<LogEntry[]>();
    const refetch = client.fetchQuery({
      queryKey: key,
      queryFn: () => fetchScanLogs("scan-1", () => response.promise),
    });
    applyScanEvent(client, { type: "log", entry: makeLog({ id: "l2", scanId: "scan-1", ts: 200 }) });
    expect(client.getQueryData<LogEntry[]>(key)).toHaveLength(2);
    response.resolve([makeLog({ id: "l1", scanId: "scan-1", ts: 100 })]);
    await refetch;
    expect(client.getQueryData<LogEntry[]>(key)?.map((entry) => entry.id)).toEqual(["l1", "l2"]);
  });
});

describe("events delivered while a scan detail is being fetched", () => {
  const key = queryKeys.scanDetail("scan-1");
  const running = makeDetail({
    summary: makeSummary({ id: "scan-1", status: "running", costUsd: 0, healthScore: null }),
    steps: [makeStep({ id: "s1", scanId: "scan-1", status: "running" })],
  });
  const report = makeDetail({
    summary: makeSummary({ id: "scan-1", status: "completed", costUsd: 0.31, healthScore: 64 }),
    steps: [makeStep({ id: "s1", scanId: "scan-1", status: "completed" })],
    overview: "Two bidders time out on every auction.",
  });

  function fetchDetail(client: QueryClient, getScan: () => Promise<ScanDetail>) {
    return client.fetchQuery({
      queryKey: key,
      queryFn: ({ signal }) => fetchScanDetail("scan-1", getScan, signal),
    });
  }

  test("step, finding and summary events survive a response that predates them", async () => {
    const client = new QueryClient();
    client.setQueryData<ScanDetail>(key, running);
    const response = deferred<ScanDetail>();
    const refetch = fetchDetail(client, () => response.promise);

    applyScanEvent(client, {
      type: "stepUpdated",
      step: makeStep({ id: "s1", scanId: "scan-1", status: "completed" }),
    });
    applyScanEvent(client, { type: "stepUpdated", step: makeStep({ id: "s2", scanId: "scan-1", status: "running" }) });
    applyScanEvent(client, { type: "findingAdded", finding: makeFinding({ id: "f1", scanId: "scan-1" }) });
    applyScanEvent(client, {
      type: "scanUpdated",
      summary: makeSummary({ id: "scan-1", status: "running", costUsd: 0.12, healthScore: null }),
    });
    // Built by the backend before any of the events above.
    response.resolve(running);
    await refetch;

    const detail = client.getQueryData<ScanDetail>(key);
    expect(detail?.steps.map((step) => [step.id, step.status])).toEqual([
      ["s1", "completed"],
      ["s2", "running"],
    ]);
    expect(detail?.findings.map((finding) => finding.id)).toEqual(["f1"]);
    expect(detail?.summary.costUsd).toBe(0.12);
  });

  test("events the response already contains are not applied twice", async () => {
    const client = new QueryClient();
    const response = deferred<ScanDetail>();
    const load = fetchDetail(client, () => response.promise);
    const finding = makeFinding({ id: "f1", scanId: "scan-1" });
    applyScanEvent(client, { type: "findingAdded", finding });
    response.resolve({ ...running, findings: [finding] });
    expect((await load).findings).toHaveLength(1);
  });

  test("a first load that predates the finish is fetched again instead of staying live for ever", async () => {
    const client = new QueryClient();
    const responses = [deferred<ScanDetail>(), deferred<ScanDetail>()];
    const getScan = vi.fn(
      () => responses[getScan.mock.calls.length - 1]?.promise ?? Promise.reject(new Error("unexpected")),
    );
    const load = fetchDetail(client, getScan);

    applyScanEvent(client, { type: "scanUpdated", summary: report.summary });
    responses[0]?.resolve(running);
    await vi.waitFor(() => expect(getScan).toHaveBeenCalledTimes(2));
    // Never shown: a running detail here would leave the live view up with nothing left to end it.
    expect(client.getQueryData(key)).toBeUndefined();

    responses[1]?.resolve(report);
    expect(await load).toEqual(report);
  });

  test("a response that already carries the report is kept, whatever arrived during the fetch", async () => {
    const client = new QueryClient();
    client.setQueryData<ScanDetail>(key, running);
    const response = deferred<ScanDetail>();
    const refetch = fetchDetail(client, () => response.promise);
    applyScanEvent(client, {
      type: "scanUpdated",
      summary: makeSummary({ id: "scan-1", status: "running", costUsd: 0.3, healthScore: null }),
    });
    applyScanEvent(client, { type: "scanUpdated", summary: report.summary });
    response.resolve(report);
    await refetch;
    expect(client.getQueryData<ScanDetail>(key)).toEqual(report);
  });

  test("when the report cannot be fetched the query fails visibly and a retry recovers", async () => {
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    client.setQueryData<ScanDetail>(key, running);
    const getScan = vi.fn<() => Promise<ScanDetail>>().mockRejectedValueOnce(new Error("database is locked"));
    const observer = new QueryObserver(client, {
      queryKey: key,
      queryFn: ({ signal }) => fetchScanDetail("scan-1", getScan, signal),
      staleTime: 60_000,
    });
    const unsubscribe = observer.subscribe(() => {});

    applyScanEvent(client, { type: "scanUpdated", summary: report.summary });
    await vi.waitFor(() => expect(observer.getCurrentResult().isError).toBe(true));
    expect(observer.getCurrentResult().data?.summary.status).toBe("running");

    getScan.mockResolvedValueOnce(report);
    await observer.refetch();
    expect(observer.getCurrentResult().isError).toBe(false);
    expect(observer.getCurrentResult().data).toEqual(report);
    unsubscribe();
  });

  test("a cancelled fetch does not take events away from the fetch that replaced it", async () => {
    const client = new QueryClient();
    const responses = [deferred<ScanDetail>(), deferred<ScanDetail>()];
    const getScan = vi.fn(
      () => responses[getScan.mock.calls.length - 1]?.promise ?? Promise.reject(new Error("unexpected")),
    );
    const cancelled = new AbortController();
    const first = fetchScanDetail("scan-1", getScan, cancelled.signal);
    applyScanEvent(client, { type: "scanUpdated", summary: report.summary });
    cancelled.abort();
    const second = fetchScanDetail("scan-1", getScan, new AbortController().signal);

    // The first response predates the finish; fetching again would start a new collection.
    responses[0]?.resolve(running);
    await expect(first).rejects.toThrow();
    applyScanEvent(client, {
      type: "stepUpdated",
      step: makeStep({ id: "s1", scanId: "scan-1", status: "completed" }),
    });
    responses[1]?.resolve(running);

    expect((await second).steps[0]?.status).toBe("completed");
    expect(getScan).toHaveBeenCalledTimes(2);
  });
});

describe("lists and stats that are still loading when a scan finishes", () => {
  test("a first load in flight is restarted, because its response may predate the finish", async () => {
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    const listKey = queryKeys.scanList({ status: "running" });
    const running = makeSummary({ id: "scan-1", status: "running" });
    const listScans = vi
      .fn<() => Promise<ScanList>>()
      .mockImplementationOnce(() => new Promise(() => {}))
      .mockResolvedValue({ items: [], total: 0 });
    const observer = new QueryObserver(client, { queryKey: listKey, queryFn: listScans });
    const unsubscribe = observer.subscribe(() => {});
    expect(listScans).toHaveBeenCalledTimes(1);

    applyScanEvent(client, { type: "scanUpdated", summary: { ...running, status: "completed" } });
    await vi.waitFor(() => expect(observer.getCurrentResult().data).toEqual({ items: [], total: 0 }));
    expect(listScans).toHaveBeenCalledTimes(2);
    unsubscribe();
  });
});
