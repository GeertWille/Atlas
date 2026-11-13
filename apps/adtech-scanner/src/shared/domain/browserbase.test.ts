import { describe, expect, test } from "vitest";
import { browserbaseSettingsFixture, settingsFixture } from "@/test/fixtures";
import {
  BROWSERBASE_DISCLOSURE,
  BROWSERBASE_LIST_PRICE,
  BROWSERBASE_PLAN_CAVEAT,
  BROWSERBASE_RATES,
  BROWSERBASE_REGION_META,
  BROWSERBASE_REGIONS,
  browserbaseUsd,
  browserKindOf,
  countryName,
  estimateBrowserbaseUsage,
  formatTraffic,
  locationFromRecord,
  locationLabel,
  parseLocation,
  reachableFromCloud,
  regionName,
  regionOptionLabel,
  regionPlace,
  webdriverReading,
} from "./browserbase";

test("the four regions are named by place with their id, and each has the timezone the scanner sets there", () => {
  // One name everywhere the app writes a region: the place people know, and the id the backend's lines use.
  expect(BROWSERBASE_REGIONS.map(regionName)).toEqual([
    "Oregon (us-west-2)",
    "Virginia (us-east-1)",
    "Frankfurt (eu-central-1)",
    "Singapore (ap-southeast-1)",
  ]);
  // The place alone is for the two narrow spots that have the whole name beside them.
  expect(BROWSERBASE_REGIONS.map(regionPlace)).toEqual(["Oregon", "Virginia", "Frankfurt", "Singapore"]);
  expect(BROWSERBASE_REGIONS.map((region) => BROWSERBASE_REGION_META[region].timezone)).toEqual([
    "America/Los_Angeles",
    "America/New_York",
    "Europe/Berlin",
    "Asia/Singapore",
  ]);
  expect(regionOptionLabel("eu-central-1")).toBe("Frankfurt, Germany (eu-central-1)");
  // A city state is not named twice.
  expect(regionOptionLabel("ap-southeast-1")).toBe("Singapore (ap-southeast-1)");
});

test("the disclosure is the owner's wording, with what was found about Browserbase's own scripts", () => {
  expect(BROWSERBASE_DISCLOSURE).toBe(
    "With Browserbase, the browser runs in Browserbase's cloud, not on this Mac. The pages you scan, their cookies, and everything the scanner reads from them (scripts, requests, bid data) travel through Browserbase, a third party. Browserbase keeps a video of each session and a log that includes the content the scanner reads and the cookies it reads or sets. CAPTCHAs on scanned pages are solved automatically, and the browser does not tell the page it is automated. To do this, Browserbase runs its own scripts inside every page you scan; the scanner leaves them out of its results, but they can change how a page behaves. Results are also stored on this Mac. Browserbase bills browser time per minute, and proxy traffic per megabyte.",
  );
});

