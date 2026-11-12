import { FilterXIcon, ShieldCheckIcon } from "lucide-react";
import { useEffect, useState } from "react";
import type { Finding, FindingSource } from "@/bindings";
import { FilterMenu } from "@/shared/components/filter-menu";
import { SEVERITY_ICON } from "@/shared/components/status";
import { FINDING_CATEGORIES, FINDING_CATEGORY_LABEL, SEVERITIES, SEVERITY_META } from "@/shared/domain/vocabulary";
import { countOf } from "@/shared/lib/format";
import { Button } from "@/shared/ui/button";
import { Empty } from "@/shared/ui/empty";
import { ToggleGroup, ToggleGroupItem } from "@/shared/ui/toggle-group";
import { FindingCard } from "./FindingCard";
import {
  countBy,
  type FindingFilters,
  filterFindings,
  hasActiveFilters,
  NO_FINDING_FILTERS,
  sortFindings,
} from "./finding-filters";

export function FindingsTab({
  findings,
  focusedFindingId,
  onOpenScript,
}: {
  findings: Finding[];
  /** Deep link target: expanded and scrolled into view. */
  focusedFindingId?: string;
  onOpenScript: (scriptUrl: string) => void;
}) {
  const sorted = sortFindings(findings);
  const [filters, setFilters] = useState<FindingFilters>(NO_FINDING_FILTERS);
  const [expandedIds, setExpandedIds] = useState<Set<string>>(
    () => new Set([focusedFindingId ?? sorted[0]?.id].filter((id): id is string => Boolean(id))),
  );
  const visible = filterFindings(sorted, filters);
  const severityCounts = countBy(findings, (finding) => finding.severity);
  const categoryCounts = countBy(findings, (finding) => finding.category);

  useEffect(() => {
    if (!focusedFindingId) return;
    setExpandedIds((current) => new Set(current).add(focusedFindingId));
    document.getElementById(`finding-${focusedFindingId}`)?.scrollIntoView({ block: "start" });
  }, [focusedFindingId]);

  const toggle = (id: string) =>
    setExpandedIds((current) => {
      const next = new Set(current);
      if (!next.delete(id)) next.add(id);
      return next;
    });

  if (findings.length === 0) {
    return (
      <Empty
        icon={ShieldCheckIcon}
        title="No findings"
        description="Neither the deterministic checks nor the analysis reported a problem for this page."
      />
    );
  }

  return (
    <div className="flex flex-col gap-3">
      <div className="flex items-center gap-2">
        <FilterMenu
          label="Severity"
          selected={filters.severities}
          onChange={(severities) => setFilters({ ...filters, severities })}
          options={SEVERITIES.filter((severity) => severityCounts[severity]).map((severity) => {
            const Icon = SEVERITY_ICON[severity];
            return {
              value: severity,
              label: SEVERITY_META[severity].label,
              icon: <Icon />,
              count: severityCounts[severity],
            };
          })}
        />
        <FilterMenu
          label="Category"
          selected={filters.categories}
          onChange={(categories) => setFilters({ ...filters, categories })}
          options={FINDING_CATEGORIES.filter((category) => categoryCounts[category]).map((category) => ({
            value: category,
            label: FINDING_CATEGORY_LABEL[category],
            count: categoryCounts[category],
          }))}
        />
        <ToggleGroup<FindingSource | "all">
          size="sm"
          aria-label="Source"
          value={filters.source}
          onValueChange={(source) => setFilters({ ...filters, source })}
        >
          <ToggleGroupItem value="all">All sources</ToggleGroupItem>
          <ToggleGroupItem value="ai">AI</ToggleGroupItem>
          <ToggleGroupItem value="heuristic">Rules</ToggleGroupItem>
        </ToggleGroup>
        <span className="ml-auto text-[13px] text-muted-foreground tabular-nums" aria-live="polite">
          {countOf(visible.length, findings.length, "finding")}
        </span>
      </div>

      {visible.length === 0 ? (
        <Empty icon={FilterXIcon} title="No findings match these filters">
          <Button
            variant="outline"
            size="sm"
            disabled={!hasActiveFilters(filters)}
            onClick={() => setFilters(NO_FINDING_FILTERS)}
          >
            Clear filters
          </Button>
        </Empty>
      ) : (
        visible.map((finding) => (
          <FindingCard
            key={finding.id}
            finding={finding}
            expanded={expandedIds.has(finding.id)}
            onToggle={() => toggle(finding.id)}
            onOpenScript={onOpenScript}
          />
        ))
      )}
    </div>
  );
}
