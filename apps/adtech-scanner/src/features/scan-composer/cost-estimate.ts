import type { Effort, Settings } from "@/bindings";
import { aiAvailability } from "@/shared/domain/ai-availability";
import { costOf, JEV_MODEL } from "@/shared/domain/model-pricing";

/** Assumed token volume of a typical scan at medium effort; real spend is reported by the backend. */
const FILTER_TOKENS = { input: 15_000, output: 1_300 };
const ANALYSIS_TOKENS = { input: 145_000, output: 7_000 };
/**
 * Jev reads the parts of every undecided script that name an ad identifier: 33,847 input tokens on the
 * first real publisher page (2026-11-12, 15 requests, about 2.2 characters of minified JavaScript per token).
 */
const JEV_TRIAGE_TOKENS = 35_000;
/**
 * When logic analysis follows, Jev reads the selected scripts again for the reading guide. Not measured
 * yet: the modelled volume, rescaled to the measured characters per token.
 */
const JEV_READING_GUIDE_TOKENS = 195_000;
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
  settings: Pick<
    Settings,
    "filterModel" | "analysisModel" | "analysisEffort" | "budgetUsd" | "hasApiKey" | "hasTypesafeKey" | "jevEnabled"
  >,
): CostEstimate {
  const { triage, logicAnalysis } = aiAvailability(settings);
  const filter =
    triage === "jev"
      ? costOf(JEV_MODEL, {
          input: JEV_TRIAGE_TOKENS + (logicAnalysis ? JEV_READING_GUIDE_TOKENS : 0),
          output: 0,
        })
      : costOf(settings.filterModel, FILTER_TOKENS);
  const analysis = logicAnalysis ? costOf(settings.analysisModel, ANALYSIS_TOKENS) : { usd: 0, knownModel: true };
  const uncapped = filter.usd + analysis.usd * EFFORT_SCALE[settings.analysisEffort];
  return {
    expectedUsd: Math.min(uncapped, settings.budgetUsd),
    cappedByBudget: uncapped > settings.budgetUsd,
    fallbackPricing: !filter.knownModel || !analysis.knownModel,
  };
}