test.each([
  ["https://www.example.com/", true],
  ["http://pub.adscan-fixture.example:8123/", true],
  ["http://8.8.8.8/", true],
  ["http://[2001:db8::1]/", true],
  ["http://localhost:3000/", false],
  ["http://127.0.0.1:47615/demo-publisher/", false],
  ["http://192.168.1.10/", false],
  ["http://10.0.0.5:8080/", false],
  ["http://[::1]/", false],
  ["http://printer.local/", false],
  ["http://app.localhost/", false],
  // What only the backend's rule knows on top of private addresses.
  ["http://intranet/", false],
  ["http://wiki.internal/", false],
  ["http://router.home.arpa/", false],
  ["http://255.255.255.255/", false],
  ["not a url", false],
  // The backend's own cases (`domain/reach.rs`): the composer must not offer what a start would refuse.
  ["http://172.16.0.9/", false],
  ["http://169.254.10.1/", false],
  ["http://100.64.0.1/", false],
  ["http://0.0.0.0/", false],
  ["http://0.1.2.3/", false],
  ["http://224.0.0.251/", false],
  ["http://239.255.255.250/", false],
  ["http://[fd00::1]/", false],
  ["http://[fe80::1]/", false],
  ["http://[ff02::1]/", false],
  ["http://[::]/", false],
  ["http://[::ffff:192.168.1.20]/", false],
  ["http://[::ffff:224.0.0.1]/", false],
  ["http://[::127.0.0.1]:8080/admin?token=1", false],
  ["http://[::10.0.0.5]/", false],
  ["http://[64:ff9b::7f00:1]/", false],
  ["http://[64:ff9b::10.0.0.5]/", false],
  ["http://[64:ff9b:1::1]/", false],
  ["http://[2002:c0a8:0114::1]/", false],
  ["http://[2002:7f00:1::1]/", false],
  ["http://[2002:a9fe:a01::1]/", false],
  // Numbers a browser reads as an IPv4 address.
  ["http://0x7f.1/", false],
  ["http://2130706433/", false],
  // A public address is as public wrapped in IPv6 as it was.
  ["http://[::8.8.8.8]/", true],
  ["http://[64:ff9b::8.8.8.8]/", true],
  ["http://[2002:808:808::1]/", true],
  ["http://[::ffff:8.8.8.8]/", true],
  ["http://[2606:4700:4700::1111]/", true],
  ["http://172.32.0.1/", true],
  ["http://100.128.0.1/", true],
  ["http://223.255.255.1/", true],
  ["https://localhost.example.com/", true],
  ["https://local.example/", true],
])("reachableFromCloud(%s) is %s", (url, expected) => {
  expect(reachableFromCloud(url)).toBe(expected);
});

describe("where a scan runs", () => {
  const cloudDefault = { ...browserbaseSettingsFixture, defaultBrowser: "browserbase" as const };
  const url = "https://www.example.com/";

  test("what the options say wins", () => {
    expect(browserKindOf({ url, browser: { kind: "local" } }, cloudDefault)).toBe("local");
    expect(browserKindOf({ url, browser: { kind: "browserbase" } }, settingsFixture)).toBe("browserbase");
  });

  test("options that do not say follow the settings, but never to Browserbase without credentials or for a local address", () => {
    expect(browserKindOf({ url }, settingsFixture)).toBe("local");
    expect(browserKindOf({ url }, browserbaseSettingsFixture)).toBe("local");
    expect(browserKindOf({ url }, cloudDefault)).toBe("browserbase");
    expect(browserKindOf({ url: "http://127.0.0.1:47615/demo-publisher/" }, cloudDefault)).toBe("local");
    expect(browserKindOf({ url }, { ...settingsFixture, defaultBrowser: "browserbase" })).toBe("local");
  });
});

describe("a Browserbase location", () => {
  test("is stored as Browserbase takes it: upper case, a city with underscores", () => {
    expect(parseLocation({ country: " be ", state: "", city: "" })).toEqual({
      ok: true,
      location: { kind: "browserbaseLocation", country: "BE" },
    });
    expect(parseLocation({ country: "us", state: "ny", city: " new  york " })).toEqual({
      ok: true,
      location: { kind: "browserbaseLocation", country: "US", state: "NY", city: "NEW_YORK" },
    });
  });

  test.each([
    [{ country: "", state: "", city: "" }, "country", "Enter the country as a two-letter code, for example BE."],
    [{ country: "BEL", state: "", city: "" }, "country", "Enter the country as a two-letter code, for example BE."],
    [{ country: "US", state: "New York", city: "" }, "state", "Enter the state as a two-letter code, for example NY."],
    [{ country: "US", state: "", city: "R2D2" }, "city", "Enter the city in letters, at most 60 characters."],
    [{ country: "US", state: "", city: "a".repeat(61) }, "city", "Enter the city in letters, at most 60 characters."],
  ])("%o is refused at its %s", (fields, field, error) => {
    expect(parseLocation(fields)).toEqual({ ok: false, error: { field, message: error } });
  });

  test("is named by country, most specific part first", () => {
    expect(countryName("be")).toBe("Belgium");
    expect(countryName("ZZZ")).toBe("ZZZ");
    expect(locationLabel({ country: "BE" })).toBe("Belgium");
    expect(locationLabel({ country: "US", state: "NY", city: "NEW_YORK" })).toBe("New York, NY, United States");
    expect(locationLabel({ country: "DE", city: "FRANKFURT_AM_MAIN" })).toBe("Frankfurt Am Main, Germany");
  });

  test("reads back from the name a finished scan records", () => {
    expect(locationFromRecord("BE")).toEqual({ country: "BE", state: undefined, city: undefined });
    expect(locationFromRecord("US, NY, NEW_YORK")).toEqual({ country: "US", state: "NY", city: "NEW_YORK" });
    expect(locationFromRecord("DE, BERLIN")).toEqual({ country: "DE", state: undefined, city: "BERLIN" });
  });
});

