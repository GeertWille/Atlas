import type { NetworkCategory } from "@/bindings";
import { CATEGORY_META } from "../labels";
import { CATEGORY_VALUES } from "./keys";
import { parseQuery, type QueryTerm } from "./parse";

/** The chips are shortcuts for words of the filter: they add and remove tokens, nothing else. */
export const STATE_TOKENS = {
  failed: "is:failed",
  slow: "is:slow",
  blocked: "is:blocked,cancelled",
} as const;
export type StateChip = keyof typeof STATE_TOKENS;

const isCategoryTerm = (term: QueryTerm) => term.kind === "filter" && term.key === "cat" && !term.negated;

function without(query: string, drop: (term: QueryTerm) => boolean): string {
  return parseQuery(query)
    .terms.filter((term) => !drop(term))
    .map((term) => term.token.raw)
    .join(" ");
}

export function hasToken(query: string, token: string): boolean {
  return parseQuery(query).terms.some((term) => term.token.raw.toLowerCase() === token);
}

/** Adds the token, or removes it when it is there. */
export function toggleToken(query: string, token: string): string {
  if (hasToken(query, token)) return without(query, (term) => term.token.raw.toLowerCase() === token);
  return query.trim() === "" ? token : `${query.trim()} ${token}`;
}

/** The single category the filter narrows to, when it does so with one plain `cat:` word. */
export function activeCategory(query: string): NetworkCategory | null {
  const terms = parseQuery(query).terms.filter(isCategoryTerm);
  const [term] = terms;
  if (terms.length !== 1 || term?.kind !== "filter" || term.values.length !== 1) return null;
  return CATEGORY_VALUES[term.values[0]?.toLowerCase() ?? ""] ?? null;
}

/** Narrows the filter to one category, or with `null` to all of them. The rest of the filter stays. */
export function withCategory(query: string, category: NetworkCategory | null): string {
  const rest = without(query, isCategoryTerm);
  if (category === null) return rest;
  const token = `cat:${CATEGORY_META[category].token}`;
  return rest === "" ? token : `${rest} ${token}`;
}

/** The filter without its category words: what category counts are taken under. */
export function withoutCategory(query: string): string {
  return without(query, isCategoryTerm);
}

export function withoutToken(query: string, token: string): string {
  return without(query, (term) => term.token.raw.toLowerCase() === token);
}
