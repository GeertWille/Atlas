import { expect, test } from "vitest";
import { settingsFixture } from "@/test/fixtures";
import { estimateScanCost } from "./cost-estimate";

test("default settings stay under the budget cap", () => {
  const estimate = estimateScanCost(settingsFixture);
  expect(estimate.expectedUsd).toBeCloseTo(0.36215, 5);
  expect(estimate.cappedByBudget).toBe(false);
});

test("high effort is capped at the budget", () => {
  expect(estimateScanCost({ ...settingsFixture, analysisEffort: "high" })).toEqual({
    expectedUsd: 0.5,
    cappedByBudget: true,
  });
});

test("a more expensive analysis model raises the estimate", () => {
  const sonnet = estimateScanCost({ ...settingsFixture, budgetUsd: 5 });
  const opus = estimateScanCost({ ...settingsFixture, budgetUsd: 5, analysisModel: "claude-opus-5-5" });
  expect(opus.expectedUsd).toBeCloseTo(0.72215, 5);
  expect(opus.expectedUsd).toBeGreaterThan(sonnet.expectedUsd);
});
