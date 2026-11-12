import type {
  BrowserInfo,
  DashboardStats,
  ProxyProfile,
  ProxyProfileDraft,
  ScanConfiguration,
  ScanConfigurationDraft,
  ScanConfigurationRef,
  ScanEvent,
  ScanListQuery,
  ScanOptions,
  ScanProfileInfo,
  ScanSummary,
  Settings,
} from "@/bindings";
import { ApiError } from "@/lib/api/error";
import type { ApiPort } from "@/lib/api/port";
import { isProxyHost, PROXY_NAME_MAX } from "@/shared/domain/proxy";
import { CONFIGURATION_NAME_MAX, MANUAL_SESSION_LIMIT_MS, visitorOf } from "@/shared/domain/scan-options";
import { type FakeConfig, readFakeConfig } from "./config";
import { downloadDocument, renderExport } from "./export";
import { searchLogs } from "./log-search";
import { type ResolvedScan, resolveScanOptions, UNKNOWN_PROXY } from "./resolve";
import { applyBeat, cancelScan, completeScan, createRunningScan, type StoredScan } from "./runtime";
import { buildScenario, type ScenarioContext } from "./scenario";
import { FAKE_BROWSER_VERSION, SEED_PROXIES, seedConfigurations, seedScans } from "./seed";

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
  checkSellersJson: false,
  defaultVisitor: "scroll",
  defaultExtraPages: 2,
  persistentProfile: false,
  proxies: [],
  defaultProxy: { kind: "system" },
  keepNetworkData: true,
  archiveLimitBytes: 5 * 1024 ** 3,
  storeFullHeaders: false,
};

/** What one scan adds to the kept profile (cache, cookies, storage), and where Chrome's cache cap stops it. */
const PROFILE_GROWTH_BYTES = 21_400_000;
const PROFILE_MAX_BYTES = 180_000_000;

const sameName = (a: string, b: string) => a.trim().toLowerCase() === b.trim().toLowerCase();

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
  configurations?: ScanConfiguration[];
  scanProfile?: StoredProfile;
};

type StoredProfile = { sizeBytes: number; lastUsedAt: number | null };

/** A manual visit that is being recorded: how to end it, and the timer of its 15-minute limit. */
type ManualSession = { finish: () => void; limit: ReturnType<typeof setTimeout> };

const clone = <T>(value: T): T => structuredClone(value);
const latency = (ms = 60) => new Promise<void>((resolve) => setTimeout(resolve, ms));

class FakeBackend {
  private readonly storageKey: string;
  /** Keyed by subscription, not by handler: the same function may subscribe twice. */
  private readonly listeners = new Map<symbol, (event: ScanEvent) => void>();
  private readonly timers = new Map<string, ReturnType<typeof setTimeout>>();
  private readonly manualSessions = new Map<string, ManualSession>();
  private settings: Settings;
  private scans: StoredScan[];
  private deletedSeedIds: string[];
  private configurations: ScanConfiguration[];
  private scanProfile: StoredProfile;

