import { expect, test } from "vitest";
import { estimateBrowserbaseUsage } from "@/shared/domain/browserbase";
import { settingsFixture } from "@/test/fixtures";
import { describeBrowserbaseUsage, estimateScanCost } from "./cost-estimate";

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

test("each page after the entry page adds its telemetry to the analysis, and nothing without logic analysis", () => {
  const roomy = { ...settingsFixture, budgetUsd: 5 };
  const onePage = estimateScanCost(roomy).expectedUsd;
  const threePages = estimateScanCost(roomy, { extraPages: 2 }).expectedUsd;
  const sixPages = estimateScanCost(roomy, { extraPages: 5 }).expectedUsd;
  expect(estimateScanCost(roomy, { extraPages: 0 }).expectedUsd).toBe(onePage);
  // 1,500 input tokens per extra page at the analysis model's input price.
  expect(threePages - onePage).toBeCloseTo((2 * 1_500 * 2) / 1_000_000, 6);
  expect(sixPages - threePages).toBeCloseTo((3 * 1_500 * 2) / 1_000_000, 6);
  const triageOnly = { ...roomy, hasApiKey: false, hasTypesafeKey: true, jevEnabled: true };
  expect(estimateScanCost(triageOnly, { extraPages: 5 })).toEqual(estimateScanCost(triageOnly));
});

test("a scan's own budget replaces the settings' cap", () => {
  expect(estimateScanCost(settingsFixture, { budgetUsd: 0.2 })).toEqual({
    expectedUsd: 0.2,
    cappedByBudget: true,
    fallbackPricing: false,
  });
  expect(estimateScanCost({ ...settingsFixture, analysisEffort: "high" }, { budgetUsd: 2 }).cappedByBudget).toBe(false);
});

test("Browserbase usage is described in units, apart from any model cost", () => {
  const scan = { visitor: "scroll", pages: 1, observeSeconds: 20, browserbaseProxy: false } as const;
  expect(describeBrowserbaseUsage(estimateBrowserbaseUsage(scan))).toBe("about 1 browser minute, no proxy traffic");
  expect(
    describeBrowserbaseUsage(
      estimateBrowserbaseUsage({ ...scan, visitor: "browse", pages: 3, browserbaseProxy: true }),
    ),
  ).toBe("about 2 browser minutes and 54 MB of proxy traffic");
  expect(
    describeBrowserbaseUsage(estimateBrowserbaseUsage({ ...scan, visitor: "manual", browserbaseProxy: true })),
  ).toBe("up to 15 browser minutes and 18 MB of proxy traffic per page");
});

test("the model estimate knows nothing of where the browser runs", () => {
  const cloud = { ...settingsFixture, defaultBrowser: "browserbase" as const };
  expect(estimateScanCost(cloud)).toEqual(estimateScanCost(settingsFixture));
});
