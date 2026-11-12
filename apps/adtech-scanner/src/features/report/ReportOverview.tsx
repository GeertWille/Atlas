import type { ScanDetail } from "@/bindings";
import { Stat, StatGrid } from "@/shared/components/stat";
import { Badge } from "@/shared/ui/badge";
import { buildKeyMetrics, detectedStack } from "./key-metrics";

export function ReportOverview({ detail }: { detail: ScanDetail }) {
  const stack = detectedStack(detail.metrics, detail.summary.prebidVersion);
  return (
    <section aria-label="Overview" className="flex flex-col gap-4">
      {detail.overview ? (
        <div className="flex flex-col gap-2.5 rounded-xl border bg-card px-4 py-3.5 shadow-xs">
          <p className="selectable text-sm leading-relaxed text-pretty">{detail.overview}</p>
          {stack.length > 0 ? (
            <div className="flex flex-wrap items-center gap-1.5">
              <span className="text-xs text-subtle-foreground">Detected</span>
              {stack.map((name) => (
                <Badge key={name} tone="outline">
                  {name}
                </Badge>
              ))}
            </div>
          ) : null}
        </div>
      ) : null}
      <StatGrid className="grid-cols-4">
        {buildKeyMetrics(detail.metrics, detail.slots).map((metric) => (
          <Stat key={metric.label} label={metric.label} value={metric.value} hint={metric.hint} />
        ))}
      </StatGrid>
    </section>
  );
}