  constructor(private readonly config: FakeConfig) {
    this.storageKey = `adscan.fake.db.${config.profile}`;
    const persisted = this.load();
    // State persisted before a setting existed lacks it; the defaults fill the gap.
    const seeded = config.profile === "seeded";
    this.settings = {
      ...DEFAULT_SETTINGS,
      hasApiKey: seeded,
      proxies: seeded ? structuredClone(SEED_PROXIES) : [],
      ...persisted?.settings,
    };
    this.deletedSeedIds = persisted?.deletedSeedIds ?? [];
    this.configurations = persisted?.configurations ?? (seeded ? seedConfigurations(Date.now()) : []);
    this.scanProfile = persisted?.scanProfile ?? { sizeBytes: 0, lastUsedAt: null };
    const seeds = seeded ? seedScans(this.settings, Date.now()) : [];
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
    const state: Persisted = {
      settings: this.settings,
      scans: finishedOwnScans,
      deletedSeedIds: this.deletedSeedIds,
      configurations: this.configurations,
      scanProfile: this.scanProfile,
    };
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

  private stopTimers(scanId: string) {
    clearTimeout(this.timers.get(scanId));
    this.timers.delete(scanId);
    clearTimeout(this.manualSessions.get(scanId)?.limit);
    this.manualSessions.delete(scanId);
  }

  private profileInfo(): ScanProfileInfo {
    return {
      exists: this.scanProfile.sizeBytes > 0,
      sizeBytes: this.scanProfile.sizeBytes,
      lastUsedAt: this.scanProfile.lastUsedAt,
      inUse: this.scans.some(
        (scan) => scan.detail.summary.status === "running" && scan.detail.options.persistentProfile === true,
      ),
    };
  }

  private start(resolved: ResolvedScan, scanId: string, configuration: ScanConfigurationRef | null): ScanSummary {
    if (!this.browser()) {
      throw new ApiError("browserNotFound", "No Chrome or Chromium installation was found. Choose one in Settings.");
    }
    const now = Date.now();
    const scan = createRunningScan(scanId, resolved.options, now, resolved.proxy, configuration);
    this.scans.unshift(scan);
    if (resolved.options.persistentProfile) {
      this.scanProfile = {
        sizeBytes: Math.min(PROFILE_MAX_BYTES, this.scanProfile.sizeBytes + PROFILE_GROWTH_BYTES),
        lastUsedAt: now,
      };
    }
    const summary = clone(scan.detail.summary);
    this.emit([{ type: "scanUpdated", summary }]);
    this.play(scan, resolved);
    return summary;
  }

  private play(scan: StoredScan, { options, proxy }: ResolvedScan) {
    const { id } = scan.detail.summary;
    const context: ScenarioContext = {
      scanId: id,
      options,
      proxy,
      settings: this.settings,
      browserVersion: this.browser()?.version ?? FAKE_BROWSER_VERSION,
      jevFails: this.config.jevFails,
    };
    let { beats, result } = buildScenario(context);
    let cursor = 0;
    // Simulated time is `offset` plus what has passed since `resumedAt`; a manual visit stops the clock.
    let offset = 0;
    let resumedAt = performance.now();

    const settle = () => {
      this.stopTimers(id);
      this.persist();
    };

    const tick = () => {
      const elapsed = offset + (performance.now() - resumedAt) * this.config.speed;
      while (cursor < beats.length) {
        const beat = beats[cursor];
        if (!beat || beat.at > elapsed || beat.type === "awaitFinish") break;
        this.emit(applyBeat(scan, beat, Date.now()));
        cursor += 1;
      }
      if (scan.detail.summary.status !== "running") return settle();
      const next = beats[cursor];
      if (!next) {
        if (result) this.emit(completeScan(scan, result, Date.now()));
        return settle();
      }
      // The recording waits for the user (or the session limit) from here on.
      if (next.type === "awaitFinish") return;
      this.timers.set(id, setTimeout(tick, Math.max(8, (next.at - elapsed) / this.config.speed)));
    };

    if (visitorOf(options) === "manual") {
      /** Ends the recording with the pages seen so far and lets the rest of the pipeline run. */
      const finish = () => {
        this.stopTimers(id);
        const seen = scan.logs.filter(
          (entry) => entry.kind === "visitor" && (entry.data as { activity?: string }).activity === "pageView",
        ).length;
        ({ beats, result } = buildScenario({ ...context, manualPages: Math.max(1, seen) }));
        const waitingAt = beats.findIndex((beat) => beat.type === "awaitFinish");
        cursor = waitingAt + 1;
        offset = beats[waitingAt]?.at ?? 0;
        resumedAt = performance.now();
        tick();
      };
      this.manualSessions.set(id, { finish, limit: setTimeout(finish, MANUAL_SESSION_LIMIT_MS) });
    }
    this.timers.set(id, setTimeout(tick, 0));
  }

  private proxyDraft(draft: ProxyProfileDraft): Omit<ProxyProfile, "id" | "hasPassword"> {
    const name = draft.name.trim();
    const host = draft.host.trim();
    const username = draft.username?.trim() || null;
    if (name === "" || name.length > PROXY_NAME_MAX) {
      throw new ApiError("invalidInput", `Give the proxy a name of at most ${PROXY_NAME_MAX} characters`);
    }
    if (this.settings.proxies.some((proxy) => proxy.id !== draft.id && sameName(proxy.name, name))) {
      throw new ApiError("invalidInput", `A proxy named "${name}" already exists`);
    }
    if (!isProxyHost(host)) {
      throw new ApiError(
        "invalidInput",
        "Enter the proxy's host name or IP address only, without scheme, port, path or credentials",
      );
    }
    if (!Number.isInteger(draft.port) || draft.port < 1 || draft.port > 65_535) {
      throw new ApiError("invalidInput", "The proxy port must be between 1 and 65535");
    }
    if (draft.scheme === "socks5" && username !== null) {
      throw new ApiError(
        "invalidInput",
        "Chrome cannot sign in to a SOCKS5 proxy. Remove the user name or use an http or https proxy.",
      );
    }
    return { name, scheme: draft.scheme, host, port: draft.port, username };
  }

  private proxy(id: string): ProxyProfile {
    const proxy = this.settings.proxies.find((candidate) => candidate.id === id);
    if (!proxy) throw new ApiError("notFound", `Proxy ${id} was not found`);
    return proxy;
  }

  private configuration(id: string): ScanConfiguration {
    const configuration = this.configurations.find((candidate) => candidate.id === id);
    if (!configuration) throw new ApiError("notFound", `Scan configuration ${id} was not found`);
    return configuration;
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
      const { defaultExtraPages, defaultProxy } = patch;
      if (defaultExtraPages !== undefined && (defaultExtraPages < 0 || defaultExtraPages > 5)) {
        throw new ApiError("invalidInput", "A browse visit can follow links to at most 5 further pages");
      }
      if (defaultProxy?.kind === "profile" && !this.settings.proxies.some((proxy) => proxy.id === defaultProxy.id)) {
        throw new ApiError("invalidInput", UNKNOWN_PROXY);
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
      const scanId = `scan-${crypto.randomUUID().slice(0, 8)}`;
      return this.start(resolveScanOptions(clone(options), this.settings, scanId), scanId, null);
    },
    runScanConfiguration: async (configurationId) => {
      await latency();
      const configuration = this.configuration(configurationId);
      const scanId = `scan-${crypto.randomUUID().slice(0, 8)}`;
      let resolved: ResolvedScan;
      try {
        resolved = resolveScanOptions(clone(configuration.options), this.settings, scanId);
      } catch (reason) {
        if (!(reason instanceof ApiError) || reason.message !== UNKNOWN_PROXY) throw reason;
        throw new ApiError(
          "invalidInput",
          `The proxy of the scan configuration "${configuration.name}" no longer exists. Edit the configuration and pick another proxy.`,
        );
      }
      const summary = this.start(resolved, scanId, { id: configuration.id, name: configuration.name });
      configuration.lastScanId = scanId;
      configuration.lastRunAt = summary.startedAt;
      return summary;
    },
    cancelScan: async (scanId) => {
      const scan = this.find(scanId);
      if (scan.detail.summary.status !== "running") return;
      this.stopTimers(scanId);
      this.emit(cancelScan(scan, Date.now()));
      this.persist();
    },
    finishCapture: async (scanId) => {
      const scan = this.find(scanId);
      const capturing = scan.detail.steps.some((step) => step.node === "capture" && step.status === "running");
      if (capturing) this.manualSessions.get(scanId)?.finish();
    },
    listScans: async (query: ScanListQuery) => {
      await latency();
      const search = query.search?.trim().toLowerCase();
      const matching = this.scans
        .map((scan) => scan.detail.summary)
        .filter((summary: ScanSummary) => !search || summary.url.toLowerCase().includes(search))
        .filter((summary) => !query.status || summary.status === query.status)
        .filter((summary) => !query.device || summary.device === query.device)
        .filter((summary) => !query.configurationId || summary.configuration?.id === query.configurationId);
      const offset = query.offset ?? 0;
      return { items: clone(matching.slice(offset, offset + (query.limit ?? 50))), total: matching.length };
    },
    getScan: async (scanId) => {
      await latency();
      return clone(this.find(scanId).detail);
    },
    deleteScan: async (scanId) => {
      this.find(scanId);
      this.stopTimers(scanId);
      this.scans = this.scans.filter((scan) => scan.detail.summary.id !== scanId);
      if (scanId.startsWith("seed-")) this.deletedSeedIds.push(scanId);
      this.persist();
    },
    deleteAllScans: async () => {
      for (const scanId of [...this.timers.keys(), ...this.manualSessions.keys()]) this.stopTimers(scanId);
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
    getScanProfile: async () => {
      await latency();
      return this.profileInfo();
    },
    clearScanProfile: async () => {
      await latency(200);
      if (this.profileInfo().inUse) {
        throw new ApiError(
          "profileBusy",
          "The scan profile is in use by a running scan. Try again when that scan has finished.",
        );
      }
      this.scanProfile = { sizeBytes: 0, lastUsedAt: null };
      this.persist();
      return this.profileInfo();
    },
    saveProxyProfile: async (draft) => {
      await latency();
      const values = this.proxyDraft(draft);
      if (draft.id === undefined) {
        this.settings.proxies.push({ id: `proxy-${crypto.randomUUID().slice(0, 8)}`, ...values, hasPassword: false });
      } else {
        Object.assign(this.proxy(draft.id), values);
      }
      this.settings.proxies.sort((a, b) => a.name.localeCompare(b.name));
      this.persist();
      return clone(this.settings);
    },
    deleteProxyProfile: async (id) => {
      await latency();
      const profile = this.proxy(id);
      const users = this.configurations.filter(
        ({ options }) => options.proxy?.kind === "profile" && options.proxy.id === id,
      );
      if (users.length > 0) {
        const one = users.length === 1;
        throw new ApiError(
          "invalidInput",
          `The proxy "${profile.name}" is used by the scan configuration${one ? "" : "s"} ${users.map((configuration) => `"${configuration.name}"`).join(", ")}. Change or delete ${one ? "it" : "them"} first.`,
        );
      }
      this.settings.proxies = this.settings.proxies.filter((proxy) => proxy.id !== id);
      const { defaultProxy } = this.settings;
      if (defaultProxy.kind === "profile" && defaultProxy.id === id) this.settings.defaultProxy = { kind: "system" };
      this.persist();
      return clone(this.settings);
    },
    setProxyPassword: async (id, password) => {
      await latency();
      if (password === "") throw new ApiError("invalidInput", "Enter the proxy password.");
      this.proxy(id).hasPassword = true;
      this.persist();
      return clone(this.settings);
    },
    clearProxyPassword: async (id) => {
      await latency();
      this.proxy(id).hasPassword = false;
      this.persist();
      return clone(this.settings);
    },
    listScanConfigurations: async () => {
      await latency();
      return clone(
        this.configurations
          .map((configuration) => ({
            ...configuration,
            // Like the backend: the last run is only named while that scan still exists.
            lastScanId: this.scans.some((scan) => scan.detail.summary.id === configuration.lastScanId)
              ? configuration.lastScanId
              : null,
          }))
          .sort((a, b) => a.name.localeCompare(b.name)),
      );
    },
    saveScanConfiguration: async (draft: ScanConfigurationDraft) => {
      await latency();
      const name = draft.name.trim();
      if (name === "" || name.length > CONFIGURATION_NAME_MAX) {
        throw new ApiError(
          "invalidInput",
          `Give the scan configuration a name of at most ${CONFIGURATION_NAME_MAX} characters`,
        );
      }
      if (this.configurations.some((existing) => existing.id !== draft.id && sameName(existing.name, name))) {
        throw new ApiError("invalidInput", `A scan configuration named "${name}" already exists`);
      }
      // Validated like a scan start; what the options leave open stays open until each run.
      resolveScanOptions(draft.options, this.settings, "validation");
      const { seed: _seed, ...options } = clone(draft.options);
      const { proxy } = options;
      if (proxy?.kind === "profile") {
        const profile = this.proxy(proxy.id);
        options.proxy = { kind: "profile", id: profile.id, name: profile.name, host: profile.host };
      }
      const now = Date.now();
      let saved: ScanConfiguration;
      if (draft.id === undefined) {
        saved = {
          id: `config-${crypto.randomUUID().slice(0, 8)}`,
          name,
          options,
          createdAt: now,
          updatedAt: now,
          lastScanId: null,
          lastRunAt: null,
        };
        this.configurations.push(saved);
      } else {
        saved = Object.assign(this.configuration(draft.id), { name, options, updatedAt: now });
      }
      this.persist();
      return clone(saved);
    },
    deleteScanConfiguration: async (id) => {
      await latency();
      this.configuration(id);
      this.configurations = this.configurations.filter((configuration) => configuration.id !== id);
      this.persist();
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
