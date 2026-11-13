import type {
  BrowserChoice,
  DeviceProfile,
  ProxyChoice,
  ProxyProfile,
  ScanConfiguration,
  ScanOptions,
  Settings,
  VisitorBehavior,
} from "@/bindings";
import { DISCONNECTED, FAKE_PROJECT } from "./browserbase";
import { resolveScanOptions } from "./resolve";
import { applyBeat, cancelScan, completeScan, createRunningScan, type StoredScan } from "./runtime";
import { buildScenario } from "./scenario";

export const FAKE_BROWSER_VERSION = "Google Chrome 141.0.7390.55";

const BELGIUM: ProxyProfile = {
  id: "proxy-belgium",
  name: "Belgium",
  scheme: "http",
  host: "be.proxy.example",
  port: 8080,
  username: "scanner",
  hasPassword: true,
};
const NETHERLANDS: ProxyProfile = {
  id: "proxy-netherlands",
  name: "Netherlands",
  scheme: "socks5",
  host: "nl.proxy.example",
  port: 1080,
  username: null,
  hasPassword: false,
};

export const SEED_PROXIES: ProxyProfile[] = [BELGIUM, NETHERLANDS];

const SPORTSDESK_URL = "https://www.sportsdesk.example/football/transfer-news";
const LEDGER_URL = "https://www.morningledger.example/";
const SPORTSDESK_BELGIUM = { id: "config-sportsdesk-belgium", name: "Sportsdesk from Belgium" };
const LEDGER_DIRECT = { id: "config-morningledger-direct", name: "Morning Ledger, no proxy" };
const GARDEN_URL = "https://www.gardenweekly.example/spring-planting";
const FRANKFURT: BrowserChoice = { kind: "browserbase", region: "eu-central-1" };
const BELGIAN_HOUSEHOLD: ProxyChoice = { kind: "browserbaseLocation", country: "BE" };

type SeedSpec = {
  id: string;
  url: string;
  device: DeviceProfile;
  minutesAgo: number;
  aiEnabled?: boolean;
  visitor?: VisitorBehavior;
  extraPages?: number;
  proxy?: ProxyChoice;
  /** Where the browser ran; this Mac unless said otherwise. */
  browser?: BrowserChoice;
  /** The saved configuration the scan was started from. */
  configuration?: { id: string; name: string };
  /** Stored before visits had pages: no page views, no environment, no network data, only the options of that time. */
  legacy?: boolean;
  /** What became of the scan's network data. Kept unless said otherwise. */
  archive?: "disabled" | "evicted";
  /** Stop the scan this many ms in, as if the user cancelled it. */
  cancelAfterMs?: number;
};

const MINUTES_PER_DAY = 1440;

const FEATURED: SeedSpec[] = [
  { id: "seed-sportsdesk-desktop", url: SPORTSDESK_URL, device: "desktop", minutesAgo: 34 },
  { id: "seed-sportsdesk-mobile", url: SPORTSDESK_URL, device: "mobile", minutesAgo: 29 },
  {
    id: "seed-morningledger-browserbase",
    url: LEDGER_URL,
    device: "desktop",
    minutesAgo: 75,
    browser: FRANKFURT,
    proxy: BELGIAN_HOUSEHOLD,
  },
  {
    id: "seed-morningledger-desktop",
    url: LEDGER_URL,
    device: "desktop",
    minutesAgo: 190,
    proxy: { kind: "none" },
    configuration: LEDGER_DIRECT,
    // Scanned while "Keep network data" was off.
    archive: "disabled",
  },
  {
    id: "seed-sportsdesk-browse",
    url: SPORTSDESK_URL,
    device: "desktop",
    minutesAgo: 260,
    visitor: "browse",
    extraPages: 2,
    proxy: { kind: "profile", id: BELGIUM.id },
    configuration: SPORTSDESK_BELGIUM,
  },
  {
    id: "seed-techpulse-desktop",
    url: "https://www.techpulse.example/reviews/laptops",
    device: "desktop",
    minutesAgo: MINUTES_PER_DAY + 40,
    // The oldest scan with network data: it was the first to go when the storage limit was reached.
    archive: "evicted",
  },
  { id: "seed-unreachable", url: "https://unreachable.example/", device: "desktop", minutesAgo: MINUTES_PER_DAY + 95 },
  {
    id: "seed-gardenweekly-cancelled",
    url: "https://www.gardenweekly.example/spring-planting",
    device: "mobile",
    minutesAgo: 2 * MINUTES_PER_DAY + 15,
    cancelAfterMs: 9_000,
  },
  {
    id: "seed-morningledger-mobile",
    url: LEDGER_URL,
    device: "mobile",
    minutesAgo: 3 * MINUTES_PER_DAY + 200,
    aiEnabled: false,
    legacy: true,
  },
];

