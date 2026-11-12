import type { DeviceProfile, ScanOptions, Settings } from "@/bindings";

/** What the composer and the settings accept; the backend tolerates a wider range. */
export const OBSERVE_SECONDS = { min: 5, max: 60, step: 5 } as const;

/** Options for a scan started without the composer (re-scan, demo, other device). */
export function defaultScanOptions(
  settings: Pick<Settings, "defaultObserveSeconds" | "hasApiKey">,
  url: string,
  device: DeviceProfile,
): ScanOptions {
  return {
    url,
    device,
    observeSeconds: settings.defaultObserveSeconds,
    autoScroll: true,
    acceptConsent: true,
    aiEnabled: settings.hasApiKey,
  };
}
