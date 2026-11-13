import type { BrowserbaseRegion, BrowserKind, ProxyChoice, ScanOptions, Settings, VisitorBehavior } from "@/bindings";
import { isLocalOrPrivateUrl } from "@/shared/lib/url";
import { MANUAL_SESSION_LIMIT_MS } from "./scan-options";

/** In the order Browserbase lists them; the first is its own default. */
export const BROWSERBASE_REGIONS: readonly BrowserbaseRegion[] = [
  "us-west-2",
  "us-east-1",
  "eu-central-1",
  "ap-southeast-1",
];

/** Where each region is, and the timezone the scanner sets there when the scan uses no Browserbase proxy. */
export const BROWSERBASE_REGION_META: Record<BrowserbaseRegion, { place: string; country: string; timezone: string }> =
  {
    "us-west-2": { place: "Oregon", country: "United States", timezone: "America/Los_Angeles" },
    "us-east-1": { place: "Virginia", country: "United States", timezone: "America/New_York" },
    "eu-central-1": { place: "Frankfurt", country: "Germany", timezone: "Europe/Berlin" },
    "ap-southeast-1": { place: "Singapore", country: "Singapore", timezone: "Asia/Singapore" },
  };

/** "Frankfurt (eu-central-1)": the region wherever the app names it, so that it reads the same as in the backend's lines. */
export function regionName(region: BrowserbaseRegion): string {
  return `${BROWSERBASE_REGION_META[region].place} (${region})`;
}

/**
 * "Frankfurt": the place alone, for the two narrow spots that have the whole name right beside them (a menu
 * whose items carry it, a table cell whose title does). Everywhere else it is `regionName`.
 */
export function regionPlace(region: BrowserbaseRegion): string {
  return BROWSERBASE_REGION_META[region].place;
}

/** "Frankfurt, Germany (eu-central-1)", for a menu. */
export function regionOptionLabel(region: BrowserbaseRegion): string {
  const { place, country } = BROWSERBASE_REGION_META[region];
  return `${place === country ? place : `${place}, ${country}`} (${region})`;
}

export const BROWSERBASE_UNREACHABLE = "This address is on this Mac or your network; a cloud browser cannot reach it.";

/** The words the settings screen must show before Browserbase credentials can be saved. */
export const BROWSERBASE_DISCLOSURE =
  "With Browserbase, the browser runs in Browserbase's cloud, not on this Mac. The pages you scan, their cookies, and everything the scanner reads from them (scripts, requests, bid data) travel through Browserbase, a third party. Browserbase keeps a video of each session and a log that includes the content the scanner reads and the cookies it reads or sets. CAPTCHAs on scanned pages are solved automatically, and the browser does not tell the page it is automated. To do this, Browserbase runs its own scripts inside every page you scan; the scanner leaves them out of its results, but they can change how a page behaves. Results are also stored on this Mac. Browserbase bills browser time per minute, and proxy traffic per megabyte.";

/**
 * Whether a cloud browser can reach the address. The backend's rule: nothing on this computer or a private
 * network, and no name without a dot or under `.internal` or `.home.arpa`.
 */
export function reachableFromCloud(url: string): boolean {
  let hostname: string;
  try {
    hostname = new URL(url).hostname.toLowerCase().replace(/\.$/, "");
  } catch {
    return false;
  }
  if (isLocalOrPrivateUrl(url)) return false;
  const isAddress = hostname.startsWith("[") || /^[\d.]+$/.test(hostname);
  if (!isAddress && !hostname.includes(".")) return false;
  return !hostname.endsWith(".internal") && !hostname.endsWith(".home.arpa");
}

/**
 * Where a scan with these options runs, by the backend's rule: what the options say, else the settings
 * default, and never Browserbase for an address only this computer can reach or without credentials.
 */
export function browserKindOf(
  options: Pick<ScanOptions, "browser" | "url">,
  settings: Pick<Settings, "defaultBrowser" | "browserbase">,
): BrowserKind {
  if (options.browser) return options.browser.kind;
  const available = settings.browserbase.connected && reachableFromCloud(options.url);
  return settings.defaultBrowser === "browserbase" && available ? "browserbase" : "local";
}

