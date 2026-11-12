import { expect, test } from "vitest";
import { makeSummary } from "@/test/fixtures";
import { sortScans } from "./history-sort";

const counts = (high: number) => ({ critical: 0, high, medium: 0, low: 0, info: 0 });
const scans = [
  makeSummary({ id: "old-healthy", startedAt: 100, healthScore: 95, costUsd: 0.1, findingCounts: counts(1) }),
  makeSummary({ id: "new-leaking", startedAt: 300, healthScore: 30, costUsd: 0.4, findingCounts: counts(5) }),
  makeSummary({
    id: "failed",
    startedAt: 200,
    healthScore: null,
    costUsd: 0,
    status: "failed",
    findingCounts: counts(0),
  }),
];

const ids = (list: typeof scans) => list.map((scan) => scan.id);

test("sorts by each key", () => {
  expect(ids(sortScans(scans, "newest"))).toEqual(["new-leaking", "failed", "old-healthy"]);
  expect(ids(sortScans(scans, "oldest"))).toEqual(["old-healthy", "failed", "new-leaking"]);
  expect(ids(sortScans(scans, "findingsDesc"))).toEqual(["new-leaking", "old-healthy", "failed"]);
  expect(ids(sortScans(scans, "costDesc"))).toEqual(["new-leaking", "old-healthy", "failed"]);
});

test("scans without a health score sort last", () => {
  expect(ids(sortScans(scans, "healthAsc"))).toEqual(["new-leaking", "old-healthy", "failed"]);
});

test("does not mutate its input", () => {
  sortScans(scans, "oldest");
  expect(ids(scans)).toEqual(["old-healthy", "new-leaking", "failed"]);
});
