import type { ScanDetail } from "@/bindings";
import { isScripted } from "@/shared/domain/scan-options";

export type EnvironmentNoticeContent = {
  /** What the pages could see, as facts: "navigator.webdriver was true". */
  signals: string[];
  /** The scan can be repeated with the browser window shown: it ran scripted and headless. */
  canShowBrowser: boolean;
};

/**
 * Whether a report has to say that its visit was recognisable as automated, and with what facts.
 * Read from the recorded environment alone, the same two signs the backend's own rule goes by.
 * `null` for a visit that gave no such sign, and for scans stored before the environment was recorded.
 */
export function environmentNotice(detail: Pick<ScanDetail, "environment">): EnvironmentNoticeContent | null {
  const { environment } = detail;
  if (!environment || !(environment.webdriver === true || environment.headless)) return null;
  return {
    signals: [
      environment.webdriver === true ? "navigator.webdriver was true" : null,
      environment.headless ? "Chrome ran headless" : null,
      ...environment.detectors.map(
        (detector) =>
          `${detector.vendor} (${detector.kind === "botProtection" ? "bot protection" : "invalid-traffic measurement"}) was on the page`,
      ),
    ].filter((signal) => signal !== null),
    canShowBrowser: environment.headless && isScripted(environment.visitor),
  };
}
