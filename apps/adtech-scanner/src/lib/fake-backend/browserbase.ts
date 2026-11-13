import type {
  BrowserbaseProbe,
  BrowserbaseProject,
  BrowserbaseRegion,
  BrowserbaseSettings,
  BrowserbaseVisit,
  BrowserChoice,
  ProxyUse,
} from "@/bindings";
import { BROWSERBASE_REGION_META } from "@/shared/domain/browserbase";

/** What goes wrong with the next Browserbase scans, for the specs that show a failure. */
export type BrowserbaseFailure = "limit" | "lost";

/** The account of the measurements: one project, three browsers at once, shared with another application. */
export const FAKE_PROJECT: BrowserbaseProject = { id: "prj-7f3a91c2", name: "Local", concurrency: 3 };
/** A key with "multi" in it has a second project, so the choice between projects can be tried. */
export const FAKE_SECOND_PROJECT: BrowserbaseProject = { id: "prj-2b8d04e7", name: "Staging", concurrency: 3 };
/** The project's running totals, the other application's sessions included. */
export const FAKE_PROJECT_USAGE = { browserMinutes: 483, proxyBytes: 1_998_349_077 };
/** The other application's browser that is always running in the shared project. */
export const OTHER_APPLICATION_SESSIONS = 1;

export const DISCONNECTED: BrowserbaseSettings = {
  region: "us-west-2",
  defaultProxy: { kind: "none" },
  persistentProfile: false,
  connected: false,
  projectName: null,
  concurrency: null,
};

const BROWSER_VERSION = "Chrome/157.0.8103.79";
const USER_AGENT =
  "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/157.0.0.0 Safari/537.36";
/** Every region reports this until the scanner sets the region's own. */
const UNSET_TIMEZONE = "America/Los_Angeles";
/** Median round trip of one command from the US, as measured. */
const ROUND_TRIP_MS: Record<BrowserbaseRegion, number> = {
  "us-west-2": 83.1,
  "us-east-1": 43.8,
  "eu-central-1": 118.6,
  "ap-southeast-1": 249.6,
};
/** With a Browserbase proxy the service sets the timezone of the proxy's location itself. */
const PROXY_TIMEZONE: Record<string, string> = {
  BE: "Europe/Brussels",
  DE: "Europe/Berlin",
  NL: "Europe/Amsterdam",
  FR: "Europe/Paris",
  GB: "Europe/London",
  US: "America/Chicago",
};
/** Traffic of one page view through a Browserbase proxy. */
const PROXY_BYTES_PER_PAGE = 18_486_000;

export const LIMIT_MESSAGE = `Browserbase is at its limit of browsers running at once (${FAKE_PROJECT.concurrency} of ${FAKE_PROJECT.concurrency}, shared by everything that uses this project). Try again when one has finished.`;
export const CONNECTION_LOST_MESSAGE =
  "The connection to the Browserbase browser was lost. The browser has ended. Nothing was kept.";

export function fakeProbe(region: BrowserbaseRegion): BrowserbaseProbe {
  return {
    region,
    browserVersion: BROWSER_VERSION,
    userAgent: USER_AGENT,
    webdriver: false,
    timezone: UNSET_TIMEZONE,
    language: "en-US",
    roundTripMs: ROUND_TRIP_MS[region],
    sessionSeconds: 6,
  };
}

/** A session id as Browserbase writes them, the same for the same scan. */
function fakeSessionId(seed: number): string {
  const hex = (seed >>> 0).toString(16).padStart(8, "0");
  return `${hex}-3c1e-4f7a-9b2d-${hex}a41c`;
}

/** What the capture reports of a Browserbase browser. Usage is filled in when the browser has ended. */
export function fakeBrowserbaseVisit(
  browser: Extract<BrowserChoice, { kind: "browserbase" }>,
  proxy: ProxyUse,
  seed: number,
): BrowserbaseVisit {
  const region = browser.region ?? "us-west-2";
  const country = proxy.kind === "browserbaseLocation" ? (proxy.name?.split(",")[0] ?? null) : null;
  const viaBrowserbase = proxy.kind === "browserbaseLocation";
  return {
    region,
    sessionId: fakeSessionId(seed),
    browserVersion: BROWSER_VERSION,
    userAgent: USER_AGENT,
    proxyKind: viaBrowserbase ? "browserbase" : proxy.kind === "profile" ? "external" : "none",
    proxyCountry: country,
    settings: { solveCaptchas: true, recordSession: true, logSession: true, blockAds: false, keepAlive: false },
    originalTimezone: viaBrowserbase ? (PROXY_TIMEZONE[country ?? ""] ?? "Etc/UTC") : UNSET_TIMEZONE,
    appliedTimezone: viaBrowserbase ? null : BROWSERBASE_REGION_META[region].timezone,
    language: "en-US",
    roundTripMs: ROUND_TRIP_MS[region],
    sessionSeconds: null,
    proxyBytes: null,
  };
}

/** What Browserbase counts for a visit of this many page views; nothing without one of its own proxies. */
export function fakeProxyBytes(visit: BrowserbaseVisit, pageViews: number): number {
  return visit.proxyKind === "browserbase" ? PROXY_BYTES_PER_PAGE * pageViews : 0;
}
