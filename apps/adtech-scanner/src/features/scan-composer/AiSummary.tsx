import { SparklesIcon } from "lucide-react";
import type { Settings } from "@/bindings";
import type { AiAvailability } from "@/shared/domain/ai-availability";
import { formatUsd, pluralize } from "@/shared/lib/format";
import { estimateScanCost } from "./cost-estimate";
import { SettingsLink } from "./SettingsLink";
import type { ScanForm } from "./scan-form";

/** What the scan will cost in model spend, or why it will cost nothing. */
export function AiSummary({
  settings,
  ai,
  form,
  budgetUsd,
}: {
  settings: Settings;
  ai: AiAvailability;
  form: ScanForm;
  /** This scan's own cap, when the form sets a valid one. */
  budgetUsd: number | undefined;
}) {
  if (ai.triage === null) {
    return (
      <p className="text-xs text-muted-foreground">
        No API key: this scan runs the deterministic checks only. <SettingsLink>Add a key</SettingsLink> to analyse
        wrapper logic.
      </p>
    );
  }
  if (!form.aiEnabled) {
    return <p className="text-xs text-muted-foreground">Deterministic checks only. No LLM cost.</p>;
  }
  const extraPages = form.visitor === "browse" ? form.pages - 1 : 0;
  const estimate = estimateScanCost(settings, { extraPages, budgetUsd });
  const amount = <span className="font-medium text-foreground tabular-nums">{formatUsd(estimate.expectedUsd)}</span>;
  if (!ai.logicAnalysis) {
    return (
      <p className="text-xs text-muted-foreground">
        <SparklesIcon className="mr-1 inline size-3.5 -translate-y-px text-primary-text" />
        Estimated {amount} for Jev script triage. <SettingsLink>Add an Anthropic key</SettingsLink> to analyse wrapper
        logic.
      </p>
    );
  }
  return (
    <p className="text-xs text-muted-foreground">
      <SparklesIcon className="mr-1 inline size-3.5 -translate-y-px text-primary-text" />
      Estimated {amount} with {settings.analysisModel}
      {ai.triage === "jev" ? " and Jev script triage" : ""}
      {extraPages > 0 ? ` for ${pluralize(extraPages + 1, "page")}` : ""}, hard cap{" "}
      {formatUsd(budgetUsd ?? settings.budgetUsd)} per scan
      {estimate.cappedByBudget ? ". Analysis may stop early at the cap." : "."}
      {estimate.fallbackPricing ? " No list price is known for this model, so a conservative rate applies." : ""}
    </p>
  );
}
