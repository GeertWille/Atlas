import prices from "./model-prices.json";

/** USD per million tokens. */
type Rates = { input: number; output: number };
type Price = Rates & { longPrompt: Rates | null };

/**
 * The backend's price table. `model-prices.json` is written from `crates/engine/src/domain/pricing.rs` by that
 * file's own test, never by hand. The backend charges scans against their budget; here the prices only feed the
 * estimate shown before a scan.
 */
const PRICE_TABLE: Record<string, Price> = prices.models;

/** The pinned TypeSafe Jev System One version the backend sends script triage to. */
export const JEV_MODEL = prices.jevModel;

function ratesFor(model: string, promptTokens: number): Rates | null {
  const price = PRICE_TABLE[model];
  if (!price) return null;
  return price.longPrompt && promptTokens > prices.longPromptTokens ? price.longPrompt : price;
}

export type CallCost = {
  usd: number;
  /** False when the model is not in the price table and the conservative fallback price applies. */
  knownModel: boolean;
};

/** Price of one model call with uncached prompt tokens, as the backend would charge it. */
export function costOf(model: string, tokens: { input: number; output: number }): CallCost {
  const rates = ratesFor(model, tokens.input);
  // What the backend charges for a model it has no price for, so it can never under-count.
  const applied = rates ?? prices.unknownModel;
  return {
    usd: (tokens.input * applied.input + tokens.output * applied.output) / 1_000_000,
    knownModel: rates !== null,
  };
}

export function isJevModel(model: string): boolean {
  return model.startsWith("jev-");
}

/** The Anthropic models the backend has prices for; suggested wherever a Claude model id is typed. */
export const PRICED_MODELS = Object.keys(PRICE_TABLE).filter((model) => !isJevModel(model));

/** `claude-haiku-5-5` → `Claude Haiku 5.5`, any Jev version → `Jev System One`; unknown ids are shown as they are. */
export function modelLabel(model: string): string {
  if (isJevModel(model)) return "Jev System One";
  const claude = /^claude-([a-z]+)-(\d+)-(\d+)$/.exec(model);
  if (!claude) return model;
  const [, family = "", major, minor] = claude;
  return `Claude ${family.charAt(0).toUpperCase()}${family.slice(1)} ${major}.${minor}`;
}
