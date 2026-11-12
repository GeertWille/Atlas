import { describe, expect, it } from "vitest";
import { clampPaneSize } from "./split-pane";

describe("clampPaneSize", () => {
  const limits = { min: 280, max: 720, available: 1000, minOther: 360 };

  it("keeps a size inside its own limits", () => {
    expect(clampPaneSize(400, limits)).toBe(400);
    expect(clampPaneSize(100, limits)).toBe(280);
  });

  it("leaves the other pane its minimum", () => {
    expect(clampPaneSize(700, limits)).toBe(640);
  });

  it("never goes below the minimum, even when the container is too small for both", () => {
    expect(clampPaneSize(500, { ...limits, available: 500 })).toBe(280);
  });

  it("rounds to whole pixels", () => {
    expect(clampPaneSize(400.6, limits)).toBe(401);
  });
});