const ARCHIVE_HOSTS = [
  "www.citywire-daily.example",
  "www.foodcourt.example",
  "www.autoreview.example",
  "www.travelnotes.example",
  "www.techpulse.example",
  "www.homefinance.example",
  "www.gamerdispatch.example",
  "www.sportsdesk.example",
  "www.morningledger.example",
];

function archiveSpecs(): SeedSpec[] {
  return Array.from({ length: 18 }, (_, index) => {
    const host = ARCHIVE_HOSTS[index % ARCHIVE_HOSTS.length] as string;
    const device: DeviceProfile = index % 3 === 1 ? "mobile" : "desktop";
    return {
      id: `seed-archive-${index + 1}`,
      url: `https://${host}/`,
      device,
      minutesAgo: (4 + index * 1.4) * MINUTES_PER_DAY + index * 37,
      aiEnabled: index % 5 !== 4,
      legacy: true,
    };
  });
}

/** Strips a finished scan back to what was stored before visits, pages and proxies were recorded. */
function asLegacy(scan: StoredScan): StoredScan {
  const { detail } = scan;
  const { url, device, observeSeconds, autoScroll, acceptConsent, aiEnabled } = detail.options;
  return {
    detail: {
      ...detail,
      summary: { ...detail.summary, proxy: null },
      options: { url, device, observeSeconds, autoScroll, acceptConsent, aiEnabled },
      pages: [],
      environment: null,
      findings: detail.findings.map((finding) => ({ ...finding, pages: [] })),
      scripts: detail.scripts.map((script) => ({ ...script, pages: [] })),
      slots: detail.slots.map((slot) => ({ ...slot, requests: [] })),
    },
    logs: scan.logs.filter((entry) => entry.kind !== "visitor"),
  };
}

function runSeed(spec: SeedSpec, settings: Settings, now: number, padTo: number): StoredScan {
  const startedAt = now - spec.minutesAgo * 60_000;
  const visitor = spec.visitor ?? "scroll";
  const { options, proxy } = resolveScanOptions(
    {
      url: spec.url,
      device: spec.device,
      observeSeconds: 20,
      autoScroll: visitor !== "none",
      acceptConsent: true,
      aiEnabled: spec.aiEnabled ?? true,
      visitor,
      extraPages: spec.extraPages,
      proxy: spec.proxy,
      browser: spec.browser ?? { kind: "local" },
    },
    settings,
    spec.id,
  );
  const scan = createRunningScan(spec.id, options, startedAt, proxy, spec.configuration ?? null);
  const scenario = buildScenario({
    scanId: spec.id,
    options,
    proxy,
    settings,
    browserVersion: FAKE_BROWSER_VERSION,
    projectName: settings.browserbase.projectName,
  });
  let elapsed = 0;
  for (const beat of scenario.beats) {
    if (spec.cancelAfterMs !== undefined && beat.at > spec.cancelAfterMs) {
      cancelScan(scan, startedAt + spec.cancelAfterMs);
      return scan;
    }
    elapsed = beat.at;
    applyBeat(scan, beat, startedAt + beat.at);
  }
  if (scenario.result) {
    const archive = spec.legacy
      ? null
      : { keep: spec.archive !== "disabled", evicted: spec.archive === "evicted", redactHeaders: true, padTo };
    completeScan(scan, scenario.result, startedAt + elapsed + 80, archive);
  }
  return spec.legacy ? asLegacy(scan) : scan;
}

