import { ChevronRightIcon, CpuIcon, SparklesIcon, TrendingDownIcon } from "lucide-react";
import type { Finding } from "@/bindings";
import { CopyButton } from "@/shared/components/copy-button";
import { Markdown } from "@/shared/components/markdown";
import { SeverityBadge } from "@/shared/components/status";
import { FINDING_CATEGORY_LABEL, FINDING_SOURCE_LABEL } from "@/shared/domain/vocabulary";
import { cn } from "@/shared/lib/cn";
import { formatPercent } from "@/shared/lib/format";
import { Badge } from "@/shared/ui/badge";
import { EvidenceBlock } from "./EvidenceBlock";
import { findingToMarkdown } from "./finding-markdown";

function Chips({ label, items }: { label: string; items: string[] }) {
  if (items.length === 0) return null;
  return (
    <div className="flex flex-wrap items-center gap-1.5">
      <span className="w-14 text-xs text-subtle-foreground">{label}</span>
      {items.map((item) => (
        <Badge key={item} tone="outline" className="selectable font-mono text-[11px]">
          {item}
        </Badge>
      ))}
    </div>
  );
}

export function FindingCard({
  finding,
  expanded,
  onToggle,
  onOpenScript,
}: {
  finding: Finding;
  expanded: boolean;
  onToggle: () => void;
  onOpenScript: (scriptUrl: string) => void;
}) {
  const SourceIcon = finding.source === "ai" ? SparklesIcon : CpuIcon;
  return (
    <article
      id={`finding-${finding.id}`}
      aria-label={finding.title}
      className={cn("scroll-mt-4 rounded-xl border bg-card shadow-xs transition-shadow", expanded && "shadow-pop/40")}
    >
      <div className="flex items-start gap-1 pr-2">
        <button
          type="button"
          aria-expanded={expanded}
          onClick={onToggle}
          className="flex min-w-0 flex-1 items-start gap-3 rounded-xl py-3.5 pl-4 text-left"
        >
          <ChevronRightIcon
            className={cn(
              "mt-0.5 size-4 shrink-0 text-subtle-foreground transition-transform",
              expanded && "rotate-90",
            )}
          />
          <span className="flex min-w-0 flex-1 flex-col gap-1.5">
            <span className="text-sm leading-5 font-semibold text-pretty">{finding.title}</span>
            <span className="flex flex-wrap items-center gap-x-2 gap-y-1.5">
              <SeverityBadge severity={finding.severity} />
              <Badge tone="outline">{FINDING_CATEGORY_LABEL[finding.category]}</Badge>
              <Badge tone={finding.source === "ai" ? "violet" : "outline"}>
                <SourceIcon />
                {FINDING_SOURCE_LABEL[finding.source]}
              </Badge>
              <span className="text-xs text-muted-foreground">{finding.impact}</span>
            </span>
          </span>
          <span className="flex shrink-0 flex-col items-end gap-0.5 pt-0.5 text-xs tabular-nums">
            {finding.estimatedRevenueImpactPct !== null ? (
              <span className="flex items-center gap-1 font-medium text-foreground">
                <TrendingDownIcon className="size-3.5 text-red-fg" />~{finding.estimatedRevenueImpactPct}% revenue at
                risk
              </span>
            ) : null}
            <span className="text-subtle-foreground">{formatPercent(finding.confidence)} confidence</span>
          </span>
        </button>
        <CopyButton text={() => findingToMarkdown(finding)} label="Copy finding as Markdown" className="mt-2.5" />
      </div>

      {expanded ? (
        <div className="flex animate-enter flex-col gap-4 border-t px-4 pt-4 pb-4 pl-11">
          <Markdown>{finding.description}</Markdown>
          <div className="flex flex-col gap-2 rounded-lg border border-green-line bg-green-soft/50 px-3.5 py-3">
            <h3 className="text-xs font-semibold tracking-wide text-green-fg uppercase">Recommendation</h3>
            <Markdown className="text-foreground/90">{finding.recommendation}</Markdown>
          </div>
          {finding.evidence.length > 0 ? (
            <div className="flex flex-col gap-2.5">
              <h3 className="text-xs font-semibold tracking-wide text-subtle-foreground uppercase">Evidence</h3>
              {finding.evidence.map((evidence) => (
                <EvidenceBlock
                  key={`${evidence.kind}-${evidence.label}`}
                  evidence={evidence}
                  onOpenScript={onOpenScript}
                />
              ))}
            </div>
          ) : null}
          <div className="flex flex-col gap-1.5">
            <Chips label="Slots" items={finding.affectedSlots} />
            <Chips label="Bidders" items={finding.affectedBidders} />
          </div>
        </div>
      ) : null}
    </article>
  );
}
