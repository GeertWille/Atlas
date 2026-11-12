import { expect, test } from "vitest";
import { aiAvailability, anyAiAvailable } from "./ai-availability";
import { defaultScanOptions } from "./scan-options";

const keys = (hasApiKey: boolean, hasTypesafeKey: boolean, jevEnabled: boolean) => ({
  hasApiKey,
  hasTypesafeKey,
  jevEnabled,
});

test.each([
  ["no keys", keys(false, false, false), { triage: null, logicAnalysis: false }],
  ["Anthropic key only", keys(true, false, false), { triage: "claude", logicAnalysis: true }],
  ["both keys, Jev off", keys(true, true, false), { triage: "claude", logicAnalysis: true }],
  ["both keys, Jev on", keys(true, true, true), { triage: "jev", logicAnalysis: true }],
  ["TypeSafe key only, Jev on", keys(false, true, true), { triage: "jev", logicAnalysis: false }],
  ["TypeSafe key only, Jev off", keys(false, true, false), { triage: null, logicAnalysis: false }],
  // The backend clears the switch with the key; a stale flag must still not count.
  ["Jev on without a key", keys(true, false, true), { triage: "claude", logicAnalysis: true }],
])("%s", (_name, settings, expected) => {
  expect(aiAvailability(settings)).toEqual(expected);
  expect(anyAiAvailable(settings)).toBe(expected.triage !== null);
});

test("scans started outside the composer use AI whenever any of it is available", () => {
  const start = (settings: ReturnType<typeof keys>) =>
    defaultScanOptions(
      { ...settings, defaultObserveSeconds: 20, defaultVisitor: "scroll", defaultExtraPages: 2 },
      "https://example.com/",
      "desktop",
    ).aiEnabled;
  expect(start(keys(false, false, false))).toBe(false);
  expect(start(keys(false, true, true))).toBe(true);
  expect(start(keys(true, false, false))).toBe(true);
});
