import {
  ArrowDownLeftIcon,
  ArrowUpRightIcon,
  BrainIcon,
  ChevronRightIcon,
  CornerDownRightIcon,
  FlagIcon,
  type LucideIcon,
  TerminalIcon,
  WrenchIcon,
} from "lucide-react";
import { useState } from "react";
import type { LogEntry, LogKind } from "@/bindings";
import {
  findingSeverity,
  isExpandable,
  logEntryAsText,
  logPayload,
  summarizeToolInput,
} from "@/shared/domain/log-entry";
import { PIPELINE_NODE_META } from "@/shared/domain/pipeline";
import { LOG_KIND_LABEL, LOG_LEVEL_LABEL } from "@/shared/domain/vocabulary";
import { cn } from "@/shared/lib/cn";
import { formatClock } from "@/shared/lib/format";
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
};

const KIND_ICON_TONE: Record<LogKind, string> = {
  message: "text-subtle-foreground",
  llmRequest: "text-blue-fg",
  llmResponse: "text-blue-fg",
  thinking: "text-violet-fg",
  toolCall: "text-amber-fg",
  toolResult: "text-amber-fg",
  finding: "text-red-fg",
};

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

function Details({ entry }: { entry: LogEntry }) {
  const { text, json } = logPayload(entry);
  const showMessage = entry.kind !== "toolCall";
  return (
    <div className="flex flex-col gap-2 pr-3 pb-3 pl-[7.25rem]">
      {showMessage ? (
        <p className={cn("selectable text-[13px] leading-relaxed whitespace-pre-wrap", entry.kind === "thinking" && "text-muted-foreground italic")}>
          {entry.message}
        </p>
      ) : null}
      {text !== null ? (
        <pre
          // biome-ignore lint/a11y/noNoninteractiveTabindex: scroll container
          tabIndex={0}
          className="selectable max-h-80 overflow-auto rounded-md border bg-code p-3 font-mono text-xs leading-[1.6] whitespace-pre"
        >
          {text}
        </pre>
      ) : null}
      {json !== null ? <CodeBlock code={json} language="json" className="max-h-80 overflow-auto rounded-md border" /> : null}
    </div>
  );
}

/** One audit-trail entry. Shared by the live activity feed and the audit trail tab. */
export function LogEntryRow({
  entry,
  live = false,
  highlighted = false,
  showNode = false,
  defaultExpanded = false,
}: {
  entry: LogEntry;
  /** The newest line of a running scan shimmers. */
  live?: boolean;
  highlighted?: boolean;
  showNode?: boolean;
  defaultExpanded?: boolean;
}) {
  const [expanded, setExpanded] = useState(defaultExpanded);
  const expandable = isExpandable(entry);
  const Icon = LOG_KIND_ICON[entry.kind];

  return (
    <div
      id={`log-${entry.id}`}
      data-kind={entry.kind}
      data-level={entry.level}
      className={cn(
        "group/log offscreen-auto border-b last:border-b-0",
        highlighted && "bg-primary-soft/60",
      )}
    >
      <div className="flex h-9 items-center gap-2 pr-2 pl-3 text-[13px]">
        <button
          type="button"
          disabled={!expandable}
          aria-expanded={expandable ? expanded : undefined}
          aria-label={expandable ? `${expanded ? "Collapse" : "Expand"} log entry: ${entry.message}` : undefined}
          onClick={() => setExpanded((current) => !current)}
          className="flex h-full min-w-0 flex-1 items-center gap-2 text-left disabled:cursor-default"
        >
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
        </button>
        {showNode ? (
          <span className="hidden shrink-0 text-xs text-subtle-foreground lg:inline">
            {PIPELINE_NODE_META[entry.node].label}
          </span>
        ) : null}
        <CopyButton
          text={() => logEntryAsText(entry)}
          label="Copy entry"
          className="opacity-0 group-focus-within/log:opacity-100 group-hover/log:opacity-100"
        />
      </div>
      {expanded ? <Details entry={entry} /> : null}
    </div>
  );
}
