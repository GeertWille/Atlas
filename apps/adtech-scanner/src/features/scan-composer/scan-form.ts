import type {
  BrowserbaseRegion,
  BrowserKind,
  DeviceProfile,
  ProxyChoice,
  ScanOptions,
  Settings,
  VisitorBehavior,
} from "@/bindings";
import { browserKindOf, cityName, parseLocation, reachableFromCloud } from "@/shared/domain/browserbase";
import { type ProxyChoiceKey, proxyChoiceFromKey, proxyChoiceKey } from "@/shared/domain/proxy";
import { BUDGET_USD, defaultHeadless, VISIT_PAGES, visitorOf } from "@/shared/domain/scan-options";
import { normalizeScanUrl } from "./scan-url";

/** `settings`: the scan leaves the choice to the settings at the time it runs. */
export type ProxyField = "settings" | ProxyChoiceKey;
export type ProfileField = "settings" | "fresh" | "kept";

export type ScanForm = {
  url: string;
  device: DeviceProfile;
  observeSeconds: number;
  visitor: VisitorBehavior;
  /** Pages of a browse visit, the entry page included. Kept while another behaviour is selected. */
  pages: number;
  showBrowser: boolean;
  acceptConsent: boolean;
  aiEnabled: boolean;
  /** Where the browser runs, as chosen. An address a cloud browser cannot reach is scanned on this Mac anyway. */
  browser: BrowserKind;
  /** The Browserbase region; kept while the browser runs on this Mac. */
  region: "settings" | BrowserbaseRegion;
  proxy: ProxyField;
  /** The Browserbase location as typed; used while `proxy` is `browserbaseLocation`. */
  locationCountry: string;
  locationState: string;
  locationCity: string;
  profile: ProfileField;
  /** LLM budget for this scan in dollars as typed; empty follows the settings. */
  budget: string;
};

type FormSettings = Pick<
  Settings,
  | "defaultDevice"
  | "defaultObserveSeconds"
  | "defaultVisitor"
  | "defaultExtraPages"
  | "headless"
  | "defaultBrowser"
  | "browserbase"
>;

export function newScanForm(settings: FormSettings): ScanForm {
  return {
    url: "",
    device: settings.defaultDevice,
    observeSeconds: settings.defaultObserveSeconds,
    visitor: settings.defaultVisitor,
    pages: settings.defaultExtraPages + 1,
    showBrowser: !defaultHeadless(settings.defaultVisitor, settings),
    acceptConsent: true,
    aiEnabled: true,
    browser: settings.browserbase.connected ? settings.defaultBrowser : "local",
    region: "settings",
    proxy: "settings",
    locationCountry: "",
    locationState: "",
    locationCity: "",
    profile: "settings",
    budget: "",
  };
}

/** The form that produces these options again: how a saved configuration is opened for editing. */
export function formFromOptions(options: ScanOptions, settings: FormSettings): ScanForm {
  const visitor = visitorOf(options);
  let profile: ProfileField = "settings";
  if (options.persistentProfile !== undefined) profile = options.persistentProfile ? "kept" : "fresh";
  const location = options.proxy?.kind === "browserbaseLocation" ? options.proxy : null;
  return {
    url: options.url,
    device: options.device,
    observeSeconds: options.observeSeconds,
    visitor,
    pages: (visitor === "browse" ? (options.extraPages ?? settings.defaultExtraPages) : settings.defaultExtraPages) + 1,
    showBrowser: !(options.headless ?? defaultHeadless(visitor, settings)),
    // A manual visit never clicks the banner; the switch keeps its usual value for the other behaviours.
    acceptConsent: visitor === "manual" ? true : options.acceptConsent,
    aiEnabled: options.aiEnabled,
    // Options that do not say run where the settings put them, like the backend decides.
    browser: browserKindOf(options, settings),
    region: (options.browser?.kind === "browserbase" ? options.browser.region : undefined) ?? "settings",
    proxy: options.proxy ? proxyChoiceKey(options.proxy) : "settings",
    locationCountry: location?.country ?? "",
    locationState: location?.state ?? "",
    locationCity: location?.city ? cityName(location.city) : "",
    profile,
    budget: options.budgetUsd === undefined ? "" : options.budgetUsd.toFixed(2),
  };
}

/** Changing the behaviour resets the browser window to that behaviour's default. */
export function withVisitor(form: ScanForm, visitor: VisitorBehavior, settings: Pick<Settings, "headless">): ScanForm {
  return { ...form, visitor, showBrowser: !defaultHeadless(visitor, settings) };
}

