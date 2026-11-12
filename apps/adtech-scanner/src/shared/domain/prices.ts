import type { Bid } from "@/bindings";

type Priced = Pick<Bid, "cpm" | "currency">;

/**
 * The currencies of the prices given, each once. Nothing in the app converts between currencies, so prices in
 * more than one are never added up, averaged or ranked against each other. A price that names no currency
 * (`null`) is not assumed to be in any of the others.
 */
export function currenciesOf(prices: Priced[]): (string | null)[] {
  return [...new Set(prices.filter((price) => price.cpm !== null).map((price) => price.currency))];
}

/** Whether prices in different currencies stand side by side, so that each has to say which it is in. */
export function hasMixedCurrencies(prices: Priced[]): boolean {
  return currenciesOf(prices).length > 1;
}

/** Sorts the higher price first. Prices in different currencies are left in the order they have. */
export function byPriceDescending(a: Priced, b: Priced): number {
  if (a.cpm !== null && b.cpm !== null && a.currency !== b.currency) return 0;
  return (b.cpm ?? 0) - (a.cpm ?? 0);
}

/** "EUR and USD", for saying which currencies were not converted. */
export function currencyList(currencies: (string | null)[]): string {
  return currencies
    .map((currency) => currency ?? "an unnamed currency")
    .sort()
    .join(" and ");
}
