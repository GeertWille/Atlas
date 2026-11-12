import type { Effort, Settings } from "@/bindings";
import { costOf } from "@/shared/domain/model-pricing";

/** Assumed token volume of a typical scan at medium effort; real spend is reported by the backend. */
const FILTER_TOKENS = { input: 15_000, output: 1_300 };
const ANALYSIS_TOKENS = { input: 145_000, output: 7_000 };
/** The backend lets the analysis loop run 12, 20 or 30 turns. */
const EFFORT_SCALE: Record<Effort, number> = { low: 0.6, medium: 1, high: 1.5 };

export type CostEstimate = {
  /** Expected LLM spend in USD, never above the budget cap. */
  expectedUsd: number;
  /** The uncapped estimate exceeds the budget, so analysis may stop early. */
  cappedByBudget: boolean;
  /** A model has no known price: the backend counts it against the cap at a conservative fallback rate. */
  fallbackPricing: boolean;
};

export function estimateScanCost(
  settings: Pick<Settings, "filterModel" | "analysisModel" | "analysisEffort" | "budgetUsd">,
): CostEstimate {
  const filter = costOf(settings.filterModel, FILTER_TOKENS);
  const analysis = costOf(settings.analysisModel, ANALYSIS_TOKENS);
  const uncapped = filter.usd + analysis.usd * EFFORT_SCALE[settings.analysisEffort];
  return {
    expectedUsd: Math.min(uncapped, settings.budgetUsd),
    cappedByBudget: uncapped > settings.budgetUsd,
    fallbackPricing: !filter.knownModel || !analysis.knownModel,
  };
}
