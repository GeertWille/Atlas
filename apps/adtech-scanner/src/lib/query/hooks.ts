import {
  keepPreviousData,
  useInfiniteQuery,
  useMutation,
  useQueries,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";
import type {
  ApiProvider,
  BodySide,
  ExportFormat,
  ProxyProfileDraft,
  ScanConfigurationDraft,
  ScanListQuery,
  ScanOptions,
  ScanSummary,
  Settings,
  SettingsPatch,
  SourceView,
} from "@/bindings";
import { api } from "@/lib/api";
import type { SourcePosition } from "@/lib/api/port";
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

/**
 * The archive of a finished scan only changes when it is evicted or cleared, and both invalidate
 * these queries: there is nothing to refetch in between. Large answers are dropped from memory a few
 * minutes after their screen closes.
 */
const ARCHIVE_QUERY = { staleTime: Number.POSITIVE_INFINITY, gcTime: 5 * 60_000, structuralSharing: false } as const;

export function useNetworkSummary(scanId: string, enabled = true) {
  return useQuery({
    queryKey: queryKeys.networkSummary(scanId),
    queryFn: () => api.getNetworkSummary(scanId),
    enabled,
    ...ARCHIVE_QUERY,
  });
}

const NETWORK_CHUNK_ROWS = 2000;
/** Beyond this many requests the list is cut, and the screen says so. */
export const NETWORK_ROW_LIMIT = 50_000;

/** The request list of a scan, in chunks so the first rows paint before the last arrive. */
export function useNetworkRowChunks(scanId: string, enabled = true) {
  return useInfiniteQuery({
    queryKey: queryKeys.networkRows(scanId),
    queryFn: ({ pageParam }) => api.listNetworkRequests(scanId, { cursor: pageParam, limit: NETWORK_CHUNK_ROWS }),
    initialPageParam: undefined as string | undefined,
    getNextPageParam: (lastPage, pages) => {
      const loaded = pages.reduce((sum, page) => sum + page.rows.length, 0);
      return loaded >= NETWORK_ROW_LIMIT ? undefined : (lastPage.nextCursor ?? undefined);
    },
    enabled,
    ...ARCHIVE_QUERY,
  });
}

export function useNetworkRequest(scanId: string, requestId: string | undefined) {
  return useQuery({
    queryKey: queryKeys.networkRequest(scanId, requestId ?? ""),
    queryFn: () => api.getNetworkRequest(scanId, requestId ?? ""),
    enabled: requestId !== undefined,
    ...ARCHIVE_QUERY,
  });
}

/** Several requests at once: the Prebid Server exchanges of an auction, whose answers say how each bidder did. */
export function useNetworkRequests(scanId: string, requestIds: string[]) {
  return useQueries({
    queries: requestIds.map((requestId) => ({
      queryKey: queryKeys.networkRequest(scanId, requestId),
      queryFn: () => api.getNetworkRequest(scanId, requestId),
      ...ARCHIVE_QUERY,
    })),
  });
}

/** A stored body, read 2 MiB at a time; the next stretch is fetched when the reader asks for more. */
export function useNetworkBody(scanId: string, requestId: string, side: BodySide, enabled = true) {
  return useInfiniteQuery({
    queryKey: queryKeys.networkBody(scanId, requestId, side),
    queryFn: ({ pageParam }) => api.getNetworkBody(scanId, requestId, { side, offset: pageParam }),
    initialPageParam: 0,
    getNextPageParam: (lastPage) => lastPage.nextOffset ?? undefined,
    enabled,
    ...ARCHIVE_QUERY,
  });
}

/** At most this much of one body is read into the window, however large it is stored. */
export const MAX_BODY_VIEW_BYTES = 16 * 1024 * 1024;

/** A stored body as one text, for copying: read stretch by stretch up to the view limit. `null` when it cannot be read. */
export async function readNetworkBody(scanId: string, requestId: string, side: BodySide): Promise<string | null> {
  let text = "";
  let offset: number | undefined = 0;
  while (offset !== undefined && offset < MAX_BODY_VIEW_BYTES) {
    const chunk = await api.getNetworkBody(scanId, requestId, { side, offset });
    if (chunk.text === null) return null;
    text += chunk.text;
    offset = chunk.nextOffset ?? undefined;
  }
  return text;
}

/** `original`: a position in the script as captured, answered as `focusLine` of the text returned. */
export function useScriptSource(scanId: string, scriptId: string, view: SourceView, original?: SourcePosition) {
  return useQuery({
    queryKey: [...queryKeys.scriptSource(scanId, scriptId, view), original?.line ?? null, original?.column ?? null],
    queryFn: () => api.getScriptSource(scanId, scriptId, view, original),
    ...ARCHIVE_QUERY,
  });
}

export function useArchiveStorage() {
  return useQuery({ queryKey: queryKeys.archiveStorage, queryFn: api.getArchiveStorage });
}

export function useClearArchive() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: api.clearArchive,
    onSuccess: (storage) => {
      client.setQueryData(queryKeys.archiveStorage, storage);
      // Every report now says that its network data is gone.
      void client.invalidateQueries({ queryKey: queryKeys.scans });
    },
  });
}

