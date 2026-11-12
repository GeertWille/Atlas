import { expect, test } from "vitest";
import { splitOverview } from "./overview-caveats";

test("an overview without caveats is all summary", () => {
  expect(splitOverview("3 of 6 ad slots rendered a creative.")).toEqual({
    summary: "3 of 6 ad slots rendered a creative.",
    caveats: [],
  });
});

test("note and warning paragraphs become caveats without their markdown label", () => {
  const overview = [
    "3 of 6 ad slots rendered a creative.",
    "**Warning:** The model stopped answering. The results below come from the deterministic checks.",
    "**Note:** Jev failed (HTTP 429); 9 scripts fell back to claude-haiku-5-5.",
  ].join("\n\n");
  expect(splitOverview(overview)).toEqual({
    summary: "3 of 6 ad slots rendered a creative.",
    caveats: [
      "The model stopped answering. The results below come from the deterministic checks.",
      "Jev failed (HTTP 429); 9 scripts fell back to claude-haiku-5-5.",
    ],
  });
});

test("bold text inside the summary is not mistaken for a caveat", () => {
  expect(splitOverview("The wrapper **never** auctions on desktop.").caveats).toEqual([]);
});
