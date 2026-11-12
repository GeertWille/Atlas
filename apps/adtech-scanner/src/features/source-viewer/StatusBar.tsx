import type { ScriptSource, SourceFormat } from "@/bindings";
import { formatInteger, pluralize } from "@/shared/lib/format";

/** How the text on screen came to be. Never claims a parse that did not happen. */
const FORMAT_NOTE: Record<SourceFormat, string> = {
  parser: "Formatted by the app (parser)",
  reflow: "Formatted approximately; the file did not parse",
  unformatted: "Shown as captured; too large to format",
};

export type CaretInfo = { line: number; column: number; selectedLines: number };

function describeSource(source: ScriptSource, guarded: boolean): string {
  const note = source.view === "original" ? "Original, as captured" : FORMAT_NOTE[source.format];
  return guarded ? `${note} · highlighting off (very long lines)` : note;
}

export function StatusBar({
  source,
  guarded,
  caret,
  findingCount,
  searchText,
}: {
  source: ScriptSource | undefined;
  /** Highlighting is off and wrapping forced because a line is too long. */
  guarded: boolean;
  caret: CaretInfo | null;
  findingCount: number;
  /** "3 of 41" while the find bar is open with a query. */
  searchText: string | null;
}) {
  return (
    <footer className="flex h-6 shrink-0 items-center gap-4 border-t bg-sidebar px-3 text-[11px] text-muted-foreground tabular-nums">
      {source?.text != null ? (
        <>
          <span className="min-w-0 truncate" data-testid="source-format">
            {describeSource(source, guarded)}
          </span>
          <span className="shrink-0">{pluralize(source.lineCount, "line")}</span>
        </>
      ) : null}
      <span className="ml-auto flex shrink-0 items-center gap-4">
        {searchText ? <span>Find: {searchText}</span> : null}
        {source?.view === "formatted" && source.text != null ? (
          <span>{findingCount === 0 ? "No findings in this file" : pluralize(findingCount, "finding")}</span>
        ) : null}
        {caret ? (
          <span aria-live="off" data-testid="caret-position">
            Ln {formatInteger(caret.line)}, Col {formatInteger(caret.column)}
            {caret.selectedLines > 1 ? ` (${formatInteger(caret.selectedLines)} lines selected)` : ""}
          </span>
        ) : null}
      </span>
    </footer>
  );
}
