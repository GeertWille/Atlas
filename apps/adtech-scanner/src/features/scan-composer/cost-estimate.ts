import type { Settings } from "@/bindings";
import { pricingFor } from "@/shared/domain/model-pricing";

/** Typical token volume of a scan, from the pipeline design (about $0.50 at default settings is the ceiling). */
const FILTER_TOKENS = { input: 15_000, output: 1_300 };
const ANALYSIS_TOKENS = { input: 145_000, output: 7_000 };
const EFFORT_SCALE = { low: 0.7, medium: 1, high: 1.45 } as const;

function cost(model: string, tokens: { input: number; output: number }, scale = 1): number {
  const { inputPerMTok, outputPerMTok } = pricingFor(model);
  return ((tokens.input * inputPerMTok + tokens.output * outputPerMTok) * scale) / 1_000_000;
}

export type CostEstimate = {
  /** Expected LLM spend in USD, never above the budget cap. */
  expectedUsd: number;
  /** The uncapped estimate exceeds the budget, so analysis may stop early. */
  cappedByBudget: boolean;
};

export function estimateScanCost(
  settings: Pick<Settings, "filterModel" | "analysisModel" | "analysisEffort" | "budgetUsd">,
): CostEstimate {
  const uncapped =
    cost(settings.filterModel, FILTER_TOKENS) +
    cost(settings.analysisModel, ANALYSIS_TOKENS, EFFORT_SCALE[settings.analysisEffort]);
  return {
    expectedUsd: Math.min(uncapped, settings.budgetUsd),
    cappedByBudget: uncapped > settings.budgetUsd,
  };
}