describe("what a scan is likely to use at Browserbase", () => {
  const scan = { visitor: "scroll", pages: 1, observeSeconds: 20, browserbaseProxy: false } as const;

  test("a short visit is one billed minute and no proxy traffic", () => {
    expect(estimateBrowserbaseUsage(scan)).toEqual({
      minutes: 1,
      upTo: false,
      proxyMb: 0,
      proxyPerPage: false,
      proxyUsd: 0,
      usd: BROWSERBASE_RATES.browserUsdPerHour / 60,
    });
  });

  test("every started minute counts, and each page view adds its time", () => {
    // 10 s to start, then 3 pages of 8 s loading and 60 s watching.
    expect(estimateBrowserbaseUsage({ ...scan, visitor: "browse", pages: 3, observeSeconds: 60 }).minutes).toBe(4);
    expect(estimateBrowserbaseUsage({ ...scan, observeSeconds: 45 }).minutes).toBe(2);
  });

  test("a Browserbase location is what costs: 18 MB per page view at $12 per GB", () => {
    const estimate = estimateBrowserbaseUsage({ ...scan, visitor: "browse", pages: 3, browserbaseProxy: true });
    expect(estimate.proxyMb).toBe(54);
    expect(estimate.proxyUsd).toBeCloseTo(0.648, 6);
    expect(estimate.usd).toBeCloseTo(0.648 + (2 / 60) * 0.12, 6);
  });

  test("a manual visit is estimated at its limit, with proxy traffic per page", () => {
    expect(estimateBrowserbaseUsage({ ...scan, visitor: "manual", browserbaseProxy: true })).toMatchObject({
      minutes: 15,
      upTo: true,
      proxyMb: 18,
      proxyPerPage: true,
    });
  });

  test("actual usage is priced the same way: started minutes and started megabytes", () => {
    expect(browserbaseUsd(27, 18_486_000)).toBeCloseTo(0.002 + 0.228, 6);
    expect(browserbaseUsd(61, 0)).toBeCloseTo(0.004, 6);
    expect(browserbaseUsd(0, 0)).toBeCloseTo(0.002, 6);
  });

  test("the prices are Browserbase's list price with the day it was read, and name no plan", () => {
    expect(BROWSERBASE_RATES).toEqual({
      asOf: "2026-11-12",
      browserUsdPerHour: 0.12,
      proxyUsdPerGb: 12,
    });
    expect(BROWSERBASE_LIST_PRICE).toBe(
      "Browserbase's list price of $0.12 per browser hour and $12 per GB (2026-11-12)",
    );
    expect(BROWSERBASE_PLAN_CAVEAT).toBe("your plan's included usage is not counted");
    expect(`${BROWSERBASE_LIST_PRICE} ${BROWSERBASE_PLAN_CAVEAT}`).not.toMatch(/Developer|Startup|overage/i);
  });
});

test("what navigator.webdriver read is said the same way for one blank page and for the pages of a visit", () => {
  expect(webdriverReading(false, "the page is")).toBe("false: the page is not told the visit is automated");
  expect(webdriverReading(true, "the page is")).toBe("true: the page is told the visit is automated");
  expect(webdriverReading(false, "the pages were")).toBe("false: the pages were not told the visit is automated");
});

test("proxy traffic is written in decimal megabytes, as Browserbase bills it", () => {
  expect(formatTraffic(0)).toBe("0 MB");
  expect(formatTraffic(131_844)).toBe("0.1 MB");
  expect(formatTraffic(18_486_000)).toBe("18 MB");
  expect(formatTraffic(1_998_349_077)).toBe("2.00 GB");
});
