import type { DeviceProfile, ScanDetail, ScanOptions, Settings, VisitorBehavior } from "@/bindings";
import { anyAiAvailable } from "./ai-availability";

/** What the composer and the settings accept; the backend tolerates a wider range. */
export const OBSERVE_SECONDS = { min: 5, max: 60, step: 5 } as const;

/** Pages of one browse visit: the entry page plus up to five reached by following links. */
export const VISIT_PAGES = { min: 1, max: 6 } as const;

export const BUDGET_USD = { min: 0.05, max: 20 } as const;

/** A saved configuration's name can be this long; the backend enforces the same. */
export const CONFIGURATION_NAME_MAX = 80;

/** A manual session is recorded for at most this long; the backend ends the capture then. */
export const MANUAL_SESSION_LIMIT_MS = 15 * 60_000;

/** Least to most involved. */
export const VISITOR_BEHAVIORS: readonly VisitorBehavior[] = ["none", "scroll", "browse", "manual"];

export const VISITOR_META: Record<VisitorBehavior, { label: string; description: string }> = {
  none: {
    label: "Observe only",
    description: "Loads the page and watches it. Nothing is scrolled or clicked.",
  },
  scroll: {
    label: "Scroll",
    description: "Scrolls to the bottom in steps, so lazy-loaded slots get their chance.",
  },
  browse: {
    label: "Browse like a visitor",
    description: "Moves the pointer, scrolls and follows links to further pages of the same site.",
  },
  manual: {
    label: "I'll browse myself",
    description: "Chrome opens and records while you use the site. Nothing is clicked or scrolled for you.",
  },
};

/** Scans stored before visitor behaviours existed only know whether they scrolled. */
export function visitorOf(options: Pick<ScanOptions, "visitor" | "autoScroll">): VisitorBehavior {
  return options.visitor ?? (options.autoScroll ? "scroll" : "none");
}

/** A scripted visit is automated; a manual one is driven by a person. */
export function isScripted(visitor: VisitorBehavior): boolean {
  return visitor !== "manual";
}

/** Whether Chrome runs without a window when the scan does not say: browse and manual visits are shown. */
export function defaultHeadless(visitor: VisitorBehavior, settings: Pick<Settings, "headless">): boolean {
  return visitor === "none" || visitor === "scroll" ? settings.headless : false;
}

/** Options for a scan started without the composer (demo, other device, re-scan of a history row). */
export function defaultScanOptions(
  settings: Pick<
    Settings,
    "defaultObserveSeconds" | "defaultVisitor" | "defaultExtraPages" | "hasApiKey" | "hasTypesafeKey" | "jevEnabled"
  >,
  url: string,
  device: DeviceProfile,
): ScanOptions {
  const visitor = settings.defaultVisitor;
  return {
    url,
    device,
    observeSeconds: settings.defaultObserveSeconds,
    autoScroll: visitor !== "none",
    acceptConsent: visitor !== "manual",
    aiEnabled: anyAiAvailable(settings),
    visitor,
    extraPages: visitor === "browse" ? settings.defaultExtraPages : 0,
  };
}

/** The built-in demo site is served from this computer, so its scan runs here whatever the settings say. */
export function demoScanOptions(settings: Parameters<typeof defaultScanOptions>[0], demoUrl: string): ScanOptions {
  return { ...defaultScanOptions(settings, demoUrl, "desktop"), browser: { kind: "local" } };
}

/**
 * The same scan again: a scripted visit gets a new seed, so it moves and picks links afresh. A scan stored
 * before the browser could run anywhere else ran on this computer, and says nothing about it: it is sent
 * back there, not to wherever the settings would put a new scan today.
 */
export function rescanOptions(options: ScanOptions): ScanOptions {
  const { seed: _seed, ...rest } = options;
  return { ...rest, browser: options.browser ?? { kind: "local" } };
}

/**
 * The stored options replay a scripted visit exactly: same seed, so the same gestures and, on an
 * unchanged page, the same links. `null` when there is nothing to replay.
 */
export function repeatVisitOptions(detail: Pick<ScanDetail, "options">): ScanOptions | null {
  const { options } = detail;
  const visitor = visitorOf(options);
  if (options.seed === undefined || !(visitor === "scroll" || visitor === "browse")) return null;
  // Like a re-scan: a scan that does not say where its browser ran, ran on this computer.
  return { ...options, browser: options.browser ?? { kind: "local" } };
}
