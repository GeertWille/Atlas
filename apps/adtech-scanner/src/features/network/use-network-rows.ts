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
  error: unknown;
  retry: () => void;
};

/**
 * The request list of a scan. Chunks are asked for one after the other until the list is complete,
 * so the first rows can be drawn and filtered while the rest arrive.
 */
export function useNetworkRows(scanId: string, enabled: boolean): NetworkRows {
  const query = useNetworkRowChunks(scanId, enabled);
  const { hasNextPage, isFetchingNextPage, fetchNextPage } = query;

  useEffect(() => {
    if (hasNextPage && !isFetchingNextPage) void fetchNextPage();
  }, [hasNextPage, isFetchingNextPage, fetchNextPage]);

  const pages = query.data?.pages;
  const rows = useMemo(() => (pages ?? []).flatMap((page) => page.rows.map(indexRow)), [pages]);
  const byId = useMemo(() => new Map(rows.map((indexed) => [indexed.row.id, indexed])), [rows]);
  const total = pages?.[0]?.total ?? 0;
  return {
    rows,
    byId,
    total,
    isPending: enabled && query.isPending,
    isLoadingMore: Boolean(hasNextPage),
    cut: !hasNextPage && rows.length >= NETWORK_ROW_LIMIT && total > rows.length,
    error: query.error,
    retry: () => void query.refetch(),
  };
}