export type BrowserbaseLocation = Extract<ProxyChoice, { kind: "browserbaseLocation" }>;

const COUNTRY_NAMES = new Intl.DisplayNames(["en"], { type: "region" });

/** "Belgium" for "BE". A code the runtime does not know is shown as it is. */
export function countryName(code: string): string {
  const upper = code.trim().toUpperCase();
  if (!/^[A-Z]{2}$/.test(upper)) return upper;
  try {
    return COUNTRY_NAMES.of(upper) ?? upper;
  } catch {
    return upper;
  }
}

/** Browserbase takes a city in upper case with underscores; people read "New York". */
export function cityName(city: string): string {
  return city
    .split(/[_\s]+/)
    .filter(Boolean)
    .map((word) => `${word.charAt(0).toUpperCase()}${word.slice(1).toLowerCase()}`)
    .join(" ");
}

/** "Belgium", "New York, NY, United States": most specific part first. */
export function locationLabel(location: Pick<BrowserbaseLocation, "country" | "state" | "city">): string {
  return [
    location.city ? cityName(location.city) : null,
    location.state ? location.state.toUpperCase() : null,
    countryName(location.country),
  ]
    .filter((part) => part !== null)
    .join(", ");
}

/** A location as a finished scan records it: "BE", or "US, NY, NEW_YORK". */
export function locationFromRecord(name: string): Pick<BrowserbaseLocation, "country" | "state" | "city"> {
  const [country = "", ...rest] = name.split(",").map((part) => part.trim());
  // With two parts the second is a state when it is a two-letter code, and a city otherwise.
  const state = rest.length > 0 && /^[A-Za-z]{2}$/.test(rest[0] ?? "") ? rest[0] : undefined;
  const city = state ? rest[1] : rest[0];
  return { country, state, city };
}

/** A location as it is typed, field by field. */
export type LocationDraft = { country: string; state: string; city: string };

/** What is wrong with a typed location, and the field it is wrong in. */
export type LocationError = { field: keyof LocationDraft; message: string };

export type LocationResult = { ok: true; location: BrowserbaseLocation } | { ok: false; error: LocationError };

/**
 * The location as typed, in the form the backend stores, or what is wrong with it and in which field. The
 * backend checks the same three rules; its sentences are longer because they have no field beside them.
 */
