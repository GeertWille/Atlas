import type {
  ApiKeyStatus,
  ApiProvider,
  ArchiveStorage,
  BodySide,
  BrowserInfo,
  DashboardStats,
  ExportFormat,
  LogEntry,
  LogSearchHit,
  NetworkBodyChunk,
  NetworkRequestDetail,
  NetworkRowChunk,
  NetworkSummary,
  ProxyProfileDraft,
  ScanConfiguration,
  ScanConfigurationDraft,
  ScanDetail,
  ScanEvent,
  ScanList,
  ScanListQuery,
  ScanOptions,
  ScanProfileInfo,
  ScanSummary,
  ScriptSource,
  Settings,
  SettingsPatch,
  SourceView,
} from "@/bindings";

export type Unsubscribe = () => void;

export type NetworkRowsQuery = {
  /** Page views to list; omitted for the whole visit. */
  pages?: number[];
  /** `nextCursor` of the previous chunk. */
  cursor?: string;
  limit?: number;
};

export type NetworkBodyQuery = {
  side: BodySide;
  /** Byte offset into the stored body. */
  offset?: number;
  maxBytes?: number;
};

/** A position in a script as captured (1-based), to be mapped onto the text that is returned. */
export type SourcePosition = { line: number; column?: number };

/**
 * Everything the UI may ask of the outside world. Two adapters implement it:
 * the Tauri command layer and the in-browser fake backend.
 */
export interface ApiPort {
  getSettings(): Promise<Settings>;
  updateSettings(patch: SettingsPatch): Promise<Settings>;
  /** `provider` defaults to Anthropic. The key is stored only when the provider accepts it. */
  setApiKey(key: string, provider?: ApiProvider): Promise<ApiKeyStatus>;
  clearApiKey(provider?: ApiProvider): Promise<void>;
  detectBrowser(): Promise<BrowserInfo | null>;
  getDemoUrl(): Promise<string>;
  startScan(options: ScanOptions): Promise<ScanSummary>;
  /** Starts a scan with the saved options of a configuration, and records the scan as its last run. */
  runScanConfiguration(configurationId: string): Promise<ScanSummary>;
  cancelScan(scanId: string): Promise<void>;
  /** Ends the recording of a manual visit and keeps what was captured. Does nothing outside the capture step. */
  finishCapture(scanId: string): Promise<void>;
  listScans(query: ScanListQuery): Promise<ScanList>;
  getScan(scanId: string): Promise<ScanDetail>;
  deleteScan(scanId: string): Promise<void>;
  deleteAllScans(): Promise<void>;
  getLogs(scanId: string, stepId?: string): Promise<LogEntry[]>;
  searchLogs(query: string, limit?: number): Promise<LogSearchHit[]>;
  getDashboardStats(): Promise<DashboardStats>;
  exportScan(scanId: string, format: ExportFormat, path?: string): Promise<string>;
  getScanProfile(): Promise<ScanProfileInfo>;
  /** Fails with `profileBusy` while a scan is using the profile. */
  clearScanProfile(): Promise<ScanProfileInfo>;
  /** Creates a proxy profile, or with `id` changes one. The password is set separately. */
  saveProxyProfile(profile: ProxyProfileDraft): Promise<Settings>;
  /** Refused while a saved scan configuration uses the profile. */
  deleteProxyProfile(id: string): Promise<Settings>;
  setProxyPassword(id: string, password: string): Promise<Settings>;
  clearProxyPassword(id: string): Promise<Settings>;
  listScanConfigurations(): Promise<ScanConfiguration[]>;
  saveScanConfiguration(configuration: ScanConfigurationDraft): Promise<ScanConfiguration>;
  /** Scans started from the configuration are kept. */
  deleteScanConfiguration(id: string): Promise<void>;
  /** The digest of a scan's network record. Answers for every scan; `archive.availability` says why it is empty. */
  getNetworkSummary(scanId: string): Promise<NetworkSummary>;
  /** One chunk of the request list, ordered by page view, start time and sequence. */
  listNetworkRequests(scanId: string, query?: NetworkRowsQuery): Promise<NetworkRowChunk>;
  getNetworkRequest(scanId: string, requestId: string): Promise<NetworkRequestDetail>;
  /** A stretch of a stored body as text. `text` is `null` whenever the state is not `kept` or `truncated`. */
  getNetworkBody(scanId: string, requestId: string, query: NetworkBodyQuery): Promise<NetworkBodyChunk>;
  /** A script's text from the scan's archive. Unavailability is data (`availability`), never an error. */
  getScriptSource(scanId: string, scriptId: string, view: SourceView, original?: SourcePosition): Promise<ScriptSource>;
  getArchiveStorage(): Promise<ArchiveStorage>;
  /** Removes the network data of every finished scan. The reports stay. */
  clearArchive(): Promise<ArchiveStorage>;
  subscribeScanEvents(handler: (event: ScanEvent) => void): Promise<Unsubscribe>;

  /** Native save dialog. Resolves to `null` when the user cancels. */
  pickExportPath(defaultFileName: string, format: ExportFormat): Promise<string | null>;
  /** Native open dialog for a Chrome/Chromium executable. */
  pickBrowserExecutable(): Promise<string | null>;
  openExternal(url: string): Promise<void>;
}
