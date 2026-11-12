import { Link } from "@tanstack/react-router";
import { useVirtualizer } from "@tanstack/react-virtual";
import { ChevronRightIcon, FileCode2Icon, PanelLeftCloseIcon, PanelLeftOpenIcon } from "lucide-react";
import { type KeyboardEvent, useEffect, useRef, useState } from "react";
import type { Finding, ScriptRecord } from "@/bindings";
import { SearchInput } from "@/shared/components/search-input";
import { SEVERITY_ICON } from "@/shared/components/status";
import { SCRIPT_CATEGORY_META, SEVERITY_META, scriptCategoryLabel } from "@/shared/domain/vocabulary";
import { cn } from "@/shared/lib/cn";
import { pluralize } from "@/shared/lib/format";
import { MOD_KEY } from "@/shared/lib/platform";
import { Button } from "@/shared/ui/button";
import { Hint } from "@/shared/ui/tooltip";
import {
  FILE_GROUP_LABEL,
  type FileEntry,
  type FileGroupKey,
  type FileListItem,
  flattenGroups,
  groupFiles,
} from "./file-groups";

const GROUP_HEIGHT = 28;
const FILE_HEIGHT = 40;

const DOT_TONE: Record<string, string> = {
  violet: "bg-violet",
  blue: "bg-blue",
  amber: "bg-amber",
  green: "bg-green",
  gray: "bg-gray",
  outline: "border border-border-strong",
  red: "bg-red",
  orange: "bg-orange",
};

function FindingCount({ file }: { file: FileEntry }) {
  if (!file.worst) return null;
  const Icon = SEVERITY_ICON[file.worst];
  const meta = SEVERITY_META[file.worst];
  return (
    <span
      className={cn(
        "flex shrink-0 items-center gap-1 text-xs font-medium tabular-nums",
        { red: "text-red-fg", orange: "text-orange-fg", amber: "text-amber-fg", blue: "text-blue-fg" }[
          meta.tone as string
        ] ?? "text-muted-foreground",
      )}
    >
      {file.findingCount}
      <Icon
        role="img"
        aria-label={`${pluralize(file.findingCount, "finding")}, worst ${meta.label}`}
        className="size-3.5"
      />
    </span>
  );
}

function FileRow({ scanId, file, current }: { scanId: string; file: FileEntry; current: boolean }) {
  const { script } = file;
  const category = SCRIPT_CATEGORY_META[script.category];
  return (
    <Link
      to="/scans/$scanId/source/$scriptId"
      params={{ scanId, scriptId: script.id }}
      aria-current={current ? "page" : undefined}
      // One tab stop for the list; the arrow keys move inside it.
      tabIndex={current ? 0 : -1}
      data-file-row=""
      title={script.url}
      className={cn(
        "mx-1.5 flex h-[38px] items-center gap-2 rounded-md px-2 outline-offset-[-2px] hover:bg-accent",
        current && "bg-primary-soft hover:bg-primary-soft",
      )}
    >
      <span
        role="img"
        aria-label={scriptCategoryLabel(script.category, script.adRelated)}
        className={cn("size-2 shrink-0 rounded-full", DOT_TONE[category.tone])}
      />
      <span className="flex min-w-0 flex-1 flex-col">
        <span className={cn("truncate text-[13px] leading-[18px]", current ? "font-semibold" : "font-medium")}>
          {file.name}
        </span>
        <span className="truncate text-[11px] leading-[14px] text-subtle-foreground">{file.origin}</span>
      </span>
      <FindingCount file={file} />
    </Link>
  );
}

/** The narrow form of the list: a strip that opens it again, or the quick-open dialog. */
export function FileRail({ onExpand, onQuickOpen }: { onExpand: () => void; onQuickOpen: () => void }) {
  return (
    <nav aria-label="Scripts" className="flex w-10 shrink-0 flex-col items-center gap-1 border-r bg-sidebar py-2">
      <Hint label={`Show the file list (${MOD_KEY}B)`}>
        <Button variant="ghost" size="icon-sm" aria-label="Show the file list" onClick={onExpand}>
          <PanelLeftOpenIcon />
        </Button>
      </Hint>
      <Hint label={`Go to file (${MOD_KEY}P)`}>
        <Button variant="ghost" size="icon-sm" aria-label="Go to file" onClick={onQuickOpen}>
          <FileCode2Icon />
        </Button>
      </Hint>
    </nav>
  );
}

