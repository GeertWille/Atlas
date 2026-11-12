import type { AdSlot, ScanMetrics } from "@/bindings";
import { createTimeScale, type TimeScale } from "@/shared/lib/time-scale";

export const SLOT_PHASES = ["defined", "requested", "response", "rendered", "viewable"] as const;
export type SlotPhase = (typeof SLOT_PHASES)[number];

export const SLOT_PHASE_LABEL: Record<SlotPhase, string> = {
  defined: "Defined",
  requested: "Ad requested",
  response: "Ad response",
  rendered: "Rendered",
  viewable: "Viewable",
};

export type SlotMark = { phase: SlotPhase; atMs: number; percent: number };

export type SlotFlag = {
  key: "earlyRequest" | "noPrebidTargeting" | "belowFold";
  label: string;
  description: string;
  warning: boolean;
};

export type SlotRow = {
  slot: AdSlot;
  marks: SlotMark[];
  /** Solid line from the first to the last recorded phase. */
  span: { startPercent: number; widthPercent: number } | null;
  /** The lifecycle stopped early: draw a faint continuation to the end of the axis. */
  unfinishedFromPercent: number | null;
  flags: SlotFlag[];
};

export type PageMarker = {
  label: string;
  atMs: number;
  percent: number;
  /** Which side of its line the label is written on, and on which of the two label rows. */
  labelSide: "before" | "after";
  labelRow: 0 | 1;
};

export type SlotTimeline = { scale: TimeScale; markers: PageMarker[]; rows: SlotRow[] };

const PHASE_TIME: Record<SlotPhase, (slot: AdSlot) => number | null> = {
  defined: (slot) => slot.definedAtMs,
  requested: (slot) => slot.requestedAtMs,
  response: (slot) => slot.responseAtMs,
  rendered: (slot) => slot.renderedAtMs,
  viewable: (slot) => slot.viewableAtMs,
};

const AXIS_HEADROOM = 1.05;

export function slotFlags(slot: AdSlot): SlotFlag[] {
  const flags: SlotFlag[] = [];
  if (slot.requestedBeforeAuctionEnd) {
    flags.push({
      key: "earlyRequest",
      label: "Requested before auction end",
      description: "The ad request left while Prebid bidders were still answering.",
      warning: true,
    });
  }
  if (slot.hadPrebidTargeting === false) {
    flags.push({
      key: "noPrebidTargeting",
      label: "No Prebid targeting",
      description: "The request carried no hb_* key-values, so header bids could not compete.",
      warning: true,
    });
  }
  if (slot.inInitialViewport === false) {
    flags.push({
      key: "belowFold",
      label: "Below the fold",
      description: "The container was outside the initial viewport.",
      warning: false,
    });
  }
  return flags;
}

function slotMarkTimes(slot: AdSlot): { phase: SlotPhase; atMs: number }[] {
  return SLOT_PHASES.flatMap((phase) => {
    const atMs = PHASE_TIME[phase](slot);
    return atMs === null ? [] : [{ phase, atMs }];
  });
}

function pageMarkers(metrics: ScanMetrics, scale: TimeScale): PageMarker[] {
  const candidates: [string, number | null][] = [
    ["First auction", metrics.timeToFirstAuctionMs],
    ["Page load", metrics.pageLoadMs],
    ["First ad", metrics.timeToFirstAdMs],
  ];
  // The three labels can never overlap, however close the lines are and however narrow the plot:
  // the earliest is written before its line, the other two after theirs on separate rows.
  return candidates
    .flatMap(([label, atMs]) => (atMs === null ? [] : [{ label, atMs, percent: scale.toPercent(atMs) }]))
    .sort((a, b) => a.atMs - b.atMs)
    .map((marker, index) => ({
      ...marker,
      labelSide: index === 0 ? "before" : "after",
      labelRow: index === 2 ? 1 : 0,
    }));
}

/** Puts every slot's lifecycle and the page lifecycle markers on one axis (ms since navigation start). */
export function layoutSlotTimeline(slots: AdSlot[], metrics: ScanMetrics): SlotTimeline {
  const times = slots.flatMap((slot) => slotMarkTimes(slot).map((mark) => mark.atMs));
  const pageTimes = [metrics.pageLoadMs, metrics.timeToFirstAuctionMs, metrics.timeToFirstAdMs].filter(
    (value): value is number => value !== null,
  );
  const scale = createTimeScale(Math.max(1000, ...times, ...pageTimes) * AXIS_HEADROOM, 8);

  const rows = slots.map((slot): SlotRow => {
    const marks = slotMarkTimes(slot).map((mark) => ({ ...mark, percent: scale.toPercent(mark.atMs) }));
    const first = marks[0];
    const last = marks[marks.length - 1];
    const unfinished = slot.outcome === "neverRequested" || slot.outcome === "pending";
    return {
      slot,
      marks,
      span:
        first && last && last !== first
          ? { startPercent: first.percent, widthPercent: last.percent - first.percent }
          : null,
      unfinishedFromPercent: unfinished && last ? last.percent : null,
      flags: slotFlags(slot),
    };
  });

  return { scale, markers: pageMarkers(metrics, scale), rows };
}
