import { type KeyboardEvent, type PointerEvent, type ReactNode, useRef } from "react";
import { cn } from "@/shared/lib/cn";

const KEY_STEP = 24;

/** Keeps `size` inside the limits, and leaves at least `minOther` for the pane that flexes. */
export function clampPaneSize(size: number, limits: { min: number; max: number; available: number; minOther: number }) {
  const upper = Math.max(limits.min, Math.min(limits.max, limits.available - limits.minOther));
  return Math.round(Math.min(Math.max(size, limits.min), upper));
}

/**
 * Two panes with a draggable divider. The `end` pane has a size in pixels and the `start` pane takes
 * the rest; the caller owns the size, so it can persist it. The divider is a focusable separator:
 * arrow keys resize, Home and End go to the limits.
 *
 * `end` may be absent, and `start` may be hidden to give `end` the whole area. The `start` pane stays
 * mounted in the same place through all of it, so what is inside keeps its focus and its scroll position.
 */
export function SplitPane({
  orientation,
  size,
  onSizeChange,
  min,
  max,
  minOther = 240,
  label,
  start,
  end,
  startHidden = false,
  className,
}: {
  /** `horizontal` puts the panes side by side; `vertical` stacks them. */
  orientation: "horizontal" | "vertical";
  /** Width (or height) of the `end` pane in pixels. */
  size: number;
  onSizeChange: (size: number) => void;
  min: number;
  max: number;
  /** The least the `start` pane keeps. */
  minOther?: number;
  /** Accessible name of the divider, e.g. "Resize request details". */
  label: string;
  start: ReactNode;
  /** `null` while there is nothing to show beside `start`. */
  end: ReactNode | null;
  /** Gives `end` the whole area. */
  startHidden?: boolean;
  className?: string;
}) {
  const container = useRef<HTMLDivElement>(null);
  const horizontal = orientation === "horizontal";

  const available = () => {
    const rect = container.current?.getBoundingClientRect();
    if (!rect) return Number.POSITIVE_INFINITY;
    return horizontal ? rect.width : rect.height;
  };
  const resize = (next: number) => {
    const clamped = clampPaneSize(next, { min, max, available: available(), minOther });
    if (clamped !== size) onSizeChange(clamped);
  };

  const onPointerDown = (event: PointerEvent<HTMLDivElement>) => {
    if (event.button !== 0) return;
    event.preventDefault();
    event.currentTarget.setPointerCapture(event.pointerId);
  };
  const onPointerMove = (event: PointerEvent<HTMLDivElement>) => {
    if (!event.currentTarget.hasPointerCapture(event.pointerId)) return;
    const rect = container.current?.getBoundingClientRect();
    if (!rect) return;
    resize(horizontal ? rect.right - event.clientX : rect.bottom - event.clientY);
  };
  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    // The end pane grows towards the start: Left or Up makes it larger.
    const grow = horizontal ? "ArrowLeft" : "ArrowUp";
    const shrink = horizontal ? "ArrowRight" : "ArrowDown";
    const next = { [grow]: size + KEY_STEP, [shrink]: size - KEY_STEP, Home: max, End: min }[event.key];
    if (next === undefined) return;
    event.preventDefault();
    resize(next);
  };

  return (
    <div ref={container} className={cn("flex min-h-0 min-w-0", horizontal ? "flex-row" : "flex-col", className)}>
      <div className={cn("min-h-0 min-w-0 flex-1", startHidden && end !== null && "hidden")}>{start}</div>
      {end === null ? null : startHidden ? (
        <div className="min-h-0 min-w-0 flex-1">{end}</div>
      ) : (
        <>
          {/* biome-ignore lint/a11y/useSemanticElements: an <hr> cannot be focused or dragged */}
          <div
            role="separator"
            tabIndex={0}
            aria-label={label}
            aria-orientation={horizontal ? "vertical" : "horizontal"}
            aria-valuenow={size}
            aria-valuemin={min}
            aria-valuemax={max}
            onPointerDown={onPointerDown}
            onPointerMove={onPointerMove}
            onKeyDown={onKeyDown}
            className={cn(
              "group/divider relative z-10 shrink-0 touch-none bg-border outline-none",
              horizontal ? "w-px cursor-col-resize" : "h-px cursor-row-resize",
            )}
          >
            {/* A wider invisible hit area, and the line that lights up on hover, focus and drag. */}
            <span
              className={cn(
                "absolute bg-transparent transition-colors group-hover/divider:bg-primary/50 group-focus-visible/divider:bg-primary group-active/divider:bg-primary",
                horizontal ? "inset-y-0 -left-[3px] w-[7px]" : "inset-x-0 -top-[3px] h-[7px]",
                "bg-clip-content",
                horizontal ? "px-[2px]" : "py-[2px]",
              )}
            />
          </div>
          <div
            className="min-h-0 min-w-0 shrink-0"
            style={
              horizontal
                ? { width: size, minWidth: min, maxWidth: `calc(100% - ${minOther}px)` }
                : { height: size, minHeight: min, maxHeight: `calc(100% - ${minOther}px)` }
            }
          >
            {end}
          </div>
        </>
      )}
    </div>
  );
}
