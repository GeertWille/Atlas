import type { AutomationDetector, ScanDetail } from "@/bindings";
import { isScripted } from "@/shared/domain/scan-options";

export type EnvironmentNoticeContent =
  | {
      /** The pages could recognise the visit as automated. */
      kind: "identifiable";
      /** What the pages could see, as facts: "navigator.webdriver was true". */
      signals: string[];
      /** The scan can be repeated with the browser window shown: it ran scripted and headless on this computer. */
      canShowBrowser: boolean;
    }
  | {
      /**
       * The visit ran in a Browserbase browser, which gave the pages no sign of automation. What was recorded
       * of that browser is listed once, under the report's Browser details.
       */
      kind: "browserbase";
      /** A person drove the browser through the live view. */
      manual: boolean;
      /** Services on the pages that tell automated visits apart: "DoubleVerify (…) was on the page". */
      detectors: string[];
    };

function detectorSignals(detectors: AutomationDetector[]): string[] {
  return detectors.map(
    (detector) =>
      `${detector.vendor} (${detector.kind === "botProtection" ? "bot protection" : "invalid-traffic measurement"}) was on the page`,
  );
}

/**
 * What a report has to say about how its visit looked to the pages, from the recorded environment alone.
 * A visit on this computer is reported when the pages could tell it was automated, by the same two signs
 * the backend's own rule goes by. A Browserbase visit is always reported: its browser does not show the
 * pages that it is automated or headless, so saying they could tell would be wrong, but it is not the
 * browser of a person either. `null` for a visit that gave no sign on this computer, and for scans stored
 * before the environment was recorded.
 */
export function environmentNotice(detail: Pick<ScanDetail, "environment">): EnvironmentNoticeContent | null {
  const { environment } = detail;
  if (!environment) return null;
  const { browser } = environment;
  if (browser.kind === "browserbase" && environment.webdriver !== true) {
    return {
      kind: "browserbase",
      manual: !isScripted(environment.visitor),
      detectors: detectorSignals(environment.detectors),
    };
  }
  // A Browserbase browser that did show the flag is automated in the pages' eyes; its headless mode still does not show.
  const headless = environment.headless && browser.kind === "local";
  if (!(environment.webdriver === true || headless)) return null;
  return {
    kind: "identifiable",
    signals: [
      environment.webdriver === true ? "navigator.webdriver was true" : null,
      headless ? "Chrome ran headless" : null,
      ...detectorSignals(environment.detectors),
    ].filter((signal) => signal !== null),
    canShowBrowser: headless && isScripted(environment.visitor),
  };
}
