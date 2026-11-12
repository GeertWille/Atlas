import { expect, test } from "vitest";
import type { AdSlot } from "@/bindings";
import { makeDetail, makeSlot } from "@/test/fixtures";
import { buildKeyMetrics, detectedStack } from "./key-metrics";

const metricsOf = (overrides: Partial<ReturnType<typeof makeDetail>["metrics"]> = {}) => ({
  ...makeDetail().metrics,
  ...overrides,
});
const metric = (label: string, overrides?: Parameters<typeof metricsOf>[0], slots: AdSlot[] = []) =>
  buildKeyMetrics(metricsOf(overrides), slots).find((item) => item.label === label);

const filled = (elementId: string) => makeSlot({ elementId, outcome: "filled", requestedAtMs: 900 });
const neverRequested = (elementId: string) =>
  makeSlot({ elementId, outcome: "neverRequested", requestedAtMs: null, responseAtMs: null, renderedAtMs: null });

test("the winning CPM is an average over a stated number of bids, never a dollar total", () => {
  expect(metric("Avg winning CPM", { avgWinningCpm: 2.4, totalWinningCpm: 12 })).toEqual({
    label: "Avg winning CPM",
    value: "$2.40",
    hint: "across 5 winning bids",
  });
  expect(metric("Avg winning CPM", { avgWinningCpm: null, totalWinningCpm: 0 })).toMatchObject({
    value: "–",
    hint: "across 0 winning bids",
  });
});

test("a perfect fill rate does not hide the slots that never made an ad request", () => {
  const slots = [filled("a"), filled("b"), filled("c"), neverRequested("d"), neverRequested("e"), neverRequested("f")];
  expect(metric("Requested slots filled", { slotCount: 6, emptySlotCount: 0, fillRate: 1 }, slots)).toMatchObject({
    value: "100%",
    hint: "3 of 6 never requested",
  });
  expect(
    metric("Requested slots filled", { slotCount: 2, emptySlotCount: 1, fillRate: 0.5 }, [
      filled("a"),
      makeSlot({ elementId: "b", outcome: "empty", requestedAtMs: 900 }),
    ]),
  ).toMatchObject({ value: "50%", hint: "1 of 2 slots empty" });
});

test("rates are blank when there was nothing to measure them against", () => {
  expect(metric("Requested slots filled", { fillRate: 0 }, [neverRequested("a")])?.value).toBe("–");
  expect(metric("Requested slots filled", { fillRate: 0 })?.value).toBe("–");
  expect(metric("Bid timeout rate", { bidRequestCount: 0, bidTimeoutCount: 0, timeoutRate: 0 })?.value).toBe("–");
  expect(metric("Bid timeout rate")).toMatchObject({ value: "50%", hint: "1 of 2 requests" });
});

test("the detected stack names what was found, with the Prebid version when known", () => {
  expect(detectedStack(metricsOf(), "9.41.0")).toEqual(["Prebid.js 9.41.0", "Google Publisher Tag"]);
  expect(detectedStack(metricsOf({ hasPrebid: false, hasGpt: false }), null)).toEqual([]);
});
