import { describe, expect, test } from "vitest";
import {
  countOf,
  formatBytes,
  formatCompact,
  formatCpm,
  formatDuration,
  formatOffset,
  formatPercent,
  formatRelativeTime,
  formatUsd,
  pluralize,
} from "./format";

test("countOf names the hidden part only while a filter hides something", () => {
  expect(countOf(12, 12, "script")).toBe("12 scripts");
  expect(countOf(3, 1200, "entry", "entries")).toBe("3 of 1,200 entries");
  expect(countOf(1, 1, "finding")).toBe("1 finding");
});

describe("formatUsd", () => {
  test.each([
    [0, "$0.00"],
    [0.0004, "<$0.001"],
    [0.0234, "$0.023"],
    [0.353, "$0.35"],
    [0.5, "$0.50"],
    [12.5, "$12.50"],
  ])("%d → %s", (amount, expected) => {
    expect(formatUsd(amount)).toBe(expected);
  });
});

describe("formatDuration", () => {
  test.each([
    [null, "–"],
    [0, "0 ms"],
    [840, "840 ms"],
    [1240, "1.24 s"],
    [12_400, "12.4 s"],
    [125_000, "2m 05s"],
    // Unit boundaries: the rounded value picks the unit.
    [999.6, "1.00 s"],
    [9_996, "10.0 s"],
    [59_980, "1m 00s"],
    [119_600, "2m 00s"],
  ])("%s → %s", (ms, expected) => {
    expect(formatDuration(ms)).toBe(expected);
  });
});

test("formatOffset keeps millisecond precision with grouping", () => {
  expect(formatOffset(1523.4)).toBe("1,523 ms");
  expect(formatOffset(null)).toBe("–");
});

describe("formatBytes", () => {
  test.each([
    [512, "512 B"],
    [1536, "1.5 KB"],
    [49_357, "48 KB"],
    [1_048_300, "1.0 MB"],
    [3_565_158, "3.4 MB"],
  ])("%d → %s", (bytes, expected) => {
    expect(formatBytes(bytes)).toBe(expected);
  });
});

test("formatCpm prints USD with a dollar sign and other currencies by code", () => {
  expect(formatCpm(2.5)).toBe("$2.50");
  expect(formatCpm(1.2, "EUR")).toBe("1.20 EUR");
  expect(formatCpm(null)).toBe("–");
});

test("formatPercent rounds a ratio to a whole percentage", () => {
  expect(formatPercent(0.066)).toBe("7%");
  expect(formatPercent(1)).toBe("100%");
});

test("formatCompact abbreviates from one thousand", () => {
  expect(formatCompact(999)).toBe("999");
  expect(formatCompact(14_820)).toBe("14.8K");
});

describe("formatRelativeTime", () => {
  const now = Date.UTC(2026, 10, 12, 12, 0, 0);
  test.each([
    [10_000, "just now"],
    [5 * 60_000, "5 min ago"],
    [3 * 3_600_000, "3 h ago"],
    [26 * 3_600_000, "yesterday"],
    [4 * 86_400_000, "4 days ago"],
  ])("%d ms ago → %s", (ago, expected) => {
    expect(formatRelativeTime(now - ago, now)).toBe(expected);
  });
});

test("pluralize picks the form by count", () => {
  expect(pluralize(1, "scan")).toBe("1 scan");
  expect(pluralize(2, "entry", "entries")).toBe("2 entries");
  expect(pluralize(1200, "request")).toBe("1,200 requests");
});
