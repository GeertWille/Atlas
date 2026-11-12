import { CalendarClockIcon, XIcon } from "lucide-react";
import { Slider as SliderPrimitive } from "radix-ui";
import { type PointerEvent, useRef, useState } from "react";
import { useElementSize } from "@/shared/hooks/use-element-size";
import { cn } from "@/shared/lib/cn";
import { formatDuration, pluralize } from "@/shared/lib/format";
import { Button } from "@/shared/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import { Hint } from "@/shared/ui/tooltip";
import type { IndexedRow } from "../query/match";
import { axisTicks, binRequests, type LifecycleMarks, type StripAxis, type TimeWindow } from "./strip-layout";

const BIN_PX = 4;
const DRAG_THRESHOLD_PX = 3;
/** How much is shown around a single moment picked from the events menu. */
const EVENT_MARGIN_MS = 1000;

type StripEvent = { key: string; label: string; fromMs: number; toMs: number };

function eventsOf(marks: LifecycleMarks, axis: StripAxis): StripEvent[] {
  const around = (atMs: number) => ({
    fromMs: Math.max(0, atMs - EVENT_MARGIN_MS),
    toMs: Math.min(axis.endMs, atMs + EVENT_MARGIN_MS),
  });
  return [
    ...axis.pages.map((page, index) => ({
      key: `page-${page.index}`,
      label: `Page ${page.label}`,
      fromMs: page.startMs,
      toMs: axis.pages[index + 1]?.startMs ?? axis.endMs,
    })),
    ...(marks.consentAtMs === null
      ? []
      : [
          {
            key: "consent",
            label: `Consent known at ${formatDuration(marks.consentAtMs)}`,
            ...around(marks.consentAtMs),
          },
        ]),
    ...marks.auctions.map((auction) => ({
      key: `auction-${auction.id}`,
      label: `Auction ${auction.number}: ${formatDuration(auction.startMs)} to ${formatDuration(auction.endMs)}${auction.timedOutBids > 0 ? `, ${pluralize(auction.timedOutBids, "bid")} timed out` : ""}`,
      fromMs: Math.max(0, auction.startMs - 200),
      toMs: Math.min(axis.endMs, Math.max(auction.endMs, auction.timeoutAtMs ?? 0) + 400),
    })),
    ...marks.renders.map((render, index) => ({
      key: `render-${index}`,
      label: `${render.slot} rendered at ${formatDuration(render.atMs)}`,
      ...around(render.atMs),
    })),
    ...marks.loads.map((load) => ({
      key: `load-${load.page}`,
      label: `Page ${load.page + 1} loaded at ${formatDuration(load.atMs)}`,
      ...around(load.atMs),
    })),
  ].sort((a, b) => a.fromMs - b.fromMs);
}

/**
 * The whole visit, or one page view, on a time axis: requests per moment above, what the ad stack
 * did below. Dragging selects a window, which filters the list and rescales its waterfall. The
 * slider under it and the events menu do the same from the keyboard.
 */
