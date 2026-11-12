import type { Browser } from "./app";

export type CspViolation = { directive: string; blocked: string };

type CspWindow = Window & { __cspViolations?: CspViolation[] };

/** Records every Content-Security-Policy violation from now until the next reload. */
export async function watchCspViolations(browser: Browser) {
  await browser.execute(() => {
    const violations: CspViolation[] = [];
    (window as CspWindow).__cspViolations = violations;
    document.addEventListener("securitypolicyviolation", (event) => {
      violations.push({ directive: event.effectiveDirective, blocked: event.blockedURI });
    });
  });
}

export function cspViolations(browser: Browser): Promise<CspViolation[]> {
  return browser.execute(() => (window as CspWindow).__cspViolations ?? []);
}
