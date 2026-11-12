import { describe, expect, it } from "vitest";
import { hasLongLine } from "./long-lines";

describe("hasLongLine", () => {
  it("is false for empty text and for lines at the limit", () => {
    expect(hasLongLine("", 5)).toBe(false);
    expect(hasLongLine("12345\n12345", 5)).toBe(false);
  });

  it("finds a long line wherever it sits", () => {
    expect(hasLongLine("123456\nab", 5)).toBe(true);
    expect(hasLongLine("ab\n123456\nab", 5)).toBe(true);
    expect(hasLongLine("ab\n123456", 5)).toBe(true);
  });

  it("does not count the line break towards the length", () => {
    expect(hasLongLine("12345\n", 5)).toBe(false);
  });
});
