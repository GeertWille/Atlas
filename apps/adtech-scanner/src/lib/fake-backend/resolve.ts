import type { ProxyUse, ScanOptions, Settings } from "@/bindings";
import { ApiError } from "@/lib/api/error";
import { BUDGET_USD, defaultHeadless, visitorOf } from "@/shared/domain/scan-options";
import { hashString } from "./rng";
import { MAX_LATER_PAGES } from "./visit";

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
    throw new ApiError("invalidInput", "Enter an absolute http(s) URL");
  }
  const visitor = visitorOf(options);
  const extraPages = visitor === "browse" ? (options.extraPages ?? settings.defaultExtraPages) : 0;
  if ((options.extraPages ?? 0) > MAX_LATER_PAGES) {
    throw new ApiError("invalidInput", `A visit can follow links to at most ${MAX_LATER_PAGES} further pages`);
  }
  const budgetUsd = options.budgetUsd ?? settings.budgetUsd;
  if (budgetUsd < BUDGET_USD.min || budgetUsd > BUDGET_USD.max) {
    throw new ApiError("invalidInput", `The per-scan budget must be between $${BUDGET_USD.min} and $${BUDGET_USD.max}`);
  }

  const choice = options.proxy ?? settings.defaultProxy;
  let proxy: ProxyUse = { kind: choice.kind, name: null, host: null };
  if (choice.kind === "profile") {
    const profile = settings.proxies.find((candidate) => candidate.id === choice.id);
    if (!profile) throw new ApiError("invalidInput", "The proxy profile this scan names no longer exists");
    proxy = { kind: "profile", name: profile.name, host: profile.host };
  }

  return {
    proxy,
    options: {
      ...options,
      visitor,
      extraPages,
      autoScroll: visitor !== "none",
      acceptConsent: visitor === "manual" ? false : options.acceptConsent,
      seed: options.seed ?? hashString(scanId),
      headless: visitor === "manual" ? false : (options.headless ?? defaultHeadless(visitor, settings)),
      persistentProfile: options.persistentProfile ?? settings.persistentProfile,
      proxy:
        choice.kind === "profile"
          ? { kind: "profile", id: choice.id, name: proxy.name ?? undefined, host: proxy.host ?? undefined }
          : choice,
      budgetUsd,
    },
  };
}
