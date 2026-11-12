import type { DashboardStats as Stats } from "@/bindings";
import { Stat, StatGrid } from "@/shared/components/stat";
import { healthBand } from "@/shared/domain/health";
import { EMPTY, formatInteger, formatUsd, pluralize } from "@/shared/lib/format";
import { Skeleton } from "@/shared/ui/skeleton";

export function DashboardStats({ stats }: { stats: Stats | undefined }) {
  if (!stats) {
    return (
      <StatGrid className="grid-cols-4">
        {["scans", "health", "critical", "spend"].map((key) => (
          <div key={key} className="flex flex-col gap-2 px-4 py-3.5">
            <Skeleton className="h-4 w-20" />
            <Skeleton className="h-7 w-14" />
            <Skeleton className="h-3 w-24" />
          </div>
        ))}
      </StatGrid>
    );
  }
  const health = stats.avgHealthScore === null ? null : Math.round(stats.avgHealthScore);
  return (
    <StatGrid className="grid-cols-4">
      <Stat label="Scans" value={formatInteger(stats.totalScans)} hint={`across ${pluralize(stats.sitesScanned, "site")}`} />
      <Stat
        label="Average health"
        value={health === null ? EMPTY : <>{health}<span className="text-sm font-normal text-subtle-foreground"> / 100</span></>}
        hint={health === null ? "No completed scans yet" : healthBand(health).label}
      />
      <Stat
        label="Open critical findings"
        value={formatInteger(stats.openCriticalFindings)}
        hint={`${pluralize(stats.totalFindings, "finding")} in latest scans`}
      />
      <Stat label="LLM spend" value={formatUsd(stats.totalCostUsd)} hint={`${formatUsd(stats.avgCostUsd)} average per scan`} />
    </StatGrid>
  );
}
