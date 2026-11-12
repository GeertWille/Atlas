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

/** First run: something is still missing and the user has not chosen to skip setup. */
export function needsOnboarding(
  browser: BrowserInfo | null,
  settings: Pick<Settings, "hasApiKey">,
  dismissed: boolean,
): boolean {
  return !dismissed && (browser === null || !settings.hasApiKey);
}
