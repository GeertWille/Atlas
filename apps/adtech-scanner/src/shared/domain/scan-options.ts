import type { DeviceProfile, ScanOptions, Settings } from "@/bindings";

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
