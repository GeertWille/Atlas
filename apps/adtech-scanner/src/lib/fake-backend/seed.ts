import type { DeviceProfile, ScanOptions, Settings } from "@/bindings";
import { applyBeat, cancelScan, completeScan, createRunningScan, type StoredScan } from "./runtime";
import { buildScenario } from "./scenario";

export const FAKE_BROWSER_VERSION = "Google Chrome 141.0.7390.55";

type SeedSpec = {
  id: string;
  url: string;
  device: DeviceProfile;
  minutesAgo: number;
  aiEnabled?: boolean;
  /** Stop the scan this many ms in, as if the user cancelled it. */
  cancelAfterMs?: number;
};

const MINUTES_PER_DAY = 1440;

const FEATURED: SeedSpec[] = [
  {
    id: "seed-sportsdesk-desktop",
    url: "https://www.sportsdesk.example/football/transfer-news",
    device: "desktop",
    minutesAgo: 34,
  },
  {
    id: "seed-sportsdesk-mobile",
    url: "https://www.sportsdesk.example/football/transfer-news",
    device: "mobile",
    minutesAgo: 29,
  },
  { id: "seed-morningledger-desktop", url: "https://www.morningledger.example/", device: "desktop", minutesAgo: 190 },
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
    url: "https://www.morningledger.example/",
    device: "mobile",
    minutesAgo: 3 * MINUTES_PER_DAY + 200,
    aiEnabled: false,
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
    };
  });
}

function runSeed(spec: SeedSpec, settings: Settings, now: number): StoredScan {
  const startedAt = now - spec.minutesAgo * 60_000;
  const options: ScanOptions = {
    url: spec.url,
    device: spec.device,
    observeSeconds: 20,
    autoScroll: true,
    acceptConsent: true,
    aiEnabled: spec.aiEnabled ?? true,
  };
  const scan = createRunningScan(spec.id, options, startedAt);
  const scenario = buildScenario({ scanId: spec.id, options, settings, browserVersion: FAKE_BROWSER_VERSION });
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
  return scan;
}

/** Deterministic history: the reference-bug site on both devices, a healthy site, failures and an archive. */
export function seedScans(settings: Settings, now: number): StoredScan[] {
  // History predates whatever the Jev switch says now.
  const seedSettings = { ...settings, hasApiKey: true, jevEnabled: false };
  return [...FEATURED, ...archiveSpecs()].map((spec) => runSeed(spec, seedSettings, now));
}
