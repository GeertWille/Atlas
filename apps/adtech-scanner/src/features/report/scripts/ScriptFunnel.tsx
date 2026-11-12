import { ChevronRightIcon } from "lucide-react";
import { Fragment } from "react";
import type { ScriptRecord } from "@/bindings";
import { cn } from "@/shared/lib/cn";
import { formatBytes, formatInteger, formatPercent } from "@/shared/lib/format";
import { type ScriptScope, scriptFunnel } from "./script-filters";

/** Summary of how Phase 1 cut the volume. Each stage doubles as a scope filter. */
export function ScriptFunnel({
  scripts,
  scope,
  onScopeChange,
}: {
  scripts: ScriptRecord[];
  scope: ScriptScope;
  onScopeChange: (scope: ScriptScope) => void;
}) {
  return (
    <div role="group" aria-label="Script volume by stage" className="flex items-stretch gap-1 rounded-xl border bg-card p-1 shadow-xs">
      {scriptFunnel(scripts).map((stage, index) => (
        <Fragment key={stage.key}>
          {index > 0 ? <ChevronRightIcon aria-hidden="true" className="size-4 shrink-0 self-center text-subtle-foreground" /> : null}
          <button
            type="button"
            aria-pressed={scope === stage.key}
            onClick={() => onScopeChange(stage.key)}
            className={cn(
              "flex min-w-0 flex-1 flex-col gap-1.5 rounded-lg px-3 py-2.5 text-left transition-colors hover:bg-accent/60",
              scope === stage.key && "bg-primary-soft/70 hover:bg-primary-soft/70",
            )}
          >
            <span className="text-[13px] text-muted-foreground">{stage.label}</span>
            <span className="flex items-baseline gap-2">
              <span className="text-xl leading-6 font-semibold tracking-tight">{formatInteger(stage.count)}</span>
              <span className="text-xs text-subtle-foreground tabular-nums">
                {formatBytes(stage.bytes)}
                {index > 0 ? ` · ${formatPercent(stage.shareOfBytes)} of bytes` : ""}
              </span>
            </span>
            <span className="h-1 overflow-hidden rounded-full bg-muted">
              <span className="block h-full rounded-full bg-violet" style={{ width: `${Math.max(1, stage.shareOfBytes * 100)}%` }} />
            </span>
          </button>
        </Fragment>
      ))}
    </div>
  );
}
