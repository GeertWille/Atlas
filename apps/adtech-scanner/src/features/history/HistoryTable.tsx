import { Link } from "@tanstack/react-router";
import {
  BookmarkIcon,
  CloudIcon,
  ExternalLinkIcon,
  GlobeIcon,
  MoreHorizontalIcon,
  RotateCwIcon,
  Trash2Icon,
} from "lucide-react";
import type { ScanSummary } from "@/bindings";
import { HealthRing } from "@/shared/components/health-ring";
import { ScanIdentity } from "@/shared/components/scan-identity";
import { DeviceLabel, ScanStatusBadge, SeverityCountPills } from "@/shared/components/status";
import { regionName } from "@/shared/domain/browserbase";
import { cn } from "@/shared/lib/cn";
import { EMPTY, formatDateTime, formatDuration, formatRelativeTime, formatUsd } from "@/shared/lib/format";
import { Badge } from "@/shared/ui/badge";
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
import { Hint } from "@/shared/ui/tooltip";

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
      <Hint label="Actions">
        <DropdownMenuTrigger asChild>
          <Button variant="ghost" size="icon-sm" aria-label={`Actions for ${scan.hostname}`}>
            <MoreHorizontalIcon />
          </Button>
        </DropdownMenuTrigger>
      </Hint>
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
          <th scope="col" className={HEAD}>
            Site
          </th>
          <th scope="col" className={`${HEAD} w-52`}>
            Result
          </th>
          <th scope="col" className={`${HEAD} w-20 text-right`}>
            Cost
          </th>
          <th scope="col" className={`${HEAD} hidden w-20 text-right xl:table-cell`}>
            Duration
          </th>
          <th scope="col" className={`${HEAD} w-28 text-right`}>
            Scanned
          </th>
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
              <Link to="/scans/$scanId" params={{ scanId: scan.id }} className="flex flex-col rounded-sm py-1">
                <ScanIdentity scan={scan} />
                <span className="flex min-w-0 items-center gap-2">
                  <DeviceLabel device={scan.device} className="text-xs text-subtle-foreground" />
                  {scan.remote ? (
                    <span
                      className="inline-flex shrink-0 items-center gap-1 text-xs text-subtle-foreground"
                      title={`Ran in a Browserbase browser in ${regionName(scan.remote.region)}`}
                    >
                      <CloudIcon aria-hidden="true" className="size-3.5 shrink-0" />
                      <span className="max-xl:sr-only">Browserbase</span>
                    </span>
                  ) : null}
                  {scan.proxy?.kind === "profile" ? (
                    <span
                      className="inline-flex max-w-28 shrink-0 items-center gap-1 text-xs text-subtle-foreground"
                      title={scan.proxy.host ? `Scanned through the proxy ${scan.proxy.host}` : undefined}
                    >
                      <GlobeIcon aria-hidden="true" className="size-3.5 shrink-0" />
                      {/* A configuration's name usually says where it scans from: in a narrow window it gets the room. */}
                      <span className={cn("truncate", scan.configuration && "max-xl:sr-only")}>
                        <span className="sr-only">Proxy </span>
                        {scan.proxy.name ?? "Proxy"}
                      </span>
                    </span>
                  ) : null}
                  {scan.configuration ? (
                    <Badge
                      tone="outline"
                      className="h-4 min-w-0 shrink px-1.5 text-[11px]"
                      title={`Started from the configuration “${scan.configuration.name}”`}
                    >
                      <BookmarkIcon />
                      <span className="truncate">{scan.configuration.name}</span>
                    </Badge>
                  ) : null}
                </span>
              </Link>
            </td>
            <td className={CELL}>
              {scan.status === "completed" ? (
                <SeverityCountPills compact counts={scan.findingCounts} />
              ) : (
                <ScanStatusBadge status={scan.status} />
              )}
            </td>
            <td className={`${CELL} text-right tabular-nums`}>
              {scan.costUsd > 0 ? formatUsd(scan.costUsd) : <span className="text-subtle-foreground">{EMPTY}</span>}
            </td>
            <td className={`${CELL} hidden text-right text-muted-foreground tabular-nums xl:table-cell`}>
              {formatDuration(scan.durationMs)}
            </td>
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
