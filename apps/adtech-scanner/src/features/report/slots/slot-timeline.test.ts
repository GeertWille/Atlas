import { expect, test } from "vitest";
import { makeDetail, makeSlot } from "@/test/fixtures";
import { layoutSlotTimeline, slotFlags } from "./slot-timeline";

const metrics = { ...makeDetail().metrics, timeToFirstAdMs: 2400 };
const filled = makeSlot({ renderedAtMs: 2400 });
const starved = makeSlot({
  elementId: "div-gpt-sidebar",
  outcome: "neverRequested",
  definedAtMs: 450,
  requestedAtMs: null,
  responseAtMs: null,
  renderedAtMs: null,
  viewableAtMs: null,
});

test("all slots share one axis that covers the latest event", () => {
  const timeline = layoutSlotTimeline([filled, starved], metrics);
  expect(timeline.scale.domainMs).toBe(4000);
  expect(timeline.scale.ticks).toEqual([0, 500, 1000, 1500, 2000, 2500, 3000, 3500, 4000]);
});

test("a filled slot has a mark per phase and a span from first to last", () => {
  const [row] = layoutSlotTimeline([filled, starved], metrics).rows;
  expect(row?.marks.map((mark) => [mark.phase, mark.percent])).toEqual([
    ["defined", 10],
    ["requested", 50],
    ["response", 55],
    ["rendered", 60],
    ["viewable", 87.5],
  ]);
  expect(row?.span).toEqual({ startPercent: 10, widthPercent: 77.5 });
  expect(row?.unfinishedFromPercent).toBeNull();
});

test("a never-requested slot has only its definition and an open continuation", () => {
  const row = layoutSlotTimeline([filled, starved], metrics).rows[1];
  expect(row?.marks).toHaveLength(1);
  expect(row?.span).toBeNull();
  expect(row?.unfinishedFromPercent).toBe(11.25);
});

test("page markers are ordered by time and colliding labels drop to a second row", () => {
  const { markers } = layoutSlotTimeline([filled], metrics);
  expect(markers.map((marker) => [marker.label, marker.percent, marker.labelRow])).toEqual([
    ["First auction", 20, 0],
    ["Page load", 52.5, 0],
    ["First ad", 60, 1],
  ]);
});

test("missing page timings produce no markers", () => {
  const bare = { ...metrics, pageLoadMs: null, timeToFirstAuctionMs: null, timeToFirstAdMs: null };
  expect(layoutSlotTimeline([filled], bare).markers).toEqual([]);
});

test("flags call out early requests, missing targeting and below-the-fold slots", () => {
  expect(slotFlags(filled)).toEqual([]);
  const flagged = makeSlot({ requestedBeforeAuctionEnd: true, hadPrebidTargeting: false, inInitialViewport: false });
  expect(slotFlags(flagged).map((flag) => [flag.key, flag.warning])).toEqual([
    ["earlyRequest", true],
    ["noPrebidTargeting", true],
    ["belowFold", false],
  ]);
  // Unknown (null) values are not flagged.
  expect(slotFlags(makeSlot({ hadPrebidTargeting: null, inInitialViewport: null }))).toEqual([]);
});
