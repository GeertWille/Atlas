import { describe, expect, it } from "vitest";
import { currentMatch, describeMatches, type SearchSummary } from "./search-summary";

const summary = (starts: number[], capped = false): SearchSummary => ({ starts, capped, active: true });

describe("currentMatch", () => {
  it("is the 1-based index of the match that starts at the selection", () => {
    expect(currentMatch(summary([4, 10, 30]), 4)).toBe(1);
    expect(currentMatch(summary([4, 10, 30]), 30)).toBe(3);
  });

  it("is null when the selection is between matches", () => {
    expect(currentMatch(summary([4, 10, 30]), 11)).toBeNull();
    expect(currentMatch(summary([]), 0)).toBeNull();
  });
});

describe("describeMatches", () => {
  it("names the position when the selection is on a match", () => {
    expect(describeMatches(summary([4, 10, 30]), 10)).toBe("2 of 3");
  });

  it("gives only the total otherwise", () => {
    expect(describeMatches(summary([4, 10, 30]), 0)).toBe("3 results");
    expect(describeMatches(summary([4]), 0)).toBe("1 result");
  });

  it("says when there is nothing", () => {
    expect(describeMatches(summary([]), 0)).toBe("No results");
  });

  it("marks a capped count as a lower bound", () => {
    const capped = summary(
      Array.from({ length: 10_000 }, (_, index) => index * 2),
      true,
    );
    expect(describeMatches(capped, 1)).toBe("10,000+ results");
    expect(describeMatches(capped, 2)).toBe("2 of 10,000+");
  });
});
