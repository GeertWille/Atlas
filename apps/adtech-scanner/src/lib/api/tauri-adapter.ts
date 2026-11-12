import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open, save } from "@tauri-apps/plugin-dialog";
import { openUrl } from "@tauri-apps/plugin-opener";
import type { ScanEvent } from "@/bindings";
import { toApiError } from "./error";
import type { ApiPort } from "./port";

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(command, args);
  } catch (reason) {
    throw toApiError(reason);
  }
}

export function createTauriAdapter(): ApiPort {
  return {
    getSettings: () => call("get_settings"),
    updateSettings: (patch) => call("update_settings", { patch }),
    setApiKey: (key, provider) => call("set_api_key", { key, provider }),
    clearApiKey: (provider) => call("clear_api_key", { provider }),
    detectBrowser: () => call("detect_browser"),
    getDemoUrl: () => call("get_demo_url"),
    startScan: (options) => call("start_scan", { options }),
    runScanConfiguration: (configurationId) => call("start_scan", { configurationId }),
    cancelScan: (scanId) => call("cancel_scan", { scanId }),
    finishCapture: (scanId) => call("finish_capture", { scanId }),
    listScans: (query) => call("list_scans", { query }),
    getScan: (scanId) => call("get_scan", { scanId }),
    deleteScan: (scanId) => call("delete_scan", { scanId }),
    deleteAllScans: () => call("delete_all_scans"),
    getLogs: (scanId, stepId) => call("get_logs", { scanId, stepId }),
    searchLogs: (query, limit) => call("search_logs", { query, limit }),
    getDashboardStats: () => call("get_dashboard_stats"),
    exportScan: (scanId, format, path) => call("export_scan", { scanId, format, path }),
    getScanProfile: () => call("get_scan_profile"),
    clearScanProfile: () => call("clear_scan_profile"),
    saveProxyProfile: (profile) => call("save_proxy_profile", { profile }),
    deleteProxyProfile: (id) => call("delete_proxy_profile", { id }),
    setProxyPassword: (id, password) => call("set_proxy_password", { id, password }),
    clearProxyPassword: (id) => call("clear_proxy_password", { id }),
    listScanConfigurations: () => call("list_scan_configurations"),
    saveScanConfiguration: (configuration) => call("save_scan_configuration", { configuration }),
    deleteScanConfiguration: (id) => call("delete_scan_configuration", { id }),
    getNetworkSummary: (scanId) => call("get_network_summary", { scanId }),
    listNetworkRequests: (scanId, query) => call("list_network_requests", { scanId, ...query }),
    getNetworkRequest: (scanId, requestId) => call("get_network_request", { scanId, requestId }),
    getNetworkBody: (scanId, requestId, query) => call("get_network_body", { scanId, requestId, ...query }),
    getScriptSource: (scanId, scriptId, view, original) =>
      call("get_script_source", {
        scanId,
        scriptId,
        view,
        originalLine: original?.line,
        originalColumn: original?.column,
      }),
    getArchiveStorage: () => call("get_archive_storage"),
    clearArchive: () => call("clear_archive"),
    subscribeScanEvents: (handler) => listen<ScanEvent>("scan-event", (event) => handler(event.payload)),
    pickExportPath: (defaultFileName, format) =>
      save({
        defaultPath: defaultFileName,
        filters: [
          format === "markdown" ? { name: "Markdown", extensions: ["md"] } : { name: "JSON", extensions: ["json"] },
        ],
      }),
    pickBrowserExecutable: () => open({ multiple: false, directory: false }),
    openExternal: (url) => openUrl(url),
  };
}
