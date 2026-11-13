import type { BrowserInfo, Settings } from "@/bindings";

const DISMISSED_KEY = "adscan.onboarding.done";

export function isOnboardingDismissed(): boolean {
  try {
    return localStorage.getItem(DISMISSED_KEY) === "1";
  } catch {
    return false;
  }
}

export function dismissOnboarding() {
  try {
    localStorage.setItem(DISMISSED_KEY, "1");
  } catch {
    // Without storage the welcome screen simply shows again next launch.
  }
}

/**
 * First run: something is still missing and the user has not chosen to skip setup. A stored key of
 * either provider counts as set up: someone who works with a TypeSafe key alone has made their choice
 * and is not asked for an Anthropic key on every launch. So does Browserbase in place of a local
 * browser: scans have somewhere to run.
 */
export function needsOnboarding(
  browser: BrowserInfo | null,
  settings: Pick<Settings, "hasApiKey" | "hasTypesafeKey" | "browserbase">,
  dismissed: boolean,
): boolean {
  const hasAnyKey = settings.hasApiKey || settings.hasTypesafeKey;
  const hasBrowser = browser !== null || settings.browserbase.connected;
  return !dismissed && (!hasBrowser || !hasAnyKey);
}
