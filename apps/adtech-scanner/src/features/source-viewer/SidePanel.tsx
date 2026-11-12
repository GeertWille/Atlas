import { PanelRightCloseIcon } from "lucide-react";
import { type ReactNode, useId } from "react";
import type { AdCall, ScriptRecord, ScriptSource } from "@/bindings";
import { SEVERITY_ICON } from "@/shared/components/status";
import { SEVERITY_META } from "@/shared/domain/vocabulary";
import { cn } from "@/shared/lib/cn";
import { formatInteger, formatPercent } from "@/shared/lib/format";
import { MOD_KEY } from "@/shared/lib/platform";
import { Button } from "@/shared/ui/button";
import { Switch } from "@/shared/ui/switch";
import { Hint } from "@/shared/ui/tooltip";
import { type FindingMark, readRanges, readShare } from "./anchors";
import { lensKey } from "./finding-extension";

const SEVERITY_TEXT = {
  red: "text-red-fg",
  orange: "text-orange-fg",
  amber: "text-amber-fg",
  blue: "text-blue-fg",
  gray: "text-gray-fg",
} as const;

function Section({ title, count, children }: { title: string; count?: number; children: ReactNode }) {
  const headingId = useId();
  return (
    <section aria-labelledby={headingId} className="flex flex-col border-b last:border-b-0">
      <h2 id={headingId} className="flex h-8 shrink-0 items-center gap-1.5 px-3 text-xs font-semibold">
        {title}
        {count !== undefined ? <span className="font-normal text-subtle-foreground tabular-nums">{count}</span> : null}
      </h2>
      {children}
    </section>
  );
}

function JumpButton({
  line,
  onJump,
  children,
  label,
}: {
  line: number;
  onJump: () => void;
  children: ReactNode;
  label: string;
}) {
  return (
    <button
      type="button"
      aria-label={label}
      onClick={onJump}
      className="flex w-full items-start gap-2 px-3 py-1 text-left text-[13px] outline-offset-[-2px] hover:bg-accent"
    >
      {children}
      <span className="ml-auto shrink-0 pt-px font-mono text-[11px] text-subtle-foreground tabular-nums">L{line}</span>
    </button>
  );
}

function Note({ children }: { children: ReactNode }) {
  return <p className="px-3 pb-2.5 text-xs text-pretty text-muted-foreground">{children}</p>;
}

function findingsNote(script: ScriptRecord, aiUsed: boolean): string {
  if (!aiUsed) return "Code findings need an AI scan. This scan ran the deterministic checks only.";
  if (!script.analyzed) return "This file was not handed to the AI, so it has no code findings.";
  return "The AI read this file and reported nothing in it.";
}

