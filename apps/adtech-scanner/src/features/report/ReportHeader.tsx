import {
  BookmarkIcon,
  CalendarIcon,
  ChevronDownIcon,
  CoinsIcon,
  DownloadIcon,
  FileJsonIcon,
  FileTextIcon,
  MoreHorizontalIcon,
  RepeatIcon,
  RotateCwIcon,
  SparklesIcon,
  TimerIcon,
} from "lucide-react";
import { toast } from "sonner";
import type { ExportFormat, ScanDetail } from "@/bindings";
import { useExportScan, useScanConfigurations } from "@/lib/query/hooks";
import { HealthRing } from "@/shared/components/health-ring";
import { ScanIdentity } from "@/shared/components/scan-identity";
import { DeviceIcon, DeviceLabel, ScanStatusBadge, SeverityCountPills } from "@/shared/components/status";
import { healthBand } from "@/shared/domain/health";
import { repeatVisitOptions, rescanOptions } from "@/shared/domain/scan-options";
import { DEVICE_LABEL, otherDevice } from "@/shared/domain/vocabulary";
import { useLaunchScan } from "@/shared/hooks/use-launch-scan";
import { formatDateTime, formatDuration, formatUsd } from "@/shared/lib/format";
import { notifyError } from "@/shared/lib/notify-error";
import { Badge } from "@/shared/ui/badge";
import { Button } from "@/shared/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import { Hint } from "@/shared/ui/tooltip";
import { exportFileName } from "./export-file-name";

function ExportMenu({ detail }: { detail: ScanDetail }) {
  const exportScan = useExportScan();
  const run = (format: ExportFormat) =>
    exportScan.mutate(
      { scanId: detail.summary.id, format, fileName: exportFileName(detail.summary, format) },
      {
        onSuccess: (path) => {
          if (path) toast.success("Report exported", { description: path });
        },
        onError: (reason) => notifyError("Export failed", reason),
      },
    );
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button variant="outline" disabled={exportScan.isPending}>
          <DownloadIcon />
          Export
          <ChevronDownIcon className="text-subtle-foreground" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end">
        <DropdownMenuGroup>
          <DropdownMenuItem onSelect={() => run("markdown")}>
            <FileTextIcon />
            Markdown report…
          </DropdownMenuItem>
          <DropdownMenuItem onSelect={() => run("json")}>
            <FileJsonIcon />
            JSON data…
          </DropdownMenuItem>
        </DropdownMenuGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

export function ReportHeader({ detail }: { detail: ScanDetail }) {
  const { summary } = detail;
  const configurations = useScanConfigurations();
  // The chip keeps the name the scan was started under; only a configuration that still exists can run again.
  const { configuration: startedFrom } = summary;
  const configuration = configurations.data?.some((saved) => saved.id === startedFrom?.id) ? startedFrom : null;
  const options = rescanOptions(detail.options);
  const repeatVisit = repeatVisitOptions(detail);
  const { launch, launchConfiguration, isPending } = useLaunchScan();
  const other = otherDevice(summary.device);
  const completed = summary.status === "completed";
  // With only a TypeSafe key the model classified scripts but nothing analysed their logic.
  const logicAnalysed = detail.steps.some((step) => step.node === "analyzeLogic" && step.status !== "skipped");
  const aiLabel = logicAnalysed ? "AI analysis" : "AI script triage";

  return (
    <header className="flex items-start justify-between gap-6">
      <div className="flex min-w-0 items-center gap-4">
        <HealthRing score={summary.healthScore} size={72} />
        <div className="flex min-w-0 flex-col gap-1.5">
          <div className="flex items-center gap-2.5">
            <ScanIdentity scan={summary} className="selectable text-xl font-semibold tracking-tight" />
            {completed ? null : <ScanStatusBadge status={summary.status} />}
          </div>
          <p className="flex flex-wrap items-center gap-x-3.5 gap-y-1 text-[13px] text-muted-foreground [&_svg]:size-3.5">
            {summary.healthScore !== null ? (
              <span className="font-medium text-foreground">{healthBand(summary.healthScore).label}</span>
            ) : null}
            <DeviceLabel device={summary.device} />
            <span className="inline-flex items-center gap-1.5">
              <CalendarIcon aria-hidden="true" />
              <time dateTime={new Date(summary.startedAt).toISOString()}>{formatDateTime(summary.startedAt)}</time>
            </span>
            <span className="inline-flex items-center gap-1.5 tabular-nums">
              <TimerIcon aria-hidden="true" />
              {formatDuration(summary.durationMs)}
            </span>
            <span className="inline-flex items-center gap-1.5 tabular-nums">
              <CoinsIcon aria-hidden="true" />
              {formatUsd(summary.costUsd)} LLM cost
            </span>
          </p>
          <div className="flex items-center gap-2">
            <SeverityCountPills counts={summary.findingCounts} />
            {startedFrom ? (
              <Badge tone="outline" title={`Started from the configuration “${startedFrom.name}”`}>
                <BookmarkIcon />
                <span className="max-w-48 truncate">{startedFrom.name}</span>
              </Badge>
            ) : null}
            {completed ? (
              <Badge tone={summary.aiUsed ? "violet" : "outline"}>
                {summary.aiUsed ? <SparklesIcon /> : null}
                {summary.aiUsed ? aiLabel : "Deterministic checks only"}
              </Badge>
            ) : null}
          </div>
        </div>
      </div>
      <div className="flex shrink-0 items-center gap-2">
        <Hint
          label={
            configuration
              ? `Run the configuration “${configuration.name}” again`
              : "Scan this page again with the same options"
          }
        >
          <Button
            variant="outline"
            disabled={isPending}
            onClick={() => (configuration ? launchConfiguration(configuration.id) : launch(options))}
          >
            <RotateCwIcon />
            Re-scan
          </Button>
        </Hint>
        <Hint label={`Scan the same page as a ${DEVICE_LABEL[other].toLowerCase()} visitor`}>
          <Button
            variant="outline"
            disabled={isPending}
            aria-label={`Scan on ${DEVICE_LABEL[other].toLowerCase()}`}
            onClick={() => launch({ ...options, device: other })}
          >
            <DeviceIcon device={other} className="size-4" />
            <span className="hidden xl:inline">Scan on {DEVICE_LABEL[other].toLowerCase()}</span>
          </Button>
        </Hint>
        {completed ? <ExportMenu detail={detail} /> : null}
        {repeatVisit ? (
          <DropdownMenu>
            <Hint label="More">
              <DropdownMenuTrigger asChild>
                <Button variant="outline" size="icon-sm" className="size-8" aria-label="More scan actions">
                  <MoreHorizontalIcon />
                </Button>
              </DropdownMenuTrigger>
            </Hint>
            <DropdownMenuContent align="end" className="max-w-72">
              <DropdownMenuGroup>
                <DropdownMenuItem disabled={isPending} onSelect={() => launch(repeatVisit)} className="items-start">
                  <RepeatIcon className="mt-0.5" />
                  <span className="flex flex-col gap-0.5">
                    Repeat this visit
                    <span className="text-xs text-pretty text-muted-foreground">
                      The same gestures and link choices again (seed {repeatVisit.seed}), to see whether a result
                      repeats.
                    </span>
                  </span>
                </DropdownMenuItem>
              </DropdownMenuGroup>
            </DropdownMenuContent>
          </DropdownMenu>
        ) : null}
      </div>
    </header>
  );
}
