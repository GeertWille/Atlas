import { cn } from "@/shared/lib/cn";
import { formatUsd } from "@/shared/lib/format";
import { type BudgetLevel, budgetUsage } from "./progress";

const FILL: Record<BudgetLevel, string> = {
  ok: "bg-violet",
  warning: "bg-amber",
  exhausted: "bg-red",
};

const NOTE: Record<BudgetLevel, string | null> = {
  ok: null,
  warning: "Near cap",
  exhausted: "Cap reached",
};

/** Running LLM spend against the per-scan cap. */
export function BudgetMeter({ costUsd, budgetUsd }: { costUsd: number; budgetUsd: number }) {
  const { ratio, level } = budgetUsage(costUsd, budgetUsd);
  return (
    <div className="flex w-44 flex-col gap-1.5">
      <div className="flex items-baseline justify-between text-xs">
        <span className="text-muted-foreground">LLM cost{NOTE[level] ? ` · ${NOTE[level]}` : ""}</span>
        <span className="tabular-nums">
          <span className="font-medium">{formatUsd(costUsd)}</span>
          <span className="text-subtle-foreground"> / {formatUsd(budgetUsd)}</span>
        </span>
      </div>
      <div
        role="meter"
        aria-label="LLM cost against the per-scan budget"
        aria-valuemin={0}
        aria-valuemax={budgetUsd}
        aria-valuenow={Math.min(costUsd, budgetUsd)}
        aria-valuetext={`${formatUsd(costUsd)} of ${formatUsd(budgetUsd)}`}
        className="h-1.5 overflow-hidden rounded-full bg-muted"
      >
        <div
          className={cn("h-full rounded-full transition-[width] duration-500 ease-out", FILL[level])}
          style={{ width: `${ratio * 100}%` }}
        />
      </div>
    </div>
  );
}
