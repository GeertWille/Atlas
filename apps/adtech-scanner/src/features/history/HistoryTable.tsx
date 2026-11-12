import { Link } from "@tanstack/react-router";
import { ExternalLinkIcon, MoreHorizontalIcon, RotateCwIcon, Trash2Icon } from "lucide-react";
import type { ScanSummary } from "@/bindings";
import { HealthRing } from "@/shared/components/health-ring";
import { ScanIdentity } from "@/shared/components/scan-identity";
import { DeviceLabel, ScanStatusBadge, SeverityCountPills } from "@/shared/components/status";
import { EMPTY, formatDateTime, formatDuration, formatRelativeTime, formatUsd } from "@/shared/lib/format";
import { Button } from "@/shared/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import { Skeleton } from "@/shared/ui/skeleton";

const HEAD = "px-3 py-2 text-left text-xs font-medium text-muted-foreground whitespace-nowrap";
const CELL = "px-3 py-0 h-[52px] whitespace-nowrap";

function RowActions({
  scan,
  onRescan,
  onDelete,
}: {
  scan: ScanSummary;
  onRescan: (scan: ScanSummary) => void;
  onDelete: (scan: ScanSummary) => void;
}) {
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button variant="ghost" size="icon-sm" aria-label={`Actions for ${scan.hostname}`}>
          <MoreHorizontalIcon />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end">
        <DropdownMenuGroup>
          <DropdownMenuItem asChild>
            <Link to="/scans/$scanId" params={{ scanId: scan.id }}>
              <ExternalLinkIcon />
              Open
            </Link>
          </DropdownMenuItem>
          <DropdownMenuItem onSelect={() => onRescan(scan)}>
            <RotateCwIcon />
            Re-scan
          </DropdownMenuItem>
        </DropdownMenuGroup>
        <DropdownMenuSeparator />
        <DropdownMenuGroup>
          <DropdownMenuItem destructive onSelect={() => onDelete(scan)}>
            <Trash2Icon />
            Delete…
          </DropdownMenuItem>
        </DropdownMenuGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

export function HistoryTableSkeleton() {
  return (
    <div className="flex flex-col">
      {[0, 1, 2, 3, 4, 5].map((row) => (
        <div key={row} className="flex h-[52px] items-center gap-4 border-b px-4 last:border-b-0">
          <Skeleton className="size-8 rounded-full" />
          <Skeleton className="h-3.5 w-64" />
          <Skeleton className="ml-auto h-3.5 w-40" />
        </div>
      ))}
    </div>
  );
}

export function HistoryTable({
  scans,
  onRescan,
  onDelete,
}: {
  scans: ScanSummary[];
  onRescan: (scan: ScanSummary) => void;
  onDelete: (scan: ScanSummary) => void;
}) {
  return (
    <table className="w-full table-fixed text-[13px]">
      <thead>
        <tr className="border-b">
          <th scope="col" className={`${HEAD} w-14 pl-4`}>
            <span className="sr-only">Health</span>
          </th>
          <th scope="col" className={HEAD}>Site</th>
          <th scope="col" className={`${HEAD} w-24`}>Device</th>
          <th scope="col" className={`${HEAD} w-44`}>Result</th>
          <th scope="col" className={`${HEAD} w-20 text-right`}>Cost</th>
          <th scope="col" className={`${HEAD} w-20 text-right`}>Duration</th>
          <th scope="col" className={`${HEAD} w-28 text-right`}>Scanned</th>
          <th scope="col" className={`${HEAD} w-12`}>
            <span className="sr-only">Actions</span>
          </th>
        </tr>
      </thead>
      <tbody>
        {scans.map((scan) => (
          <tr key={scan.id} className="group border-b transition-colors last:border-b-0 hover:bg-accent/50">
            <td className={`${CELL} pl-4`}>
              <HealthRing score={scan.healthScore} size={32} />
            </td>
            <td className={`${CELL} max-w-0`}>
              <Link to="/scans/$scanId" params={{ scanId: scan.id }} className="block rounded-sm py-1">
                <ScanIdentity scan={scan} />
              </Link>
            </td>
            <td className={CELL}>
              <DeviceLabel device={scan.device} />
            </td>
            <td className={CELL}>
              {scan.status === "completed" ? <SeverityCountPills counts={scan.findingCounts} /> : <ScanStatusBadge status={scan.status} />}
            </td>
            <td className={`${CELL} text-right tabular-nums`}>
              {scan.costUsd > 0 ? formatUsd(scan.costUsd) : <span className="text-subtle-foreground">{EMPTY}</span>}
            </td>
            <td className={`${CELL} text-right text-muted-foreground tabular-nums`}>{formatDuration(scan.durationMs)}</td>
            <td className={`${CELL} text-right text-muted-foreground`} title={formatDateTime(scan.startedAt)}>
              {formatRelativeTime(scan.startedAt)}
            </td>
            <td className={`${CELL} pr-3 text-right`}>
              <RowActions scan={scan} onRescan={onRescan} onDelete={onDelete} />
            </td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}
