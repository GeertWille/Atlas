import type { AdSlot } from "@/bindings";
import { createTimeScale, type TimeScale } from "@/shared/lib/time-scale";
import type { PageMarkers } from "../pages/page-scope";

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
  key: "earlyRequest" | "noPrebidTargeting" | "belowFold" | "refreshed";
  label: string;
  description: string;
  warning: boolean;
};

export type SlotRefresh = {
  atMs: number;
  percent: number;
  hadPrebidTargeting: boolean;
  /** The request carried the same `hb_adid` as the one before it: a bid that was already used. */
  staleBid: boolean;
};

export type SlotRow = {
  slot: AdSlot;
  marks: SlotMark[];
  /** Solid line from the first to the last recorded phase. */
  span: { startPercent: number; widthPercent: number } | null;
  /** The lifecycle stopped early: draw a faint continuation to the end of the axis. */
  unfinishedFromPercent: number | null;
  /** Ad requests after the first one: refreshes of the slot. */
  refreshes: SlotRefresh[];
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
  const refreshes = slot.requests.length - 1;
  if (refreshes > 0) {
    const stale = slotRefreshes(slot).some((refresh) => refresh.staleBid);
    flags.push({
      key: "refreshed",
      label: stale ? "Refreshed with a used bid" : refreshes === 1 ? "Refreshed once" : `Refreshed ${refreshes} times`,
      description: stale
        ? "A refresh carried the hb_adid of the previous request: no new auction ran for it."
        : "The slot made more than one ad request on this page view.",
      warning: stale,
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

function slotRefreshes(slot: AdSlot): Omit<SlotRefresh, "percent">[] {
  return slot.requests.slice(1).map((request, index) => {
    const previous = slot.requests[index];
    return {
      atMs: request.atMs,
      hadPrebidTargeting: request.hadPrebidTargeting,
      staleBid: request.prebidAdId !== null && request.prebidAdId === previous?.prebidAdId,
    };
  });
}

function slotMarkTimes(slot: AdSlot): { phase: SlotPhase; atMs: number }[] {
  return SLOT_PHASES.flatMap((phase) => {
    const atMs = PHASE_TIME[phase](slot);
    return atMs === null ? [] : [{ phase, atMs }];
  });
}

/** Past this share of the axis a label written after its line would run out of the plot. */
const LABEL_FLIP_PERCENT = 75;

function pageMarkers(metrics: PageMarkers, scale: TimeScale): PageMarker[] {
  const candidates: [string, number | null][] = [
    ["First auction", metrics.timeToFirstAuctionMs],
    ["Page load", metrics.pageLoadMs],
    ["First ad", metrics.timeToFirstAdMs],
  ];
  // The three labels can never overlap, however close the lines are and however narrow the plot:
  // the earliest is written before its line, the other two after theirs on separate rows. A line
  // near the right edge has no room after it, so its label goes before it, on the row the label
  // of the line to its left does not use.
  const sorted = candidates
    .flatMap(([label, atMs]) => (atMs === null ? [] : [{ label, atMs, percent: scale.toPercent(atMs) }]))
    .sort((a, b) => a.atMs - b.atMs);
  const flipped = sorted.map((marker, index) => index > 0 && marker.percent > LABEL_FLIP_PERCENT);
  return sorted.map((marker, index): PageMarker => {
    if (index === 0) return { ...marker, labelSide: "before", labelRow: 0 };
    if (!flipped[index]) return { ...marker, labelSide: "after", labelRow: index === 2 ? 1 : 0 };
    return { ...marker, labelSide: "before", labelRow: index === 2 && flipped[1] ? 0 : 1 };
  });
}

/** Puts every slot's lifecycle and the page lifecycle markers on one axis (ms since navigation start). */
export function layoutSlotTimeline(slots: AdSlot[], metrics: PageMarkers): SlotTimeline {
  const times = slots.flatMap((slot) => [
    ...slotMarkTimes(slot).map((mark) => mark.atMs),
    ...slotRefreshes(slot).map((refresh) => refresh.atMs),
  ]);
  const pageTimes = [metrics.pageLoadMs, metrics.timeToFirstAuctionMs, metrics.timeToFirstAdMs].filter(
    (value): value is number => value !== null,
  );
  const scale = createTimeScale(Math.max(1000, ...times, ...pageTimes) * AXIS_HEADROOM, 8);

  const rows = slots.map((slot): SlotRow => {
    const marks = slotMarkTimes(slot).map((mark) => ({ ...mark, percent: scale.toPercent(mark.atMs) }));
    const refreshes = slotRefreshes(slot).map((refresh) => ({ ...refresh, percent: scale.toPercent(refresh.atMs) }));
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
      refreshes,
      flags: slotFlags(slot),
    };
  });

  return { scale, markers: pageMarkers(metrics, scale), rows };
}
