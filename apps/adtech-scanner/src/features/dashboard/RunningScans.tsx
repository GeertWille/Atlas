import { Link } from "@tanstack/react-router";
import { ArrowRightIcon } from "lucide-react";
import type { ScanSummary } from "@/bindings";
import { ScanIdentity } from "@/shared/components/scan-identity";
import { DeviceLabel, LiveDot } from "@/shared/components/status";
import { useNow } from "@/shared/hooks/use-now";
import { formatDuration, formatUsd } from "@/shared/lib/format";

export function RunningScans({ scans }: { scans: ScanSummary[] }) {
  const now = useNow(scans.length > 0, 1000);
  if (scans.length === 0) return null;
  return (
    <section aria-label="Running scans" className="flex flex-col gap-2">
      {scans.map((scan) => (
        <Link
          key={scan.id}
          to="/scans/$scanId"
          params={{ scanId: scan.id }}
          className="group flex items-center gap-3 rounded-xl border border-violet-line bg-primary-soft/50 px-4 py-3 transition-colors hover:bg-primary-soft"
        >
          <LiveDot />
          <div className="flex min-w-0 flex-1 flex-col">
            <ScanIdentity scan={scan} />
            <span className="text-xs text-muted-foreground">
              {scan.status === "queued"
                ? "Queued, starts when a running scan finishes"
                : `Scanning now, ${formatDuration(Math.max(0, now - scan.startedAt))} elapsed`}
            </span>
          </div>
          <DeviceLabel device={scan.device} className="text-[13px]" />
          <span className="w-16 text-right text-[13px] text-muted-foreground tabular-nums">
            {formatUsd(scan.costUsd)}
          </span>
          <span className="flex items-center gap-1 text-[13px] font-medium text-primary-text">
            View live
            <ArrowRightIcon className="size-3.5 transition-transform group-hover:translate-x-0.5" />
          </span>
        </Link>
      ))}
    </section>
  );
}
