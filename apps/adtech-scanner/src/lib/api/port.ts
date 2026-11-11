import type {
  ApiKeyStatus,
  BrowserInfo,
  DashboardStats,
  ExportFormat,
  LogEntry,
  LogSearchHit,
  ScanDetail,
  ScanEvent,
  ScanList,
  ScanListQuery,
  ScanOptions,
  ScanSummary,
  Settings,
  SettingsPatch,
} from "@/bindings";

export type Unsubscribe = () => void;

/**
 * Everything the UI may ask of the outside world. Two adapters implement it:
 * the Tauri command layer and the in-browser fake backend.
 */
export interface ApiPort {
  getSettings(): Promise<Settings>;
  updateSettings(patch: SettingsPatch): Promise<Settings>;
  setApiKey(key: string): Promise<ApiKeyStatus>;
  clearApiKey(): Promise<void>;
  detectBrowser(): Promise<BrowserInfo | null>;
  getDemoUrl(): Promise<string>;
  startScan(options: ScanOptions): Promise<ScanSummary>;
  cancelScan(scanId: string): Promise<void>;
  listScans(query: ScanListQuery): Promise<ScanList>;
  getScan(scanId: string): Promise<ScanDetail>;
  deleteScan(scanId: string): Promise<void>;
  deleteAllScans(): Promise<void>;
  getLogs(scanId: string, stepId?: string): Promise<LogEntry[]>;
  searchLogs(query: string, limit?: number): Promise<LogSearchHit[]>;
  getDashboardStats(): Promise<DashboardStats>;
  exportScan(scanId: string, format: ExportFormat, path?: string): Promise<string>;
  subscribeScanEvents(handler: (event: ScanEvent) => void): Promise<Unsubscribe>;

  /** Native save dialog. Resolves to `null` when the user cancels. */
  pickExportPath(defaultFileName: string, format: ExportFormat): Promise<string | null>;
  /** Native open dialog for a Chrome/Chromium executable. */
  pickBrowserExecutable(): Promise<string | null>;
  openExternal(url: string): Promise<void>;
}
