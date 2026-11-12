import { defaultRangeExtractor, useVirtualizer } from "@tanstack/react-virtual";
import { ArrowDownIcon, ArrowUpIcon, ChevronRightIcon } from "lucide-react";
import { type KeyboardEvent, memo, useEffect, useRef, useState } from "react";
import { cn } from "@/shared/lib/cn";
import { formatBytes, formatDuration, formatInteger, pluralize } from "@/shared/lib/format";
import { hasModifier } from "@/shared/lib/platform";
import type { IndexedRow } from "../query/match";
import { CellContent } from "./cells";
import { type ColumnDef, type ColumnKey, gridTemplate, type SortState } from "./columns";
import type { GridGroup, GridItem } from "./grouping";
import { barBackground, describeBar, layoutBar } from "./waterfall-bar";

/** Row heights of the two densities. */
export const ROW_HEIGHT = { compact: 28, comfortable: 36 } as const;
const HEADER_HEIGHT = 28;
const OVERSCAN = 8;

/** What the waterfall column needs: the window on the axis, and where each row and auction falls on it. */
export type WaterfallScale = {
  startMs: number;
  endMs: number;
  /** A row's start on the axis. */
  startOf: (indexed: IndexedRow) => number;
  /** For a bid call: when its auction closed, on the axis. */
  closeOf: (indexed: IndexedRow) => number | null;
  /** Auction windows and their timeouts on the axis, drawn behind the rows. */
  bands: { id: string; startMs: number; endMs: number; timeoutAtMs: number | null }[];
};

export function itemKey(item: GridItem): string {
  return item.kind === "group" ? `group:${item.group.key}` : item.indexed.row.id;
}

const rowDomId = (key: string) => `net-item-${key.replaceAll(/[^\w-]/g, "_")}`;

function GroupRow({ group, active, onToggle }: { group: GridGroup; active: boolean; onToggle: () => void }) {
  const span =
    group.endMs === null
      ? `from ${formatDuration(group.startMs)}`
      : `${formatDuration(group.startMs)} to ${formatDuration(group.endMs)}`;
  return (
    // biome-ignore lint/a11y/useKeyWithClickEvents: the grid handles keys for its active row
    <div
      role="row"
      aria-expanded={!group.collapsed}
      aria-selected={active}
      onClick={onToggle}
      className={cn(
        "flex h-full items-center gap-2 border-b bg-muted/60 px-2 text-xs",
        active &&
          "group-focus-visible/grid:outline group-focus-visible/grid:outline-2 group-focus-visible/grid:-outline-offset-2 group-focus-visible/grid:outline-ring",
      )}
    >
      <span role="gridcell" className="flex min-w-0 items-center gap-2">
        <ChevronRightIcon
          aria-hidden="true"
          className={cn("size-3.5 shrink-0 text-subtle-foreground", !group.collapsed && "rotate-90")}
        />
        <span className="truncate font-semibold whitespace-pre">{group.label}</span>
        {group.note ? <span className="truncate text-muted-foreground">({group.note})</span> : null}
        <span className="shrink-0 text-muted-foreground tabular-nums">
          {pluralize(group.count, "request")} · {formatBytes(group.transferredBytes)}
          {group.failed > 0 ? ` · ${formatInteger(group.failed)} failed` : ""} · {span}
        </span>
      </span>
    </div>
  );
}

