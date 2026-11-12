import type {
  BrowserInfo,
  DashboardStats,
  ScanEvent,
  ScanListQuery,
  ScanOptions,
  ScanSummary,
  Settings,
} from "@/bindings";
import { ApiError } from "@/lib/api/error";
import type { ApiPort } from "@/lib/api/port";
import { type FakeConfig, readFakeConfig } from "./config";
import { downloadDocument, renderExport } from "./export";
import { searchLogs } from "./log-search";
import { applyBeat, cancelScan, completeScan, createRunningScan, type StoredScan } from "./runtime";
import { buildScenario } from "./scenario";
import { FAKE_BROWSER_VERSION, seedScans } from "./seed";

const DEFAULT_SETTINGS: Settings = {
  filterModel: "claude-haiku-5-5",
  analysisModel: "claude-sonnet-5-5",
  analysisEffort: "medium",
  budgetUsd: 0.5,
  chromePath: null,
  headless: true,
  defaultDevice: "desktop",
  defaultObserveSeconds: 20,
  theme: "system",
  hasApiKey: false,
  jevEnabled: false,
  hasTypesafeKey: false,
};

const DEFAULT_BROWSER: BrowserInfo = {
  path: "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
  version: FAKE_BROWSER_VERSION,
  name: "Google Chrome",
};

const DEMO_URL = "http://127.0.0.1:47615/demo-publisher/";

type Persisted = {
  settings: Settings;
  /** Scans started in this browser; seeded history is regenerated on every load. */
  scans: StoredScan[];
  deletedSeedIds: string[];
};

const clone = <T>(value: T): T => structuredClone(value);
const latency = (ms = 60) => new Promise<void>((resolve) => setTimeout(resolve, ms));

class FakeBackend {
  private readonly storageKey: string;
  /** Keyed by subscription, not by handler: the same function may subscribe twice. */
  private readonly listeners = new Map<symbol, (event: ScanEvent) => void>();
  private readonly timers = new Map<string, ReturnType<typeof setTimeout>>();
  private settings: Settings;
  private scans: StoredScan[];
  private deletedSeedIds: string[];

  constructor(private readonly config: FakeConfig) {
    this.storageKey = `adscan.fake.db.${config.profile}`;
    const persisted = this.load();
    // State persisted before a setting existed lacks it; the defaults fill the gap.
    this.settings = { ...DEFAULT_SETTINGS, hasApiKey: config.profile === "seeded", ...persisted?.settings };
    this.deletedSeedIds = persisted?.deletedSeedIds ?? [];
    const seeds = config.profile === "seeded" ? seedScans(this.settings, Date.now()) : [];
    this.scans = [
      ...(persisted?.scans ?? []),
      ...seeds.filter((scan) => !this.deletedSeedIds.includes(scan.detail.summary.id)),
    ].sort((a, b) => b.detail.summary.startedAt - a.detail.summary.startedAt);
  }

  private load(): Persisted | null {
    try {
      const raw = localStorage.getItem(this.storageKey);
      return raw ? (JSON.parse(raw) as Persisted) : null;
    } catch {
      return null;
    }
  }

  private persist() {
    const finishedOwnScans = this.scans.filter(
      (scan) => !scan.detail.summary.id.startsWith("seed-") && scan.detail.summary.status !== "running",
    );
    const state: Persisted = { settings: this.settings, scans: finishedOwnScans, deletedSeedIds: this.deletedSeedIds };
    try {
      localStorage.setItem(this.storageKey, JSON.stringify(state));
    } catch {
      // Storage full or unavailable: the fake simply stops remembering across reloads.
    }
  }

  private emit(events: ScanEvent[]) {
    for (const event of events) for (const listener of this.listeners.values()) listener(event);
  }

  private find(scanId: string): StoredScan {
    const scan = this.scans.find((candidate) => candidate.detail.summary.id === scanId);
    if (!scan) throw new ApiError("notFound", `Scan ${scanId} does not exist`);
    return scan;
  }

