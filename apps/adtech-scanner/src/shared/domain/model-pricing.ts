export type ModelPricing = { inputPerMTok: number; outputPerMTok: number };

/**
 * Anthropic list prices (USD per million tokens) for the current model of each
 * family, as of 2026-11. Used only for estimates shown before a scan; real
 * spend is always reported by the backend.
 */
const FAMILY_PRICING: Record<string, ModelPricing> = {
  haiku: { inputPerMTok: 0.1, outputPerMTok: 0.5 },
  sonnet: { inputPerMTok: 2, outputPerMTok: 10 },
  opus: { inputPerMTok: 4, outputPerMTok: 20 },
};

export const SUGGESTED_MODELS = ["claude-haiku-5-5", "claude-sonnet-5-5", "claude-opus-5-5"] as const;

/** Unknown model ids are priced like Sonnet, the default analysis model. */
export function pricingFor(model: string): ModelPricing {
  const family = Object.keys(FAMILY_PRICING).find((name) => model.includes(name));
  return FAMILY_PRICING[family ?? "sonnet"] as ModelPricing;
}
