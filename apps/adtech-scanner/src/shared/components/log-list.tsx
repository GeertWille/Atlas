import { useVirtualizer } from "@tanstack/react-virtual";
import { type ComponentProps, type Ref, useEffect, useImperativeHandle, useRef, useState } from "react";
import type { LogEntry } from "@/bindings";
import { cn } from "@/shared/lib/cn";
import { LogEntryRow } from "./log-entry-row";

const COLLAPSED_ROW_HEIGHT = 36;

/**
 * Audit-trail entries in a scrolling viewport. Only the rows near the viewport
 * are mounted: one scan can record many hundreds of entries.
 */
export function LogList({
  entries,
  showNode,
  liveTail = false,
  focusedId,
  onOpenFinding,
  ref,
  className,
  ...props
}: Omit<ComponentProps<"div">, "children" | "ref"> & {
  entries: LogEntry[];
  showNode: boolean;
  /** The newest entry of a running scan shimmers. */
  liveTail?: boolean;
  /** Deep link target: scrolled into view, expanded and highlighted. */
  focusedId?: string;
  onOpenFinding?: (findingId: string) => void;
  /** The scrolling element, for callers that follow the newest entry. */
  ref?: Ref<HTMLDivElement | null>;
}) {
  // TanStack Virtual mutates its instance between renders; the React Compiler must not memoise around it.
  "use no memo";
  const scroller = useRef<HTMLDivElement>(null);
  useImperativeHandle<HTMLDivElement | null, HTMLDivElement | null>(ref, () => scroller.current, []);
  // Kept here, not in the rows: a row scrolled out of the window is unmounted.
  const [expandedIds, setExpandedIds] = useState<ReadonlySet<string>>(() => new Set(focusedId ? [focusedId] : []));

  const virtualizer = useVirtualizer({
    count: entries.length,
    getScrollElement: () => scroller.current,
    estimateSize: () => COLLAPSED_ROW_HEIGHT,
    getItemKey: (index) => entries[index]?.id ?? index,
    overscan: 12,
  });

  const focusedIndex = focusedId ? entries.findIndex((entry) => entry.id === focusedId) : -1;
  const focusedListed = focusedIndex >= 0;
  // biome-ignore lint/correctness/useExhaustiveDependencies: reveal once per target, not whenever its index shifts
  useEffect(() => {
    if (!focusedId || !focusedListed) return;
    setExpandedIds((current) => new Set(current).add(focusedId));
    scroller.current?.scrollIntoView({ block: "nearest" });
    virtualizer.scrollToIndex(focusedIndex, { align: "center" });
  }, [focusedId, focusedListed]);

  const toggle = (id: string) =>
    setExpandedIds((current) => {
      const next = new Set(current);
      if (!next.delete(id)) next.add(id);
      return next;
    });

  const rows = virtualizer.getVirtualItems();
  return (
    <div
      ref={scroller}
      // Scrollable regions must be keyboard reachable.
      // biome-ignore lint/a11y/noNoninteractiveTabindex: scroll container
      tabIndex={0}
      className={cn("overflow-y-auto", className)}
      {...props}
    >
      <div className="relative w-full" style={{ height: virtualizer.getTotalSize() }}>
        <div className="absolute inset-x-0 top-0" style={{ transform: `translateY(${rows[0]?.start ?? 0}px)` }}>
          {rows.map((row) => {
            const entry = entries[row.index];
            if (!entry) return null;
            return (
              <div key={entry.id} ref={virtualizer.measureElement} data-index={row.index}>
                <LogEntryRow
                  entry={entry}
                  showNode={showNode}
                  live={liveTail && row.index === entries.length - 1}
                  highlighted={entry.id === focusedId}
                  expanded={expandedIds.has(entry.id)}
                  onToggle={() => toggle(entry.id)}
                  onOpenFinding={onOpenFinding}
                />
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}
