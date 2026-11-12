/**
 * Mirror of the backend price table (`crates/engine/src/domain/pricing.rs`),
 * in USD per million tokens. The backend charges scans against their budget;
 * this copy only feeds the estimate shown before a scan, and has to be changed
 * together with the backend table.
 */
type Rates = { input: number; output: number };

const HAIKU_LONG_PROMPT_TOKENS = 100_000;
const HAIKU_5_5: Rates = { input: 0.1, output: 0.5 };
const HAIKU_5_5_LONG_PROMPT: Rates = { input: 0.5, output: 2.5 };
const SONNET_5_5: Rates = { input: 2, output: 10 };
const JEV: Rates = { input: 0.042, output: 0 };
/** What the backend charges for a model it has no price for, so it can never under-count. */
const CONSERVATIVE: Rates = { input: 15, output: 75 };

/** The pinned TypeSafe Jev System One version the backend sends script triage to. */
export const JEV_MODEL = "jev-1.14.0";
/** The alias TypeSafe may report instead of the versioned id; priced the same. */
const JEV_ALIAS = "jev-latest";

function ratesFor(model: string, promptTokens: number): Rates | null {
  if (model === "claude-haiku-5-5") {
    return promptTokens > HAIKU_LONG_PROMPT_TOKENS ? HAIKU_5_5_LONG_PROMPT : HAIKU_5_5;
  }
  if (model === JEV_MODEL || model === JEV_ALIAS) return JEV;
  return model === "claude-sonnet-5-5" ? SONNET_5_5 : null;
}

export type CallCost = {
  usd: number;
  /** False when the model is not in the price table and the conservative fallback price applies. */
  knownModel: boolean;
};

/** Price of one model call with uncached prompt tokens, as the backend would charge it. */
export function costOf(model: string, tokens: { input: number; output: number }): CallCost {
  const rates = ratesFor(model, tokens.input);
  const applied = rates ?? CONSERVATIVE;
  return {
    usd: (tokens.input * applied.input + tokens.output * applied.output) / 1_000_000,
    knownModel: rates !== null,
  };
}

/** The Anthropic models the backend has prices for; suggested wherever a Claude model id is typed. */
export const PRICED_MODELS = ["claude-haiku-5-5", "claude-sonnet-5-5"] as const;

export function isJevModel(model: string): boolean {
  return model.startsWith("jev-");
}

/** `claude-haiku-5-5` → `Claude Haiku 5.5`, any Jev version → `Jev System One`; unknown ids are shown as they are. */
export function modelLabel(model: string): string {
  if (isJevModel(model)) return "Jev System One";
  const claude = /^claude-([a-z]+)-(\d+)-(\d+)$/.exec(model);
  if (!claude) return model;
  const [, family = "", major, minor] = claude;
  return `Claude ${family.charAt(0).toUpperCase()}${family.slice(1)} ${major}.${minor}`;
}
