import { describe, expect, test } from "vitest";
import type { ScanOptions } from "@/bindings";
import { settingsFixture } from "@/test/fixtures";
import { formFromOptions, newScanForm, type ScanForm, validateScanForm, withVisitor } from "./scan-form";

const form = (patch: Partial<ScanForm> = {}): ScanForm => ({
  ...newScanForm(settingsFixture),
  url: "example.com/news",
  ...patch,
});
const optionsOf = (patch: Partial<ScanForm>) => {
  const result = validateScanForm(form(patch));
  if (!result.ok) throw new Error(JSON.stringify(result.errors));
  return result.options;
};

test("a new form starts from the settings and leaves proxy, profile and budget to them", () => {
  expect(
    newScanForm({ ...settingsFixture, defaultVisitor: "browse", defaultExtraPages: 4, defaultDevice: "mobile" }),
  ).toEqual({
    url: "",
    device: "mobile",
    observeSeconds: 20,
    visitor: "browse",
    pages: 5,
    // A browse visit is shown even though the settings run headless.
    showBrowser: true,
    acceptConsent: true,
    aiEnabled: true,
    proxy: "settings",
    profile: "settings",
    budget: "",
  });
});

describe("the options each visitor behaviour produces", () => {
  test("observe only: no scrolling, no extra pages, headless as chosen", () => {
    expect(optionsOf({ visitor: "none", showBrowser: false })).toEqual({
      url: "https://example.com/news",
      device: "desktop",
      observeSeconds: 20,
      autoScroll: false,
      acceptConsent: true,
      aiEnabled: true,
      visitor: "none",
      extraPages: 0,
      headless: true,
    });
  });

  test("scroll: the old auto-scroll, with the page count ignored", () => {
    expect(optionsOf({ visitor: "scroll", pages: 4, showBrowser: true })).toMatchObject({
      visitor: "scroll",
      autoScroll: true,
      extraPages: 0,
      headless: false,
    });
  });

  test("browse: the pages to visit become extra pages after the entry page", () => {
    expect(optionsOf({ visitor: "browse", pages: 3, showBrowser: true })).toMatchObject({
      visitor: "browse",
      autoScroll: true,
      extraPages: 2,
      headless: false,
    });
    expect(optionsOf({ visitor: "browse", pages: 1 }).extraPages).toBe(0);
    // The stepper stops at six pages; a form that somehow holds more is clamped, not sent.
    expect(optionsOf({ visitor: "browse", pages: 9 }).extraPages).toBe(5);
  });

  test("manual: always a window, never a consent click, whatever the hidden switches say", () => {
    expect(optionsOf({ visitor: "manual", showBrowser: false, acceptConsent: true, pages: 4 })).toMatchObject({
      visitor: "manual",
      headless: false,
      acceptConsent: false,
      extraPages: 0,
      autoScroll: true,
    });
  });
});

test("proxy, profile and budget are only sent when the scan sets them", () => {
  expect(optionsOf({})).not.toHaveProperty("proxy");
  expect(optionsOf({})).not.toHaveProperty("persistentProfile");
  expect(optionsOf({})).not.toHaveProperty("budgetUsd");
  expect(optionsOf({ proxy: "none", profile: "kept", budget: " $1.25 " })).toMatchObject({
    proxy: { kind: "none" },
    persistentProfile: true,
    budgetUsd: 1.25,
  });
  expect(optionsOf({ proxy: "profile:proxy-be", profile: "fresh" })).toMatchObject({
    proxy: { kind: "profile", id: "proxy-be" },
    persistentProfile: false,
  });
});

test("a bad URL and a budget outside the allowed range are reported together", () => {
  expect(validateScanForm(form({ url: "", budget: "0.01" }))).toEqual({
    ok: false,
    errors: { url: "Enter the address of a page to scan.", budget: "Enter an amount from $0.05 to $20.00." },
  });
  expect(validateScanForm(form({ budget: "lots" }))).toMatchObject({ ok: false, errors: { url: undefined } });
  expect(validateScanForm(form({ budget: "20" })).ok).toBe(true);
  expect(validateScanForm(form({ budget: "20.01" })).ok).toBe(false);
});

test("changing the behaviour resets the browser window to that behaviour's default", () => {
  const start = form({ visitor: "scroll", showBrowser: false });
  expect(withVisitor(start, "browse", settingsFixture)).toMatchObject({ visitor: "browse", showBrowser: true });
  expect(withVisitor({ ...start, showBrowser: true }, "none", settingsFixture).showBrowser).toBe(false);
  expect(withVisitor(start, "none", { headless: false }).showBrowser).toBe(true);
});

test("saved options open as the form that produces them again", () => {
  const saved: ScanOptions = {
    url: "https://example.com/news",
    device: "mobile",
    observeSeconds: 35,
    autoScroll: true,
    acceptConsent: false,
    aiEnabled: false,
    visitor: "browse",
    extraPages: 4,
    headless: true,
    proxy: { kind: "profile", id: "proxy-be", name: "Belgium", host: "be.proxy.example" },
    persistentProfile: true,
    budgetUsd: 2,
  };
  const opened = formFromOptions(saved, settingsFixture);
  expect(opened).toMatchObject({
    visitor: "browse",
    pages: 5,
    showBrowser: false,
    proxy: "profile:proxy-be",
    profile: "kept",
    budget: "2.00",
  });
  const again = validateScanForm(opened);
  // The proxy goes back as a bare reference: its name and host are the backend's snapshot.
  expect(again).toEqual({ ok: true, options: { ...saved, proxy: { kind: "profile", id: "proxy-be" } } });
});

test("options stored before visitor behaviours open as scroll or observe-only with the settings' defaults", () => {
  const legacy: ScanOptions = {
    url: "https://example.com/",
    device: "desktop",
    observeSeconds: 20,
    autoScroll: false,
    acceptConsent: true,
    aiEnabled: true,
  };
  expect(formFromOptions(legacy, settingsFixture)).toMatchObject({
    visitor: "none",
    pages: 3,
    showBrowser: false,
    proxy: "settings",
    profile: "settings",
    budget: "",
  });
});
