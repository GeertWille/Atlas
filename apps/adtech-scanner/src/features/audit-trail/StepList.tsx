import { LayersIcon } from "lucide-react";
import type { PipelineStep } from "@/bindings";
import { StepStatusIcon } from "@/shared/components/status";
import { PIPELINE_NODE_META } from "@/shared/domain/pipeline";
import { formatCompact, formatDuration, formatUsd } from "@/shared/lib/format";

const ITEM =
  "flex w-full flex-col gap-0.5 rounded-lg px-2.5 py-2 text-left transition-colors hover:bg-accent/70 aria-[current=true]:bg-primary-soft/70";

export function StepList({
  steps,
  logCounts,
  totalLogs,
  selectedStepId,
  onSelect,
}: {
  steps: PipelineStep[];
  logCounts: Map<string, number>;
  totalLogs: number;
  selectedStepId: string | null;
  onSelect: (stepId: string | null) => void;
}) {
  return (
    <nav aria-label="Pipeline steps" className="flex flex-col gap-0.5">
      <button type="button" aria-current={selectedStepId === null} onClick={() => onSelect(null)} className={ITEM}>
        <span className="flex items-center gap-2 text-[13px] font-medium">
          <LayersIcon className="size-4 text-subtle-foreground" />
          <span className="flex-1">All steps</span>
          <span className="text-xs font-normal text-subtle-foreground tabular-nums">{totalLogs}</span>
        </span>
      </button>
      {steps.map((step) => {
        const usedModel = step.inputTokens + step.outputTokens > 0;
        return (
          <button
            key={step.id}
            type="button"
            aria-current={selectedStepId === step.id}
            onClick={() => onSelect(step.id)}
            className={ITEM}
          >
            <span className="flex items-center gap-2 text-[13px] font-medium">
              <StepStatusIcon status={step.status} />
              <span className="flex-1 truncate">{PIPELINE_NODE_META[step.node].label}</span>
              <span className="text-xs font-normal text-subtle-foreground tabular-nums">
                {logCounts.get(step.id) ?? 0}
              </span>
            </span>
            <span className="truncate pl-6 text-xs text-muted-foreground">
              {step.summary ?? PIPELINE_NODE_META[step.node].description}
            </span>
            <span className="pl-6 text-xs text-subtle-foreground tabular-nums">
              {[
                step.durationMs === null ? null : formatDuration(step.durationMs),
                usedModel ? `${formatCompact(step.inputTokens + step.outputTokens)} tokens` : null,
                usedModel ? formatUsd(step.costUsd) : null,
              ]
                .filter(Boolean)
                .join(" · ")}
            </span>
          </button>
        );
      })}
    </nav>
  );
}
