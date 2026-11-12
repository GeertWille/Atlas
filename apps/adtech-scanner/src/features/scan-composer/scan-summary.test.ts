import { expect, test } from "vitest";
import { aiAvailability } from "@/shared/domain/ai-availability";
import { makeProxy, settingsFixture } from "@/test/fixtures";
import { newScanForm, type ScanForm } from "./scan-form";
import { scanSummary } from "./scan-summary";

const line = (form: Partial<ScanForm>, settings = settingsFixture, extra: { budgetUsd?: number } = {}) =>
  scanSummary({
    form: { ...newScanForm(settings), ...form },
    settings,
    ai: aiAvailability(settings),
    budgetUsd: extra.budgetUsd,
    proxyMissing: false,
  })
    .map((part) => part.text)
    .join(" · ");

test("the default scan reads as one plain line", () => {
  expect(line({})).toBe("Scroll · browser hidden · system proxy · AI on, about $0.36");
});

test("a browse visit names its pages, its proxy and what the extra pages cost", () => {
  const belgium = makeProxy({ id: "proxy-be", name: "Belgium" });
  const settings = { ...settingsFixture, proxies: [belgium] };
  expect(line({ visitor: "browse", pages: 3, showBrowser: true, proxy: "profile:proxy-be" }, settings)).toBe(
    "Browse like a visitor · 3 pages · browser shown · Belgium proxy · AI on, about $0.37",
  );
});

test("pages are only named for a browse visit, and a manual session always shows the browser", () => {
  expect(line({ visitor: "none", pages: 4 })).toBe("Observe only · browser hidden · system proxy · AI on, about $0.36");
  expect(line({ visitor: "manual", showBrowser: false, proxy: "none" })).toBe(
    "I'll browse myself · browser shown · no proxy · AI on, about $0.36",
  );
});

test("the settings decide what the scan leaves to them: default proxy and kept browser profile", () => {
  const belgium = makeProxy({ id: "proxy-be", name: "Belgium" });
  const settings = {
    ...settingsFixture,
    proxies: [belgium],
    defaultProxy: { kind: "profile" as const, id: "proxy-be" },
    persistentProfile: true,
  };
  expect(line({}, settings)).toBe(
    "Scroll · browser hidden · Belgium proxy · kept browser profile · AI on, about $0.36",
  );
  expect(line({ profile: "fresh", proxy: "system" }, settings)).toBe(
    "Scroll · browser hidden · system proxy · AI on, about $0.36",
  );
  expect(line({ profile: "kept" })).toContain("kept browser profile");
});

test("the AI part says what runs: nothing without a key, off, triage only, or capped by this scan's budget", () => {
  expect(line({}, { ...settingsFixture, hasApiKey: false })).toMatch(/· checks only, no API key$/);
  expect(line({ aiEnabled: false })).toMatch(/· AI off$/);
  const jevOnly = { ...settingsFixture, hasApiKey: false, hasTypesafeKey: true, jevEnabled: true };
  expect(line({}, jevOnly)).toMatch(/· script triage, about \$0\.001$/);
  expect(line({}, settingsFixture, { budgetUsd: 0.2 })).toMatch(/· AI on, about \$0\.20$/);
});

test("a deleted proxy is named as such and flagged", () => {
  const parts = scanSummary({
    form: { ...newScanForm(settingsFixture), proxy: "profile:proxy-gone" },
    settings: settingsFixture,
    ai: aiAvailability(settingsFixture),
    budgetUsd: undefined,
    proxyMissing: true,
  });
  expect(parts.find((part) => part.warning)).toEqual({ text: "deleted proxy", warning: true });
});