export function parseLocation(fields: LocationDraft): LocationResult {
  const country = fields.country.trim().toUpperCase();
  const state = fields.state.trim().toUpperCase();
  const city = fields.city.trim().replace(/\s+/g, " ");
  if (!/^[A-Z]{2}$/.test(country)) {
    return {
      ok: false,
      error: { field: "country", message: "Enter the country as a two-letter code, for example BE." },
    };
  }
  if (state !== "" && !/^[A-Z]{2}$/.test(state)) {
    return { ok: false, error: { field: "state", message: "Enter the state as a two-letter code, for example NY." } };
  }
  if (city !== "" && (!/^[\p{L}][\p{L} .'-]*$/u.test(city) || city.length > 60)) {
    return { ok: false, error: { field: "city", message: "Enter the city in letters, at most 60 characters." } };
  }
  return {
    ok: true,
    location: {
      kind: "browserbaseLocation",
      country,
      ...(state ? { state } : {}),
      ...(city ? { city: city.toUpperCase().replace(/ /g, "_") } : {}),
    },
  };
}

/**
 * Browserbase's list price, read from its plans page on 2026-11-12. The API names neither the account's plan
 * nor what is left of the usage that plan includes, so every dollar figure made from these is an estimate that
 * counts none of it. Change both the numbers and the date when the plans page changes.
 */
export const BROWSERBASE_RATES = {
  asOf: "2026-11-12",
  browserUsdPerHour: 0.12,
  proxyUsdPerGb: 12,
} as const;

/** How every dollar figure for Browserbase is qualified: the price it is made from, and what it leaves out. */
export const BROWSERBASE_LIST_PRICE = `Browserbase's list price of $${BROWSERBASE_RATES.browserUsdPerHour.toFixed(2)} per browser hour and $${BROWSERBASE_RATES.proxyUsdPerGb} per GB (${BROWSERBASE_RATES.asOf})`;
export const BROWSERBASE_PLAN_CAVEAT = "your plan's included usage is not counted";

/**
 * What `navigator.webdriver` read, and what that means. `told` fits the sentence to its place: one blank page
 * of a check ("the page is"), or the pages of a finished visit ("the pages were").
 */
export function webdriverReading(value: boolean, told: "the page is" | "the pages were"): string {
  return `${value}: ${told} ${value ? "" : "not "}told the visit is automated`;
}

/** Seconds a session spends starting and connecting before the first page, and loading each page view. */
const SESSION_START_SECONDS = 10;
const PAGE_LOAD_SECONDS = 8;
/** Traffic of one page view of an ad-heavy page through a proxy, as it crossed on the one page measured. */
export const PROXY_BYTES_PER_PAGE = 18_486_000;
const PROXY_MB_PER_PAGE = Math.round(PROXY_BYTES_PER_PAGE / 1_000_000);

export type BrowserbaseEstimate = {
  /** Billed browser minutes: every started minute counts, one at least. */
  minutes: number;
  /** The minutes are the most the session can last, not what it is expected to take (a manual visit). */
  upTo: boolean;
  /** Megabytes through a Browserbase proxy; 0 without one. */
  proxyMb: number;
  /** The proxy figure is per page view, because nobody knows how many a person will open. */
  proxyPerPage: boolean;
  /** The proxy's share of `usd`. */
  proxyUsd: number;
  usd: number;
};

/** What a scan is likely to use at Browserbase. Never part of the LLM budget. */
export function estimateBrowserbaseUsage(scan: {
  visitor: VisitorBehavior;
  /** Page views, the entry page included. */
  pages: number;
  observeSeconds: number;
  /** The visit leaves through a Browserbase location, which is billed per megabyte. */
  browserbaseProxy: boolean;
}): BrowserbaseEstimate {
  const manual = scan.visitor === "manual";
  const seconds = manual
    ? MANUAL_SESSION_LIMIT_MS / 1000
    : SESSION_START_SECONDS + scan.pages * (PAGE_LOAD_SECONDS + scan.observeSeconds);
  const minutes = Math.max(1, Math.ceil(seconds / 60));
  const proxyMb = scan.browserbaseProxy ? PROXY_MB_PER_PAGE * (manual ? 1 : scan.pages) : 0;
  return {
    minutes,
    upTo: manual,
    proxyMb,
    proxyPerPage: manual && scan.browserbaseProxy,
    proxyUsd: proxyUsd(proxyMb * 1_000_000),
    usd: browserbaseUsd(minutes * 60, proxyMb * 1_000_000),
  };
}

function proxyUsd(proxyBytes: number): number {
  return (Math.ceil(proxyBytes / 1_000_000) / 1000) * BROWSERBASE_RATES.proxyUsdPerGb;
}

/** Dollars for a session at the list price: started minutes plus started megabytes. */
export function browserbaseUsd(sessionSeconds: number, proxyBytes: number): number {
  const minutes = Math.max(1, Math.ceil(sessionSeconds / 60));
  return (minutes / 60) * BROWSERBASE_RATES.browserUsdPerHour + proxyUsd(proxyBytes);
}

/** Proxy traffic the way Browserbase bills it, in decimal megabytes: "0.1 MB", "18 MB", "1.91 GB". */
export function formatTraffic(bytes: number): string {
  if (bytes >= 1_000_000_000) return `${(bytes / 1_000_000_000).toFixed(2)} GB`;
  const megabytes = bytes / 1_000_000;
  return `${megabytes.toFixed(megabytes > 0 && megabytes < 10 ? 1 : 0)} MB`;
}
