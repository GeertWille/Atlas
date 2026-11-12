import { describe, expect, it } from "vitest";
import { createRng } from "./rng";
import { fillerLines, generateFile, lineAtOffset, minify } from "./source-text";

describe("fillerLines", () => {
  it.each([0, 1, 2, 3, 4, 7, 10, 37, 200])("gives exactly %i lines", (count) => {
    expect(fillerLines(createRng(count + 1), count, "wrapper")).toHaveLength(count);
  });

  it("is the same text for the same seed", () => {
    expect(fillerLines(createRng(9), 80, "application")).toEqual(fillerLines(createRng(9), 80, "application"));
  });
});

describe("generateFile", () => {
  const first = "var A = 1;\nvar B = 2;";
  const second = "function gate() {\n  return true;\n}";

  it("puts every excerpt at the line it is cited at", () => {
    const lines = generateFile(4, 120, "wrapper", [
      { line: 60, body: second },
      { line: 12, body: first },
    ]);
    expect(lines).toHaveLength(120);
    expect(lines.slice(11, 13)).toEqual(first.split("\n"));
    expect(lines.slice(59, 62)).toEqual(second.split("\n"));
    expect(lines[10]).toBe("");
  });

  it("keeps the first of two excerpts that overlap", () => {
    const lines = generateFile(4, 40, "wrapper", [
      { line: 10, body: second },
      { line: 11, body: first },
    ]);
    expect(lines.slice(9, 12)).toEqual(second.split("\n"));
    expect(lines).toHaveLength(40);
  });

  it("grows past the asked length when an excerpt lies beyond it", () => {
    const lines = generateFile(4, 20, "library", [{ line: 30, body: first }]);
    expect(lines.slice(29, 31)).toEqual(first.split("\n"));
  });
});

describe("minify", () => {
  it("is one line whose offsets map back to the formatted lines", () => {
    const lines = ["var a = 1;", "", "// note", "function f() {", "  return a; // why", "}"];
    const { text, lineStarts } = minify(lines);
    expect(text).toBe("var a = 1;/* note */function f() {return a; /* why */}");
    expect(text).not.toContain("\n");
    expect(lineAtOffset(lineStarts, 0)).toBe(1);
    expect(lineAtOffset(lineStarts, text.indexOf("function"))).toBe(4);
    expect(lineAtOffset(lineStarts, text.indexOf("return"))).toBe(5);
    expect(lineAtOffset(lineStarts, text.length - 1)).toBe(6);
  });
});
