import type {
  BrowserbaseProbe,
  BrowserbaseProject,
  BrowserbaseRegion,
  BrowserbaseSettings,
  BrowserbaseVisit,
  BrowserChoice,
  ProxyUse,
  VisitorBehavior,
} from "@/bindings";
import { BROWSERBASE_REGION_META, PROXY_BYTES_PER_PAGE } from "@/shared/domain/browserbase";

/**
 * What goes wrong with the next Browserbase scans, for the specs that show a failure: the project is at its
 * limit of browsers, the connection drops during the visit, or the scan succeeds but Browserbase never
 * confirms that its browser has ended.
 */
export type BrowserbaseFailure = "limit" | "lost" | "unconfirmed";

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
/** What Browserbase runs of its own in each document, as measured: five scripts, 271,667 characters. */
export const SERVICE_SCRIPTS_PER_DOCUMENT = { count: 5, characters: 271_667 };
/**
 * Every simulated site has one feed that a remote browser cannot hand over: text is only asked for up to
 * 8 MiB (images, media and fonts up to 32 MiB), because the reply can take six times the body's size.
 */
export const OVERSIZED_FEED_BYTES = 12_163_481;

export const LIMIT_MESSAGE = `Browserbase is at its limit of browsers running at once (${FAKE_PROJECT.concurrency} of ${FAKE_PROJECT.concurrency}, shared by everything that uses this project). Try again when one has finished.`;
export const CONNECTION_LOST_MESSAGE =
  "The connection to the Browserbase browser was lost. The browser has ended. Nothing was kept.";
export const RELEASE_UNCONFIRMED_MESSAGE =
  "Browserbase did not confirm that the browser has ended; it ends by itself at its time limit at the latest";
export const systemProxyDroppedMessage = (region: BrowserbaseRegion) =>
  `The system proxy of this Mac does not apply to a Browserbase browser: the visit leaves from Browserbase's ${region} region.`;

/**
 * The time limit a session is started with, like the browser crate's: two minutes to start, the visit, and
 * two minutes of margin. 325 s for a 20 s scroll visit, 765 s for a browse visit, 1140 s for a manual one.
 */
export function sessionTimeoutSeconds(visitor: VisitorBehavior, observeSeconds: number): number {
  let visit = 45 + 2 * observeSeconds;
  if (visitor === "browse") visit = 45 + 480;
  if (visitor === "manual") visit = 900;
  return Math.min(21_600, Math.max(60, 120 + visit + 120));
}

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
