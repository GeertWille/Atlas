import { describe, expect, test } from "vitest";
import { createTimeScale, niceStep } from "./time-scale";

describe("niceStep", () => {
  test.each([
    [1000, 6, 200],
    [1590, 6, 500],
    [3675, 8, 500],
    [120, 6, 20],
    [9000, 6, 2000],
  ])("domain %d with at most %d ticks → step %d", (domain, maxTicks, expected) => {
    expect(niceStep(domain, maxTicks)).toBe(expected);
  });
});

describe("createTimeScale", () => {
  test("extends the domain to a whole number of steps and puts a tick on both ends", () => {
    const scale = createTimeScale(1590);
    expect(scale.domainMs).toBe(2000);
    expect(scale.ticks).toEqual([0, 500, 1000, 1500, 2000]);
  });

  test("maps time to a percentage of the domain", () => {
    const scale = createTimeScale(1000);
    expect(scale.toPercent(0)).toBe(0);
    expect(scale.toPercent(500)).toBe(50);
    expect(scale.toPercent(1000)).toBe(100);
  });

  test("clamps positions outside the axis", () => {
    const scale = createTimeScale(1000);
    expect(scale.toPercent(-50)).toBe(0);
    expect(scale.toPercent(5000)).toBe(100);
  });

  test("an empty range still yields a usable axis", () => {
    const scale = createTimeScale(0);
    expect(scale.domainMs).toBeGreaterThan(0);
    expect(scale.toPercent(0)).toBe(0);
    expect(scale.ticks.length).toBeGreaterThanOrEqual(2);
  });
});
