import type { NetworkRow, RowPhases } from "@/bindings";
import { formatDuration } from "@/shared/lib/format";

/** The stretch of time the waterfall column shows, and how wide the column is. */
export type WaterfallAxis = { startMs: number; endMs: number; widthPx: number };

export type Phase = "queue" | "connect" | "wait" | "receive";
export type BarSegment = { phase: Phase; fromPx: number; toPx: number };

export type BarGeometry = {
  /** Left edge in the column; negative when the request began before the window. */
  leftPx: number;
  widthPx: number;
  /** The phases, left to right, in pixels from the bar's own left edge. Empty when the browser gave no timing. */
  segments: BarSegment[];
  /** The request never finished: the bar runs to the end of the window. */
  openEnded: boolean;
  /** Where the part after the auction closed begins, in pixels from the bar's left edge. */
  lateFromPx: number | null;
};

/** A bar is never thinner than this, and a phase that took any time at all gets at least one pixel. */
export const MIN_BAR_PX = 3;
const MIN_PHASE_PX = 1;
const PHASES: readonly Phase[] = ["queue", "connect", "wait", "receive"];

function segmentsOf(phases: RowPhases, widthPx: number): BarSegment[] {
  const present = PHASES.filter((phase) => phases[phase] > 0);
  const total = present.reduce((sum, phase) => sum + phases[phase], 0);
  if (total <= 0) return [];
  const widths = new Map(present.map((phase) => [phase, (phases[phase] / total) * widthPx]));
  // Short phases are brought up to a pixel; the longest one gives up what they take.
  let borrowed = 0;
  for (const phase of present) {
    const width = widths.get(phase) ?? 0;
    if (width >= MIN_PHASE_PX) continue;
    borrowed += MIN_PHASE_PX - width;
    widths.set(phase, MIN_PHASE_PX);
  }
  const longest = present.reduce((a, b) => ((widths.get(b) ?? 0) > (widths.get(a) ?? 0) ? b : a));
  widths.set(longest, Math.max(MIN_PHASE_PX, (widths.get(longest) ?? 0) - borrowed));
  let fromPx = 0;
  return present.map((phase) => {
    const toPx = fromPx + (widths.get(phase) ?? 0);
    const segment = { phase, fromPx, toPx };
    fromPx = toPx;
    return segment;
  });
}

/**
 * Places one request on the waterfall. `startMs` and `lateFromMs` are on the axis' own clock.
 * `null` when the request lies wholly outside the window.
 */
export function layoutBar(
  request: { startMs: number; durationMs: number | null; phases: RowPhases | null; lateFromMs?: number | null },
  axis: WaterfallAxis,
): BarGeometry | null {
  const span = axis.endMs - axis.startMs;
  if (span <= 0 || axis.widthPx <= 0) return null;
  const scale = axis.widthPx / span;
  const openEnded = request.durationMs === null;
  const endMs = openEnded ? axis.endMs : request.startMs + (request.durationMs ?? 0);
  if (request.startMs > axis.endMs || endMs < axis.startMs) return null;
  const leftPx = (request.startMs - axis.startMs) * scale;
  const widthPx = Math.max(MIN_BAR_PX, (endMs - request.startMs) * scale);
  const lateMs = request.lateFromMs ?? null;
  const isLate = lateMs !== null && !openEnded && endMs > lateMs;
  return {
    leftPx,
    widthPx,
    segments: request.phases && !openEnded ? segmentsOf(request.phases, widthPx) : [],
    openEnded,
    lateFromPx: isLate ? Math.min(widthPx - MIN_BAR_PX, Math.max(0, (lateMs - request.startMs) * scale)) : null,
  };
}

const PHASE_COLOR: Record<Exclude<Phase, "queue">, string> = {
  connect: "var(--net-connect)",
  wait: "var(--net-wait)",
  receive: "var(--net-receive)",
};

const px = (value: number) => `${Math.round(value * 100) / 100}px`;

/**
 * The phases as CSS backgrounds for a single element: queueing is a hairline, the rest are bands of
 * one hue from light to dark, and the part of a bid response after its auction closed is hatched.
 */
export function barBackground(geometry: BarGeometry): string | undefined {
  const layers: string[] = [];
  if (geometry.lateFromPx !== null) {
    layers.push(
      `repeating-linear-gradient(135deg, var(--net-late) 0 2px, transparent 2px 5px) ${px(geometry.lateFromPx)} 0 / ${px(geometry.widthPx - geometry.lateFromPx)} 100% no-repeat`,
    );
  }
  if (geometry.segments.length > 0) {
    const stops = geometry.segments.map((segment) => {
      const color = segment.phase === "queue" ? "transparent" : PHASE_COLOR[segment.phase];
      return `${color} ${px(segment.fromPx)} ${px(segment.toPx)}`;
    });
    layers.push(`linear-gradient(to right, ${stops.join(", ")}) 0 0 / 100% 100% no-repeat`);
    const queue = geometry.segments.find((segment) => segment.phase === "queue");
    if (queue) {
      layers.push(`linear-gradient(var(--net-queue), var(--net-queue)) 0 50% / ${px(queue.toPx)} 2px no-repeat`);
    }
  }
  return layers.length > 0 ? layers.join(", ") : undefined;
}

/** The bar in words, for screen readers and the tooltip. */
export function describeBar(row: Pick<NetworkRow, "startedAtMs" | "durationMs" | "phases">): string {
  const started = `started ${formatDuration(row.startedAtMs)}`;
  if (row.durationMs === null) return `${started}, no response before the visit ended`;
  const took = `${started}, took ${formatDuration(row.durationMs)}`;
  if (!row.phases) return took;
  const parts = [
    row.phases.connect >= 1 ? `connecting ${formatDuration(row.phases.connect)}` : null,
    `waiting ${formatDuration(row.phases.wait)}`,
    `receiving ${formatDuration(row.phases.receive)}`,
  ].filter((part) => part !== null);
  return `${took}: ${parts.join(", ")}`;
}
