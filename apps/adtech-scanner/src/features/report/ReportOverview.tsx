import { TriangleAlertIcon } from "lucide-react";
import type { LogEntry, ScanDetail } from "@/bindings";
import { Stat, StatGrid } from "@/shared/components/stat";
import { Badge } from "@/shared/ui/badge";
import { BrowserbaseVisitDetails } from "./BrowserbaseVisitDetails";
import { browserbaseLogFacts } from "./browserbase-visit";
import { EnvironmentNotice } from "./EnvironmentNotice";
import { buildKeyMetrics, detectedStack } from "./key-metrics";
import { splitOverview } from "./overview-caveats";
import { isMultiPage } from "./pages/page-scope";
import { visitFacts } from "./visit-summary";

export function ReportOverview({
  detail,
  logs,
}: {
  detail: ScanDetail;
  /** The audit trail, once loaded: it says two things about a Browserbase browser that the scan itself does not. */
  logs?: LogEntry[];
}) {
  const browserbaseLog = browserbaseLogFacts(logs);
  const stack = detectedStack(detail.metrics, detail.summary.prebidVersion);
  const { summary, caveats } = splitOverview(detail.overview ?? "");
  const visit = visitFacts(detail);
  return (
    <section aria-label="Overview" className="flex flex-col gap-4">
      {detail.overview || visit ? (
        <div className="flex flex-col gap-2.5 rounded-xl border bg-card px-4 py-3.5 shadow-xs">
          {detail.overview ? <p className="selectable text-sm leading-relaxed text-pretty">{summary}</p> : null}
          {caveats.length > 0 ? (
            <ul aria-label="Scan caveats" className="flex flex-col gap-1">
              {caveats.map((caveat) => (
                <li key={caveat} className="selectable flex items-start gap-2 text-[13px] text-pretty text-amber-fg">
                  <TriangleAlertIcon aria-hidden="true" className="mt-0.5 size-3.5 shrink-0" />
                  {caveat}
                </li>
              ))}
            </ul>
          ) : null}
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
          {visit ? (
            <div className="flex flex-col gap-1.5">
              <div className="flex flex-wrap items-center gap-1.5">
                <span className="text-xs text-subtle-foreground">Visit</span>
                {visit.map((fact) => (
                  <Badge key={fact} tone="outline">
                    {fact}
                  </Badge>
                ))}
              </div>
              <BrowserbaseVisitDetails detail={detail} log={browserbaseLog} />
              <EnvironmentNotice detail={detail} serviceScripts={browserbaseLog.serviceScripts} />
            </div>
          ) : null}
        </div>
      ) : null}
      <StatGrid className="grid-cols-4">
        {buildKeyMetrics(detail.metrics, detail.slots, isMultiPage(detail), detail.auctions).map((metric) => (
          <Stat key={metric.label} label={metric.label} value={metric.value} hint={metric.hint} />
        ))}
      </StatGrid>
    </section>
  );
}