const DataRow = memo(function DataRow({
  indexed,
  columns,
  template,
  active,
  selected,
  rowIndex,
  scale,
  waterfallWidth,
  onSelect,
}: {
  indexed: IndexedRow;
  columns: ColumnDef[];
  template: string;
  active: boolean;
  selected: boolean;
  rowIndex: number;
  scale: WaterfallScale;
  waterfallWidth: number;
  onSelect: (id: string) => void;
}) {
  const { row } = indexed;
  const failed = row.outcome === "failed" || row.outcome === "httpError";
  const bar = layoutBar(
    {
      startMs: scale.startOf(indexed),
      durationMs: row.durationMs,
      phases: row.phases,
      lateFromMs: scale.closeOf(indexed),
    },
    { startMs: scale.startMs, endMs: scale.endMs, widthPx: waterfallWidth },
  );
  return (
    // biome-ignore lint/a11y/useKeyWithClickEvents: the grid handles keys for its active row
    <div
      role="row"
      aria-rowindex={rowIndex}
      aria-selected={selected}
      title={row.url}
      onClick={() => onSelect(row.id)}
      style={{ gridTemplateColumns: template }}
      className={cn(
        "grid h-full items-center gap-x-3 border-b border-border/60 px-2 text-xs hover:bg-accent/60",
        failed && "text-red-fg",
        selected && "bg-primary-soft hover:bg-primary-soft",
        active &&
          "group-focus-visible/grid:outline group-focus-visible/grid:outline-2 group-focus-visible/grid:-outline-offset-2 group-focus-visible/grid:outline-ring",
      )}
    >
      {columns.map((column) => (
        <span
          key={column.key}
          role="gridcell"
          className={cn(
            "flex min-w-0 items-center",
            column.align === "right" && "justify-end",
            column.key === "waterfall" && "relative h-full overflow-hidden",
          )}
        >
          {column.key === "waterfall" ? (
            bar ? (
              <span
                role="img"
                aria-label={describeBar(row)}
                className={cn(
                  "absolute top-1/2 h-2.5 -translate-y-1/2 rounded-[2px]",
                  failed
                    ? "bg-red"
                    : bar.openEnded
                      ? "hatch bg-gray/40"
                      : bar.segments.length === 0
                        ? "bg-gray/55"
                        : "",
                )}
                style={{
                  left: bar.leftPx,
                  width: bar.widthPx,
                  background: failed || bar.openEnded ? undefined : barBackground(bar),
                }}
              />
            ) : null
          ) : (
            <CellContent column={column.key} row={row} />
          )}
        </span>
      ))}
    </div>
  );
});

function HeaderCell({
  column,
  sort,
  onSort,
  range,
}: {
  column: ColumnDef;
  sort: SortState;
  onSort: (key: ColumnKey) => void;
  /** The stretch of time the waterfall column covers, said in its heading. */
  range: string;
}) {
  const active = sort.key === column.key;
  const sortable = column.key !== "waterfall";
  const Arrow = sort.descending ? ArrowDownIcon : ArrowUpIcon;
  return (
    <span
      role="columnheader"
      aria-sort={active ? (sort.descending ? "descending" : "ascending") : sortable ? "none" : undefined}
      data-column={column.key}
      className={cn("flex min-w-0 items-center", column.align === "right" && "justify-end")}
    >
      {sortable ? (
        <button
          type="button"
          // The grid is one tab stop; headers are reached with the pointer, and sorting is in the URL.
          tabIndex={-1}
          onClick={() => onSort(column.key)}
          className={cn(
            "flex min-w-0 items-center gap-1 rounded-sm hover:text-foreground",
            active && "text-foreground",
          )}
        >
          <span className="truncate">{column.label}</span>
          {active ? <Arrow aria-hidden="true" className="size-3 shrink-0" /> : null}
        </button>
      ) : (
        <span className="truncate">
          {column.label} <span className="font-normal tabular-nums">{range}</span>
        </span>
      )}
    </span>
  );
}

