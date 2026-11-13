import { expect, test } from "vitest";
import type { ScanOptions } from "@/bindings";
import { settingsFixture } from "@/test/fixtures";
import { defaultHeadless, defaultScanOptions, repeatVisitOptions, rescanOptions, visitorOf } from "./scan-options";

const stored: ScanOptions = {
  url: "https://www.example.com/",
  device: "desktop",
  observeSeconds: 20,
  autoScroll: true,
  acceptConsent: true,
  aiEnabled: true,
};

test("scans stored before visitor behaviours only know whether they scrolled", () => {
  expect(visitorOf(stored)).toBe("scroll");
  expect(visitorOf({ ...stored, autoScroll: false })).toBe("none");
  expect(visitorOf({ ...stored, autoScroll: false, visitor: "browse" })).toBe("browse");
});

test("browse and manual visits show the browser by default; the others follow the headless setting", () => {
  expect(defaultHeadless("none", { headless: true })).toBe(true);
  expect(defaultHeadless("scroll", { headless: false })).toBe(false);
  expect(defaultHeadless("browse", { headless: true })).toBe(false);
  expect(defaultHeadless("manual", { headless: true })).toBe(false);
});

test("a scan started outside the composer takes the visitor defaults from the settings", () => {
  const browse = defaultScanOptions(
    { ...settingsFixture, defaultVisitor: "browse", defaultExtraPages: 3 },
    "https://example.com/",
    "mobile",
  );
  expect(browse).toMatchObject({ visitor: "browse", extraPages: 3, autoScroll: true, acceptConsent: true });
  const manual = defaultScanOptions(
    { ...settingsFixture, defaultVisitor: "manual" },
    "https://example.com/",
    "desktop",
  );
  expect(manual).toMatchObject({ visitor: "manual", extraPages: 0, acceptConsent: false });
  expect(
    defaultScanOptions({ ...settingsFixture, defaultVisitor: "none" }, "https://example.com/", "desktop"),
  ).toMatchObject({
    visitor: "none",
    autoScroll: false,
  });
});

test("a re-scan drops the seed; repeating the visit keeps it, and only a scripted visit that acted can be repeated", () => {
  const scripted: ScanOptions = { ...stored, visitor: "browse", extraPages: 2, seed: 4711 };
  expect(rescanOptions(scripted)).not.toHaveProperty("seed");
  expect(rescanOptions(scripted)).toMatchObject({ visitor: "browse", extraPages: 2 });
  expect(repeatVisitOptions({ options: scripted })).toEqual({ ...scripted, browser: { kind: "local" } });
  expect(repeatVisitOptions({ options: { ...scripted, visitor: "scroll" } })?.seed).toBe(4711);
  expect(repeatVisitOptions({ options: { ...scripted, visitor: "manual" } })).toBeNull();
  expect(repeatVisitOptions({ options: { ...scripted, visitor: "none" } })).toBeNull();
  // A scan stored before seeds existed has nothing to replay.
  expect(repeatVisitOptions({ options: stored })).toBeNull();
});

test("a scan that does not say where its browser ran is repeated on this Mac, whatever the settings say today", () => {
  // Stored before Browserbase existed: no `browser`. Sent as it is, the backend would follow `defaultBrowser`.
  const old: ScanOptions = { ...stored, visitor: "scroll", seed: 7 };
  expect(old).not.toHaveProperty("browser");
  expect(rescanOptions(old).browser).toEqual({ kind: "local" });
  expect(repeatVisitOptions({ options: old })?.browser).toEqual({ kind: "local" });
  // A scan that does say keeps what it says.
  const cloud: ScanOptions = { ...old, browser: { kind: "browserbase", region: "eu-central-1" } };
  expect(rescanOptions(cloud).browser).toEqual({ kind: "browserbase", region: "eu-central-1" });
  expect(rescanOptions({ ...old, browser: { kind: "local" } }).browser).toEqual({ kind: "local" });
  expect(repeatVisitOptions({ options: cloud })?.browser).toEqual({ kind: "browserbase", region: "eu-central-1" });
});