  private browser(): BrowserInfo | null {
    if (this.settings.chromePath) {
      return { path: this.settings.chromePath, version: "Chromium 141.0.7390.37", name: "Chromium" };
    }
    return this.config.profile === "no-browser" ? null : DEFAULT_BROWSER;
  }

  private play(scan: StoredScan) {
    const { id } = scan.detail.summary;
    const scenario = buildScenario({
      scanId: id,
      options: scan.detail.options,
      settings: this.settings,
      browserVersion: this.browser()?.version ?? FAKE_BROWSER_VERSION,
      jevFails: this.config.jevFails,
    });
    const startedAt = performance.now();
    let cursor = 0;

    const tick = () => {
      const elapsed = (performance.now() - startedAt) * this.config.speed;
      while (cursor < scenario.beats.length) {
        const beat = scenario.beats[cursor];
        if (!beat || beat.at > elapsed) break;
        this.emit(applyBeat(scan, beat, Date.now()));
        cursor += 1;
      }
      if (scan.detail.summary.status !== "running") {
        this.timers.delete(id);
        this.persist();
        return;
      }
      const next = scenario.beats[cursor];
      if (!next) {
        if (scenario.result) this.emit(completeScan(scan, scenario.result, Date.now()));
        this.timers.delete(id);
        this.persist();
        return;
      }
      this.timers.set(id, setTimeout(tick, Math.max(8, (next.at - elapsed) / this.config.speed)));
    };
    this.timers.set(id, setTimeout(tick, 0));
  }

