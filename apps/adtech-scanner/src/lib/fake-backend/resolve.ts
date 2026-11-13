import type { BrowserChoice, ProxyChoice, ProxyProfile, ProxyUse, ScanOptions, Settings } from "@/bindings";
import { ApiError } from "@/lib/api/error";
import { reachableFromCloud } from "@/shared/domain/browserbase";
import { defaultHeadless, visitorOf } from "@/shared/domain/scan-options";
import { hashString } from "./rng";
import { MAX_LATER_PAGES } from "./visit";

/** The backend accepts a wider budget than the composer and the settings screen offer. */
const MAX_BUDGET_USD = 50;

export const UNKNOWN_PROXY = "The chosen proxy no longer exists. Pick another proxy.";
export const BROWSERBASE_NOT_SET_UP = "Browserbase is not set up. Add the API key in Settings.";
const UNREACHABLE_FROM_CLOUD =
  "This address is on this Mac or your network; a cloud browser cannot reach it. Scan it with this Mac's Chrome.";
const LOCATION_NEEDS_BROWSERBASE = "A Browserbase location can only be used when the browser runs on Browserbase.";

/** Where the scan's browser runs, by the backend's rule; a Browserbase choice always leaves with its region. */
function resolveBrowser(options: ScanOptions, settings: Settings): BrowserChoice {
  const reachable = reachableFromCloud(options.url);
  const { connected, region } = settings.browserbase;
  if (!options.browser) {
    return settings.defaultBrowser === "browserbase" && connected && reachable
      ? { kind: "browserbase", region }
      : { kind: "local" };
  }
  if (options.browser.kind === "local") return { kind: "local" };
  if (!connected) throw new ApiError("browserbaseAuth", BROWSERBASE_NOT_SET_UP);
  if (!reachable) throw new ApiError("invalidInput", UNREACHABLE_FROM_CLOUD);
  return { kind: "browserbase", region: options.browser.region ?? region };
}

function findProfile(id: string, settings: Pick<Settings, "proxies">): ProxyProfile {
  const profile = settings.proxies.find((candidate) => candidate.id === id);
  if (!profile) throw new ApiError("invalidInput", UNKNOWN_PROXY);
  return profile;
}

/**
 * The proxy of a Browserbase browser as the backend stores it: this computer's own proxy means nothing to a
 * cloud browser, a saved proxy must be reachable from the internet, and a location is written in upper case.
 */
export function resolveBrowserbaseProxy(choice: ProxyChoice, settings: Pick<Settings, "proxies">): ProxyChoice {
  switch (choice.kind) {
    case "none":
    case "system":
      return { kind: "none" };
    case "profile": {
      const profile = findProfile(choice.id, settings);
      if (!reachableFromCloud(`http://${profile.host.includes(":") ? `[${profile.host}]` : profile.host}/`)) {
        throw new ApiError(
          "invalidInput",
          `Browserbase cannot reach the proxy "${profile.name}": its address is on this Mac or your network. Use a proxy that is reachable from the internet.`,
        );
      }
      return { kind: "profile", id: profile.id, name: profile.name, host: profile.host };
    }
    case "browserbaseLocation": {
      const country = choice.country.trim().toUpperCase();
      const state = choice.state?.trim().toUpperCase();
      const city = choice.city?.trim().toUpperCase().replace(/\s+/g, "_");
      if (!/^[A-Z]{2}$/.test(country)) {
        throw new ApiError(
          "invalidInput",
          "Enter the country of the Browserbase location as a two-letter code, for example BE",
        );
      }
      if (state && !/^[A-Z]{2}$/.test(state)) {
        throw new ApiError(
          "invalidInput",
          "Enter the state of the Browserbase location as a two-letter code, for example NY",
        );
      }
      if (city && (!/^[\p{L}][\p{L}_.'-]*$/u.test(city) || city.length > 60)) {
        throw new ApiError(
          "invalidInput",
          "Enter the city of the Browserbase location in letters, at most 60 characters",
        );
      }
      return { kind: "browserbaseLocation", country, ...(state ? { state } : {}), ...(city ? { city } : {}) };
    }
  }
}

function proxyUseOf(choice: ProxyChoice): ProxyUse {
  switch (choice.kind) {
    case "profile":
      return { kind: "profile", name: choice.name ?? null, host: choice.host ?? null };
    case "browserbaseLocation":
      return {
        kind: "browserbaseLocation",
        name: [choice.country, choice.state, choice.city].filter(Boolean).join(", "),
        host: null,
      };
    default:
      return { kind: choice.kind, name: null, host: null };
  }
}

export type ResolvedScan = {
  /** The options as a backend stores them: nothing is left to the settings any more. */
  options: ScanOptions;
  proxy: ProxyUse;
};

/**
 * Fills in what the options leave to the settings and normalises what a visitor behaviour implies,
 * like the backend does when a scan starts. `scanId` gives a scripted visit its seed.
 */
export function resolveScanOptions(options: ScanOptions, settings: Settings, scanId: string): ResolvedScan {
  if (!/^https?:\/\/[^\s/]+/.test(options.url)) {
    throw new ApiError("invalidInput", "Enter a valid web address, for example https://example.com");
  }
  const visitor = visitorOf(options);
  const requestedPages = options.extraPages ?? settings.defaultExtraPages;
  if (requestedPages > MAX_LATER_PAGES) {
    throw new ApiError("invalidInput", `A browse visit can follow links to at most ${MAX_LATER_PAGES} further pages`);
  }
  const extraPages = visitor === "browse" ? requestedPages : 0;
  const budgetUsd = options.budgetUsd ?? settings.budgetUsd;
  if (!(budgetUsd >= 0 && budgetUsd <= MAX_BUDGET_USD)) {
    throw new ApiError("invalidInput", `The per-scan budget must be between $0 and $${MAX_BUDGET_USD}`);
  }

  const browser = resolveBrowser(options, settings);
  const remote = browser.kind === "browserbase";
  let choice: ProxyChoice;
  if (remote) {
    choice = resolveBrowserbaseProxy(options.proxy ?? settings.browserbase.defaultProxy, settings);
  } else {
    choice = options.proxy ?? settings.defaultProxy;
    if (choice.kind === "browserbaseLocation") throw new ApiError("invalidInput", LOCATION_NEEDS_BROWSERBASE);
    if (choice.kind === "profile") {
      const profile = findProfile(choice.id, settings);
      choice = { kind: "profile", id: profile.id, name: profile.name, host: profile.host };
    }
  }

  let headless = visitor === "manual" ? false : (options.headless ?? defaultHeadless(visitor, settings));
  let persistentProfile = options.persistentProfile ?? settings.persistentProfile;
  if (remote) {
    // A cloud browser has no window, and keeps a profile only while the Browserbase switch allows it.
    headless = true;
    persistentProfile = settings.browserbase.persistentProfile && (options.persistentProfile ?? true);
  }

  return {
    proxy: proxyUseOf(choice),
    options: {
      ...options,
      visitor,
      extraPages,
      autoScroll: visitor !== "none",
      acceptConsent: visitor === "manual" ? false : options.acceptConsent,
      seed: options.seed ?? hashString(scanId),
      headless,
      persistentProfile,
      proxy: choice,
      budgetUsd,
      browser,
    },
  };
}
