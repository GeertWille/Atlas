import type {
  ArchiveStorage,
  BrowserbaseProfileInfo,
  BrowserbaseProject,
  BrowserInfo,
  BrowserKind,
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
  SellersJsonCacheInfo,
  Settings,
} from "@/bindings";
import { ApiError } from "@/lib/api/error";
import type { ApiPort } from "@/lib/api/port";
import { isProxyHost, PROXY_NAME_MAX } from "@/shared/domain/proxy";
import { CONFIGURATION_NAME_MAX, MANUAL_SESSION_LIMIT_MS, visitorOf } from "@/shared/domain/scan-options";
import {
  DISCONNECTED,
  FAKE_PROJECT,
  FAKE_PROJECT_USAGE,
  FAKE_SECOND_PROJECT,
  fakeProbe,
  OTHER_APPLICATION_SESSIONS,
} from "./browserbase";
import { type FakeConfig, readFakeConfig } from "./config";
import { isOnThisMachine } from "./diagnostics";
import { downloadDocument, renderExport } from "./export";
import { searchLogs } from "./log-search";
import { bodyChunk, type FakeNetwork, listRows, materializeNetwork, requestDetail, summarise } from "./network";
import {
  BROWSERBASE_NOT_SET_UP,
  type ResolvedScan,
  resolveBrowserbaseProxy,
  resolveScanOptions,
  UNKNOWN_PROXY,
} from "./resolve";
import { applyBeat, cancelScan, completeScan, createRunningScan, type StoredScan, settleProxyUsage } from "./runtime";
import { buildScenario, type ScenarioContext } from "./scenario";
import { FAKE_BROWSER_VERSION, SEED_PROXIES, seedConfigurations, seedScans } from "./seed";
import { fakeScriptSource } from "./sources";

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
  checkSellersJson: true,
  defaultVisitor: "scroll",
  defaultExtraPages: 2,
  persistentProfile: false,
  proxies: [],
  defaultProxy: { kind: "system" },
  keepNetworkData: true,
  archiveLimitBytes: 5 * 1024 ** 3,
  storeFullHeaders: false,
  defaultBrowser: "local",
  browserbase: DISCONNECTED,
};

/** Browserbase reads a session's proxy traffic again this long after its end, because the count lags. */
const PROXY_USAGE_REREAD_MS = 30_000;

/** The sellers.json files of the exchanges the simulated sites sell through, as a scan with the check on leaves them. */
const KEPT_SELLERS_JSON: SellersJsonCacheInfo = { entries: 6, storedBytes: 7_340_000 };
const NO_SELLERS_JSON: SellersJsonCacheInfo = { entries: 0, storedBytes: 0 };

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
  /** Seeded scans whose network data "Clear stored scan data" removed. */
  clearedSeedArchives?: string[];
  sellersJsonCache?: SellersJsonCacheInfo;
  /** When a scan last used the browser profile kept at Browserbase; `null` while none is kept. */
  browserbaseProfileUsedAt?: number | null;
};

const ARCHIVE_LIMIT_MIN_BYTES = 100 * 1024 ** 2;
const ARCHIVE_LIMIT_MAX_BYTES = 1024 ** 4;

type StoredProfile = { sizeBytes: number; lastUsedAt: number | null };

/** A manual visit that is being recorded: how to end it, and the timer of its 15-minute limit. */
type ManualSession = { finish: () => void; limit: ReturnType<typeof setTimeout> };

const clone = <T>(value: T): T => structuredClone(value);

