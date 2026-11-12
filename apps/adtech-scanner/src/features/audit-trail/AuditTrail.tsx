import { FilterXIcon, ScrollTextIcon } from "lucide-react";
import { useEffect, useState } from "react";
import type { LogEntry, PipelineStep } from "@/bindings";
import { CopyButton } from "@/shared/components/copy-button";
import { FilterMenu } from "@/shared/components/filter-menu";
import { LOG_KIND_ICON, LogEntryRow } from "@/shared/components/log-entry-row";
import { Panel } from "@/shared/components/panel";
import { SearchInput } from "@/shared/components/search-input";
import { LOG_KIND_LABEL, LOG_KINDS, LOG_LEVEL_LABEL, LOG_LEVELS } from "@/shared/domain/vocabulary";
import { pluralize } from "@/shared/lib/format";
import { Button } from "@/shared/ui/button";
import { Empty } from "@/shared/ui/empty";
import { Skeleton } from "@/shared/ui/skeleton";
import { countLogsByStep, filterLogs, type LogFilters, logsAsText, NO_LOG_FILTERS } from "./log-filters";
import { StepList } from "./StepList";

function LogListSkeleton() {
  return (
    <div className="flex flex-col">
      {[64, 48, 72, 40, 56, 68].map((width) => (
        <div key={width} className="flex h-9 items-center gap-3 border-b px-3 last:border-b-0">
          <Skeleton className="h-3 w-16" />
          <Skeleton className="h-3" style={{ width: `${width}%` }} />
        </div>
      ))}
    </div>
  );
}

/** How the scan (and the model) arrived at its conclusions: steps on the left, their log entries on the right. */
export function AuditTrail({
  steps,
  logs,
  focusedLogId,
  onOpenFinding,
}: {
  steps: PipelineStep[];
  logs: LogEntry[] | undefined;
  /** Deep link target from log search: revealed, expanded and highlighted. */
  focusedLogId?: string;
  onOpenFinding: (findingId: string) => void;
}) {
  const [filters, setFilters] = useState<LogFilters>(NO_LOG_FILTERS);
  const focusedLoaded = Boolean(focusedLogId && logs?.some((entry) => entry.id === focusedLogId));

  useEffect(() => {
    if (!focusedLoaded || !focusedLogId) return;
    setFilters(NO_LOG_FILTERS);
    // Wait a frame so the row exists even if filters were hiding it.
    const frame = requestAnimationFrame(() => {
      document.getElementById(`log-${focusedLogId}`)?.scrollIntoView({ block: "center" });
    });
    return () => cancelAnimationFrame(frame);
  }, [focusedLogId, focusedLoaded]);

  const all = logs ?? [];
  const visible = filterLogs(all, filters);
  const kindsPresent = LOG_KINDS.filter((kind) => all.some((entry) => entry.kind === kind));
  const filtered = visible.length !== all.length;

  return (
    <div className="grid grid-cols-[264px_minmax(0,1fr)] items-start gap-4">
      <Panel className="sticky top-0 p-1.5">
        <StepList
          steps={steps}
          logCounts={countLogsByStep(all)}
          totalLogs={all.length}
          selectedStepId={filters.stepId}
          onSelect={(stepId) => setFilters({ ...filters, stepId })}
        />
      </Panel>

      <div className="flex min-w-0 flex-col gap-3">
        <div className="flex items-center gap-2">
          <SearchInput
            aria-label="Search this audit trail"
            placeholder="Search messages and data"
            value={filters.search}
            onValueChange={(search) => setFilters({ ...filters, search })}
            className="w-64"
          />
          <FilterMenu
            label="Kind"
            selected={filters.kinds}
            onChange={(kinds) => setFilters({ ...filters, kinds })}
            options={kindsPresent.map((kind) => {
              const Icon = LOG_KIND_ICON[kind];
              return {
                value: kind,
                label: LOG_KIND_LABEL[kind],
                icon: <Icon />,
                count: all.filter((entry) => entry.kind === kind).length,
              };
            })}
          />
          <FilterMenu
            label="Level"
            selected={filters.levels}
            onChange={(levels) => setFilters({ ...filters, levels })}
            options={LOG_LEVELS.map((level) => ({
              value: level,
              label: LOG_LEVEL_LABEL[level],
              count: all.filter((entry) => entry.level === level).length,
            }))}
          />
          <span className="ml-auto text-[13px] text-muted-foreground tabular-nums" aria-live="polite">
            {filtered
              ? `${visible.length} of ${pluralize(all.length, "entry", "entries")}`
              : pluralize(all.length, "entry", "entries")}
          </span>
          <CopyButton text={() => logsAsText(visible)} label="Copy visible entries" />
        </div>

        <Panel aria-label="Log entries" className="overflow-hidden">
          {logs === undefined ? (
            <LogListSkeleton />
          ) : visible.length === 0 ? (
            all.length === 0 ? (
              <Empty
                icon={ScrollTextIcon}
                title="No log entries"
                description="This scan did not record an audit trail."
              />
            ) : (
              <Empty icon={FilterXIcon} title="No entries match these filters">
                <Button variant="outline" size="sm" onClick={() => setFilters(NO_LOG_FILTERS)}>
                  Clear filters
                </Button>
              </Empty>
            )
          ) : (
            visible.map((entry) => (
              <LogEntryRow
                key={entry.id}
                entry={entry}
                showNode={filters.stepId === null}
                highlighted={entry.id === focusedLogId}
                defaultExpanded={entry.id === focusedLogId}
                onOpenFinding={onOpenFinding}
              />
            ))
          )}
        </Panel>
      </div>
    </div>
  );
}
