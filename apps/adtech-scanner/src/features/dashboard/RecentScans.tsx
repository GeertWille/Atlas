import { Link } from "@tanstack/react-router";
import { ChevronRightIcon, RadarIcon } from "lucide-react";
import type { ScanSummary } from "@/bindings";
import { HealthRing } from "@/shared/components/health-ring";
import { Panel, PanelHeader } from "@/shared/components/panel";
import { ScanIdentity } from "@/shared/components/scan-identity";
import { DeviceLabel, ScanStatusBadge, SeverityCountPills } from "@/shared/components/status";
import { formatRelativeTime } from "@/shared/lib/format";
import { Button } from "@/shared/ui/button";
import { Empty } from "@/shared/ui/empty";
import { Skeleton } from "@/shared/ui/skeleton";

function Row({ scan }: { scan: ScanSummary }) {
  return (
    <li>
      <Link
        to="/scans/$scanId"
        params={{ scanId: scan.id }}
        className="group flex h-14 items-center gap-3 border-b px-4 transition-colors group-last:border-b-0 hover:bg-accent/60"
      >
        <HealthRing score={scan.healthScore} size={32} />
        <div className="flex min-w-0 flex-1 flex-col">
          <ScanIdentity scan={scan} />
          <span className="flex items-center gap-2 text-xs text-subtle-foreground">
            <DeviceLabel device={scan.device} className="text-subtle-foreground" />
            <span aria-hidden="true">·</span>
            {formatRelativeTime(scan.startedAt)}
          </span>
        </div>
        {scan.status === "completed" ? (
          <SeverityCountPills counts={scan.findingCounts} />
        ) : (
          <ScanStatusBadge status={scan.status} />
        )}
        <ChevronRightIcon className="size-4 text-subtle-foreground transition-transform group-hover:translate-x-0.5" />
      </Link>
    </li>
  );
}

export function RecentScans({ scans }: { scans: ScanSummary[] | undefined }) {
  return (
    <Panel aria-label="Recent scans" className="overflow-hidden">
      <PanelHeader
        title="Recent scans"
        actions={
          <Button asChild variant="ghost" size="sm">
            <Link to="/history">View all</Link>
          </Button>
        }
      />
      {scans === undefined ? (
        <div className="flex flex-col">
          {[0, 1, 2, 3].map((row) => (
            <div key={row} className="flex h-14 items-center gap-3 border-b px-4 last:border-b-0">
              <Skeleton className="size-8 rounded-full" />
              <div className="flex flex-1 flex-col gap-1.5">
                <Skeleton className="h-3.5 w-56" />
                <Skeleton className="h-3 w-28" />
              </div>
            </div>
          ))}
        </div>
      ) : scans.length === 0 ? (
        <Empty
          icon={RadarIcon}
          title="No scans yet"
          description="Enter a page URL above to capture its auctions and look for revenue leaks."
        />
      ) : (
        <ul className="[&>li:last-child>a]:border-b-0">
          {scans.map((scan) => (
            <Row key={scan.id} scan={scan} />
          ))}
        </ul>
      )}
    </Panel>
  );
}
