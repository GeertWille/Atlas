import { describe, expect, it } from "vitest";
import type { SourceAnchor, SourceRegion } from "@/bindings";
import { makeFinding } from "@/test/fixtures";
import { adjacentMark, findingMarks, findingStep, gutterMarks, lineTints, readRanges, readShare } from "./anchors";

const anchor = (overrides: Partial<SourceAnchor>): SourceAnchor => ({
  findingId: "f1",
  evidenceIndex: 0,
  startLine: 10,
  endLine: 12,
  located: true,
  ...overrides,
});

const findings = [
  makeFinding({ id: "f1", severity: "high", title: "Scroll gate" }),
  makeFinding({ id: "f2", severity: "medium", title: "Short timeout" }),
  makeFinding({ id: "f3", severity: "critical", title: "Device check" }),
];

describe("findingMarks", () => {
  it("orders the marks by line and takes title and severity from the finding", () => {
    const marks = findingMarks(
      [anchor({ findingId: "f2", startLine: 40, endLine: 44 }), anchor({ findingId: "f1" })],
      findings,
      100,
    );
    expect(marks.map((mark) => [mark.startLine, mark.title, mark.severity])).toEqual([
      [10, "Scroll gate", "high"],
      [40, "Short timeout", "medium"],
    ]);
  });

  it("leaves out an anchor whose finding is gone", () => {
    expect(findingMarks([anchor({ findingId: "deleted" })], findings, 100)).toEqual([]);
  });

  it("clamps a stale anchor to the file", () => {
    const [mark] = findingMarks([anchor({ startLine: 95, endLine: 130 })], findings, 100);
    expect(mark).toMatchObject({ startLine: 95, endLine: 100 });
    const [beyond] = findingMarks([anchor({ startLine: 400, endLine: 410 })], findings, 100);
    expect(beyond).toMatchObject({ startLine: 100, endLine: 100 });
    expect(findingMarks([anchor({})], findings, 0)).toEqual([]);
  });

  it("puts the more severe of two findings on one line first", () => {
    const marks = findingMarks([anchor({ findingId: "f1" }), anchor({ findingId: "f3" })], findings, 100);
    expect(marks.map((mark) => mark.findingId)).toEqual(["f3", "f1"]);
  });
});

describe("lineTints", () => {
  it("tints every line of a range and marks its edges", () => {
    const tints = lineTints(findingMarks([anchor({})], findings, 100));
    expect(tints).toEqual([
      { line: 10, severity: "high", first: true, last: false },
      { line: 11, severity: "high", first: false, last: false },
      { line: 12, severity: "high", first: false, last: true },
    ]);
  });

  it("gives overlapping ranges the worse severity, once per line", () => {
    const tints = lineTints(
      findingMarks([anchor({}), anchor({ findingId: "f3", startLine: 12, endLine: 13 })], findings, 100),
    );
    expect(tints.map((tint) => [tint.line, tint.severity])).toEqual([
      [10, "high"],
      [11, "high"],
      [12, "critical"],
      [13, "critical"],
    ]);
    expect(tints[2]).toMatchObject({ first: true, last: true });
  });
});

describe("gutterMarks", () => {
  it("draws one marker per start line with the worst severity", () => {
    const marks = findingMarks(
      [
        anchor({}),
        anchor({ findingId: "f3" }),
        anchor({ findingId: "f2", startLine: 30, endLine: 30, located: false }),
      ],
      findings,
      100,
    );
    expect(gutterMarks(marks)).toMatchObject([
      { line: 10, severity: "critical", located: true, marks: [{ findingId: "f3" }, { findingId: "f1" }] },
      { line: 30, severity: "medium", located: false, marks: [{ findingId: "f2" }] },
    ]);
  });
});

describe("adjacentMark", () => {
  const marks = findingMarks(
    [
      anchor({}),
      anchor({ findingId: "f2", startLine: 40, endLine: 44 }),
      anchor({ findingId: "f3", startLine: 70, endLine: 70 }),
    ],
    findings,
    100,
  );

  it("goes to the next finding below the caret", () => {
    expect(adjacentMark(marks, 1, 1)?.startLine).toBe(10);
    expect(adjacentMark(marks, 10, 1)?.startLine).toBe(40);
    expect(adjacentMark(marks, 41, 1)?.startLine).toBe(70);
  });

  it("wraps round at the ends", () => {
    expect(adjacentMark(marks, 70, 1)?.startLine).toBe(10);
    expect(adjacentMark(marks, 10, -1)?.startLine).toBe(70);
  });

  it("goes to the previous finding above the caret", () => {
    expect(adjacentMark(marks, 41, -1)?.startLine).toBe(40);
    expect(adjacentMark(marks, 40, -1)?.startLine).toBe(10);
  });

  it("has nowhere to go in a file without findings", () => {
    expect(adjacentMark([], 5, 1)).toBeNull();
  });
});

describe("what the AI read", () => {
  const region = (startLine: number, endLine: number, kind: SourceRegion["kind"] = "read"): SourceRegion => ({
    kind,
    startLine,
    endLine,
    rank: null,
  });

  it("merges overlapping and touching regions", () => {
    expect(readRanges([region(40, 60), region(1, 20), region(21, 30), region(55, 80, "guide")], 200)).toEqual([
      { startLine: 1, endLine: 30 },
      { startLine: 40, endLine: 80 },
    ]);
  });

  it("clamps to the file and drops what lies outside", () => {
    expect(readRanges([region(90, 140), region(300, 320)], 100)).toEqual([{ startLine: 90, endLine: 100 }]);
  });

  it("counts each line once in the share", () => {
    expect(readShare([region(1, 50), region(26, 75)], 100)).toBe(0.75);
    expect(readShare([], 100)).toBe(0);
    expect(readShare([region(1, 5)], 0)).toBe(0);
  });
});

describe("findingStep", () => {
  const press = (key: string, held: Partial<Record<"shiftKey" | "altKey" | "ctrlKey" | "metaKey", boolean>> = {}) =>
    findingStep({ key, shiftKey: false, altKey: false, ctrlKey: false, metaKey: false, ...held });

  it("steps with F8 and Shift+F8, and with Alt and the vertical arrows", () => {
    expect(press("F8")).toBe(1);
    expect(press("F8", { shiftKey: true })).toBe(-1);
    expect(press("ArrowDown", { altKey: true })).toBe(1);
    expect(press("ArrowUp", { altKey: true })).toBe(-1);
  });

  it("leaves the arrows alone without Alt, and with a second modifier that means something else", () => {
    expect(press("ArrowDown")).toBeNull();
    expect(press("ArrowUp")).toBeNull();
    // Selecting by paragraph, and the editor's own document start and end.
    expect(press("ArrowDown", { altKey: true, shiftKey: true })).toBeNull();
    expect(press("ArrowDown", { altKey: true, metaKey: true })).toBeNull();
    expect(press("ArrowUp", { altKey: true, ctrlKey: true })).toBeNull();
    expect(press("ArrowLeft", { altKey: true })).toBeNull();
    expect(press("j", { altKey: true })).toBeNull();
  });
});
