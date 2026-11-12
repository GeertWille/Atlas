import { ChevronDownIcon, DownloadIcon, FileJsonIcon, FileTextIcon, RotateCwIcon, SparklesIcon } from "lucide-react";
import { toast } from "sonner";
import type { ExportFormat, ScanDetail } from "@/bindings";
import { useExportScan } from "@/lib/query/hooks";
import { HealthRing } from "@/shared/components/health-ring";
import { ScanIdentity } from "@/shared/components/scan-identity";
import { DeviceIcon, DeviceLabel, ScanStatusBadge, SeverityCountPills } from "@/shared/components/status";
import { healthBand } from "@/shared/domain/health";
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
  const { summary, options } = detail;
  const { launch, isPending } = useLaunchScan();
  const other = otherDevice(summary.device);
  const completed = summary.status === "completed";

  return (
    <header className="flex items-start justify-between gap-6">
      <div className="flex min-w-0 items-center gap-4">
        <HealthRing score={summary.healthScore} size={72} />
        <div className="flex min-w-0 flex-col gap-1.5">
          <div className="flex items-center gap-2.5">
            <ScanIdentity scan={summary} className="selectable text-xl font-semibold tracking-tight" />
            {completed ? null : <ScanStatusBadge status={summary.status} />}
          </div>
          <p className="flex flex-wrap items-center gap-x-2 gap-y-1 text-[13px] text-muted-foreground">
            {summary.healthScore !== null ? (
              <>
                <span className="font-medium text-foreground">{healthBand(summary.healthScore).label}</span>
                <span aria-hidden="true">·</span>
              </>
            ) : null}
            <DeviceLabel device={summary.device} />
            <span aria-hidden="true">·</span>
            <time dateTime={new Date(summary.startedAt).toISOString()}>{formatDateTime(summary.startedAt)}</time>
            <span aria-hidden="true">·</span>
            <span className="tabular-nums">{formatDuration(summary.durationMs)}</span>
            <span aria-hidden="true">·</span>
            <span className="tabular-nums">{formatUsd(summary.costUsd)} LLM cost</span>
          </p>
          <div className="flex items-center gap-2">
            <SeverityCountPills counts={summary.findingCounts} />
            <Badge tone={summary.aiUsed ? "violet" : "outline"}>
              {summary.aiUsed ? <SparklesIcon /> : null}
              {summary.aiUsed ? "AI analysis" : "Deterministic checks only"}
            </Badge>
          </div>
        </div>
      </div>
      <div className="flex shrink-0 items-center gap-2">
        <Button variant="outline" disabled={isPending} onClick={() => launch(options)}>
          <RotateCwIcon />
          Re-scan
        </Button>
        <Button variant="outline" disabled={isPending} onClick={() => launch({ ...options, device: other })}>
          <DeviceIcon device={other} className="size-4" />
          Scan on {DEVICE_LABEL[other].toLowerCase()}
        </Button>
        {completed ? <ExportMenu detail={detail} /> : null}
      </div>
    </header>
  );
}
