import { expect, test } from "vitest";
import { costOf, PRICED_MODELS } from "./model-pricing";

// The expected values are the ones asserted by the backend's own tests in crates/engine/src/domain/pricing.rs.

test("sonnet is $2 in and $10 out per million tokens", () => {
  expect(costOf("claude-sonnet-5-5", { input: 1_000_000, output: 100_000 })).toEqual({ usd: 3, knownModel: true });
});

test("haiku switches rate card above 100K prompt tokens", () => {
  expect(costOf("claude-haiku-5-5", { input: 100_000, output: 1_000 }).usd).toBeCloseTo(0.01 + 0.0005, 9);
  expect(costOf("claude-haiku-5-5", { input: 110_000, output: 1_000 }).usd).toBeCloseTo(0.055 + 0.0025, 9);
});

test("a model without a price is charged at the conservative fallback, never below sonnet", () => {
  const unknown = costOf("claude-opus-5-5", { input: 1_000_000, output: 10_000 });
  expect(unknown).toEqual({ usd: 15.75, knownModel: false });
  expect(unknown.usd).toBeGreaterThan(costOf("claude-sonnet-5-5", { input: 1_000_000, output: 10_000 }).usd);
});

test("every suggested model has a price", () => {
  for (const model of PRICED_MODELS) expect(costOf(model, { input: 1, output: 1 }).knownModel).toBe(true);
});