/** Scans add to it without an event to say so: asked again whenever a screen shows it. */
export function useSellersJsonCache() {
  return useQuery({ queryKey: queryKeys.sellersJsonCache, queryFn: api.getSellersJsonCache, staleTime: 0 });
}

export function useClearSellersJsonCache() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: api.clearSellersJsonCache,
    onSuccess: (cache) => client.setQueryData(queryKeys.sellersJsonCache, cache),
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

/** What to scan: options from the composer, or a saved configuration by id. */
export type ScanStart = { options: ScanOptions } | { configurationId: string };

export function useStartScan() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (start: ScanStart) =>
      "options" in start ? api.startScan(start.options) : api.runScanConfiguration(start.configurationId),
    onSuccess: (_summary, start) => {
      void client.invalidateQueries({ queryKey: queryKeys.scanLists });
      void client.invalidateQueries({ queryKey: queryKeys.stats });
      void client.invalidateQueries({ queryKey: queryKeys.scanProfile });
      // A run moves the configuration's "last run".
      if ("configurationId" in start) void client.invalidateQueries({ queryKey: queryKeys.configurations });
    },
  });
}

export function useFinishCapture() {
  return useMutation({ mutationFn: (scanId: string) => api.finishCapture(scanId) });
}

export function useScanProfile() {
  return useQuery({ queryKey: queryKeys.scanProfile, queryFn: api.getScanProfile });
}

export function useClearScanProfile() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: api.clearScanProfile,
    onSuccess: (profile) => client.setQueryData(queryKeys.scanProfile, profile),
    // A refusal means a scan is using the profile: show that state.
    onError: () => client.invalidateQueries({ queryKey: queryKeys.scanProfile }),
  });
}

/**
 * Proxy profiles are part of the settings; every change answers with the settings as they are stored.
 * While a field of the settings screen is still being saved, that answer can lack what the screen
 * already shows, so it is left out and the settings are reloaded once that save has finished.
 */
function useSettingsCommand<T>(command: (input: T) => Promise<Settings>) {
  const client = useQueryClient();
  return useMutation({
    mutationFn: command,
    onSuccess: (settings) => {
      if (client.isMutating({ mutationKey: SETTINGS_UPDATE_KEY }) === 0) {
        client.setQueryData(queryKeys.settings, settings);
      }
    },
  });
}

export function useSaveProxyProfile() {
  return useSettingsCommand((profile: ProxyProfileDraft) => api.saveProxyProfile(profile));
}

export function useDeleteProxyProfile() {
  return useSettingsCommand((id: string) => api.deleteProxyProfile(id));
}

export function useSetProxyPassword() {
  return useSettingsCommand(({ id, password }: { id: string; password: string }) => api.setProxyPassword(id, password));
}

export function useClearProxyPassword() {
  return useSettingsCommand((id: string) => api.clearProxyPassword(id));
}

export function useScanConfigurations() {
  return useQuery({ queryKey: queryKeys.configurations, queryFn: api.listScanConfigurations });
}

export function useSaveScanConfiguration() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (configuration: ScanConfigurationDraft) => api.saveScanConfiguration(configuration),
    onSuccess: () => client.invalidateQueries({ queryKey: queryKeys.configurations }),
  });
}

export function useDeleteScanConfiguration() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => api.deleteScanConfiguration(id),
    onSuccess: () => client.invalidateQueries({ queryKey: queryKeys.configurations }),
  });
}

/**
 * The two newest scans started from a configuration, newest first: the last run and the one to compare it
 * with. Kept current by the scan events like every scan list.
 */
export function useLatestConfigurationScans(configurationId: string): ScanSummary[] {
  const list = useScanList({ configurationId, limit: 2 });
  return list.data?.items ?? [];
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
      client.removeQueries({ queryKey: queryKeys.scanArchive(scanId) });
      void client.invalidateQueries({ queryKey: queryKeys.archiveStorage });
      void client.invalidateQueries({ queryKey: queryKeys.scanLists });
      void client.invalidateQueries({ queryKey: queryKeys.stats });
      void client.invalidateQueries({ queryKey: queryKeys.logSearches });
      void client.invalidateQueries({ queryKey: queryKeys.configurations });
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
      void client.invalidateQueries({ queryKey: queryKeys.archiveStorage });
      void client.invalidateQueries({ queryKey: queryKeys.stats });
      void client.invalidateQueries({ queryKey: queryKeys.logSearches });
      void client.invalidateQueries({ queryKey: queryKeys.configurations });
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