export function OverviewStrip({
  rows,
  axis,
  marks,
  window,
  onWindowChange,
  selection,
  compact,
}: {
  /** The requests the filter lets through, before the window. */
  rows: IndexedRow[];
  axis: StripAxis;
  marks: LifecycleMarks;
  window: TimeWindow | null;
  onWindowChange: (window: TimeWindow | null) => void;
  /** The selected request on the axis, so its place stays visible when the waterfall column is hidden. */
  selection: { startMs: number; endMs: number | null } | null;
  compact: boolean;
}) {
  const [measure, size] = useElementSize();
  const track = useRef<HTMLDivElement | null>(null);
  const drag = useRef<{ startX: number; moved: boolean } | null>(null);
  const [dragged, setDragged] = useState<TimeWindow | null>(null);
  const width = size?.width ?? 0;
  const bins = binRequests(rows, axis, Math.max(1, Math.floor(width / BIN_PX)));
  const shown = dragged ?? window;
  const percent = (ms: number) => `${Math.min(100, Math.max(0, (ms / axis.endMs) * 100))}%`;
  const events = eventsOf(marks, axis);

  const msAt = (clientX: number) => {
    const rect = track.current?.getBoundingClientRect();
    if (!rect || rect.width === 0) return 0;
    return Math.min(axis.endMs, Math.max(0, ((clientX - rect.left) / rect.width) * axis.endMs));
  };

  const onPointerDown = (event: PointerEvent<HTMLDivElement>) => {
    if (event.button !== 0) return;
    event.currentTarget.setPointerCapture(event.pointerId);
    drag.current = { startX: event.clientX, moved: false };
  };
  const onPointerMove = (event: PointerEvent<HTMLDivElement>) => {
    const state = drag.current;
    if (!state) return;
    if (Math.abs(event.clientX - state.startX) < DRAG_THRESHOLD_PX && !state.moved) return;
    state.moved = true;
    const a = msAt(state.startX);
    const b = msAt(event.clientX);
    setDragged({ fromMs: Math.min(a, b), toMs: Math.max(a, b) });
  };
  const onPointerUp = (event: PointerEvent<HTMLDivElement>) => {
    const state = drag.current;
    drag.current = null;
    if (!state) return;
    if (state.moved && dragged) {
      onWindowChange(dragged);
    } else {
      // A click on an auction selects its window; a click anywhere else shows everything again.
      const at = msAt(event.clientX);
      const auction = marks.auctions.find(
        (candidate) => at >= candidate.startMs && at <= Math.max(candidate.endMs, candidate.timeoutAtMs ?? 0),
      );
      const inAdsLane = (event.target as HTMLElement).closest("[data-lane='ads']") !== null;
      onWindowChange(
        auction && inAdsLane
          ? {
              fromMs: Math.max(0, auction.startMs - 200),
              toMs: Math.min(axis.endMs, Math.max(auction.endMs, auction.timeoutAtMs ?? 0) + 400),
            }
          : null,
      );
    }
    setDragged(null);
  };

  const summary = `${pluralize(rows.length, "request")} over ${formatDuration(axis.endMs)}, ${pluralize(marks.auctions.length, "auction")}, ${pluralize(marks.renders.length, "ad render")}${shown ? `. Window from ${formatDuration(shown.fromMs)} to ${formatDuration(shown.toMs)}` : ""}`;

  return (
    <section aria-label="Overview" className="flex shrink-0 items-stretch gap-2 border-b bg-sidebar px-2 py-1.5">
      <div className="flex min-w-0 flex-1 flex-col gap-1">
        <div
          ref={(element) => {
            track.current = element;
            measure(element);
          }}
          role="img"
          aria-label={summary}
          onPointerDown={onPointerDown}
          onPointerMove={onPointerMove}
          onPointerUp={onPointerUp}
          className={cn(
            "relative cursor-crosshair touch-none overflow-hidden rounded-md border bg-background select-none",
            compact ? "h-10" : "h-16",
          )}
        >
          {/* Network lane: request starts per moment, failures marked beneath. */}
          <div className={cn("absolute inset-x-0 top-0 flex items-end", compact ? "h-[22px]" : "h-[34px]")}>
            {bins.counts.map((count, index) => (
              // biome-ignore lint/suspicious/noArrayIndexKey: bins are positions on the axis
              <span key={index} className="relative h-full min-w-0 flex-1">
                {count > 0 ? (
                  <span
                    className="absolute inset-x-px bottom-[3px] rounded-t-[1px] bg-gray"
                    style={{ height: `${Math.max(8, (count / bins.peak) * 82)}%` }}
                  />
                ) : null}
                {bins.failed[index] ? <span className="absolute inset-x-0 bottom-0 h-[3px] bg-red" /> : null}
              </span>
            ))}
          </div>
          {/* Ad lifecycle lane: each kind of event has its own shape. */}
          <div
            data-lane="ads"
            className={cn(
              "absolute inset-x-0 border-t border-border/70",
              compact ? "top-[22px] h-[17px]" : "top-[34px] h-[18px]",
            )}
          >
            {marks.auctions.map((auction) => (
              <span key={auction.id}>
                <span
                  className="absolute top-[4px] h-[9px] rounded-[2px] bg-violet"
                  style={{
                    left: percent(auction.startMs),
                    width: `max(3px, calc(${percent(auction.endMs)} - ${percent(auction.startMs)}))`,
                  }}
                />
                {auction.timeoutAtMs !== null ? (
                  <span
                    className="absolute top-[1px] h-[15px] w-0.5 bg-amber"
                    style={{ left: percent(auction.timeoutAtMs) }}
                  />
                ) : null}
              </span>
            ))}
            {marks.renders.map((render, index) => (
              <span
                // biome-ignore lint/suspicious/noArrayIndexKey: renders are positions on the axis
                key={index}
                className="absolute top-[5px] size-0 -translate-x-1/2 border-x-[4px] border-t-[7px] border-x-transparent border-t-green"
                style={{ left: percent(render.atMs) }}
              />
            ))}
            {marks.consentAtMs !== null ? (
              <span
                className="absolute top-[5px] size-[7px] -translate-x-1/2 rotate-45 bg-blue"
                style={{ left: percent(marks.consentAtMs) }}
              />
            ) : null}
            {marks.loads.map((load) => (
              <span
                key={load.page}
                className="absolute inset-y-0 w-px border-l border-dashed border-foreground/50"
                style={{ left: percent(load.atMs) }}
              />
            ))}
          </div>
          {compact ? null : (
            <div className="absolute inset-x-0 bottom-0 h-3 border-t border-border/70">
              {axisTicks(axis.endMs, Math.max(2, Math.floor(width / 110))).map((tick) => (
                <span
                  key={tick}
                  className="absolute top-0 pl-1 text-[9px] leading-3 text-subtle-foreground tabular-nums"
                  style={{ left: percent(tick) }}
                >
                  {tick === 0 ? "0" : formatDuration(tick)}
                </span>
              ))}
            </div>
          )}
          {axis.pages.map((page) => (
            <span
              key={page.index}
              className="absolute inset-y-0 border-l border-foreground/40"
              style={{ left: percent(page.startMs) }}
            >
              <span className="absolute top-0 left-1 max-w-28 truncate rounded-sm bg-background/85 px-0.5 font-mono text-[9px] leading-3 text-muted-foreground">
                {page.label}
              </span>
            </span>
          ))}
          {selection ? (
            <span
              className="absolute bottom-0 h-0.5 bg-primary"
              style={{
                left: percent(selection.startMs),
                width: `max(3px, calc(${percent(selection.endMs ?? axis.endMs)} - ${percent(selection.startMs)}))`,
              }}
            />
          ) : null}
          {shown ? (
            <>
              <span
                className="absolute inset-y-0 left-0 bg-foreground/[0.14]"
                style={{ width: percent(shown.fromMs) }}
              />
              <span className="absolute inset-y-0 right-0 bg-foreground/[0.14]" style={{ left: percent(shown.toMs) }} />
              <span
                className="absolute inset-y-0 border-x-2 border-primary"
                style={{
                  left: percent(shown.fromMs),
                  width: `calc(${percent(shown.toMs)} - ${percent(shown.fromMs)})`,
                }}
              />
            </>
          ) : null}
        </div>
        <SliderPrimitive.Root
          min={0}
          max={Math.round(axis.endMs)}
          step={Math.max(1, Math.round(axis.endMs / 400))}
          minStepsBetweenThumbs={1}
          value={[Math.round(shown?.fromMs ?? 0), Math.round(shown?.toMs ?? axis.endMs)]}
          onValueChange={([fromMs = 0, toMs = axis.endMs]) => setDragged({ fromMs, toMs })}
          onValueCommit={([fromMs = 0, toMs = axis.endMs]) => {
            setDragged(null);
            onWindowChange(fromMs <= 0 && toMs >= Math.round(axis.endMs) ? null : { fromMs, toMs });
          }}
          className="relative flex h-2 w-full touch-none items-center select-none"
        >
          <SliderPrimitive.Track className="relative h-0.5 w-full grow rounded-full bg-border">
            <SliderPrimitive.Range className="absolute h-full rounded-full bg-primary/60" />
          </SliderPrimitive.Track>
          {(["Window start", "Window end"] as const).map((name) => (
            <SliderPrimitive.Thumb
              key={name}
              aria-label={name}
              aria-valuetext={formatDuration(
                name === "Window start" ? (shown?.fromMs ?? 0) : (shown?.toMs ?? axis.endMs),
              )}
              className="block h-2 w-1.5 rounded-[2px] bg-foreground/60 hover:bg-foreground"
            />
          ))}
        </SliderPrimitive.Root>
      </div>
      <div className="flex shrink-0 flex-col justify-between gap-1">
        <DropdownMenu>
          <Hint label="Events on the timeline">
            <DropdownMenuTrigger asChild>
              <Button variant="ghost" size="icon-sm" aria-label="Events on the timeline">
                <CalendarClockIcon />
              </Button>
            </DropdownMenuTrigger>
          </Hint>
          <DropdownMenuContent align="end" className="max-h-80 max-w-96 overflow-y-auto">
            <DropdownMenuGroup>
              {events.length === 0 ? (
                <DropdownMenuItem disabled>No auctions, renders or page loads were recorded</DropdownMenuItem>
              ) : (
                events.map((event) => (
                  <DropdownMenuItem
                    key={event.key}
                    onSelect={() => onWindowChange({ fromMs: event.fromMs, toMs: event.toMs })}
                  >
                    <span className="truncate">{event.label}</span>
                  </DropdownMenuItem>
                ))
              )}
            </DropdownMenuGroup>
          </DropdownMenuContent>
        </DropdownMenu>
        {window ? (
          <Hint label="Show the whole timeline (Esc)">
            <Button
              variant="ghost"
              size="icon-sm"
              aria-label="Clear the time window"
              onClick={() => onWindowChange(null)}
            >
              <XIcon />
            </Button>
          </Hint>
        ) : null}
      </div>
    </section>
  );
}
