import type { DeviceProfile, ScanOptions, Settings, VisitorBehavior } from "@/bindings";
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
  proxy: ProxyField;
  profile: ProfileField;
  /** LLM budget for this scan in dollars as typed; empty follows the settings. */
  budget: string;
};

type FormSettings = Pick<
  Settings,
  "defaultDevice" | "defaultObserveSeconds" | "defaultVisitor" | "defaultExtraPages" | "headless"
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
    proxy: "settings",
    profile: "settings",
    budget: "",
  };
}

/** The form that produces these options again: how a saved configuration is opened for editing. */
export function formFromOptions(options: ScanOptions, settings: FormSettings): ScanForm {
  const visitor = visitorOf(options);
  let profile: ProfileField = "settings";
  if (options.persistentProfile !== undefined) profile = options.persistentProfile ? "kept" : "fresh";
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
    proxy: options.proxy ? proxyChoiceKey(options.proxy) : "settings",
    profile,
    budget: options.budgetUsd === undefined ? "" : options.budgetUsd.toFixed(2),
  };
}

/** Changing the behaviour resets the browser window to that behaviour's default. */
export function withVisitor(form: ScanForm, visitor: VisitorBehavior, settings: Pick<Settings, "headless">): ScanForm {
  return { ...form, visitor, showBrowser: !defaultHeadless(visitor, settings) };
}

export type ScanFormErrors = { url?: string; budget?: string };

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
  if (!url.ok || "error" in budget) {
    return {
      ok: false,
      errors: { url: url.ok ? undefined : url.error, budget: "error" in budget ? budget.error : undefined },
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
      // A manual session always opens a window: there is nobody to browse a hidden one.
      headless: manual ? false : !form.showBrowser,
      ...(form.proxy === "settings" ? {} : { proxy: proxyChoiceFromKey(form.proxy) }),
      ...(form.profile === "settings" ? {} : { persistentProfile: form.profile === "kept" }),
      ...(budget.value === undefined ? {} : { budgetUsd: budget.value }),
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
