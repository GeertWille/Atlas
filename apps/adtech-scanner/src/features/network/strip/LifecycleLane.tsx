import { cn } from "@/shared/lib/cn";
import type { LifecycleMarks } from "./strip-layout";

/**
 * What the ad stack did, drawn on a time axis: each kind of event has its own shape as well as its own
 * colour. The marks centre themselves in whatever height the lane is given.
 */
export function LifecycleLane({
  marks,
  percent,
  className,
}: {
  marks: LifecycleMarks;
  /** A time on the axis as a CSS length from the lane's left edge. */
  percent: (ms: number) => string;
  className?: string;
}) {
  return (
    <div data-lane="ads" className={cn("absolute inset-x-0", className)}>
      {marks.loads.map((load) => (
        <span
          key={load.page}
          className="absolute inset-y-0 w-px border-l border-dashed border-foreground/50"
          style={{ left: percent(load.atMs) }}
        />
      ))}
      {marks.auctions.map((auction) => (
        <span key={auction.id}>
          <span
            className="absolute top-1/2 h-3 -translate-y-1/2 rounded-[3px] bg-violet"
            style={{
              left: percent(auction.startMs),
              width: `max(4px, calc(${percent(auction.endMs)} - ${percent(auction.startMs)}))`,
            }}
          />
          {auction.timeoutAtMs !== null ? (
            <span className="absolute inset-y-[2px] w-0.5 bg-amber" style={{ left: percent(auction.timeoutAtMs) }} />
          ) : null}
        </span>
      ))}
      {marks.renders.map((render, index) => (
        <span
          // biome-ignore lint/suspicious/noArrayIndexKey: renders are positions on the axis
          key={index}
          className="absolute top-1/2 size-0 -translate-x-1/2 -translate-y-1/2 border-x-[5px] border-t-[8px] border-x-transparent border-t-green"
          style={{ left: percent(render.atMs) }}
        />
      ))}
      {marks.consentAtMs !== null ? (
        <span
          className="absolute top-1/2 size-2 -translate-x-1/2 -translate-y-1/2 rotate-45 bg-blue"
          style={{ left: percent(marks.consentAtMs) }}
        />
      ) : null}
    </div>
  );
}

const LEGEND: [label: string, shape: string][] = [
  ["Requests", "h-2.5 w-1 rounded-t-[1px] bg-gray"],
  ["Failed", "h-[3px] w-2.5 bg-red"],
  ["Auction", "h-2 w-4 rounded-[2px] bg-violet"],
  ["Timeout", "h-3 w-0.5 bg-amber"],
  ["Ad rendered", "size-0 border-x-[4px] border-t-[7px] border-x-transparent border-t-green"],
  ["Consent known", "size-[7px] rotate-45 bg-blue"],
  ["Page loaded", "h-3 w-px border-l border-dashed border-foreground/60"],
];

/** The shapes of the strip, named. */
export function StripLegend({ className }: { className?: string }) {
  return (
    <ul
      aria-label="Legend"
      className={cn("flex flex-wrap items-center gap-x-3.5 gap-y-0.5 text-[10px] text-muted-foreground", className)}
    >
      {LEGEND.map(([label, shape]) => (
        <li key={label} className="flex items-center gap-1.5 whitespace-nowrap">
          <span aria-hidden="true" className="flex w-4 items-center justify-center">
            <span className={shape} />
          </span>
          {label}
        </li>
      ))}
    </ul>
  );
}