/**
 * Moving the browser takes the proxy along where it can go: this Mac's own proxy means nothing to a cloud
 * browser, and a Browserbase location nothing to a browser on this Mac. Those fall back to the settings.
 */
export function withBrowser(form: ScanForm, browser: BrowserKind): ScanForm {
  return { ...form, browser, proxy: proxyFieldFor(form, browser === "browserbase") };
}

/**
 * The proxy choice as it holds where the browser runs. One that cannot go along (this Mac's own proxy to a
 * cloud browser, a Browserbase location to this Mac) is left to the settings, also while the form still
 * remembers it for when the address changes back.
 */
export function proxyFieldFor(form: Pick<ScanForm, "proxy">, remote: boolean): ProxyField {
  return form.proxy === (remote ? "system" : "browserbaseLocation") ? "settings" : form.proxy;
}

/** A scan runs on Browserbase when that is chosen and the address is one a cloud browser can reach. */
export function runsOnBrowserbase(form: Pick<ScanForm, "browser">, url: string): boolean {
  return form.browser === "browserbase" && reachableFromCloud(url);
}

export type ScanFormErrors = { url?: string; budget?: string; location?: string };

export type ScanFormResult = { ok: true; options: ScanOptions } | { ok: false; errors: ScanFormErrors };

function parseBudget(input: string): { value: number | undefined } | { error: string } {
  const trimmed = input.trim().replace(/^\$/, "");
  if (trimmed === "") return { value: undefined };
  const value = Number(trimmed);
  if (!Number.isFinite(value) || value < BUDGET_USD.min || value > BUDGET_USD.max) {
    return { error: `Enter an amount from $${BUDGET_USD.min.toFixed(2)} to $${BUDGET_USD.max.toFixed(2)}.` };
  }
  return { value };
}

export function validateScanForm(form: ScanForm): ScanFormResult {
  const url = normalizeScanUrl(form.url);
  const budget = parseBudget(form.budget);
  const remote = url.ok && runsOnBrowserbase(form, url.url);
  const proxyField = proxyFieldFor(form, remote);
  let proxy: ProxyChoice | undefined;
  let locationError: string | undefined;
  if (proxyField === "browserbaseLocation") {
    const location = parseLocation({
      country: form.locationCountry,
      state: form.locationState,
      city: form.locationCity,
    });
    if (location.ok) proxy = location.location;
    else locationError = location.error;
  } else if (proxyField !== "settings") {
    proxy = proxyChoiceFromKey(proxyField);
  }
  if (!url.ok || "error" in budget || locationError) {
    return {
      ok: false,
      errors: {
        url: url.ok ? undefined : url.error,
        budget: "error" in budget ? budget.error : undefined,
        location: locationError,
      },
    };
  }
  const manual = form.visitor === "manual";
  const pages = Math.min(VISIT_PAGES.max, Math.max(VISIT_PAGES.min, form.pages));
  return {
    ok: true,
    options: {
      url: url.url,
      device: form.device,
      observeSeconds: form.observeSeconds,
      autoScroll: form.visitor !== "none",
      acceptConsent: manual ? false : form.acceptConsent,
      aiEnabled: form.aiEnabled,
      visitor: form.visitor,
      extraPages: form.visitor === "browse" ? pages - 1 : 0,
      // A manual session on this Mac always opens a window: there is nobody to browse a hidden one. A cloud
      // browser never has one.
      headless: remote || (manual ? false : !form.showBrowser),
      ...(proxy ? { proxy } : {}),
      ...(form.profile === "settings" ? {} : { persistentProfile: form.profile === "kept" }),
      ...(budget.value === undefined ? {} : { budgetUsd: budget.value }),
      browser: remote
        ? { kind: "browserbase", ...(form.region === "settings" ? {} : { region: form.region }) }
        : { kind: "local" },
    },
  };
}

/**
 * Whether two forms describe the same scan. What differs only in how it was typed ("0.5" and "0.50"),
 * or in a field the chosen visitor does not use, is the same scan.
 */
export function sameScan(a: ScanForm, b: ScanForm): boolean {
  const [left, right] = [validateScanForm(a), validateScanForm(b)];
  if (!left.ok || !right.ok) return JSON.stringify(a) === JSON.stringify(b);
  return JSON.stringify(left.options) === JSON.stringify(right.options);
}
