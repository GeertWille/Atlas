import type { ScanDetail } from "@/bindings";
import { regionName } from "@/shared/domain/browserbase";
import { proxyUseLabel } from "@/shared/domain/proxy";
import { VISITOR_META } from "@/shared/domain/scan-options";
import { pluralize } from "@/shared/lib/format";

/**
 * How the scan visited the site, as short facts: behaviour, pages, browser window or cloud region, profile
 * and proxy.
 * `null` for scans stored before the visit was recorded.
 */
export function visitFacts(detail: Pick<ScanDetail, "environment" | "pages">): string[] | null {
  const { environment, pages } = detail;
  if (!environment) return null;
  const { browser } = environment;
  return [
    VISITOR_META[environment.visitor].label,
    pluralize(Math.max(1, pages.length), "page"),
    browser.kind === "browserbase"
      ? `Browserbase, ${regionName(browser.region)}`
      : environment.headless
        ? "Headless"
        : "Browser shown",
    environment.persistentProfile ? "Kept browser profile" : "Fresh browser profile",
    proxyUseLabel(environment.proxy),
    ...(environment.emulatedMobile ? ["Mobile emulation"] : []),
  ];
}
