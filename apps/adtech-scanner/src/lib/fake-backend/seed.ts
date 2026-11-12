import type {
  DeviceProfile,
  ProxyChoice,
  ProxyProfile,
  ScanConfiguration,
  ScanOptions,
  Settings,
  VisitorBehavior,
} from "@/bindings";
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

type SeedSpec = {
  id: string;
  url: string;
  device: DeviceProfile;
  minutesAgo: number;
  aiEnabled?: boolean;
  visitor?: VisitorBehavior;
  extraPages?: number;
  proxy?: ProxyChoice;
  /** The saved configuration the scan was started from. */
  configuration?: { id: string; name: string };
  /** Stored before visits had pages: no page views, no environment, only the options of that time. */
  legacy?: boolean;
  /** Stop the scan this many ms in, as if the user cancelled it. */
  cancelAfterMs?: number;
};

const MINUTES_PER_DAY = 1440;

const FEATURED: SeedSpec[] = [
  { id: "seed-sportsdesk-desktop", url: SPORTSDESK_URL, device: "desktop", minutesAgo: 34 },
  { id: "seed-sportsdesk-mobile", url: SPORTSDESK_URL, device: "mobile", minutesAgo: 29 },
  {
    id: "seed-morningledger-desktop",
    url: LEDGER_URL,
    device: "desktop",
    minutesAgo: 190,
    proxy: { kind: "none" },
    configuration: LEDGER_DIRECT,
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

function runSeed(spec: SeedSpec, settings: Settings, now: number): StoredScan {
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
  if (scenario.result) completeScan(scan, scenario.result, startedAt + elapsed + 80);
  return spec.legacy ? asLegacy(scan) : scan;
}

/**
 * Deterministic history: the reference-bug site on both devices and as a three-page visit through a
 * proxy, a healthy site, failures, and an archive stored before visits had pages.
 */
export function seedScans(settings: Settings, now: number): StoredScan[] {
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
  };
  return [...FEATURED, ...archiveSpecs()].map((spec) => runSeed(spec, seedSettings, now));
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

/** One site set up for several countries and devices, plus one whose proxy has since been deleted. */
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
      ...LEDGER_DIRECT,
      options: configurationOptions(LEDGER_URL, "desktop", "scroll", { kind: "none" }),
      createdAt: created,
      updatedAt: created,
      lastScanId: "seed-morningledger-desktop",
      lastRunAt: lastRun("seed-morningledger-desktop"),
    },
  ];
}
