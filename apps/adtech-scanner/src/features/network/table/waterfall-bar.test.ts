import { describe, expect, it } from "vitest";
import { barBackground, describeBar, layoutBar, MIN_BAR_PX } from "./waterfall-bar";

const axis = { startMs: 1000, endMs: 3000, widthPx: 400 };
const phases = { queue: 10, connect: 40, wait: 300, receive: 50 };

describe("layoutBar", () => {
  it("places a request on the window's own scale", () => {
    const bar = layoutBar({ startMs: 1500, durationMs: 400, phases }, axis);
    expect(bar).toMatchObject({ leftPx: 100, widthPx: 80, openEnded: false, lateFromPx: null });
  });

  it("splits the bar into its phases in proportion", () => {
    const bar = layoutBar({ startMs: 1500, durationMs: 400, phases }, axis);
    expect(bar?.segments.map((segment) => [segment.phase, segment.fromPx, segment.toPx])).toEqual([
      ["queue", 0, 2],
      ["connect", 2, 10],
      ["wait", 10, 70],
      ["receive", 70, 80],
    ]);
  });

  it("leaves out phases that took no time", () => {
    const reused = layoutBar(
      { startMs: 1500, durationMs: 400, phases: { queue: 0, connect: 0, wait: 350, receive: 50 } },
      axis,
    );
    expect(reused?.segments.map((segment) => segment.phase)).toEqual(["wait", "receive"]);
  });

  it("gives every phase that took time a pixel, at the cost of the longest", () => {
    const bar = layoutBar(
      { startMs: 1500, durationMs: 100, phases: { queue: 1, connect: 1, wait: 96, receive: 2 } },
      axis,
    );
    const widths = Object.fromEntries(
      (bar?.segments ?? []).map((segment) => [segment.phase, segment.toPx - segment.fromPx]),
    );
    expect(widths.queue).toBe(1);
    expect(widths.connect).toBe(1);
    expect(widths.receive).toBe(1);
    expect(widths.wait).toBeCloseTo(17, 5);
    expect(bar?.segments.at(-1)?.toPx).toBeCloseTo(20, 5);
  });

  it("never draws a bar thinner than the minimum", () => {
    const bar = layoutBar({ startMs: 1500, durationMs: 1, phases: null }, axis);
    expect(bar?.widthPx).toBe(MIN_BAR_PX);
    expect(bar?.segments).toEqual([]);
  });

  it("runs an unanswered request to the end of the window, without phases", () => {
    const bar = layoutBar({ startMs: 2000, durationMs: null, phases }, axis);
    expect(bar).toMatchObject({ leftPx: 200, widthPx: 200, openEnded: true, segments: [] });
  });

  it("starts left of the column for a request that began before the window", () => {
    const bar = layoutBar({ startMs: 500, durationMs: 1000, phases: null }, axis);
    expect(bar).toMatchObject({ leftPx: -100, widthPx: 200 });
  });

  it("has nothing to draw outside the window, or without a window", () => {
    expect(layoutBar({ startMs: 100, durationMs: 200, phases }, axis)).toBeNull();
    expect(layoutBar({ startMs: 3500, durationMs: 200, phases }, axis)).toBeNull();
    expect(layoutBar({ startMs: 1500, durationMs: 200, phases }, { ...axis, widthPx: 0 })).toBeNull();
    expect(layoutBar({ startMs: 1500, durationMs: 200, phases }, { ...axis, endMs: 1000 })).toBeNull();
  });

  it("marks where the part after the auction closed begins", () => {
    const late = layoutBar({ startMs: 1500, durationMs: 1000, phases, lateFromMs: 2400 }, axis);
    expect(late?.lateFromPx).toBe(180);
    const inTime = layoutBar({ startMs: 1500, durationMs: 400, phases, lateFromMs: 2400 }, axis);
    expect(inTime?.lateFromPx).toBeNull();
  });
});

describe("barBackground", () => {
  it("paints the phases as bands and queueing as a hairline, on one element", () => {
    const bar = layoutBar({ startMs: 1500, durationMs: 400, phases }, axis);
    const background = bar && barBackground(bar);
    expect(background).toBe(
      "linear-gradient(to right, transparent 0px 2px, var(--net-connect) 2px 10px, var(--net-wait) 10px 70px, var(--net-receive) 70px 80px) 0 0 / 100% 100% no-repeat, " +
        "linear-gradient(var(--net-queue), var(--net-queue)) 0 50% / 2px 2px no-repeat",
    );
  });

  it("hatches the late part on top", () => {
    const bar = layoutBar({ startMs: 1500, durationMs: 1000, phases: null, lateFromMs: 2400 }, axis);
    expect(bar && barBackground(bar)).toBe(
      "repeating-linear-gradient(135deg, var(--net-late) 0 2px, transparent 2px 5px) 180px 0 / 20px 100% no-repeat",
    );
  });

  it("leaves a bar without timing to its plain colour", () => {
    const bar = layoutBar({ startMs: 1500, durationMs: 400, phases: null }, axis);
    expect(bar && barBackground(bar)).toBeUndefined();
  });
});

describe("describeBar", () => {
  it("says when it started, how long it took and where the time went", () => {
    expect(
      describeBar({ startedAtMs: 1240, durationMs: 312, phases: { queue: 2, connect: 0, wait: 280, receive: 30 } }),
    ).toBe("started 1.24 s, took 312 ms: waiting 280 ms, receiving 30 ms");
    expect(
      describeBar({ startedAtMs: 80, durationMs: 300, phases: { queue: 2, connect: 60, wait: 200, receive: 38 } }),
    ).toBe("started 80 ms, took 300 ms: connecting 60 ms, waiting 200 ms, receiving 38 ms");
  });

  it("says so when there was no answer, or no timing", () => {
    expect(describeBar({ startedAtMs: 4000, durationMs: null, phases: null })).toBe(
      "started 4.00 s, no response before the visit ended",
    );
    expect(describeBar({ startedAtMs: 4000, durationMs: 6, phases: null })).toBe("started 4.00 s, took 6 ms");
  });
});