export function FileList({
  scanId,
  scripts,
  findings,
  currentScriptId,
  width,
  onCollapse,
}: {
  scanId: string;
  scripts: ScriptRecord[];
  findings: Finding[];
  currentScriptId: string;
  width: number;
  onCollapse: () => void;
}) {
  // TanStack Virtual mutates its instance between renders; the React Compiler must not memoise around it.
  "use no memo";
  const [filter, setFilter] = useState("");
  const groups = groupFiles(scripts, findings, filter);
  const currentGroup = groups.find((group) => group.files.some((file) => file.script.id === currentScriptId))?.key;
  // "Other" is most of a page and rarely what one came for: it starts folded, unless the open file is in it.
  const [collapsed, setCollapsed] = useState<ReadonlySet<FileGroupKey>>(
    () => new Set(currentGroup === "other" ? [] : (["other"] as const)),
  );
  // A filter is a search: every match is shown, whatever was folded.
  const items = flattenGroups(groups, filter.trim() ? new Set() : collapsed);
  const scroller = useRef<HTMLDivElement>(null);
  const virtualizer = useVirtualizer({
    count: items.length,
    getScrollElement: () => scroller.current,
    estimateSize: (index) => (items[index]?.kind === "group" ? GROUP_HEIGHT : FILE_HEIGHT),
    overscan: 12,
  });

  const currentIndex = items.findIndex((item) => item.kind === "file" && item.file.script.id === currentScriptId);
  useEffect(() => {
    if (currentIndex >= 0) virtualizer.scrollToIndex(currentIndex, { align: "auto" });
  }, [currentIndex, virtualizer]);

  const toggle = (key: FileGroupKey) =>
    setCollapsed((current) => {
      const next = new Set(current);
      if (!next.delete(key)) next.add(key);
      return next;
    });

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
    const rows = [...(scroller.current?.querySelectorAll<HTMLElement>("[data-file-row], [data-group-row]") ?? [])];
    const at = rows.indexOf(document.activeElement as HTMLElement);
    const next = rows[at + (event.key === "ArrowDown" ? 1 : -1)];
    if (!next) return;
    event.preventDefault();
    next.focus();
  };

  const renderItem = (item: FileListItem) => {
    if (item.kind === "file") {
      return <FileRow scanId={scanId} file={item.file} current={item.file.script.id === currentScriptId} />;
    }
    return (
      <button
        type="button"
        data-group-row=""
        aria-expanded={!item.collapsed}
        // While filtering, groups cannot be folded: every match stays in sight.
        disabled={filter.trim() !== ""}
        onClick={() => toggle(item.key)}
        className="flex h-7 w-full items-center gap-1 px-2.5 text-xs font-medium text-muted-foreground outline-offset-[-2px] hover:text-foreground"
      >
        <ChevronRightIcon className={cn("size-3.5 transition-transform", !item.collapsed && "rotate-90")} />
        {FILE_GROUP_LABEL[item.key]}
        <span className="text-subtle-foreground tabular-nums">{item.count}</span>
      </button>
    );
  };

  return (
    <nav aria-label="Scripts" className="flex shrink-0 flex-col border-r bg-sidebar" style={{ width }}>
      <div className="flex items-center gap-1 p-2 pb-1.5">
        <SearchInput
          aria-label="Filter scripts"
          placeholder="Filter scripts"
          value={filter}
          onValueChange={setFilter}
          className="min-w-0 flex-1"
          inputClassName="h-7 text-[13px]"
        />
        <Hint label={`Hide the file list (${MOD_KEY}B)`}>
          <Button variant="ghost" size="icon-sm" aria-label="Hide the file list" onClick={onCollapse}>
            <PanelLeftCloseIcon />
          </Button>
        </Hint>
      </div>
      {/* biome-ignore lint/a11y/noStaticElementInteractions: arrow keys move between the links and group buttons inside */}
      <div ref={scroller} onKeyDown={onKeyDown} className="min-h-0 flex-1 overflow-y-auto pb-2">
        {items.length === 0 ? (
          <p className="px-3 py-6 text-center text-[13px] text-muted-foreground">
            No script matches “{filter.trim()}”.
          </p>
        ) : (
          <div className="relative w-full" style={{ height: virtualizer.getTotalSize() }}>
            {virtualizer.getVirtualItems().map((virtual) => {
              const item = items[virtual.index];
              if (!item) return null;
              return (
                <div
                  key={item.kind === "group" ? `group-${item.key}` : item.file.script.id}
                  className="absolute inset-x-0 top-0 flex items-center"
                  style={{ height: virtual.size, transform: `translateY(${virtual.start}px)` }}
                >
                  <div className="w-full min-w-0">{renderItem(item)}</div>
                </div>
              );
            })}
          </div>
        )}
      </div>
    </nav>
  );
}
