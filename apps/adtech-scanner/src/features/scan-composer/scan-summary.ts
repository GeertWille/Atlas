import type { ProxyChoice, Settings } from "@/bindings";
import type { AiAvailability } from "@/shared/domain/ai-availability";
import { parseLocation, regionPlace } from "@/shared/domain/browserbase";
import { proxyChoiceFromKey, proxyChoiceLabel } from "@/shared/domain/proxy";
import { VISITOR_META } from "@/shared/domain/scan-options";
import { formatUsd, pluralize } from "@/shared/lib/format";
import { estimateScanCost } from "./cost-estimate";
import { proxyFieldFor, type ScanForm } from "./scan-form";

export type SummaryPart = { text: string; warning?: boolean };

type SummarySettings = Parameters<typeof estimateScanCost>[0] &
  Pick<Settings, "defaultProxy" | "proxies" | "persistentProfile" | "browserbase">;

/** The proxy the scan would use, or `null` while a Browserbase location is not filled in yet. */
function chosenProxy(form: ScanForm, settings: SummarySettings, remote: boolean): ProxyChoice | null {
  const field = proxyFieldFor(form, remote);
  if (field === "browserbaseLocation") {
    const location = parseLocation({
      country: form.locationCountry,
      state: form.locationState,
      city: form.locationCity,
    });
    return location.ok ? location.location : null;
  }
  if (field !== "settings") return proxyChoiceFromKey(field);
  return remote ? settings.browserbase.defaultProxy : settings.defaultProxy;
}

function proxyPart(form: ScanForm, settings: SummarySettings, proxyMissing: boolean, remote: boolean): SummaryPart {
  if (proxyMissing) return { text: "deleted proxy", warning: true };
  const choice = chosenProxy(form, settings, remote);
  if (choice === null) return { text: "Browserbase location not set", warning: true };
  if (choice.kind === "none") return { text: "no proxy" };
  if (choice.kind === "system") return { text: "system proxy" };
  if (choice.kind === "browserbaseLocation") return { text: proxyChoiceLabel(choice, settings.proxies) };
  return { text: `${proxyChoiceLabel(choice, settings.proxies)} proxy` };
}

function aiPart(form: ScanForm, settings: SummarySettings, ai: AiAvailability, budgetUsd: number | undefined): string {
  if (ai.triage === null) return "checks only, no API key";
  if (!form.aiEnabled) return "AI off";
  const extraPages = form.visitor === "browse" ? form.pages - 1 : 0;
  const cost = formatUsd(estimateScanCost(settings, { extraPages, budgetUsd }).expectedUsd);
  return ai.logicAnalysis ? `AI on, about ${cost}` : `script triage, about ${cost}`;
}

/**
 * What pressing Scan will do, as short parts for one line: "Browse like a visitor", "3 pages",
 * "browser shown", "Belgium proxy", "AI on, about $0.37". On Browserbase the place of the browser stands where
 * its window would: "Browserbase Frankfurt", "Belgium (Browserbase proxy)".
 */
export function scanSummary({
  form,
  settings,
  ai,
  budgetUsd,
  proxyMissing,
  remote,
  browserMissing,
}: {
  form: ScanForm;
  settings: SummarySettings;
  ai: AiAvailability;
  /** This scan's own cap, when the form sets a valid one. */
  budgetUsd: number | undefined;
  /** The form names a saved proxy that has been deleted. */
  proxyMissing: boolean;
  /** The scan runs in a Browserbase browser. */
  remote: boolean;
  /** Browserbase is chosen but its credentials are gone. */
  browserMissing: boolean;
}): SummaryPart[] {
  const region = form.region === "settings" ? settings.browserbase.region : form.region;
  const kept = remote
    ? settings.browserbase.persistentProfile && form.profile !== "fresh"
    : form.profile === "settings"
      ? settings.persistentProfile
      : form.profile === "kept";
  const shown = form.visitor === "manual" || form.showBrowser;
  let browser: SummaryPart = { text: shown ? "browser shown" : "browser hidden" };
  if (browserMissing) browser = { text: "Browserbase not set up", warning: true };
  else if (remote) browser = { text: `Browserbase ${regionPlace(region)}` };
  return [
    { text: VISITOR_META[form.visitor].label },
    ...(form.visitor === "browse" ? [{ text: pluralize(form.pages, "page") }] : []),
    browser,
    proxyPart(form, settings, proxyMissing, remote),
    ...(kept ? [{ text: "kept browser profile" }] : []),
    { text: aiPart(form, settings, ai, budgetUsd) },
  ];
}
