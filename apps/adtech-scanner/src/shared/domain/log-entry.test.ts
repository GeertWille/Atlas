import { expect, test } from "vitest";
import { makeLog } from "@/test/fixtures";
import {
  classificationDetails,
  findingSeverity,
  isExpandable,
  logEntryAsText,
  logPayload,
  reportedFindingId,
  summarizeToolInput,
} from "./log-entry";

test("summarizeToolInput renders an argument list", () => {
  expect(summarizeToolInput({ url: "a.js", startLine: 3 })).toBe('url: "a.js", startLine: 3');
  expect(summarizeToolInput(null)).toBe("");
  expect(summarizeToolInput(["x"])).toBe('["x"]');
});

test("logPayload separates raw tool output from the remaining data", () => {
  const entry = makeLog({ kind: "toolResult", data: { output: "line 1\nline 2", truncated: true } });
  expect(logPayload(entry)).toEqual({ text: "line 1\nline 2", json: '{\n  "truncated": true\n}' });
});

test("logPayload returns pretty JSON for structured data and nothing for none", () => {
  expect(logPayload(makeLog({ data: { a: 1 } }))).toEqual({ text: null, json: '{\n  "a": 1\n}' });
  expect(logPayload(makeLog({ data: { output: "only text" } }))).toEqual({ text: "only text", json: null });
  expect(logPayload(makeLog())).toEqual({ text: null, json: null });
});

test("an entry is expandable when it has data or a long or multi-line message", () => {
  expect(isExpandable(makeLog())).toBe(false);
  expect(isExpandable(makeLog({ data: {} }))).toBe(true);
  // Anything the one-line summary would cut off must be reachable by opening the row.
  expect(isExpandable(makeLog({ message: "x".repeat(80) }))).toBe(false);
  expect(isExpandable(makeLog({ message: "x".repeat(81) }))).toBe(true);
  expect(isExpandable(makeLog({ message: "a\nb" }))).toBe(true);
});

test("findingSeverity reads a valid severity from finding data only", () => {
  expect(findingSeverity(makeLog({ kind: "finding", data: { severity: "high" } }))).toBe("high");
  expect(findingSeverity(makeLog({ kind: "finding", data: { severity: "catastrophic" } }))).toBeNull();
  expect(findingSeverity(makeLog({ kind: "finding" }))).toBeNull();
});

test("logEntryAsText joins message, output and data", () => {
  const entry = makeLog({ message: "read_script returned 2 lines", data: { output: "a\nb", bytes: 3 } });
  expect(logEntryAsText(entry)).toBe('read_script returned 2 lines\n\na\nb\n\n{\n  "bytes": 3\n}');
});

test("reportedFindingId links a finding entry to its finding", () => {
  expect(reportedFindingId(makeLog({ kind: "finding", data: { findingId: "scan-1-f2" } }))).toBe("scan-1-f2");
  expect(reportedFindingId(makeLog({ kind: "finding", data: {} }))).toBeNull();
  expect(reportedFindingId(makeLog({ kind: "message", data: { findingId: "scan-1-f2" } }))).toBeNull();
});

const classification = (data: unknown) => makeLog({ kind: "classification", data });

test("a classification entry keeps its script excerpt as text and the rest as data", () => {
  const entry = classification({ scriptId: "s7", excerpt: "pbjs.requestBids();" });
  expect(logPayload(entry)).toEqual({ text: "pbjs.requestBids();", json: '{\n  "scriptId": "s7"\n}' });
  // An `excerpt` on any other kind is ordinary data.
  expect(logPayload(makeLog({ data: { excerpt: "x" } })).text).toBeNull();
});

test("classificationDetails reads the answers, the thresholds each was held against, and the routing decision", () => {
  const entry = classification({
    scriptId: "s7",
    answers: [
      { questionId: "ad_orchestration", pYes: 0.93 },
      { questionId: "publisher_specific", pYes: 0.71 },
      { questionId: "category", selected: "wrapper", probabilities: [["wrapper", 0.71]], confidence: 0.71 },
    ],
    thresholds: { analyse: 0.5, drop: 0.15, publisherSpecific: 0.5 },
    decision: { adRelated: true, category: "wrapper", route: "jev-yes" },
  });
  expect(classificationDetails(entry)).toEqual({
    answers: [
      {
        question: "ad_orchestration",
        probability: 0.93,
        // The routing cut-offs belong to the triage question, which comes first.
        thresholds: [
          { name: "analyse", value: 0.5 },
          { name: "drop", value: 0.15 },
        ],
      },
      { question: "publisher_specific", probability: 0.71, thresholds: [{ name: "publisherSpecific", value: 0.5 }] },
      { question: "category", selected: "wrapper", confidence: 0.71 },
    ],
    thresholds: [
      { name: "analyse", value: 0.5 },
      { name: "drop", value: 0.15 },
      { name: "publisherSpecific", value: 0.5 },
    ],
    adRelated: true,
    category: "wrapper",
    rule: "jev-yes",
  });
});

test("classificationDetails keeps what it recognises and drops the rest", () => {
  const entry = classification({
    answers: [{ questionId: "ad_orchestration", pYes: 1.4 }, { pYes: 0.2 }, "noise", { questionId: "q", pYes: 0.2 }],
    thresholds: { analyse: "high", drop: 0.15 },
    decision: { adRelated: "yes", category: "spaceship", route: "jev-no" },
  });
  expect(classificationDetails(entry)).toEqual({
    answers: [{ question: "q", probability: 0.2, thresholds: [{ name: "drop", value: 0.15 }] }],
    thresholds: [{ name: "drop", value: 0.15 }],
    adRelated: null,
    category: null,
    rule: "jev-no",
  });
});

test("classificationDetails gives up on other kinds and on data it cannot read at all", () => {
  expect(classificationDetails(makeLog({ data: { decision: { route: "jev-no" } } }))).toBeNull();
  expect(classificationDetails(classification(null))).toBeNull();
  expect(classificationDetails(classification("s7 dropped"))).toBeNull();
  expect(classificationDetails(classification({ answers: "none", decision: [] }))).toBeNull();
});
