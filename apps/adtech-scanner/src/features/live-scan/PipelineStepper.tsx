import type { PipelineStep } from "@/bindings";
import { StepStatusIcon } from "@/shared/components/status";
import { PIPELINE_NODE_META } from "@/shared/domain/pipeline";
import { cn } from "@/shared/lib/cn";
import { formatCompact, formatDuration, formatUsd } from "@/shared/lib/format";
import { stepElapsedMs } from "./progress";

function StepNode({ step, isLast, now }: { step: PipelineStep; isLast: boolean; now: number }) {
  const meta = PIPELINE_NODE_META[step.node];
  const elapsed = stepElapsedMs(step, now);
  const running = step.status === "running";
  const usedModel = step.inputTokens + step.outputTokens > 0;
  return (
    <li
      aria-current={running ? "step" : undefined}
      className={cn("relative flex gap-3 pb-4", isLast && "pb-0", step.status === "pending" && "opacity-60")}
    >
      {isLast ? null : (
        <span
          aria-hidden="true"
          className={cn(
            "absolute top-6 bottom-0.5 left-[9.5px] w-px",
            step.status === "completed" ? "bg-green-line" : "bg-border",
          )}
        />
      )}
      <span className="relative mt-0.5 flex size-5 shrink-0 items-center justify-center rounded-full bg-background">
        <StepStatusIcon status={step.status} className="size-5" />
      </span>
      <div className="flex min-w-0 flex-1 flex-col gap-0.5">
        <div className="flex items-baseline justify-between gap-2">
          <span className={cn("truncate text-[13px] font-medium", running && "text-primary-text")}>{meta.label}</span>
          <span className="shrink-0 text-xs text-subtle-foreground tabular-nums">
            {elapsed === null ? "" : formatDuration(elapsed)}
          </span>
        </div>
        <p className={cn("text-xs text-pretty text-muted-foreground", running && !step.summary && "shimmer")}>
          {step.summary ?? meta.description}
        </p>
        {usedModel ? (
          <p className="text-xs text-subtle-foreground tabular-nums">
            {step.model} · {formatCompact(step.inputTokens)} in / {formatCompact(step.outputTokens)} out ·{" "}
            <span className="text-muted-foreground">{formatUsd(step.costUsd)}</span>
          </p>
        ) : null}
      </div>
    </li>
  );
}

/** The scan graph as a vertical stepper with live status, durations, tokens and cost. */
export function PipelineStepper({ steps, now }: { steps: PipelineStep[]; now: number }) {
  return (
    <ol aria-label="Pipeline" className="flex flex-col">
      {steps.map((step, index) => (
        <StepNode key={step.id} step={step} isLast={index === steps.length - 1} now={now} />
      ))}
    </ol>
  );
}
