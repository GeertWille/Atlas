import { expect, test } from "vitest";
import { makeLog } from "@/test/fixtures";
import {
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
