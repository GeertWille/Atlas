import { expect, test } from "vitest";
import { settingsFixture } from "@/test/fixtures";
import { estimateScanCost } from "./cost-estimate";

test("default settings stay under the budget cap", () => {
  const estimate = estimateScanCost(settingsFixture);
  expect(estimate.expectedUsd).toBeCloseTo(0.36215, 5);
  expect(estimate.cappedByBudget).toBe(false);
  expect(estimate.fallbackPricing).toBe(false);
});

test("effort scales the analysis phase like the backend's turn limit (12, 20, 30 turns)", () => {
  const budgetUsd = 5;
  const at = (analysisEffort: "low" | "medium" | "high") =>
    estimateScanCost({ ...settingsFixture, budgetUsd, analysisEffort }).expectedUsd;
  expect(at("low")).toBeCloseTo(0.00215 + 0.36 * 0.6, 5);
  expect(at("high")).toBeCloseTo(0.00215 + 0.36 * 1.5, 5);
});

test("high effort is capped at the budget", () => {
  expect(estimateScanCost({ ...settingsFixture, analysisEffort: "high" })).toEqual({
    expectedUsd: 0.5,
    cappedByBudget: true,
    fallbackPricing: false,
  });
});

test("a model the backend has no price for is estimated at its conservative fallback rate", () => {
  const sonnet = estimateScanCost({ ...settingsFixture, budgetUsd: 5 });
  const opus = estimateScanCost({ ...settingsFixture, budgetUsd: 5, analysisModel: "claude-opus-5-5" });
  // 145K input at $15 and 7K output at $75 per million, plus the haiku filter phase.
  expect(opus.expectedUsd).toBeCloseTo(2.70215, 5);
  expect(opus.expectedUsd).toBeGreaterThan(sonnet.expectedUsd);
  expect(opus.fallbackPricing).toBe(true);
});

test("with Jev on, triage and the reading guide cost about a cent on top of the analysis", () => {
  const estimate = estimateScanCost({ ...settingsFixture, hasTypesafeKey: true, jevEnabled: true });
  // 230K Jev input tokens at $0.042 per million, plus the sonnet analysis phase.
  expect(estimate.expectedUsd).toBeCloseTo(0.00966 + 0.36, 5);
  expect(estimate.fallbackPricing).toBe(false);
});

test("Jev without an Anthropic key is script triage only, whatever the analysis model is", () => {
  const estimate = estimateScanCost({
    ...settingsFixture,
    hasApiKey: false,
    hasTypesafeKey: true,
    jevEnabled: true,
    analysisModel: "claude-opus-5-5",
    analysisEffort: "high",
  });
  expect(estimate.expectedUsd).toBeCloseTo(35_000 * 0.042e-6, 10);
  expect(estimate).toMatchObject({ cappedByBudget: false, fallbackPricing: false });
});

test("a stored TypeSafe key changes nothing while the Jev switch is off", () => {
  expect(estimateScanCost({ ...settingsFixture, hasTypesafeKey: true })).toEqual(estimateScanCost(settingsFixture));
});