  readonly port: ApiPort = {
    getSettings: async () => {
      await latency();
      return clone(this.settings);
    },
    updateSettings: async (patch) => {
      await latency();
      if (patch.budgetUsd !== undefined && patch.budgetUsd <= 0) {
        throw new ApiError("invalidInput", "The per-scan budget must be greater than zero");
      }
      if (patch.jevEnabled && !this.settings.hasTypesafeKey) {
        throw new ApiError("invalidInput", "Add a TypeSafe API key before turning on Jev.");
      }
      const { chromePath, ...rest } = patch;
      this.settings = { ...this.settings, ...rest };
      if (chromePath !== undefined) this.settings.chromePath = chromePath;
      this.persist();
      return clone(this.settings);
    },
    setApiKey: async (key, provider = "anthropic") => {
      await latency(500);
      if (provider === "typesafe") {
        // TypeSafe documents no key format, so any key passes unless it announces itself as invalid.
        if (key.trim() === "" || key.includes("invalid")) {
          return { valid: false, message: "TypeSafe rejected this API key." };
        }
        this.settings.hasTypesafeKey = true;
      } else {
        if (!/^sk-ant-[\w-]{16,}$/.test(key.trim())) {
          return {
            valid: false,
            message: "Anthropic rejected this key (401 authentication_error: invalid x-api-key).",
          };
        }
        this.settings.hasApiKey = true;
      }
      this.persist();
      return { valid: true, message: null };
    },
    clearApiKey: async (provider = "anthropic") => {
      await latency();
      if (provider === "typesafe") {
        this.settings.hasTypesafeKey = false;
        this.settings.jevEnabled = false;
      } else {
        this.settings.hasApiKey = false;
      }
      this.persist();
    },
    detectBrowser: async () => {
      await latency(120);
      return this.browser();
    },
    getDemoUrl: async () => DEMO_URL,
    startScan: async (options: ScanOptions) => {
      await latency();
      if (!/^https?:\/\/[^\s/]+/.test(options.url)) {
        throw new ApiError("invalidInput", "Enter an absolute http(s) URL");
      }
      if (!this.browser()) {
        throw new ApiError("browserNotFound", "No Chrome or Chromium installation was found. Choose one in Settings.");
      }
      const scan = createRunningScan(`scan-${crypto.randomUUID().slice(0, 8)}`, clone(options), Date.now());
      this.scans.unshift(scan);
      const summary = clone(scan.detail.summary);
      this.emit([{ type: "scanUpdated", summary }]);
      this.play(scan);
      return summary;
    },
    cancelScan: async (scanId) => {
      const scan = this.find(scanId);
      if (scan.detail.summary.status !== "running") return;
      clearTimeout(this.timers.get(scanId));
      this.timers.delete(scanId);
      this.emit(cancelScan(scan, Date.now()));
      this.persist();
    },
    listScans: async (query: ScanListQuery) => {
      await latency();
      const search = query.search?.trim().toLowerCase();
      const matching = this.scans
        .map((scan) => scan.detail.summary)
        .filter((summary: ScanSummary) => !search || summary.url.toLowerCase().includes(search))
        .filter((summary) => !query.status || summary.status === query.status)
        .filter((summary) => !query.device || summary.device === query.device);
      const offset = query.offset ?? 0;
      return { items: clone(matching.slice(offset, offset + (query.limit ?? 50))), total: matching.length };
    },
    getScan: async (scanId) => {
      await latency();
      return clone(this.find(scanId).detail);
    },
    deleteScan: async (scanId) => {
      this.find(scanId);
      clearTimeout(this.timers.get(scanId));
      this.scans = this.scans.filter((scan) => scan.detail.summary.id !== scanId);
      if (scanId.startsWith("seed-")) this.deletedSeedIds.push(scanId);
      this.persist();
    },
    deleteAllScans: async () => {
      for (const timer of this.timers.values()) clearTimeout(timer);
      this.timers.clear();
      this.deletedSeedIds.push(
        ...this.scans.map((scan) => scan.detail.summary.id).filter((id) => id.startsWith("seed-")),
      );
      this.scans = [];
      this.persist();
    },
    getLogs: async (scanId, stepId) => {
      await latency();
      const { logs } = this.find(scanId);
      return clone(stepId ? logs.filter((entry) => entry.stepId === stepId) : logs);
    },
    searchLogs: async (query, limit = 100) => {
      await latency(90);
      return clone(searchLogs(this.scans, query, limit));
    },
    getDashboardStats: async () => {
      await latency();
      return this.stats();
    },
    exportScan: async (scanId, format, path) => {
      const content = renderExport(this.find(scanId).detail, format);
      if (path) downloadDocument(path, format, content);
      return content;
    },
    subscribeScanEvents: async (handler) => {
      const subscription = Symbol("scan-events");
      this.listeners.set(subscription, handler);
      return () => this.listeners.delete(subscription);
    },
    pickExportPath: async (defaultFileName) => defaultFileName,
    pickBrowserExecutable: async () => "/Applications/Chromium.app/Contents/MacOS/Chromium",
    openExternal: async (url) => {
      window.open(url, "_blank", "noopener");
    },
  };

  private stats(): DashboardStats {
    const summaries = this.scans.map((scan) => scan.detail.summary);
    const completed = summaries.filter((summary) => summary.status === "completed");
    // "Open" issues are those in the newest completed scan of each site + device.
    const latest = new Map<string, ScanSummary>();
    for (const summary of completed) {
      const key = `${summary.hostname}|${summary.device}`;
      if (!latest.has(key)) latest.set(key, summary);
    }
    const scores = completed.flatMap((summary) => (summary.healthScore === null ? [] : [summary.healthScore]));
    const totalCostUsd = summaries.reduce((sum, summary) => sum + summary.costUsd, 0);
    const counts = [...latest.values()].map((summary) => summary.findingCounts);
    return {
      totalScans: summaries.length,
      sitesScanned: new Set(summaries.map((summary) => summary.hostname)).size,
      openCriticalFindings: counts.reduce((sum, count) => sum + count.critical, 0),
      totalFindings: counts.reduce(
        (sum, count) => sum + count.critical + count.high + count.medium + count.low + count.info,
        0,
      ),
      totalCostUsd,
      avgCostUsd: completed.length ? totalCostUsd / completed.length : 0,
      avgHealthScore: scores.length ? scores.reduce((sum, score) => sum + score, 0) / scores.length : null,
    };
  }
}

export function createFakeBackend(config: FakeConfig = readFakeConfig()): ApiPort {
  return new FakeBackend(config).port;
}
