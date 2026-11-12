import type { Settings } from "@/bindings";
import type { AiAvailability } from "@/shared/domain/ai-availability";
import { proxyChoiceFromKey, proxyChoiceLabel } from "@/shared/domain/proxy";
import { VISITOR_META } from "@/shared/domain/scan-options";
import { formatUsd, pluralize } from "@/shared/lib/format";
import { estimateScanCost } from "./cost-estimate";
import type { ScanForm } from "./scan-form";

export type SummaryPart = { text: string; warning?: boolean };

type SummarySettings = Parameters<typeof estimateScanCost>[0] &
  Pick<Settings, "defaultProxy" | "proxies" | "persistentProfile">;

function proxyPart(form: ScanForm, settings: SummarySettings, proxyMissing: boolean): SummaryPart {
  const choice = form.proxy === "settings" ? settings.defaultProxy : proxyChoiceFromKey(form.proxy);
  if (proxyMissing) return { text: "deleted proxy", warning: true };
  if (choice.kind === "none") return { text: "no proxy" };
  if (choice.kind === "system") return { text: "system proxy" };
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
 * "browser shown", "Belgium proxy", "AI on, about $0.37".
 */
export function scanSummary({
  form,
  settings,
  ai,
  budgetUsd,
  proxyMissing,
}: {
  form: ScanForm;
  settings: SummarySettings;
  ai: AiAvailability;
  /** This scan's own cap, when the form sets a valid one. */
  budgetUsd: number | undefined;
  /** The form names a saved proxy that has been deleted. */
  proxyMissing: boolean;
}): SummaryPart[] {
  const kept = form.profile === "settings" ? settings.persistentProfile : form.profile === "kept";
  const shown = form.visitor === "manual" || form.showBrowser;
  return [
    { text: VISITOR_META[form.visitor].label },
    ...(form.visitor === "browse" ? [{ text: pluralize(form.pages, "page") }] : []),
    { text: shown ? "browser shown" : "browser hidden" },
    proxyPart(form, settings, proxyMissing),
    ...(kept ? [{ text: "kept browser profile" }] : []),
    { text: aiPart(form, settings, ai, budgetUsd) },
  ];
}
