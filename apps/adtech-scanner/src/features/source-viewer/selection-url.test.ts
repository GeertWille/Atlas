import { describe, expect, it } from "vitest";
import { sameSpan, searchToSpan, selectionSpan, spanToSearch } from "./selection-url";

describe("searchToSpan", () => {
  it("reads a single line and a range", () => {
    expect(searchToSpan({ line: 118 }, 500)).toEqual({ from: 118, to: 118 });
    expect(searchToSpan({ line: 118, to: 124 }, 500)).toEqual({ from: 118, to: 124 });
  });

  it("puts a reversed range in order and clamps it to the file", () => {
    expect(searchToSpan({ line: 124, to: 118 }, 500)).toEqual({ from: 118, to: 124 });
    expect(searchToSpan({ line: 480, to: 9000 }, 500)).toEqual({ from: 480, to: 500 });
    expect(searchToSpan({ line: 9000 }, 500)).toEqual({ from: 500, to: 500 });
  });

  it("names nothing without a line, or in an empty file", () => {
    expect(searchToSpan({}, 500)).toBeNull();
    expect(searchToSpan({ to: 12 }, 500)).toBeNull();
    expect(searchToSpan({ line: 3 }, 0)).toBeNull();
  });
});

describe("spanToSearch", () => {
  it("writes one line without `to`", () => {
    expect(spanToSearch({ from: 118, to: 118 })).toEqual({ line: 118, to: undefined });
  });

  it("round-trips a range", () => {
    const search = spanToSearch({ from: 118, to: 124 });
    expect(search).toEqual({ line: 118, to: 124 });
    expect(searchToSpan(search, 500)).toEqual({ from: 118, to: 124 });
  });
});

describe("selectionSpan", () => {
  it("does not count a line the selection only touches the start of", () => {
    expect(selectionSpan(10, 13, 1)).toEqual({ from: 10, to: 12 });
    expect(selectionSpan(10, 13, 4)).toEqual({ from: 10, to: 13 });
    expect(selectionSpan(10, 10, 1)).toEqual({ from: 10, to: 10 });
  });
});

describe("sameSpan", () => {
  it("compares by value", () => {
    expect(sameSpan({ from: 1, to: 2 }, { from: 1, to: 2 })).toBe(true);
    expect(sameSpan({ from: 1, to: 2 }, { from: 1, to: 3 })).toBe(false);
    expect(sameSpan(null, null)).toBe(true);
    expect(sameSpan(null, { from: 1, to: 1 })).toBe(false);
  });
});
