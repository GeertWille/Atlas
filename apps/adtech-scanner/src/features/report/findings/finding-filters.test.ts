import { expect, test } from "vitest";
import { makeFinding } from "@/test/fixtures";
import { countBy, filterFindings, hasActiveFilters, NO_FINDING_FILTERS, sortFindings } from "./finding-filters";
import { findingToMarkdown } from "./finding-markdown";

const findings = [
  makeFinding({ id: "low", severity: "low", category: "hygiene", source: "heuristic" }),
  makeFinding({ id: "crit", severity: "critical", category: "logicBug", source: "ai", estimatedRevenueImpactPct: 34 }),
  makeFinding({
    id: "high-a",
    severity: "high",
    category: "timing",
    source: "heuristic",
    estimatedRevenueImpactPct: 5,
  }),
  makeFinding({
    id: "high-b",
    severity: "high",
    category: "configuration",
    source: "ai",
    estimatedRevenueImpactPct: 11,
  }),
];

const ids = (list: typeof findings) => list.map((finding) => finding.id);

test("sorts by severity, then revenue at risk", () => {
  expect(ids(sortFindings(findings))).toEqual(["crit", "high-b", "high-a", "low"]);
});

test("no filters keeps everything", () => {
  expect(filterFindings(findings, NO_FINDING_FILTERS)).toHaveLength(4);
  expect(hasActiveFilters(NO_FINDING_FILTERS)).toBe(false);
});

test("filters combine with AND across dimensions and OR within one", () => {
  expect(ids(filterFindings(findings, { ...NO_FINDING_FILTERS, severities: ["high", "critical"] }))).toEqual([
    "crit",
    "high-a",
    "high-b",
  ]);
  expect(ids(filterFindings(findings, { severities: ["high"], categories: [], source: "ai" }))).toEqual(["high-b"]);
  expect(filterFindings(findings, { severities: ["low"], categories: ["timing"], source: "all" })).toEqual([]);
  expect(hasActiveFilters({ ...NO_FINDING_FILTERS, source: "heuristic" })).toBe(true);
});

test("countBy tallies a dimension", () => {
  expect(countBy(findings, (finding) => finding.severity)).toEqual({ low: 1, critical: 1, high: 2 });
});

test("findingToMarkdown produces a self-contained note", () => {
  const markdown = findingToMarkdown(
    makeFinding({
      title: "Failsafe fires early",
      severity: "high",
      category: "configuration",
      source: "ai",
      confidence: 0.91,
      estimatedRevenueImpactPct: 11,
      impact: "Leaderboard sold without header bids",
      evidence: [
        {
          kind: "code",
          label: "Timer",
          body: "setTimeout(send, 800);",
          scriptUrl: "https://x.example/w.js",
          line: 318,
        },
        { kind: "metric", label: "Timing", body: "GPT at +800 ms", scriptUrl: null, line: null },
      ],
      affectedSlots: ["div-gpt-top"],
      affectedBidders: ["ix"],
    }),
  );
  expect(markdown).toBe(
    [
      "## Failsafe fires early",
      "*High · Configuration · AI analysis · confidence 91% · est. 11% of ad revenue at risk*",
      "**Impact:** Leaderboard sold without header bids",
      "What is wrong.",
      "### Recommendation\n\nHow to fix it.",
      "### Evidence\n\n**Timer** (https://x.example/w.js:318)\n\n```js\nsetTimeout(send, 800);\n```\n\n**Timing**\n\n> GPT at +800 ms",
      "Slots: div-gpt-top\nBidders: ix",
    ].join("\n\n"),
  );
});
