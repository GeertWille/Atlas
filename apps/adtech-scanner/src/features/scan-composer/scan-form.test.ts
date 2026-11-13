import { describe, expect, test } from "vitest";
import type { ScanOptions } from "@/bindings";
import { browserbaseSettingsFixture, settingsFixture } from "@/test/fixtures";
import {
  formFromOptions,
  newScanForm,
  proxyFieldFor,
  runsOnBrowserbase,
  type ScanForm,
  validateScanForm,
  withBrowser,
  withVisitor,
} from "./scan-form";

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
    browser: "local",
    region: "settings",
    proxy: "settings",
    locationCountry: "",
    locationState: "",
    locationCity: "",
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
      // Where the browser runs is always said: a configuration keeps it.
      browser: { kind: "local" },
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
  expect(again).toEqual({
    ok: true,
    options: { ...saved, proxy: { kind: "profile", id: "proxy-be" }, browser: { kind: "local" } },
  });
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

describe("where the browser runs", () => {
  const cloud = form({ browser: "browserbase" });

  test("Browserbase goes out with its region only when the scan names one, and never with a window", () => {
    expect(optionsOf(cloud)).toMatchObject({ browser: { kind: "browserbase" }, headless: true });
    expect(optionsOf({ ...cloud, region: "eu-central-1", showBrowser: true })).toMatchObject({
      browser: { kind: "browserbase", region: "eu-central-1" },
      headless: true,
    });
    // A manual visit has no window either: it is used through the live view.
    expect(optionsOf({ ...cloud, visitor: "manual" })).toMatchObject({ headless: true, acceptConsent: false });
  });

  test.each([
    "http://localhost:3000/",
    "http://127.0.0.1:47615/demo-publisher/",
    "http://192.168.1.20/",
    "http://printer.local/",
  ])("%s is scanned on this Mac whatever was chosen", (url) => {
    expect(optionsOf({ ...cloud, url, region: "eu-central-1" }).browser).toEqual({ kind: "local" });
    expect(runsOnBrowserbase(cloud, url)).toBe(false);
  });

  test("a Browserbase location goes out as Browserbase takes it, and only with a Browserbase browser", () => {
    const location = {
      proxy: "browserbaseLocation",
      locationCountry: "us",
      locationState: "ny",
      locationCity: "new york",
    };
    expect(optionsOf({ ...cloud, ...location } as Partial<ScanForm>).proxy).toEqual({
      kind: "browserbaseLocation",
      country: "US",
      state: "NY",
      city: "NEW_YORK",
    });
    expect(optionsOf({ ...location, browser: "local" } as Partial<ScanForm>).proxy).toBeUndefined();
    expect(validateScanForm({ ...cloud, proxy: "browserbaseLocation" })).toEqual({
      ok: false,
      errors: {
        url: undefined,
        budget: undefined,
        location: { field: "country", message: "Enter the country as a two-letter code, for example BE." },
      },
    });
  });

  test("this Mac's own proxy is not sent for a cloud browser", () => {
    expect(optionsOf({ ...cloud, proxy: "system" }).proxy).toBeUndefined();
    expect(optionsOf({ ...cloud, proxy: "none" }).proxy).toEqual({ kind: "none" });
    expect(optionsOf({ ...cloud, proxy: "profile:proxy-be" }).proxy).toEqual({ kind: "profile", id: "proxy-be" });
  });

  test("moving the browser changes nothing else: a proxy that cannot go along is masked, not forgotten", () => {
    const system = form({ proxy: "system" });
    const moved = withBrowser(system, "browserbase");
    expect(moved).toEqual({ ...system, browser: "browserbase" });
    // On Browserbase the settings hold, and nothing of this Mac's proxy is sent.
    expect(proxyFieldFor(moved, true)).toBe("settings");
    expect(optionsOf(moved).proxy).toBeUndefined();
    // Back on this Mac the choice is what it was.
    const back = withBrowser(moved, "local");
    expect(back).toEqual(system);
    expect(optionsOf(back).proxy).toEqual({ kind: "system" });

    const located = { ...cloud, proxy: "browserbaseLocation", locationCountry: "BE" } as const;
    const local = withBrowser(located, "local");
    expect(local).toEqual({ ...located, browser: "local" });
    expect(proxyFieldFor(local, false)).toBe("settings");
    expect(optionsOf(local).proxy).toBeUndefined();
    expect(optionsOf(withBrowser(local, "browserbase")).proxy).toEqual({ kind: "browserbaseLocation", country: "BE" });

    expect(withBrowser(form({ proxy: "profile:proxy-be" }), "browserbase").proxy).toBe("profile:proxy-be");
  });

  test("a new form follows the settings default only while Browserbase is set up", () => {
    const cloudDefault = { ...browserbaseSettingsFixture, defaultBrowser: "browserbase" as const };
    expect(newScanForm(cloudDefault).browser).toBe("browserbase");
    expect(newScanForm({ ...settingsFixture, defaultBrowser: "browserbase" }).browser).toBe("local");
  });

  test("saved Browserbase options open as the form that produces them again", () => {
    const saved: ScanOptions = {
      url: "https://example.com/news",
      device: "desktop",
      observeSeconds: 20,
      autoScroll: true,
      acceptConsent: true,
      aiEnabled: true,
      visitor: "scroll",
      extraPages: 0,
      headless: true,
      proxy: { kind: "browserbaseLocation", country: "US", state: "NY", city: "NEW_YORK" },
      browser: { kind: "browserbase", region: "us-east-1" },
    };
    const opened = formFromOptions(saved, browserbaseSettingsFixture);
    expect(opened).toMatchObject({
      browser: "browserbase",
      region: "us-east-1",
      proxy: "browserbaseLocation",
      locationCountry: "US",
      locationState: "NY",
      locationCity: "New York",
    });
    expect(validateScanForm(opened)).toEqual({ ok: true, options: saved });
    // Options that do not say where they run open where the settings put them.
    const { browser: _browser, proxy: _proxy, ...unsaid } = saved;
    expect(formFromOptions(unsaid, { ...browserbaseSettingsFixture, defaultBrowser: "browserbase" }).browser).toBe(
      "browserbase",
    );
    expect(formFromOptions(unsaid, browserbaseSettingsFixture).browser).toBe("local");
  });
});

test("a proxy that cannot go where the browser runs is left to the settings without being forgotten", () => {
  const location = form({ browser: "browserbase", proxy: "browserbaseLocation", locationCountry: "BE" });
  expect(proxyFieldFor(location, true)).toBe("browserbaseLocation");
  // A local address moves the scan to this Mac while Browserbase stays chosen.
  expect(proxyFieldFor(location, false)).toBe("settings");
  expect(optionsOf({ ...location, url: "localhost:3000" })).not.toHaveProperty("proxy");
  expect(location.proxy).toBe("browserbaseLocation");
  expect(proxyFieldFor(form({ proxy: "system" }), true)).toBe("settings");
  expect(proxyFieldFor(form({ proxy: "profile:proxy-be" }), true)).toBe("profile:proxy-be");
});
