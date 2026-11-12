import type { ScanDetail, ScanEvent } from "../../src/bindings";
import type { Browser } from "./app";

export type ScanProbe = {
  events: ScanEvent[];
  /** `get_scan` as answered the moment the scan's final `scanUpdated` event arrived. */
  detailAtFinalEvent: ScanDetail | null;
};

type ProbeWindow = Window & {
  __scanProbe?: ScanProbe;
  __TAURI_INTERNALS__: {
    invoke<T>(command: string, args?: unknown): Promise<T>;
    transformCallback(callback: (event: { payload: ScanEvent }) => void): number;
  };
};

/**
 * Subscribes to `scan-event` next to the app's own listener, through the same
 * IPC call `@tauri-apps/api` makes, and records what the backend pushes.
 */
export async function installScanProbe(browser: Browser) {
  await browser.execute(() => {
    const page = window as unknown as ProbeWindow;
    const probe: ScanProbe = { events: [], detailAtFinalEvent: null };
    page.__scanProbe = probe;
    const handler = page.__TAURI_INTERNALS__.transformCallback(({ payload }) => {
      probe.events.push(payload);
      if (payload.type === "scanUpdated" && !["queued", "running"].includes(payload.summary.status)) {
        void page.__TAURI_INTERNALS__.invoke<ScanDetail>("get_scan", { scanId: payload.summary.id }).then((detail) => {
          probe.detailAtFinalEvent = detail;
        });
      }
    });
    return page.__TAURI_INTERNALS__.invoke("plugin:event|listen", {
      event: "scan-event",
      target: { kind: "Any" },
      handler,
    });
  });
}

export function readScanProbe(browser: Browser): Promise<ScanProbe> {
  return browser.execute(() => {
    const probe = (window as unknown as ProbeWindow).__scanProbe;
    if (!probe) throw new Error("the scan probe is not installed");
    return probe;
  });
}
