import { expect, test } from "vitest";
import { makeStep } from "@/test/fixtures";
import { budgetUsage, pipelineProgress, runningStep, stepElapsedMs, tokenTotals } from "./progress";

const steps = [
  makeStep({ id: "a", node: "launchBrowser", status: "completed", durationMs: 900 }),
  makeStep({ id: "b", node: "filterScripts", status: "skipped", durationMs: null }),
  makeStep({
    id: "c",
    node: "analyzeLogic",
    status: "running",
    startedAt: 5000,
    durationMs: null,
    inputTokens: 1000,
    outputTokens: 200,
  }),
  makeStep({
    id: "d",
    node: "synthesize",
    status: "pending",
    startedAt: null,
    durationMs: null,
    inputTokens: 50,
    outputTokens: 5,
  }),
];

test("finds the running step and counts settled steps", () => {
  expect(runningStep(steps)?.id).toBe("c");
  expect(runningStep([])).toBeNull();
  expect(pipelineProgress(steps)).toEqual({ settled: 2, total: 4 });
});

test("sums tokens across steps", () => {
  expect(tokenTotals(steps)).toEqual({ input: 1050, output: 205 });
});

test("a step's elapsed time is final when known, live while running, absent otherwise", () => {
  expect(stepElapsedMs(steps[0] as (typeof steps)[number], 99_999)).toBe(900);
  expect(stepElapsedMs(steps[2] as (typeof steps)[number], 7500)).toBe(2500);
  expect(stepElapsedMs(steps[2] as (typeof steps)[number], 4000)).toBe(0);
  expect(stepElapsedMs(steps[3] as (typeof steps)[number], 7500)).toBeNull();
});

test.each([
  [0.1, 0.5, 0.2, "ok"],
  [0.4, 0.5, 0.8, "warning"],
  [0.5, 0.5, 1, "exhausted"],
  [0.9, 0.5, 1, "exhausted"],
  [0, 0, 1, "exhausted"],
])("budgetUsage(%d of %d) → %d, %s", (cost, budget, ratio, level) => {
  expect(budgetUsage(cost, budget)).toEqual({ ratio, level });
});