export function RequestGrid({
  label,
  items,
  columns,
  sort,
  onSort,
  activeKey,
  onActiveChange,
  selectedId,
  onOpen,
  onToggleGroup,
  onCopyUrl,
  scale,
  rowHeight,
}: {
  label: string;
  items: GridItem[];
  columns: ColumnDef[];
  sort: SortState;
  onSort: (key: ColumnKey) => void;
  /** The item the arrow keys are on: a row's id, or `group:<key>`. */
  activeKey: string | null;
  onActiveChange: (key: string) => void;
  /** The request whose detail is open. */
  selectedId: string | undefined;
  onOpen: (requestId: string) => void;
  onToggleGroup: (groupKey: string, collapsed?: boolean) => void;
  onCopyUrl: (url: string) => void;
  scale: WaterfallScale;
  rowHeight: number;
}) {
  // TanStack Virtual mutates its instance between renders; the React Compiler must not memoise around it.
  "use no memo";
  const scroller = useRef<HTMLDivElement>(null);
  const header = useRef<HTMLDivElement>(null);
  const [waterfall, setWaterfall] = useState({ left: 0, width: 0 });
  const template = gridTemplate(columns);
  const activeIndex = activeKey === null ? -1 : items.findIndex((item) => itemKey(item) === activeKey);

  const virtualizer = useVirtualizer({
    count: items.length,
    getScrollElement: () => scroller.current,
    estimateSize: () => rowHeight,
    overscan: OVERSCAN,
    scrollPaddingStart: HEADER_HEIGHT,
    // The active row stays mounted wherever the list is scrolled, so `aria-activedescendant` always resolves.
    rangeExtractor: (range) => {
      const indexes = defaultRangeExtractor(range);
      if (activeIndex >= 0 && !indexes.includes(activeIndex)) indexes.push(activeIndex);
      return indexes.sort((a, b) => a - b);
    },
  });

  // The waterfall column's place, for the bars and for the auction bands behind them.
  // biome-ignore lint/correctness/useExhaustiveDependencies: the column moves when the template does
  useEffect(() => {
    const cell = header.current?.querySelector<HTMLElement>('[data-column="waterfall"]');
    if (!cell) {
      setWaterfall({ left: 0, width: 0 });
      return;
    }
    const measure = () =>
      setWaterfall((current) =>
        current.left === cell.offsetLeft && current.width === cell.offsetWidth
          ? current
          : { left: cell.offsetLeft, width: cell.offsetWidth },
      );
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(cell);
    return () => observer.disconnect();
  }, [template]);

  // biome-ignore lint/correctness/useExhaustiveDependencies: scroll when the active item changes, not when the list does
  useEffect(() => {
    if (activeIndex >= 0) virtualizer.scrollToIndex(activeIndex, { align: "auto" });
  }, [activeKey]);

  const move = (index: number) => {
    const item = items[Math.min(items.length - 1, Math.max(0, index))];
    if (item) onActiveChange(itemKey(item));
  };

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.target !== event.currentTarget) return;
    const item = items[activeIndex];
    const page = Math.max(1, Math.floor((scroller.current?.clientHeight ?? 0) / rowHeight) - 2);
    if (
      hasModifier(event) &&
      event.key.toLowerCase() === "c" &&
      item?.kind === "row" &&
      !window.getSelection()?.toString()
    ) {
      event.preventDefault();
      onCopyUrl(item.indexed.row.url);
      return;
    }
    if (event.metaKey || event.ctrlKey || event.altKey) return;
    switch (event.key) {
      case "ArrowDown":
        move(activeIndex + 1);
        break;
      case "ArrowUp":
        move(Math.max(0, activeIndex - 1));
        break;
      case "Home":
        move(0);
        break;
      case "End":
        move(items.length - 1);
        break;
      case "PageDown":
        move(activeIndex + page);
        break;
      case "PageUp":
        move(activeIndex - page);
        break;
      case "Enter":
      case " ":
        if (item?.kind === "row") onOpen(item.indexed.row.id);
        else if (item) onToggleGroup(item.group.key);
        break;
      case "ArrowLeft":
        if (item?.kind === "group") onToggleGroup(item.group.key, true);
        // From a row, Left goes to the heading of its group.
        else if (item)
          move(items.findLastIndex((candidate, index) => index < activeIndex && candidate.kind === "group"));
        break;
      case "ArrowRight":
        if (item?.kind === "group") onToggleGroup(item.group.key, false);
        break;
      default:
        return;
    }
    event.preventDefault();
  };

  const virtualItems = virtualizer.getVirtualItems();
  const firstVisible = Math.floor((virtualizer.scrollOffset ?? 0) / rowHeight);
  // The heading of the group the top row belongs to stays in sight while its rows scroll.
  const pinnedIndex = items.findLastIndex((item, index) => index < firstVisible && item.kind === "group");
  const pinned = pinnedIndex >= 0 ? items[pinnedIndex] : undefined;
  const span = scale.endMs - scale.startMs;
  const toPx = (ms: number) => ((ms - scale.startMs) / span) * waterfall.width;
  const dataRows = items.reduce((count, item) => count + (item.kind === "row" ? 1 : 0), 0);

  return (
    <div
      ref={scroller}
      role="grid"
      aria-label={label}
      aria-rowcount={dataRows + 1}
      aria-colcount={columns.length}
      aria-activedescendant={activeIndex >= 0 && activeKey ? rowDomId(activeKey) : undefined}
      tabIndex={0}
      onKeyDown={onKeyDown}
      className="group/grid relative h-full overflow-auto outline-none"
    >
      <div
        ref={header}
        role="row"
        aria-rowindex={1}
        style={{ gridTemplateColumns: template, height: HEADER_HEIGHT }}
        className="sticky top-0 z-20 grid min-w-max items-center gap-x-3 border-b bg-background px-2 text-[11px] font-medium text-muted-foreground"
      >
        {columns.map((column) => (
          <HeaderCell
            key={column.key}
            column={column}
            sort={sort}
            onSort={onSort}
            range={`${formatDuration(scale.startMs)} to ${formatDuration(scale.endMs)}`}
          />
        ))}
      </div>
      {pinned?.kind === "group" ? (
        <div aria-hidden="true" className="sticky z-10 h-0" style={{ top: HEADER_HEIGHT }}>
          <div style={{ height: rowHeight }}>
            <GroupRow group={pinned.group} active={false} onToggle={() => onToggleGroup(pinned.group.key)} />
          </div>
        </div>
      ) : null}
      <div role="rowgroup" className="relative min-w-max" style={{ height: virtualizer.getTotalSize() }}>
        {waterfall.width > 0 && span > 0
          ? scale.bands.map((band) => (
              <div
                key={band.id}
                aria-hidden="true"
                className="pointer-events-none absolute inset-y-0 overflow-hidden"
                style={{ left: waterfall.left, width: waterfall.width }}
              >
                <span
                  className="absolute inset-y-0 bg-violet/[0.07]"
                  style={{ left: toPx(band.startMs), width: Math.max(1, toPx(band.endMs) - toPx(band.startMs)) }}
                />
                {band.timeoutAtMs !== null ? (
                  <span className="absolute inset-y-0 w-px bg-amber" style={{ left: toPx(band.timeoutAtMs) }} />
                ) : null}
              </div>
            ))
          : null}
        {virtualItems.map((virtual) => {
          const item = items[virtual.index];
          if (!item) return null;
          const key = itemKey(item);
          return (
            <div
              key={key}
              id={rowDomId(key)}
              className="absolute inset-x-0 top-0"
              style={{ height: rowHeight, transform: `translateY(${virtual.start}px)` }}
            >
              {item.kind === "group" ? (
                <GroupRow
                  group={item.group}
                  active={key === activeKey}
                  onToggle={() => onToggleGroup(item.group.key)}
                />
              ) : (
                <DataRow
                  indexed={item.indexed}
                  columns={columns}
                  template={template}
                  active={key === activeKey}
                  selected={item.indexed.row.id === selectedId}
                  rowIndex={virtual.index + 2}
                  scale={scale}
                  waterfallWidth={waterfall.width}
                  onSelect={onOpen}
                />
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}
