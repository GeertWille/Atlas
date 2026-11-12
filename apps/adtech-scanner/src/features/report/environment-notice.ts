import type { Finding, ScanDetail } from "@/bindings";
import { isScripted } from "@/shared/domain/scan-options";

/**
 * The backend's "scan identifiable as automated" finding. Findings carry no rule id, so it is
 * recognised by what it is: an informational configuration finding of a rule about automated traffic.
 */
export function isAutomatedTrafficFinding(finding: Finding): boolean {
  return (
    finding.source === "heuristic" &&
    finding.severity === "info" &&
    finding.category === "configuration" &&
    /automated/i.test(finding.title)
  );
}

export type EnvironmentNoticeContent = {
  title: string;
  /** What the pages could see, as facts: "navigator.webdriver was true". */
  signals: string[];
  /** The scan can be repeated with the browser window shown: it ran scripted and headless. */
  canShowBrowser: boolean;
};

/**
 * Whether a report has to say that its visit was recognisable as automated, and with what facts.
 * `null` for a visit that gave no such sign, and for scans stored before the environment was recorded.
 */
export function environmentNotice(
  detail: Pick<ScanDetail, "environment" | "findings">,
): EnvironmentNoticeContent | null {
  const { environment } = detail;
  const flagged = detail.findings.some(isAutomatedTrafficFinding);
  if (!flagged && environment?.webdriver !== true) return null;
  const signals = environment
    ? [
        environment.webdriver ? "navigator.webdriver was true" : null,
        environment.headless ? "Chrome ran headless" : null,
        ...environment.detectors.map(
          (detector) =>
            `${detector.vendor} (${detector.kind === "botProtection" ? "bot protection" : "invalid-traffic measurement"}) was on the page`,
        ),
      ].filter((signal) => signal !== null)
    : [];
  return {
    title: flagged
      ? "Bidders may have treated this scan as automated traffic"
      : "The pages could tell that this visit was automated",
    signals,
    canShowBrowser: environment?.headless === true && isScripted(environment.visitor),
  };
}