/**
 * Deterministic history: the reference-bug site on both devices and as a three-page visit through a
 * proxy, a healthy site scanned with network recording off, a scan whose network data was evicted,
 * failures, and an archive stored before visits had pages or network data.
 */
export function seedScans(settings: Settings, now: number, padTo = 0): StoredScan[] {
  // History predates whatever the settings say now.
  const seedSettings: Settings = {
    ...settings,
    hasApiKey: true,
    jevEnabled: false,
    headless: true,
    persistentProfile: false,
    budgetUsd: 0.5,
    proxies: SEED_PROXIES,
    defaultProxy: { kind: "system" },
    defaultBrowser: "local",
    browserbase: {
      ...DISCONNECTED,
      connected: true,
      projectName: FAKE_PROJECT.name,
      concurrency: FAKE_PROJECT.concurrency,
    },
  };
  return [...FEATURED, ...archiveSpecs()].map((spec) => runSeed(spec, seedSettings, now, padTo));
}

function configurationOptions(
  url: string,
  device: DeviceProfile,
  visitor: VisitorBehavior,
  proxy: ProxyChoice,
): ScanOptions {
  return {
    url,
    device,
    observeSeconds: 20,
    autoScroll: visitor !== "none",
    acceptConsent: true,
    aiEnabled: true,
    visitor,
    extraPages: visitor === "browse" ? 2 : 0,
    headless: visitor !== "browse",
    proxy,
  };
}

/**
 * One site set up for several countries and devices, one whose proxy has since been deleted, and one that
 * runs in a Browserbase browser.
 */
export function seedConfigurations(now: number): ScanConfiguration[] {
  const created = now - 6 * MINUTES_PER_DAY * 60_000;
  const lastRun = (id: string) => now - (FEATURED.find((spec) => spec.id === id)?.minutesAgo ?? 0) * 60_000;
  const never = { lastScanId: null, lastRunAt: null };
  return [
    {
      ...SPORTSDESK_BELGIUM,
      options: configurationOptions(SPORTSDESK_URL, "desktop", "browse", {
        kind: "profile",
        id: BELGIUM.id,
        name: BELGIUM.name,
        host: BELGIUM.host,
      }),
      createdAt: created,
      updatedAt: created,
      lastScanId: "seed-sportsdesk-browse",
      lastRunAt: lastRun("seed-sportsdesk-browse"),
    },
    {
      id: "config-sportsdesk-netherlands",
      name: "Sportsdesk from the Netherlands, mobile",
      options: configurationOptions(SPORTSDESK_URL, "mobile", "scroll", {
        kind: "profile",
        id: NETHERLANDS.id,
        name: NETHERLANDS.name,
        host: NETHERLANDS.host,
      }),
      createdAt: created,
      updatedAt: created,
      ...never,
    },
    {
      id: "config-sportsdesk-germany",
      name: "Sportsdesk from Germany",
      // Its proxy profile was deleted since.
      options: configurationOptions(SPORTSDESK_URL, "desktop", "browse", {
        kind: "profile",
        id: "proxy-germany",
        name: "Germany",
        host: "de.proxy.example",
      }),
      createdAt: created,
      updatedAt: created,
      ...never,
    },
    {
      id: "config-gardenweekly-browserbase",
      name: "Garden Weekly from Belgium, Browserbase",
      options: {
        ...configurationOptions(GARDEN_URL, "desktop", "scroll", BELGIAN_HOUSEHOLD),
        headless: true,
        browser: FRANKFURT,
      },
      createdAt: created,
      updatedAt: created,
      ...never,
    },
    {
      ...LEDGER_DIRECT,
      options: configurationOptions(LEDGER_URL, "desktop", "scroll", { kind: "none" }),
      createdAt: created,
      updatedAt: created,
      lastScanId: "seed-morningledger-desktop",
      lastRunAt: lastRun("seed-morningledger-desktop"),
    },
  ];
}
