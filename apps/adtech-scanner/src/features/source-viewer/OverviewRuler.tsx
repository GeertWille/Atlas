import type { PointerEvent } from "react";
import type { Severity, SourceRegion } from "@/bindings";
import { useElementSize } from "@/shared/hooks/use-element-size";
import { cn } from "@/shared/lib/cn";
import type { FindingMark } from "./anchors";
import { layoutRuler, yToLine } from "./ruler-layout";

const TICK_TONE: Record<Severity, string> = {
  critical: "bg-red",
  high: "bg-orange",
  medium: "bg-amber",
  low: "bg-blue",
  info: "bg-gray",
};

export type RulerViewport = { top: number; height: number };

/**
 * The whole file on one strip beside the scrollbar. Left lane: what the AI read, and the reading
 * guide's chunks. Right lane: findings, and search matches in amber. A click goes to that place.
 * Everything on it is also reachable from the side panel and the find bar, so it is a picture only.
 */
export function OverviewRuler({
  marks,
  regions,
  hitLines,
  lineCount,
  viewport,
  onJump,
}: {
  marks: FindingMark[];
  regions: SourceRegion[];
  hitLines: number[];
  lineCount: number;
  /** The stretch of the file in view, as fractions of its height. */
  viewport: RulerViewport | null;
  onJump: (line: number) => void;
}) {
  const [measure, size] = useElementSize();
  const height = size?.height ?? 0;
  const layout = layoutRuler({ marks, regions, hitLines, lineCount, height });

  const onPointerDown = (event: PointerEvent<HTMLDivElement>) => {
    if (event.button !== 0) return;
    const y = event.clientY - event.currentTarget.getBoundingClientRect().top;
    onJump(yToLine(y, lineCount, height));
  };

  return (
    <div
      ref={measure}
      role="img"
      aria-label={`Overview of the file: ${marks.length} ${marks.length === 1 ? "finding" : "findings"}, ${regions.length} ${regions.length === 1 ? "stretch" : "stretches"} read by the AI${hitLines.length > 0 ? `, ${hitLines.length} search matches` : ""}`}
      onPointerDown={onPointerDown}
      className="relative w-3.5 shrink-0 cursor-default border-l bg-code"
    >
      {viewport ? (
        <span
          className="absolute inset-x-0 bg-foreground/[0.07]"
          style={{ top: viewport.top * height, height: Math.max(8, viewport.height * height) }}
        />
      ) : null}
      {layout.bands.map((band, index) => (
        <span
          // biome-ignore lint/suspicious/noArrayIndexKey: bands of one file never reorder
          key={index}
          className={cn("absolute left-0 w-[5px]", band.kind === "read" ? "bg-violet" : "bg-violet-line")}
          style={{ top: band.top, height: band.height, opacity: band.opacity }}
        />
      ))}
      {layout.hits.map((hit) => (
        <span key={hit.top} className="absolute right-0 h-0.5 w-[7px] bg-amber" style={{ top: hit.top }} />
      ))}
      {layout.ticks.map((tick) => (
        <span
          key={`${tick.findingId}-${tick.line}`}
          className={cn("absolute right-0 w-[7px] rounded-[1px]", TICK_TONE[tick.severity])}
          style={{ top: tick.top, height: tick.height }}
        />
      ))}
    </div>
  );
}
