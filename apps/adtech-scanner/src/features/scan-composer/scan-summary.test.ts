import { expect, test } from "vitest";
import { aiAvailability } from "@/shared/domain/ai-availability";
import { browserbaseSettingsFixture, makeProxy, settingsFixture } from "@/test/fixtures";
import { newScanForm, type ScanForm } from "./scan-form";
import { scanSummary } from "./scan-summary";

const line = (
  form: Partial<ScanForm>,
  settings = settingsFixture,
  extra: { budgetUsd?: number; remote?: boolean; browserMissing?: boolean } = {},
) =>
  scanSummary({
    form: { ...newScanForm(settings), ...form },
    settings,
    ai: aiAvailability(settings),
    budgetUsd: extra.budgetUsd,
    proxyMissing: false,
    remote: extra.remote ?? false,
    browserMissing: extra.browserMissing ?? false,
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
    remote: false,
    browserMissing: false,
  });
  expect(parts.find((part) => part.warning)).toEqual({ text: "deleted proxy", warning: true });
});

test("on Browserbase the region stands where the browser window would, and the location is named", () => {
  const cloud = { remote: true };
  expect(line({ browser: "browserbase" }, browserbaseSettingsFixture, cloud)).toBe(
    "Scroll · Browserbase, Oregon (us-west-2) · no proxy · AI on, about $0.36",
  );
  expect(
    line(
      { browser: "browserbase", region: "eu-central-1", proxy: "browserbaseLocation", locationCountry: "be" },
      browserbaseSettingsFixture,
      cloud,
    ),
  ).toBe("Scroll · Browserbase, Frankfurt (eu-central-1) · Belgium (Browserbase proxy) · AI on, about $0.36");
  expect(
    line(
      {
        browser: "browserbase",
        proxy: "browserbaseLocation",
        locationCountry: "US",
        locationState: "ny",
        locationCity: "new york",
      },
      browserbaseSettingsFixture,
      cloud,
    ),
  ).toMatch(/· New York, NY, United States \(Browserbase proxy\) ·/);
  // The Browserbase default applies, not the one for this Mac.
  const withDefault = {
    ...browserbaseSettingsFixture,
    proxies: [makeProxy()],
    browserbase: {
      ...browserbaseSettingsFixture.browserbase,
      region: "ap-southeast-1" as const,
      defaultProxy: { kind: "profile" as const, id: "proxy-be" },
      persistentProfile: true,
    },
  };
  expect(line({ browser: "browserbase" }, withDefault, cloud)).toBe(
    "Scroll · Browserbase, Singapore (ap-southeast-1) · Belgium proxy · kept browser profile · AI on, about $0.36",
  );
  // The profile setting of this Mac's browser plays no part there.
  expect(
    line({ browser: "browserbase" }, { ...browserbaseSettingsFixture, persistentProfile: true }, cloud),
  ).not.toMatch(/kept browser profile/);
});

test("a Browserbase location without a country, and Browserbase without credentials, are flagged", () => {
  const parts = (extra: { remote: boolean; browserMissing: boolean }, form: Partial<ScanForm>) =>
    scanSummary({
      form: { ...newScanForm(browserbaseSettingsFixture), ...form },
      settings: browserbaseSettingsFixture,
      ai: aiAvailability(browserbaseSettingsFixture),
      budgetUsd: undefined,
      proxyMissing: false,
      ...extra,
    }).filter((part) => part.warning);
  expect(
    parts({ remote: true, browserMissing: false }, { browser: "browserbase", proxy: "browserbaseLocation" }),
  ).toEqual([{ text: "Browserbase location not set", warning: true }]);
  expect(parts({ remote: false, browserMissing: true }, { browser: "browserbase" })).toEqual([
    { text: "Browserbase not set up", warning: true },
  ]);
});
