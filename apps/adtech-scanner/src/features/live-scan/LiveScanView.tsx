import { SquareIcon } from "lucide-react";
import type { ReactNode } from "react";
import type { ScanDetail } from "@/bindings";
import { useCancelScan } from "@/lib/query/hooks";
import { Panel, PanelHeader } from "@/shared/components/panel";
import { ScanIdentity } from "@/shared/components/scan-identity";
import { DeviceLabel, LiveDot, SEVERITY_ICON, SeverityCountPills } from "@/shared/components/status";
import { PIPELINE_NODE_META } from "@/shared/domain/pipeline";
import { SEVERITY_META } from "@/shared/domain/vocabulary";
import { useNow } from "@/shared/hooks/use-now";
import { cn } from "@/shared/lib/cn";
import { formatCompact, formatDuration } from "@/shared/lib/format";
import { notifyError } from "@/shared/lib/notify-error";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogTrigger,
} from "@/shared/ui/alert-dialog";
import { Button } from "@/shared/ui/button";
import { BudgetMeter } from "./BudgetMeter";
import { PipelineStepper } from "./PipelineStepper";
import { pipelineProgress, runningStep, tokenTotals } from "./progress";

const TEXT_TONE = {
  red: "text-red-fg",
  orange: "text-orange-fg",
  amber: "text-amber-fg",
  blue: "text-blue-fg",
  gray: "text-gray-fg",
} as const;

function CancelScanButton({ scanId }: { scanId: string }) {
  const cancelScan = useCancelScan();
  return (
    <AlertDialog>
      <AlertDialogTrigger asChild>
        <Button variant="outline" size="sm" disabled={cancelScan.isPending}>
          <SquareIcon className="fill-current" />
          Cancel scan
        </Button>
      </AlertDialogTrigger>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>Cancel this scan?</AlertDialogTitle>
          <AlertDialogDescription>
            The browser closes and analysis stops. The audit trail so far is kept; LLM spend already made is not
            refunded.
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>Keep scanning</AlertDialogCancel>
          <AlertDialogAction
            destructive
            onClick={() =>
              cancelScan.mutate(scanId, { onError: (reason) => notifyError("Could not cancel the scan", reason) })
            }
          >
            Cancel scan
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}

function FindingsSoFar({ detail }: { detail: ScanDetail }) {
  const { findings } = detail;
  return (
    <Panel aria-label="Findings so far" className="flex min-h-28 flex-1 flex-col">
      <PanelHeader
        title="Findings"
        actions={findings.length > 0 ? <SeverityCountPills compact counts={detail.summary.findingCounts} /> : null}
      />
      {findings.length === 0 ? (
        <p className="px-4 py-3 text-[13px] text-muted-foreground">Findings appear here as checks report them.</p>
      ) : (
        <ul className="min-h-0 overflow-y-auto py-1.5">
          {findings.map((finding) => {
            const Icon = SEVERITY_ICON[finding.severity];
            const tone = SEVERITY_META[finding.severity].tone as keyof typeof TEXT_TONE;
            return (
              <li key={finding.id} className="flex animate-enter items-start gap-2 px-4 py-1.5 text-[13px]">
                <Icon
                  role="img"
                  aria-label={SEVERITY_META[finding.severity].label}
                  className={cn("mt-0.5 size-3.5 shrink-0", TEXT_TONE[tone])}
                />
                <span className="text-pretty">{finding.title}</span>
              </li>
            );
          })}
        </ul>
      )}
    </Panel>
  );
}

export function LiveScanView({
  detail,
  budgetUsd,
  activity,
}: {
  detail: ScanDetail;
  /** The per-scan cap from settings; the meter waits for it. */
  budgetUsd: number | undefined;
  /** The streaming audit trail, or what stands in for it when it cannot be loaded. */
  activity: ReactNode;
}) {
  const now = useNow(true);
  const { summary, steps } = detail;
  const current = runningStep(steps);
  const progress = pipelineProgress(steps);
  const tokens = tokenTotals(steps);

  return (
    <div className="mx-auto flex h-full w-full max-w-[1180px] flex-col gap-4 px-6 py-5">
      <header className="flex items-center justify-between gap-6">
        <div className="flex min-w-0 flex-col gap-1">
          <div className="flex items-center gap-2.5">
            <LiveDot />
            <ScanIdentity scan={summary} className="text-lg font-semibold tracking-tight" />
          </div>
          <p
            className="flex flex-wrap items-center gap-x-3.5 gap-y-0.5 text-[13px] whitespace-nowrap text-muted-foreground"
            aria-live="polite"
          >
            <DeviceLabel device={summary.device} />
            <span>
              Step {Math.min(progress.settled + 1, progress.total)} of {progress.total}
              {current ? `: ${PIPELINE_NODE_META[current.node].label}` : ""}
            </span>
            <span className="tabular-nums">{formatDuration(Math.max(0, now - summary.startedAt))}</span>
            {tokens.input > 0 ? (
              <span className="hidden tabular-nums xl:inline">
                {formatCompact(tokens.input + tokens.output)} tokens
              </span>
            ) : null}
          </p>
        </div>
        <div className="flex shrink-0 items-center gap-5">
          {budgetUsd === undefined ? null : <BudgetMeter costUsd={summary.costUsd} budgetUsd={budgetUsd} />}
          <CancelScanButton scanId={summary.id} />
        </div>
      </header>

      <div className="grid min-h-0 flex-1 grid-cols-[320px_minmax(0,1fr)] gap-4">
        <div className="flex min-h-0 flex-col gap-4">
          <Panel className="min-h-0 shrink overflow-y-auto p-4">
            <PipelineStepper steps={steps} now={now} />
          </Panel>
          <FindingsSoFar detail={detail} />
        </div>
        <Panel className="flex min-h-0 flex-col overflow-hidden">
          <PanelHeader title="Activity" description="Live audit trail: progress, model calls, tool use and findings" />
          {activity}
        </Panel>
      </div>
    </div>
  );
}
