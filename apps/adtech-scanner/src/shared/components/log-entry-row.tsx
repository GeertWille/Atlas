import {
  ArrowDownLeftIcon,
  ArrowRightIcon,
  ArrowUpRightIcon,
  BrainIcon,
  ChevronRightIcon,
  CornerDownRightIcon,
  FlagIcon,
  type LucideIcon,
  ScaleIcon,
  TerminalIcon,
  WrenchIcon,
} from "lucide-react";
import type { LogEntry, LogKind } from "@/bindings";
import {
  type ClassificationDetails,
  classificationDetails,
  findingSeverity,
  isExpandable,
  logEntryAsText,
  logPayload,
  messageExceedsHeadline,
  reportedFindingId,
  summarizeToolInput,
} from "@/shared/domain/log-entry";
import { PIPELINE_NODE_META } from "@/shared/domain/pipeline";
import { LOG_KIND_LABEL, LOG_LEVEL_LABEL, scriptCategoryLabel } from "@/shared/domain/vocabulary";
import { cn } from "@/shared/lib/cn";
import { formatClock } from "@/shared/lib/format";
import { Badge } from "@/shared/ui/badge";
import { Button } from "@/shared/ui/button";
import { CodeBlock } from "./code-block";
import { CopyButton } from "./copy-button";
import { SeverityBadge } from "./status";

export const LOG_KIND_ICON: Record<LogKind, LucideIcon> = {
  message: TerminalIcon,
  llmRequest: ArrowUpRightIcon,
  llmResponse: ArrowDownLeftIcon,
  thinking: BrainIcon,
  toolCall: WrenchIcon,
  toolResult: CornerDownRightIcon,
  finding: FlagIcon,
  classification: ScaleIcon,
};

const KIND_ICON_TONE: Record<LogKind, string> = {
  message: "text-subtle-foreground",
  llmRequest: "text-blue-fg",
  llmResponse: "text-blue-fg",
  thinking: "text-violet-fg",
  toolCall: "text-amber-fg",
  toolResult: "text-amber-fg",
  finding: "text-red-fg",
  classification: "text-violet-fg",
};

const ROW_BODY = "flex h-full min-w-0 flex-1 items-center gap-2 text-left";

/** The backend's rule for a script the classifier neither confirmed nor ruled out; it is kept, ranked last. */
const UNSURE_RULE = "jev-unsure";

/** A script kept only because it could not be ruled out must not read like one the classifier confirmed. */
function VerdictBadge({ adRelated, rule }: { adRelated: boolean; rule: string | null }) {
  if (rule === UNSURE_RULE) return <Badge tone="amber">Unsure, kept</Badge>;
  return <Badge tone={adRelated ? "violet" : "gray"}>{adRelated ? "Ad-related" : "Not ad-related"}</Badge>;
}

function Headline({ entry, live }: { entry: LogEntry; live: boolean }) {
  if (entry.kind === "toolCall") {
    return (
      <span className="min-w-0 truncate font-mono text-xs">
        <span className="font-medium text-foreground">{entry.message}</span>
        <span className="text-subtle-foreground">({summarizeToolInput(entry.data)})</span>
      </span>
    );
  }
  if (entry.kind === "finding") {
    const severity = findingSeverity(entry);
    return (
      <span className="flex min-w-0 items-center gap-2">
        {severity ? <SeverityBadge severity={severity} /> : null}
        <span className="truncate font-medium">{entry.message}</span>
      </span>
    );
  }
  // The verdict leads, like a finding's severity: the message itself is cut off in a narrow window.
  const classification = classificationDetails(entry);
  if (classification && classification.adRelated !== null) {
    return (
      <span className="flex min-w-0 items-center gap-2">
        <VerdictBadge adRelated={classification.adRelated} rule={classification.rule} />
        <span className="truncate">{entry.message}</span>
      </span>
    );
  }
  return (
    <span
      className={cn(
        "min-w-0 truncate",
        entry.kind === "thinking" && "text-muted-foreground italic",
        entry.level === "debug" && "text-muted-foreground",
        entry.level === "warn" && "text-amber-fg",
        entry.level === "error" && "text-red-fg",
        live && "shimmer",
      )}
    >
      {entry.message}
    </span>
  );
}

const probability = (value: number) => value.toFixed(2);

