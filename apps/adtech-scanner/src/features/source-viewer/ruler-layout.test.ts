import { describe, expect, it } from "vitest";
import type { FindingMark } from "./anchors";
import { layoutRuler, lineToY, yToLine } from "./ruler-layout";

const mark = (startLine: number, endLine: number): FindingMark => ({
  findingId: `f${startLine}`,
  evidenceIndex: 0,
  title: "A finding",
  severity: "high",
  startLine,
  endLine,
  located: true,
});

describe("lineToY and yToLine", () => {
  it("map the file onto the ruler proportionally", () => {
    expect(lineToY(1, 1000, 500)).toBe(0);
    expect(lineToY(501, 1000, 500)).toBe(250);
    expect(yToLine(250, 1000, 500)).toBe(501);
    expect(yToLine(0, 1000, 500)).toBe(1);
  });

  it("stay inside the file", () => {
    expect(lineToY(5000, 1000, 500)).toBeLessThan(500);
    expect(yToLine(9999, 1000, 500)).toBe(1000);
    expect(yToLine(-5, 1000, 500)).toBe(1);
    expect(yToLine(10, 0, 500)).toBe(1);
    expect(lineToY(1, 1, 500)).toBe(0);
  });
});

describe("layoutRuler", () => {
  const base = { marks: [], regions: [], hitLines: [], lineCount: 1000, height: 500 };

  it("gives a short finding a tick that can still be seen and clicked", () => {
    const { ticks } = layoutRuler({ ...base, marks: [mark(501, 502)] });
    expect(ticks).toEqual([{ top: 250, height: 3, severity: "high", line: 501, findingId: "f501" }]);
  });

  it("draws a long range at its real height", () => {
    const [tick] = layoutRuler({ ...base, marks: [mark(101, 300)] }).ticks;
    expect(tick?.top).toBe(50);
    expect(tick?.height).toBeCloseTo(100, 0);
  });

  it("keeps a mark on the last line on the ruler", () => {
    const [tick] = layoutRuler({ ...base, marks: [mark(1000, 1000)] }).ticks;
    expect((tick?.top ?? 0) + (tick?.height ?? 0)).toBeLessThanOrEqual(500);
  });

  it("fades reading-guide chunks by rank and keeps read lines solid", () => {
    const { bands } = layoutRuler({
      ...base,
      regions: [
        { kind: "read", startLine: 1, endLine: 100, rank: null },
        { kind: "guide", startLine: 201, endLine: 360, rank: 1 },
        { kind: "guide", startLine: 401, endLine: 560, rank: 3 },
      ],
    });
    expect(bands.map((band) => [band.kind, band.top, band.opacity])).toEqual([
      ["read", 0, 0.75],
      ["guide", 100, 0.6],
      ["guide", 200, expect.closeTo(0.4, 5)],
    ]);
    expect(bands[0]?.height).toBeCloseTo(50, 0);
  });

  it("draws at most one search hit per pixel row", () => {
    const hitLines = Array.from({ length: 400 }, (_, index) => 100 + index);
    const { hits } = layoutRuler({ ...base, hitLines });
    expect(hits.length).toBeLessThanOrEqual(201);
    expect(hits.length).toBeGreaterThan(150);
    expect(new Set(hits.map((hit) => hit.top)).size).toBe(hits.length);
  });

  it("is empty before the ruler has a size", () => {
    expect(layoutRuler({ ...base, marks: [mark(1, 2)], height: 0 })).toEqual({ bands: [], ticks: [], hits: [] });
  });
});