/** "Clear stored scan data": the report stays and says its network data is gone, rows included. */
function clearNetworkData(scan: StoredScan) {
  const { network } = scan.detail;
  if (!network || network.availability === "notRecorded" || network.availability === "disabled") return;
  scan.detail.network = { ...network, availability: "evicted", requestCount: 0, bodyCount: 0, storedBytes: 0 };
}
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
  private clearedSeedArchives: string[];
  private sellersJsonCache: SellersJsonCacheInfo;
  private browserbaseProfileUsedAt: number | null;
  /** Request lists are derived from the scans on demand and kept while the page lives. */
  private readonly networks = new Map<string, FakeNetwork>();

  /** The time a call takes to answer; none with `instant`, where a test times the interface alone. */
  private latency(ms = 60): Promise<void> {
    if (this.config.instant) return Promise.resolve();
    return new Promise((resolve) => setTimeout(resolve, ms));
  }

  constructor(private readonly config: FakeConfig) {
    this.storageKey = `adscan.fake.db.${config.profile}`;
    const persisted = this.load();
    // State persisted before a setting existed lacks it; the defaults fill the gap.
    const seeded = config.profile === "seeded";
    this.settings = {
      ...DEFAULT_SETTINGS,
      hasApiKey: seeded,
      proxies: seeded ? structuredClone(SEED_PROXIES) : [],
      // The seeded history has scans that ran on Browserbase: the account is set up.
      browserbase: seeded
        ? { ...DISCONNECTED, connected: true, projectName: FAKE_PROJECT.name, concurrency: FAKE_PROJECT.concurrency }
        : DISCONNECTED,
      ...persisted?.settings,
    };
    this.browserbaseProfileUsedAt = persisted?.browserbaseProfileUsedAt ?? null;
    this.deletedSeedIds = persisted?.deletedSeedIds ?? [];
    this.configurations = persisted?.configurations ?? (seeded ? seedConfigurations(Date.now()) : []);
    this.scanProfile = persisted?.scanProfile ?? { sizeBytes: 0, lastUsedAt: null };
    this.clearedSeedArchives = persisted?.clearedSeedArchives ?? [];
    // The seeded history was scanned with the check on.
    this.sellersJsonCache = persisted?.sellersJsonCache ?? (seeded ? KEPT_SELLERS_JSON : NO_SELLERS_JSON);
    const seeds = seeded ? seedScans(this.settings, Date.now(), config.rows) : [];
    for (const seed of seeds) {
      if (this.clearedSeedArchives.includes(seed.detail.summary.id)) clearNetworkData(seed);
    }
    this.scans = [
      ...(persisted?.scans ?? []),
      ...seeds.filter((scan) => !this.deletedSeedIds.includes(scan.detail.summary.id)),
    ].sort((a, b) => b.detail.summary.startedAt - a.detail.summary.startedAt);
    // Padded lists are built up front, so that opening one costs what the interface costs, not what the fake does.
    if (config.rows) for (const scan of this.scans) this.network(scan);
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
      clearedSeedArchives: this.clearedSeedArchives,
      sellersJsonCache: this.sellersJsonCache,
      browserbaseProfileUsedAt: this.browserbaseProfileUsedAt,
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

  /** The scan's request list; `null` when it has none to show. */
  private network(scan: StoredScan): FakeNetwork | null {
    const { id } = scan.detail.summary;
    const { network } = scan.detail;
    if (!network || network.availability === "notRecorded" || network.availability === "disabled") return null;
    if (network.requestCount === 0) return null;
    let built = this.networks.get(id);
    if (!built) {
      built = materializeNetwork(scan.detail, this.config.rows);
      this.networks.set(id, built);
    }
    return built;
  }

  private requireNetwork(scanId: string): { scan: StoredScan; network: FakeNetwork } {
    const scan = this.find(scanId);
    const network = this.network(scan);
    if (!network) throw new ApiError("notFound", `Scan ${scanId} has no network record`);
    return { scan, network };
  }

  private archiveStorage(): ArchiveStorage {
    const archives = this.scans.flatMap((scan) => (scan.detail.network ? [scan] : []));
    const kept = archives.filter((scan) => scan.detail.network?.availability === "available");
    return {
      scans: kept.length,
      evictedScans: archives.filter((scan) => scan.detail.network?.availability === "evicted").length,
      storedBytes: kept.reduce((sum, scan) => sum + (scan.detail.network?.storedBytes ?? 0), 0),
      limitBytes: this.settings.archiveLimitBytes,
      oldestAt: kept.length > 0 ? Math.min(...kept.map((scan) => scan.detail.summary.startedAt)) : null,
    };
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

  /** A scan is running in the kept profile of this kind of browser. */
  private profileInUse(kind: BrowserKind): boolean {
    return this.scans.some(
      ({ detail }) =>
        detail.summary.status === "running" &&
        detail.options.persistentProfile === true &&
        (detail.options.browser?.kind ?? "local") === kind,
    );
  }

  private profileInfo(): ScanProfileInfo {
    return {
      exists: this.scanProfile.sizeBytes > 0,
      sizeBytes: this.scanProfile.sizeBytes,
      lastUsedAt: this.scanProfile.lastUsedAt,
      inUse: this.profileInUse("local"),
    };
  }

  private browserbaseProfileInfo(): BrowserbaseProfileInfo {
    return {
      exists: this.browserbaseProfileUsedAt !== null,
      lastUsedAt: this.browserbaseProfileUsedAt,
      inUse: this.profileInUse("browserbase"),
    };
  }

  private requireBrowserbase() {
    if (!this.settings.browserbase.connected) throw new ApiError("browserbaseAuth", BROWSERBASE_NOT_SET_UP);
  }

  private start(resolved: ResolvedScan, scanId: string, configuration: ScanConfigurationRef | null): ScanSummary {
    const remote = resolved.options.browser?.kind === "browserbase";
    // A Browserbase scan needs no browser on this computer.
    if (!remote && !this.browser()) {
      throw new ApiError("browserNotFound", "No Chrome or Chromium installation was found. Choose one in Settings.");
    }
    const now = Date.now();
    const scan = createRunningScan(scanId, resolved.options, now, resolved.proxy, configuration);
    this.scans.unshift(scan);
    if (remote && resolved.options.persistentProfile) {
      this.browserbaseProfileUsedAt = now;
    } else if (resolved.options.persistentProfile) {
      this.scanProfile = {
        sizeBytes: Math.min(PROFILE_MAX_BYTES, this.scanProfile.sizeBytes + PROFILE_GROWTH_BYTES),
        lastUsedAt: now,
      };
    }
    if (this.settings.checkSellersJson && !isOnThisMachine(resolved.options.url)) {
      this.sellersJsonCache = KEPT_SELLERS_JSON;
    }
    const summary = clone(scan.detail.summary);
    this.emit([{ type: "scanUpdated", summary }]);
    this.play(scan, resolved);
    return summary;
  }

  private play(scan: StoredScan, { options, proxy, systemProxyDropped }: ResolvedScan) {
    const { id } = scan.detail.summary;
    const context: ScenarioContext = {
      scanId: id,
      options,
      proxy,
      settings: this.settings,
      browserVersion: this.browser()?.version ?? FAKE_BROWSER_VERSION,
      jevFails: this.config.jevFails,
      projectName: this.settings.browserbase.projectName,
      browserbaseFailure: this.config.browserbaseFailure,
      systemProxyDropped,
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
        if (result) {
          const archive = {
            keep: this.settings.keepNetworkData,
            redactHeaders: !this.settings.storeFullHeaders,
            padTo: this.config.rows,
          };
          this.emit(completeScan(scan, result, Date.now(), archive, true));
          const counted = result.remote?.proxyBytes ?? 0;
          settle();
          if (counted > 0) {
            const reread = () => {
              this.emit(settleProxyUsage(scan, counted));
              this.persist();
            };
            this.timers.set(id, setTimeout(reread, PROXY_USAGE_REREAD_MS / this.config.speed));
          }
          return;
        }
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
      await this.latency();
      return clone(this.settings);
    },
    updateSettings: async (patch) => {
      await this.latency();
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
      if (patch.defaultBrowser === "browserbase" && !this.settings.browserbase.connected) {
        throw new ApiError("invalidInput", "Add a Browserbase API key before making Browserbase the default.");
      }
      if (defaultProxy?.kind === "browserbaseLocation") {
        throw new ApiError(
          "invalidInput",
          "A Browserbase location can only be used when the browser runs on Browserbase.",
        );
      }
      const browserbase = { ...this.settings.browserbase, ...patch.browserbase };
      if (patch.browserbase?.defaultProxy) {
        browserbase.defaultProxy = resolveBrowserbaseProxy(patch.browserbase.defaultProxy, this.settings);
      }
      const { archiveLimitBytes } = patch;
      if (
        archiveLimitBytes !== undefined &&
        (archiveLimitBytes < ARCHIVE_LIMIT_MIN_BYTES || archiveLimitBytes > ARCHIVE_LIMIT_MAX_BYTES)
      ) {
        throw new ApiError("invalidInput", "The storage limit must be between 100 MB and 1 TB");
      }
      const { chromePath, ...rest } = patch;
      this.settings = { ...this.settings, ...rest, browserbase };
      if (chromePath !== undefined) this.settings.chromePath = chromePath;
      this.persist();
      return clone(this.settings);
    },
    setApiKey: async (key, provider = "anthropic") => {
      await this.latency(500);
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
      await this.latency();
      if (provider === "typesafe") {
        this.settings.hasTypesafeKey = false;
        this.settings.jevEnabled = false;
      } else {
        this.settings.hasApiKey = false;
      }
      this.persist();
    },
    detectBrowser: async () => {
      await this.latency(120);
      return this.browser();
    },
    getDemoUrl: async () => DEMO_URL,
    startScan: async (options: ScanOptions) => {
      await this.latency();
      const scanId = `scan-${crypto.randomUUID().slice(0, 8)}`;
      return this.start(resolveScanOptions(clone(options), this.settings, scanId), scanId, null);
    },
    runScanConfiguration: async (configurationId) => {
      await this.latency();
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
      await this.latency();
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
      await this.latency();
      return clone(this.find(scanId).detail);
    },
    deleteScan: async (scanId) => {
      this.find(scanId);
      this.stopTimers(scanId);
      this.scans = this.scans.filter((scan) => scan.detail.summary.id !== scanId);
      this.networks.delete(scanId);
      if (scanId.startsWith("seed-")) this.deletedSeedIds.push(scanId);
      this.persist();
    },
    deleteAllScans: async () => {
      for (const scanId of [...this.timers.keys(), ...this.manualSessions.keys()]) this.stopTimers(scanId);
      this.deletedSeedIds.push(
        ...this.scans.map((scan) => scan.detail.summary.id).filter((id) => id.startsWith("seed-")),
      );
      this.scans = [];
      this.networks.clear();
      this.persist();
    },
    getLogs: async (scanId, stepId) => {
      await this.latency();
      const { logs } = this.find(scanId);
      return clone(stepId ? logs.filter((entry) => entry.stepId === stepId) : logs);
    },
    searchLogs: async (query, limit = 100) => {
      await this.latency(90);
      return clone(searchLogs(this.scans, query, limit));
    },
    getDashboardStats: async () => {
      await this.latency();
      return this.stats();
    },
    exportScan: async (scanId, format, path) => {
      const content = renderExport(this.find(scanId).detail, format);
      if (path) downloadDocument(path, format, content);
      return content;
    },
    getScanProfile: async () => {
      await this.latency();
      return this.profileInfo();
    },
    clearScanProfile: async () => {
      await this.latency(200);
      if (this.profileInfo().inUse) {
        throw new ApiError(
          "profileBusy",
          "The browser profile is in use by a running scan. Try again when that scan has finished.",
        );
      }
      this.scanProfile = { sizeBytes: 0, lastUsedAt: null };
      this.persist();
      return this.profileInfo();
    },
    saveProxyProfile: async (draft) => {
      await this.latency();
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
      await this.latency();
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
      const cloudProxy = this.settings.browserbase.defaultProxy;
      if (cloudProxy.kind === "profile" && cloudProxy.id === id) {
        this.settings.browserbase = { ...this.settings.browserbase, defaultProxy: { kind: "none" } };
      }
      this.persist();
      return clone(this.settings);
    },
    setProxyPassword: async (id, password) => {
      await this.latency();
      if (password === "") throw new ApiError("invalidInput", "Enter the proxy password.");
      this.proxy(id).hasPassword = true;
      this.persist();
      return clone(this.settings);
    },
    clearProxyPassword: async (id) => {
      await this.latency();
      this.proxy(id).hasPassword = false;
      this.persist();
      return clone(this.settings);
    },
    listScanConfigurations: async () => {
      await this.latency();
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
      await this.latency();
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
      await this.latency();
      this.configuration(id);
      this.configurations = this.configurations.filter((configuration) => configuration.id !== id);
      this.persist();
    },
    getNetworkSummary: async (scanId) => {
      await this.latency(20);
      const scan = this.find(scanId);
      return clone(summarise(scan.detail, this.network(scan)));
    },
    listNetworkRequests: async (scanId, query) => {
      await this.latency(12);
      const network = this.network(this.find(scanId));
      return network ? clone(listRows(network, query)) : { rows: [], nextCursor: null, total: 0 };
    },
    getNetworkRequest: async (scanId, requestId) => {
      await this.latency(20);
      const { scan, network } = this.requireNetwork(scanId);
      return clone(requestDetail(scan.detail, network, requestId));
    },
    getNetworkBody: async (scanId, requestId, query) => {
      await this.latency(20);
      const { scan, network } = this.requireNetwork(scanId);
      return bodyChunk(scan.detail, network, requestId, query);
    },
    getScriptSource: async (scanId, scriptId, view, original) => {
      await this.latency(30);
      return fakeScriptSource(this.find(scanId).detail, scriptId, view, original, this.config.sourceLimitBytes);
    },
    getArchiveStorage: async () => {
      await this.latency();
      return this.archiveStorage();
    },
    clearArchive: async () => {
      await this.latency();
      for (const scan of this.scans) {
        if (scan.detail.summary.status === "running" || !scan.detail.network) continue;
        clearNetworkData(scan);
        const { id } = scan.detail.summary;
        if (id.startsWith("seed-") && !this.clearedSeedArchives.includes(id)) this.clearedSeedArchives.push(id);
      }
      this.networks.clear();
      this.persist();
      return this.archiveStorage();
    },
    getSellersJsonCache: async () => {
      await this.latency();
      return clone(this.sellersJsonCache);
    },
    clearSellersJsonCache: async () => {
      await this.latency();
      this.sellersJsonCache = NO_SELLERS_JSON;
      this.persist();
      return clone(this.sellersJsonCache);
    },
    setBrowserbaseCredentials: async (apiKey, projectId) => {
      await this.latency(500);
      const key = apiKey.trim();
      const refused = (message: string, projects: BrowserbaseProject[] = []) => ({ valid: false, message, projects });
      if (key === "") return refused("Enter a Browserbase API key.");
      // Browserbase documents no key format: any key passes unless it announces itself as invalid.
      if (/\s/.test(key) || key.includes("invalid")) return refused("Browserbase did not accept the API key.");
      const projects = key.includes("multi") ? [FAKE_PROJECT, FAKE_SECOND_PROJECT] : [FAKE_PROJECT];
      const [only] = projects;
      const project = projectId === undefined ? only : projects.find((candidate) => candidate.id === projectId);
      if (!project) return refused("This Browserbase project was not found for this key.");
      if (projectId === undefined && projects.length > 1) {
        return refused("This key has more than one Browserbase project. Choose the one to use.", projects);
      }
      this.settings.browserbase = {
        ...this.settings.browserbase,
        connected: true,
        projectName: project.name,
        concurrency: project.concurrency,
      };
      this.persist();
      return { valid: true, message: null, projects: [] };
    },
    clearBrowserbaseCredentials: async () => {
      await this.latency();
      this.settings.browserbase = {
        ...this.settings.browserbase,
        connected: false,
        projectName: null,
        concurrency: null,
      };
      this.settings.defaultBrowser = "local";
      // The profile kept at Browserbase is deleted there with the last use of the key.
      this.browserbaseProfileUsedAt = null;
      this.persist();
      return clone(this.settings);
    },
    testBrowserbase: async () => {
      await this.latency(400);
      this.requireBrowserbase();
      const own = this.scans.filter(
        ({ detail }) => detail.summary.status === "running" && detail.options.browser?.kind === "browserbase",
      ).length;
      return {
        projectName: this.settings.browserbase.projectName ?? FAKE_PROJECT.name,
        concurrency: this.settings.browserbase.concurrency ?? FAKE_PROJECT.concurrency,
        ...FAKE_PROJECT_USAGE,
        runningSessions: own + OTHER_APPLICATION_SESSIONS,
      };
    },
    probeBrowserbase: async (region) => {
      await this.latency(1_500);
      this.requireBrowserbase();
      return fakeProbe(region ?? this.settings.browserbase.region);
    },
    getBrowserbaseProfile: async () => {
      await this.latency();
      return this.browserbaseProfileInfo();
    },
    clearBrowserbaseProfile: async () => {
      await this.latency(200);
      this.requireBrowserbase();
      if (this.profileInUse("browserbase")) {
        throw new ApiError(
          "profileBusy",
          "The browser profile is in use by a running scan. Try again when that scan has finished.",
        );
      }
      this.browserbaseProfileUsedAt = null;
      this.persist();
      return this.browserbaseProfileInfo();
    },
    openBrowserbaseLiveView: async (scanId) => {
      await this.latency();
      const { detail } = this.find(scanId);
      const capturing = detail.steps.some((step) => step.node === "capture" && step.status === "running");
      if (detail.options.browser?.kind !== "browserbase" || !capturing) {
        throw new ApiError("invalidInput", "This scan has no Browserbase browser to show right now.");
      }
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