/** What the classifier answered about one script, against the thresholds, and where that sent the script. */
function ClassificationSummary({ details }: { details: ClassificationDetails }) {
  const { answers, thresholds, category, rule, adRelated } = details;
  if (answers.length === 0 && thresholds.length === 0 && !category && !rule) return null;
  return (
    <div className="flex flex-col gap-2.5 rounded-md border bg-card px-3 py-2.5 text-xs">
      {answers.length > 0 ? (
        <dl className="grid grid-cols-[minmax(0,auto)_minmax(4rem,1fr)_auto] items-center gap-x-3 gap-y-1.5">
          {answers.map((answer) => (
            <div key={answer.question} className="col-span-full grid grid-cols-subgrid items-center">
              <dt className="truncate font-mono text-muted-foreground">{answer.question}</dt>
              {"probability" in answer ? (
                <>
                  <dd aria-hidden="true" className="relative h-1.5 rounded-full bg-muted">
                    <span
                      className="absolute inset-y-0 left-0 rounded-full bg-violet"
                      style={{ width: `${answer.probability * 100}%` }}
                    />
                    {answer.thresholds.map((threshold) => (
                      <span
                        key={threshold.name}
                        data-threshold={threshold.name}
                        className="absolute -inset-y-1 w-px bg-foreground/60"
                        style={{ left: `${threshold.value * 100}%` }}
                      />
                    ))}
                  </dd>
                  <dd className="text-right font-medium tabular-nums">{probability(answer.probability)}</dd>
                </>
              ) : (
                <dd className="col-span-2">
                  <span className="font-medium">{answer.selected}</span>
                  {answer.confidence !== null ? (
                    <span className="text-muted-foreground tabular-nums"> · {probability(answer.confidence)}</span>
                  ) : null}
                </dd>
              )}
            </div>
          ))}
        </dl>
      ) : null}
      <p className="flex flex-wrap items-center gap-x-2 gap-y-1 text-muted-foreground empty:hidden">
        {category ? <Badge tone="outline">{scriptCategoryLabel(category, adRelated === true)}</Badge> : null}
        {rule ? (
          <span>
            Rule <span className="font-mono font-medium text-foreground">{rule}</span>
          </span>
        ) : null}
        {thresholds.length > 0 ? (
          <span className="tabular-nums">
            Thresholds {thresholds.map((threshold) => `${threshold.name} ${probability(threshold.value)}`).join(", ")}
          </span>
        ) : null}
      </p>
    </div>
  );
}

function Details({ entry, onOpenFinding }: { entry: LogEntry; onOpenFinding?: (findingId: string) => void }) {
  const { text, json } = logPayload(entry);
  const findingId = reportedFindingId(entry);
  const classification = classificationDetails(entry);
  // The headline already shows short one-line messages in full, and a tool call's name is never long.
  const showMessage = entry.kind !== "toolCall" && messageExceedsHeadline(entry);
  return (
    <div className="flex flex-col gap-2 pr-3 pb-3 pl-[7.25rem]">
      {showMessage ? (
        <p
          className={cn(
            "selectable text-[13px] leading-relaxed whitespace-pre-wrap",
            entry.kind === "thinking" && "text-muted-foreground italic",
          )}
        >
          {entry.message}
        </p>
      ) : null}
      {classification ? <ClassificationSummary details={classification} /> : null}
      {text !== null ? (
        <pre
          // biome-ignore lint/a11y/noNoninteractiveTabindex: scroll container
          tabIndex={0}
          className={cn(
            "selectable max-h-80 overflow-auto rounded-md border bg-code p-3 font-mono text-xs leading-[1.6]",
            // A script excerpt is usually one long minified line; tool output is line-oriented.
            classification ? "break-all whitespace-pre-wrap" : "whitespace-pre",
          )}
        >
          {text}
        </pre>
      ) : null}
      {findingId && onOpenFinding ? (
        <div>
          <Button variant="outline" size="sm" onClick={() => onOpenFinding(findingId)}>
            Open finding
            <ArrowRightIcon />
          </Button>
        </div>
      ) : null}
      {json !== null ? (
        <CodeBlock code={json} language="json" className="max-h-80 overflow-auto rounded-md border" />
      ) : null}
    </div>
  );
}

/** One audit-trail entry: a one-line summary that opens into the full message and payload. */
export function LogEntryRow({
  entry,
  live,
  highlighted,
  showNode,
  expanded,
  onToggle,
  onOpenFinding,
}: {
  entry: LogEntry;
  live: boolean;
  highlighted: boolean;
  showNode: boolean;
  expanded: boolean;
  onToggle: () => void;
  /** Enables the link from a `finding` entry to the finding it reported. */
  onOpenFinding?: (findingId: string) => void;
}) {
  const expandable = isExpandable(entry);
  const Icon = LOG_KIND_ICON[entry.kind];

  const summary = (
    <>
      <ChevronRightIcon
        className={cn(
          "size-3.5 shrink-0 text-subtle-foreground transition-transform",
          expanded && "rotate-90",
          !expandable && "invisible",
        )}
      />
      <time className="w-[5.25rem] shrink-0 font-mono text-[11px] text-subtle-foreground tabular-nums">
        {formatClock(entry.ts)}
      </time>
      <Icon
        role="img"
        aria-label={`${LOG_KIND_LABEL[entry.kind]}, ${LOG_LEVEL_LABEL[entry.level]}`}
        className={cn("size-3.5 shrink-0", KIND_ICON_TONE[entry.kind])}
      />
      <Headline entry={entry} live={live} />
    </>
  );

  return (
    <div
      id={`log-${entry.id}`}
      data-kind={entry.kind}
      data-level={entry.level}
      className={cn("group/log border-b", highlighted && "bg-primary-soft/60")}
    >
      <div className="flex h-9 items-center gap-2 pr-2 pl-3 text-[13px]">
        {expandable ? (
          <button type="button" aria-expanded={expanded} onClick={onToggle} className={ROW_BODY}>
            {summary}
          </button>
        ) : (
          <div className={ROW_BODY}>{summary}</div>
        )}
        {showNode ? (
          <span className="hidden shrink-0 text-xs text-subtle-foreground xl:inline">
            {PIPELINE_NODE_META[entry.node].label}
          </span>
        ) : null}
        <CopyButton
          text={() => logEntryAsText(entry)}
          label="Copy entry"
          className="opacity-0 group-focus-within/log:opacity-100 group-hover/log:opacity-100"
        />
      </div>
      {expanded ? <Details entry={entry} onOpenFinding={onOpenFinding} /> : null}
    </div>
  );
}
