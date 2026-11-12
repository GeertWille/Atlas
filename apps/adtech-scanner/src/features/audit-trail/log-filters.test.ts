import { expect, test } from "vitest";
import { makeLog } from "@/test/fixtures";
import { countLogsByStep, filterLogs, logsAsText, NO_LOG_FILTERS } from "./log-filters";

const logs = [
  makeLog({ id: "l1", stepId: "capture", message: "Navigating to the page" }),
  makeLog({ id: "l2", stepId: "capture", level: "warn", message: "Consent banner left untouched" }),
  makeLog({
    id: "l3",
    stepId: "analyze",
    node: "analyzeLogic",
    kind: "toolCall",
    message: "read_script",
    data: { url: "hb-wrapper.js" },
  }),
  makeLog({
    id: "l4",
    stepId: "analyze",
    node: "analyzeLogic",
    kind: "thinking",
    level: "debug",
    message: "The wrapper gates on scroll",
  }),
];

const ids = (list: typeof logs) => list.map((entry) => entry.id);

test("no filters keeps every entry", () => {
  expect(filterLogs(logs, NO_LOG_FILTERS)).toHaveLength(4);
});

test("filters by step, kind and level", () => {
  expect(ids(filterLogs(logs, { ...NO_LOG_FILTERS, stepId: "analyze" }))).toEqual(["l3", "l4"]);
  expect(ids(filterLogs(logs, { ...NO_LOG_FILTERS, kinds: ["toolCall", "thinking"], levels: ["debug"] }))).toEqual([
    "l4",
  ]);
  expect(ids(filterLogs(logs, { ...NO_LOG_FILTERS, levels: ["warn", "error"] }))).toEqual(["l2"]);
});

test("search looks in the message and in structured data", () => {
  expect(ids(filterLogs(logs, { ...NO_LOG_FILTERS, search: "WRAPPER" }))).toEqual(["l3", "l4"]);
  expect(ids(filterLogs(logs, { ...NO_LOG_FILTERS, search: "wrapper", stepId: "capture" }))).toEqual([]);
});

test("counts entries per step", () => {
  expect(Object.fromEntries(countLogsByStep(logs))).toEqual({ capture: 2, analyze: 2 });
});

test("logsAsText prints one greppable line per entry", () => {
  expect(logsAsText([logs[2] as (typeof logs)[number]])).toBe(
    '2023-11-14T22:13:21.000Z INFO  [analyzeLogic] toolCall: read_script {"url":"hb-wrapper.js"}',
  );
});
