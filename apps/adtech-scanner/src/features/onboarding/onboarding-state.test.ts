import { expect, test } from "vitest";
import type { BrowserInfo } from "@/bindings";
import { browserbaseSettingsFixture, settingsFixture } from "@/test/fixtures";
import { needsOnboarding } from "./onboarding-state";

const BROWSER: BrowserInfo = { path: "/usr/bin/chromium", version: "Chromium 141", name: "Chromium" };
const NO_KEYS = { hasApiKey: false, hasTypesafeKey: false, browserbase: settingsFixture.browserbase };

test("setup is offered until a key of either provider is stored", () => {
  expect(needsOnboarding(BROWSER, NO_KEYS, false)).toBe(true);
  expect(needsOnboarding(BROWSER, { ...NO_KEYS, hasApiKey: true }, false)).toBe(false);
  // A Jev-only user is not asked for an Anthropic key on every launch.
  expect(needsOnboarding(BROWSER, { ...NO_KEYS, hasTypesafeKey: true }, false)).toBe(false);
});

test("a missing browser brings setup back whatever keys are stored, until it is dismissed", () => {
  const bothKeys = { ...NO_KEYS, hasApiKey: true, hasTypesafeKey: true };
  expect(needsOnboarding(null, bothKeys, false)).toBe(true);
  expect(needsOnboarding(null, bothKeys, true)).toBe(false);
  expect(needsOnboarding(BROWSER, NO_KEYS, true)).toBe(false);
});

test("Browserbase stands in for a missing local browser", () => {
  const cloud = { hasApiKey: true, hasTypesafeKey: false, browserbase: browserbaseSettingsFixture.browserbase };
  expect(needsOnboarding(null, cloud, false)).toBe(false);
  // The model key is still asked for once.
  expect(needsOnboarding(null, { ...cloud, hasApiKey: false }, false)).toBe(true);
});
