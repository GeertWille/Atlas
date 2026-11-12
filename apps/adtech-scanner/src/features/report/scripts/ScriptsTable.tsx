import { useVirtualizer } from "@tanstack/react-virtual";
import { ArrowDownIcon, CpuIcon, MegaphoneIcon, SparklesIcon } from "lucide-react";
import { useEffect, useRef } from "react";
import type { ScriptRecord } from "@/bindings";
import { SCRIPT_CATEGORY_META } from "@/shared/domain/vocabulary";
import { cn } from "@/shared/lib/cn";
import { EMPTY, formatBytes, formatOffset, formatPercent } from "@/shared/lib/format";
import { fileNameOf } from "@/shared/lib/url";
import { Badge } from "@/shared/ui/badge";
import type { ScriptSort } from "./script-filters";

const ROW_HEIGHT = 52;
const COLUMNS =
  "grid grid-cols-[minmax(0,1fr)_104px_72px_84px_100px] items-center gap-x-3 px-4 xl:grid-cols-[minmax(0,1fr)_112px_120px_84px_92px_128px]";

function SortHeader({ label, active, onClick }: { label: string; active: boolean; onClick: () => void }) {
  return (
    <span
      role="columnheader"
      aria-sort={active ? (label === "Size" ? "descending" : "ascending") : "none"}
      className="flex justify-end"
    >
      <button
        type="button"
        onClick={onClick}
        className={cn("flex items-center gap-1 rounded-sm hover:text-foreground", active && "text-foreground")}
      >
        {label}
        <ArrowDownIcon
          className={cn("size-3", active ? "opacity-100" : "opacity-0", label !== "Size" && "rotate-180")}
        />
      </button>
    </span>
  );
}

function ScriptRow({ script, selected, onSelect }: { script: ScriptRecord; selected: boolean; onSelect: () => void }) {
  const category = SCRIPT_CATEGORY_META[script.category];
  const ClassifierIcon = script.classifiedBy === "ai" ? SparklesIcon : CpuIcon;
  return (
    <div
      role="row"
      tabIndex={0}
      aria-selected={selected}
      onClick={onSelect}
      onKeyDown={(event) => {
        if (event.key === "Enter" || event.key === " ") {
          event.preventDefault();
          onSelect();
        }
      }}
      className={cn(
        COLUMNS,
        "h-full border-b text-[13px] outline-offset-[-2px] hover:bg-accent/60",
        selected && "bg-primary-soft/60 hover:bg-primary-soft/60",
      )}
    >
      <span role="cell" className="flex min-w-0 flex-col">
        <span className="flex min-w-0 items-center gap-1.5">
          {script.adRelated ? (
            <MegaphoneIcon role="img" aria-label="Ad-related" className="size-3.5 shrink-0 text-violet-fg" />
          ) : null}
          <span className="truncate font-medium">{fileNameOf(script.url)}</span>
          {script.analyzed ? (
            <Badge tone="violet" className="h-4 px-1.5 text-[10px]">
              Phase 2
            </Badge>
          ) : null}
        </span>
        <span className="truncate text-xs text-subtle-foreground">
          {script.inline ? "inline" : script.hostname}
          {script.firstParty ? " · first party" : ""}
          {script.classificationReason ? ` · ${script.classificationReason}` : ""}
        </span>
      </span>
      <span role="cell">
        <Badge tone={category.tone}>{category.label}</Badge>
      </span>
      <span role="cell" className="hidden truncate text-muted-foreground xl:block">
        {script.vendor ?? EMPTY}
      </span>
      <span role="cell" className="text-right tabular-nums">
        {formatBytes(script.sizeBytes)}
      </span>
      <span role="cell" className="text-right text-muted-foreground tabular-nums">
        {formatOffset(script.loadedAtMs)}
      </span>
      <span role="cell" className="flex items-center justify-end gap-1.5 text-xs text-muted-foreground tabular-nums">
        <ClassifierIcon className="size-3.5 text-subtle-foreground" />
        {script.classifiedBy === "ai" ? "AI" : "Rule"}
        {script.classificationConfidence !== null ? ` · ${formatPercent(script.classificationConfidence)}` : ""}
      </span>
    </div>
  );
}

export function ScriptsTable({
  scripts,
  sort,
  onSortChange,
  selectedId,
  onSelect,
}: {
  scripts: ScriptRecord[];
  sort: ScriptSort;
  onSortChange: (sort: ScriptSort) => void;
  selectedId: string | null;
  onSelect: (script: ScriptRecord) => void;
}) {
  // TanStack Virtual mutates its instance between renders; the React Compiler must not memoise around it.
  "use no memo";
  const scroller = useRef<HTMLDivElement>(null);
  const virtualizer = useVirtualizer({
    count: scripts.length,
    getScrollElement: () => scroller.current,
    estimateSize: () => ROW_HEIGHT,
    overscan: 10,
  });

  const selectedIndex = selectedId === null ? -1 : scripts.findIndex((script) => script.id === selectedId);
  useEffect(() => {
    if (selectedIndex >= 0) virtualizer.scrollToIndex(selectedIndex, { align: "auto" });
  }, [selectedIndex, virtualizer]);

  return (
    <div role="table" aria-label="Scripts" aria-rowcount={scripts.length + 1} className="flex min-h-0 flex-col">
      <div role="rowgroup">
        <div role="row" className={cn(COLUMNS, "h-9 border-b text-xs font-medium text-muted-foreground")}>
          <span role="columnheader">Script</span>
          <span role="columnheader">Category</span>
          <span role="columnheader" className="hidden xl:block">
            Vendor
          </span>
          <SortHeader label="Size" active={sort === "sizeDesc"} onClick={() => onSortChange("sizeDesc")} />
          <SortHeader label="Loaded" active={sort === "loadOrder"} onClick={() => onSortChange("loadOrder")} />
          <span role="columnheader" className="text-right">
            Classified by
          </span>
        </div>
      </div>
      <div ref={scroller} role="rowgroup" className="max-h-[min(728px,70vh)] min-h-0 overflow-y-auto">
        <div className="relative w-full" style={{ height: virtualizer.getTotalSize() }}>
          {virtualizer.getVirtualItems().map((item) => {
            const script = scripts[item.index];
            if (!script) return null;
            return (
              <div
                key={script.id}
                className="absolute inset-x-0 top-0"
                style={{ height: item.size, transform: `translateY(${item.start}px)` }}
              >
                <ScriptRow script={script} selected={script.id === selectedId} onSelect={() => onSelect(script)} />
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}
