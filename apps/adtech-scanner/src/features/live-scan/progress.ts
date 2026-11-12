import type { PipelineStep } from "@/bindings";

export function runningStep(steps: PipelineStep[]): PipelineStep | null {
  return steps.find((step) => step.status === "running") ?? null;
}

/** Steps that will not change any more, out of all steps. */
export function pipelineProgress(steps: PipelineStep[]): { settled: number; total: number } {
  const settled = steps.filter((step) => step.status !== "pending" && step.status !== "running").length;
  return { settled, total: steps.length };
}

export function tokenTotals(steps: PipelineStep[]): { input: number; output: number } {
  return steps.reduce(
    (totals, step) => ({ input: totals.input + step.inputTokens, output: totals.output + step.outputTokens }),
    { input: 0, output: 0 },
  );
}

/** Live duration of a step: the final value once known, otherwise time since it started. */
export function stepElapsedMs(step: PipelineStep, now: number): number | null {
  if (step.durationMs !== null) return step.durationMs;
  if (step.status === "running" && step.startedAt !== null) return Math.max(0, now - step.startedAt);
  return null;
}

export type BudgetLevel = "ok" | "warning" | "exhausted";

const WARNING_RATIO = 0.8;

export function budgetUsage(costUsd: number, budgetUsd: number): { ratio: number; level: BudgetLevel } {
  if (budgetUsd <= 0) return { ratio: 1, level: "exhausted" };
  const ratio = Math.min(1, costUsd / budgetUsd);
  if (ratio >= 1) return { ratio, level: "exhausted" };
  return { ratio, level: ratio >= WARNING_RATIO ? "warning" : "ok" };
}
