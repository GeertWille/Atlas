import { expect, test } from "vitest";
import { byPriceDescending, currenciesOf, currencyList, hasMixedCurrencies } from "./prices";

const eur = { cpm: 10.01, currency: "EUR" };
const usd = { cpm: 2.05, currency: "USD" };
const cheapUsd = { cpm: 0.4, currency: "USD" };
const unnamed = { cpm: 3, currency: null };
const noBid = { cpm: null, currency: null };

test("only bids with a price have a currency to count", () => {
  expect(currenciesOf([usd, cheapUsd, noBid])).toEqual(["USD"]);
  expect(hasMixedCurrencies([usd, cheapUsd, noBid])).toBe(false);
  expect(hasMixedCurrencies([])).toBe(false);
  expect(currenciesOf([usd, eur, cheapUsd])).toEqual(["USD", "EUR"]);
  expect(hasMixedCurrencies([usd, eur])).toBe(true);
  // A price without a currency is not taken to be in dollars.
  expect(hasMixedCurrencies([usd, unnamed])).toBe(true);
  expect(currencyList(["USD", "EUR"])).toBe("EUR and USD");
});

test("prices are ranked within a currency and never across two", () => {
  expect([cheapUsd, usd].sort(byPriceDescending)).toEqual([usd, cheapUsd]);
  // 10.01 EUR is not "more than" 2.05 USD by its number, nor less: the order given stays.
  expect([usd, eur].sort(byPriceDescending)).toEqual([usd, eur]);
  expect([eur, usd].sort(byPriceDescending)).toEqual([eur, usd]);
  expect([noBid, usd].sort(byPriceDescending)).toEqual([usd, noBid]);
});
