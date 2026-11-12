import { keepPreviousData, useInfiniteQuery, useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import type {
  ApiProvider,
  ExportFormat,
  ScanListQuery,
  ScanOptions,
  ScanSummary,
  Settings,
  SettingsPatch,
} from "@/bindings";
import { api } from "@/lib/api";
import { queryKeys } from "./keys";
import { fetchScanDetail, fetchScanLogs } from "./scan-events";

export function useSettings() {
  return useQuery({ queryKey: queryKeys.settings, queryFn: api.getSettings, staleTime: Number.POSITIVE_INFINITY });
}

const SETTINGS_UPDATE_KEY = ["settings", "update"] as const;

/**
 * Saves one patch and shows it at once. Each field of the settings screen saves on its own, so several
 * saves are often under way together, and a response describes the settings as they were when that
 * save was applied: it can lack what a later save already shows. The cache therefore takes nothing
 * from the backend while another save is under way; the last one to finish reloads the stored settings.
 */
export function useUpdateSettings() {
  const client = useQueryClient();
  const isLastSaveUnderWay = () => client.isMutating({ mutationKey: SETTINGS_UPDATE_KEY }) === 1;
  return useMutation({
    mutationKey: SETTINGS_UPDATE_KEY,
    mutationFn: (patch: SettingsPatch) => api.updateSettings(patch),
    onMutate: async (patch) => {
      await client.cancelQueries({ queryKey: queryKeys.settings });
      const { chromePath, ...rest } = patch;
      client.setQueryData<Settings>(
        queryKeys.settings,
        (current) =>
          current && { ...current, ...rest, chromePath: chromePath === undefined ? current.chromePath : chromePath },
      );
    },
    onSuccess: (settings, patch) => {
      if (isLastSaveUnderWay()) client.setQueryData(queryKeys.settings, settings);
      if (patch.chromePath !== undefined) void client.invalidateQueries({ queryKey: queryKeys.browser });
    },
    onSettled: () => {
      if (isLastSaveUnderWay()) void client.invalidateQueries({ queryKey: queryKeys.settings });
    },
  });
}

export function useSetApiKey() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: ({ key, provider }: { key: string; provider: ApiProvider }) => api.setApiKey(key, provider),
    onSuccess: (status) => {
      if (status.valid) void client.invalidateQueries({ queryKey: queryKeys.settings });
    },
  });
}

export function useClearApiKey() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (provider: ApiProvider) => api.clearApiKey(provider),
    onSuccess: () => client.invalidateQueries({ queryKey: queryKeys.settings }),
  });
}

export function useBrowser() {
  return useQuery({ queryKey: queryKeys.browser, queryFn: api.detectBrowser });
}

export function useDemoUrl() {
  return useQuery({ queryKey: queryKeys.demoUrl, queryFn: api.getDemoUrl, staleTime: Number.POSITIVE_INFINITY });
}

export function useDashboardStats() {
  return useQuery({ queryKey: queryKeys.stats, queryFn: api.getDashboardStats });
}

function useScanList(query: ScanListQuery) {
  return useQuery({ queryKey: queryKeys.scanList(query), queryFn: () => api.listScans(query) });
}

/** The newest scans, shared by the sidebar, the dashboard and the command palette. */
export function useRecentScans() {
  return useScanList({ limit: 8 });
}

/** Scans that are running or waiting for a free slot, running ones first. */
export function useActiveScans(): ScanSummary[] {
  const running = useScanList({ status: "running" });
  const queued = useScanList({ status: "queued" });
  return [...(running.data?.items ?? []), ...(queued.data?.items ?? [])];
}

const HISTORY_PAGE_SIZE = 20;

export function useScanHistory(filters: Omit<ScanListQuery, "limit" | "offset">) {
  return useInfiniteQuery({
    queryKey: queryKeys.scanHistory(filters),
    queryFn: ({ pageParam }) => api.listScans({ ...filters, limit: HISTORY_PAGE_SIZE, offset: pageParam }),
    initialPageParam: 0,
    getNextPageParam: (lastPage, pages) => {
      const loaded = pages.reduce((sum, page) => sum + page.items.length, 0);
      return lastPage.items.length > 0 && loaded < lastPage.total ? loaded : undefined;
    },
    placeholderData: keepPreviousData,
  });
}

export function useScan(scanId: string) {
  return useQuery({
    queryKey: queryKeys.scanDetail(scanId),
    queryFn: ({ signal }) => fetchScanDetail(scanId, () => api.getScan(scanId), signal),
  });
}

export function useScanLogs(scanId: string) {
  return useQuery({
    queryKey: queryKeys.scanLogs(scanId),
    queryFn: () => fetchScanLogs(scanId, () => api.getLogs(scanId)),
  });
}

export function useLogSearch(query: string) {
  return useQuery({
    queryKey: queryKeys.logSearch(query),
    queryFn: () => api.searchLogs(query, 200),
    enabled: query.length >= 2,
    placeholderData: keepPreviousData,
  });
}

export function useStartScan() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (options: ScanOptions) => api.startScan(options),
    onSuccess: () => {
      void client.invalidateQueries({ queryKey: queryKeys.scanLists });
      void client.invalidateQueries({ queryKey: queryKeys.stats });
    },
  });
}

export function useCancelScan() {
  return useMutation({ mutationFn: (scanId: string) => api.cancelScan(scanId) });
}

export function useDeleteScan() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (scanId: string) => api.deleteScan(scanId),
    onSuccess: (_result, scanId) => {
      client.removeQueries({ queryKey: queryKeys.scanDetail(scanId) });
      client.removeQueries({ queryKey: queryKeys.scanLogs(scanId) });
      void client.invalidateQueries({ queryKey: queryKeys.scanLists });
      void client.invalidateQueries({ queryKey: queryKeys.stats });
      void client.invalidateQueries({ queryKey: queryKeys.logSearches });
    },
  });
}

export function useDeleteAllScans() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: api.deleteAllScans,
    onSuccess: () => {
      client.removeQueries({ queryKey: queryKeys.scans, type: "inactive" });
      void client.invalidateQueries({ queryKey: queryKeys.scans });
      void client.invalidateQueries({ queryKey: queryKeys.stats });
      void client.invalidateQueries({ queryKey: queryKeys.logSearches });
    },
  });
}

/** Asks where to save, then has the backend write the document there. Resolves to `null` when cancelled. */
export function useExportScan() {
  return useMutation({
    mutationFn: async ({ scanId, format, fileName }: { scanId: string; format: ExportFormat; fileName: string }) => {
      const path = await api.pickExportPath(fileName, format);
      if (path === null) return null;
      await api.exportScan(scanId, format, path);
      return path;
    },
  });
}
