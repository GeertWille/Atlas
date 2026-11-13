import { useEffect, useMemo } from "react";
import { NETWORK_ROW_LIMIT, useNetworkRowChunks } from "@/lib/query/hooks";
import { type IndexedRow, indexRow } from "./query/match";

export type NetworkRows = {
  rows: IndexedRow[];
  byId: Map<string, IndexedRow>;
  /** Rows of the whole scan, as the backend counts them. */
  total: number;
  /** The first chunk is not here yet. */
  isPending: boolean;
  /** More chunks are on their way. */
  isLoadingMore: boolean;
  /** The list was cut at the row limit: `total` is larger than what is loaded. */
  cut: boolean;
  /**
   * What the last fetch failed at. `load`: the first chunk, so there is no list. `nextChunk`: a later
   * one, so `rows` is the part of the list before it. `refresh`: fetching a loaded list again, so
   * `rows` is the list as it was.
   */
  failure: "load" | "nextChunk" | "refresh" | null;
  error: unknown;
  /** Repeats the fetch that failed; chunks that are loaded are not asked for again. */
  retry: () => void;
};

/**
 * The request list of a scan. Chunks are asked for one after the other until the list is complete,
 * so the first rows can be drawn and filtered while the rest arrive.
 */
export function useNetworkRows(scanId: string, enabled: boolean): NetworkRows {
  const query = useNetworkRowChunks(scanId, enabled);
  const { hasNextPage, isFetchingNextPage, fetchNextPage, isError } = query;

  const pages = query.data?.pages;
  const chunkCount = pages?.length ?? 0;
  // A chunk that is answered at once never shows up as "fetching": the next one is asked for when it has
  // arrived, which the number of chunks tells. A chunk that failed is asked for again by `retry` only.
  // biome-ignore lint/correctness/useExhaustiveDependencies: the chunk count is the trigger
  useEffect(() => {
    if (hasNextPage && !isFetchingNextPage && !isError) void fetchNextPage();
  }, [hasNextPage, isFetchingNextPage, isError, fetchNextPage, chunkCount]);

  const rows = useMemo(() => (pages ?? []).flatMap((page) => page.rows.map(indexRow)), [pages]);
  const byId = useMemo(() => new Map(rows.map((indexed) => [indexed.row.id, indexed])), [rows]);
  const total = pages?.[0]?.total ?? 0;
  let failure: NetworkRows["failure"] = null;
  if (isError) {
    if (pages === undefined) failure = "load";
    else failure = query.isFetchNextPageError ? "nextChunk" : "refresh";
  }
  return {
    rows,
    byId,
    total,
    isPending: enabled && query.isPending,
    isLoadingMore: Boolean(hasNextPage) && !isError,
    cut: !hasNextPage && rows.length >= NETWORK_ROW_LIMIT && total > rows.length,
    failure,
    error: query.error,
    retry: () => void (failure === "nextChunk" ? fetchNextPage() : query.refetch()),
  };
}