export function SidePanel({
  script,
  source,
  marks,
  aiUsed,
  dimUnread,
  onDimUnreadChange,
  onJump,
  onClose,
  className,
}: {
  script: ScriptRecord;
  source: ScriptSource;
  marks: FindingMark[];
  /** Whether the scan had any AI analysis at all. */
  aiUsed: boolean;
  dimUnread: boolean;
  onDimUnreadChange: (dim: boolean) => void;
  /** `lens`: the finding whose one-line summary opens at the target. */
  onJump: (line: number, endLine: number, lens?: string) => void;
  onClose: () => void;
  className?: string;
}) {
  const dimId = useId();
  const formatted = source.view === "formatted";
  const ranges = readRanges(source.regions, source.lineCount);
  const share = readShare(source.regions, source.lineCount);
  const guided = source.regions.some((region) => region.kind === "guide");

  return (
    <aside
      aria-label="In this file"
      className={cn("flex w-[260px] shrink-0 flex-col border-l bg-background", className)}
    >
      <div className="flex h-9 shrink-0 items-center justify-between border-b pr-1.5 pl-3">
        <span className="text-xs font-semibold tracking-wide text-subtle-foreground uppercase">In this file</span>
        <Hint label={`Hide this panel (${MOD_KEY}⌥B)`}>
          <Button variant="ghost" size="icon-sm" aria-label="Hide the file panel" onClick={onClose}>
            <PanelRightCloseIcon />
          </Button>
        </Hint>
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto">
        <Section title="Findings" count={formatted ? marks.length : undefined}>
          {!formatted ? (
            <Note>Findings cite lines of the formatted view. Switch to Formatted to see them in the code.</Note>
          ) : marks.length === 0 ? (
            <Note>{findingsNote(script, aiUsed)}</Note>
          ) : (
            <ul className="pb-1.5">
              {marks.map((mark) => {
                const Icon = SEVERITY_ICON[mark.severity];
                const meta = SEVERITY_META[mark.severity];
                return (
                  <li key={lensKey(mark)}>
                    <JumpButton
                      line={mark.startLine}
                      label={`${meta.label}: ${mark.title}, line ${mark.startLine}`}
                      onJump={() => onJump(mark.startLine, mark.endLine, lensKey(mark))}
                    >
                      <Icon
                        aria-hidden="true"
                        className={cn(
                          "mt-0.5 size-3.5 shrink-0",
                          SEVERITY_TEXT[meta.tone as keyof typeof SEVERITY_TEXT],
                        )}
                      />
                      <span className="flex min-w-0 flex-col">
                        <span className="truncate leading-[18px]" title={mark.title}>
                          {mark.title}
                        </span>
                        <span className="truncate text-xs text-muted-foreground" title={mark.label}>
                          {mark.label}
                        </span>
                      </span>
                    </JumpButton>
                  </li>
                );
              })}
            </ul>
          )}
        </Section>

        <Section title="Ad API calls" count={source.adCalls.length}>
          {!formatted && source.lineCount <= 1 && source.adCalls.length > 0 ? (
            <Note>
              This file calls {[...new Set(source.adCalls.map((call) => call.name))].join(", ")}. The original is one
              line: switch to Formatted to go to each call.
            </Note>
          ) : source.adCalls.length === 0 ? (
            <Note>No call into Prebid, Google Publisher Tag, Amazon or the consent API was found in this file.</Note>
          ) : (
            <ul className="pb-1.5">
              {source.adCalls.map((call: AdCall, index) => (
                // biome-ignore lint/suspicious/noArrayIndexKey: one line can hold the same call twice
                <li key={`${call.name}-${call.line}-${index}`}>
                  <JumpButton
                    line={call.line}
                    label={`${call.name}, line ${call.line}`}
                    onJump={() => onJump(call.line, call.line)}
                  >
                    <span className="min-w-0 truncate font-mono text-xs leading-[18px]">{call.name}</span>
                  </JumpButton>
                </li>
              ))}
            </ul>
          )}
        </Section>

        <Section title="Read by the AI">
          {!formatted ? (
            <Note>What the AI read is recorded in lines of the formatted view.</Note>
          ) : ranges.length === 0 ? (
            <Note>
              {script.analyzed
                ? "No record of which lines the AI read is stored for this file."
                : "The AI was not shown this file."}
            </Note>
          ) : (
            <>
              <div className="flex items-center justify-between gap-3 px-3 pb-2">
                <p className="text-xs text-muted-foreground">
                  <span className="font-medium text-foreground tabular-nums">{formatPercent(share)}</span> of the lines
                  {guided ? ", including the reading guide" : ""}
                </p>
                <label htmlFor={dimId} className="flex shrink-0 items-center gap-2 text-xs text-muted-foreground">
                  Dim unread
                  <Switch id={dimId} checked={dimUnread} onCheckedChange={onDimUnreadChange} />
                </label>
              </div>
              <ul className="pb-1.5">
                {ranges.map((range) => (
                  <li key={range.startLine}>
                    <JumpButton
                      line={range.startLine}
                      label={`Lines ${range.startLine} to ${range.endLine}`}
                      onJump={() => onJump(range.startLine, range.startLine)}
                    >
                      <span className="text-xs leading-[18px] text-muted-foreground tabular-nums">
                        {formatInteger(range.endLine - range.startLine + 1)} lines
                      </span>
                    </JumpButton>
                  </li>
                ))}
              </ul>
            </>
          )}
        </Section>
      </div>
    </aside>
  );
}
