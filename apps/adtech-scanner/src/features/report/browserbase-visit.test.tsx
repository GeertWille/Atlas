import { screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, test } from "vitest";
import type { BrowserUse, LogEntry, ScanDetail } from "@/bindings";
import { makeDetail, makeEnvironment, makeLog, makeSummary } from "@/test/fixtures";
import { renderWithApp } from "@/test/render";
import { BrowserbaseVisitDetails } from "./BrowserbaseVisitDetails";
import { browserbaseLogFacts, browserbaseVisitFacts, proxyUsagePending } from "./browserbase-visit";
import { EnvironmentNotice } from "./EnvironmentNotice";
import { environmentNotice } from "./environment-notice";
import { ReportOverview } from "./ReportOverview";
import { visitFacts } from "./visit-summary";

type Visit = Extract<BrowserUse, { kind: "browserbase" }>;

const FINISHED = 1_700_000_040_000;

function frankfurt(overrides: Partial<Visit> = {}): Visit {
  return {
    kind: "browserbase",
    region: "eu-central-1",
    sessionId: "d634f35c-3c1e-4f7a-9b2d-d634f35ca41c",
    browserVersion: "Chrome/157.0.8103.79",
    userAgent: "Mozilla/5.0 (X11; Linux x86_64) Chrome/157.0.0.0 Safari/537.36",
    proxyKind: "none",
    proxyCountry: null,
    settings: { solveCaptchas: true, recordSession: true, logSession: true, blockAds: false, keepAlive: false },
    originalTimezone: "America/Los_Angeles",
    appliedTimezone: "Europe/Berlin",
    language: "en-US",
    roundTripMs: 118.6,
    sessionSeconds: 41,
    proxyBytes: 0,
    ...overrides,
  };
}

const belgium = (overrides: Partial<Visit> = {}) =>
  frankfurt({
    proxyKind: "browserbase",
    proxyCountry: "BE",
    originalTimezone: "Europe/Brussels",
    appliedTimezone: null,
    proxyBytes: 18_486_000,
    ...overrides,
  });

function scanOn(browser: Visit, summary: Partial<ScanDetail["summary"]> = {}): ScanDetail {
  return makeDetail({
    summary: makeSummary({
      finishedAt: FINISHED,
      remote: { region: browser.region, sessionSeconds: browser.sessionSeconds, proxyBytes: browser.proxyBytes },
      ...summary,
    }),
    environment: makeEnvironment({
      webdriver: false,
      headless: true,
      browser,
      proxy:
        browser.proxyKind === "browserbase"
          ? { kind: "browserbaseLocation", name: browser.proxyCountry, host: null }
          : { kind: "none", name: null, host: null },
    }),
  });
}

let lines = 0;
/** A line of the capture about its Browserbase browser, as the audit trail stores it. */
function browserLine(event: string, detail: Record<string, unknown> = {}): LogEntry {
  lines += 1;
  return makeLog({
    id: `browser-${lines}`,
    kind: "visitor",
    data: { activity: "browser", page: null, detail: { event, ...detail } },
  });
}

const rowsOf = (detail: ScanDetail, now = FINISHED + 600_000) =>
  Object.fromEntries((browserbaseVisitFacts(detail, now)?.rows ?? []).map((row) => [row.name, row.value]));

test("a visit on this Mac, and a scan without a recorded environment, have no Browserbase facts", () => {
  expect(browserbaseVisitFacts(makeDetail(), FINISHED)).toBeNull();
  expect(browserbaseVisitFacts(makeDetail({ environment: makeEnvironment() }), FINISHED)).toBeNull();
});

test("a visit from the region says where it ran, what it used and what the pages could read", () => {
  const detail = scanOn(frankfurt());
  expect(browserbaseVisitFacts(detail, FINISHED + 600_000)?.summary).toEqual([
    "Browserbase",
    "41.0 s of browser time",
    "about $0.002 at list price",
  ]);
  expect(rowsOf(detail)).toEqual({
    "Browser ran in": "Browserbase, Frankfurt, Germany (eu-central-1)",
    "Left from": "Browserbase's Frankfurt (eu-central-1) region, a data centre address (no proxy)",
    "Browser time": "41.0 s, billed as 1 minute",
    "Proxy traffic": "None: the visit used no Browserbase proxy",
    "Estimated cost":
      "$0.002, an estimate at Browserbase's list price of $0.12 per browser hour and $12 per GB (2026-11-12); your plan's included usage is not counted. Not part of the LLM cost",
    "Timezone the pages saw":
      "Europe/Berlin, set by the scanner to match the region (the browser reported America/Los_Angeles)",
    Language: "en-US, as the browser reported it",
    "navigator.webdriver": "false: the pages were not told the visit is automated",
    "User agent": "Mozilla/5.0 (X11; Linux x86_64) Chrome/157.0.0.0 Safari/537.36",
    Browser: "Chrome/157.0.8103.79, headless, which the user agent does not show",
    "Browserbase's own scripts": "Ran in every page; left out of this scan's scripts and results",
    CAPTCHAs: "Solved by Browserbase automatically",
    "Round trip":
      "119 ms per command between this Mac and the browser; frames and requests the scanner held for a moment waited that long",
    "Kept at Browserbase": "A video and a log",
    Session: "d634f35c-3c1e-4f7a-9b2d-d634f35ca41c",
  });
});

test("a Browserbase location names the country asked for, its traffic and the timezone Browserbase set", () => {
  const detail = scanOn(belgium({ sessionSeconds: 95 }));
  expect(browserbaseVisitFacts(detail, FINISHED + 600_000)?.summary).toEqual([
    "Browserbase",
    "1m 35s of browser time",
    "18 MB through the proxy",
    "about $0.23 at list price",
  ]);
  expect(rowsOf(detail)).toMatchObject({
    "Left from": "Browserbase proxy: a residential connection, Belgium requested (best effort)",
    "Browser time": "1m 35s, billed as 2 minutes",
    "Proxy traffic": "18 MB",
    "Timezone the pages saw": "Europe/Brussels, set by Browserbase for its proxy",
  });
});

test("what could not be read is said plainly, and the user's own proxy is named", () => {
  const detail = scanOn(
    frankfurt({
      proxyKind: "external",
      browserVersion: null,
      userAgent: null,
      originalTimezone: null,
      appliedTimezone: null,
      language: null,
      roundTripMs: null,
      sessionSeconds: null,
      settings: { solveCaptchas: true, recordSession: false, logSession: false, blockAds: false, keepAlive: false },
    }),
  );
  const environment = detail.environment && {
    ...detail.environment,
    webdriver: null,
    proxy: { kind: "profile" as const, name: "Belgium", host: "be.proxy.example" },
  };
  const rows = rowsOf({ ...detail, environment });
  expect(rows).toMatchObject({
    "Left from": "Your proxy “Belgium”, through Browserbase",
    "Browser time": "Not reported by Browserbase",
    "Estimated cost": "Not known: Browserbase did not report the browser time",
    "Timezone the pages saw": "Not read",
    Language: "Not read",
    "navigator.webdriver": "Not read",
    "User agent": "Not recorded",
    Browser: "Version not reported, headless, which the user agent does not show",
    "Kept at Browserbase": "Nothing",
  });
  expect(rows).not.toHaveProperty("Round trip");
  // No dollar figure is made up for a time that was never reported.
  expect(browserbaseVisitFacts({ ...detail, environment }, FINISHED + 600_000)?.summary).toEqual(["Browserbase"]);
});

test("no figure names a plan: every one is an estimate at the list price that counts no included usage", () => {
  const facts = browserbaseVisitFacts(scanOn(belgium()), FINISHED + 600_000);
  const text = [...(facts?.summary ?? []), ...(facts?.rows ?? []).map((row) => row.value)].join(" ");
  expect(text).not.toMatch(/Developer|Startup|overage/i);
  expect(text).toContain("your plan's included usage is not counted");
});

test("proxy traffic is still being counted until the second read, and never for a visit without a Browserbase proxy", () => {
  const proxied = { proxyKind: "browserbase" } as const;
  expect(proxyUsagePending({ proxyKind: "none" }, { proxyBytes: 0 }, FINISHED, FINISHED)).toBe(false);
  expect(proxyUsagePending({ proxyKind: "external" }, null, FINISHED, FINISHED)).toBe(false);
  expect(proxyUsagePending(proxied, null, FINISHED, FINISHED)).toBe(true);
  expect(proxyUsagePending(proxied, { proxyBytes: null }, FINISHED, FINISHED + 5_000)).toBe(true);
  // A count that never comes is not "still being counted" for ever: the wait ends with the second read.
  expect(proxyUsagePending(proxied, { proxyBytes: null }, FINISHED, FINISHED + 61_000)).toBe(false);
  expect(proxyUsagePending(proxied, null, FINISHED, FINISHED + 600_000)).toBe(false);
  // A scan that has not finished has not been read at all.
  expect(proxyUsagePending(proxied, null, null, FINISHED + 600_000)).toBe(true);
  // The first read after the end comes too early: a zero only counts once the second read is overdue.
  expect(proxyUsagePending(proxied, { proxyBytes: 0 }, FINISHED, FINISHED + 5_000)).toBe(true);
  expect(proxyUsagePending(proxied, { proxyBytes: 0 }, FINISHED, FINISHED + 61_000)).toBe(false);
  expect(proxyUsagePending(proxied, { proxyBytes: 82_715 }, FINISHED, FINISHED + 5_000)).toBe(false);

  const early = scanOn(belgium({ proxyBytes: 0 }));
  const facts = browserbaseVisitFacts(early, FINISHED + 5_000);
  expect(facts?.pending).toBe(true);
  expect(facts?.summary).toEqual(["Browserbase", "41.0 s of browser time", "proxy traffic still being counted"]);
  expect(rowsOf(early, FINISHED + 5_000)).toMatchObject({
    "Proxy traffic": "Still being counted by Browserbase",
    "Estimated cost": "Known once the proxy traffic is counted",
  });
});

test("a browser whose end was never confirmed: nothing is counted for ever, no cost is invented, and the bill is warned about", () => {
  // No `Released` came: Browserbase reported neither the time nor the traffic.
  const detail = scanOn(belgium({ sessionSeconds: null, proxyBytes: null }));
  const unconfirmed = browserbaseLogFacts([
    browserLine("sessionCreated", { region: "eu-central-1" }),
    browserLine("releaseUnconfirmed"),
  ]);
  expect(unconfirmed.releaseUnconfirmed).toBe(true);

  // Just after the scan the traffic may still come.
  expect(browserbaseVisitFacts(detail, FINISHED + 5_000, unconfirmed)).toMatchObject({ pending: true });
  // A minute on it has not, and the report says that instead of counting on.
  const late = browserbaseVisitFacts(detail, FINISHED + 61_000, unconfirmed);
  expect(late?.pending).toBe(false);
  expect(late?.summary).toEqual(["Browserbase", "proxy traffic not reported"]);
  expect(rowsOf(detail, FINISHED + 61_000)).toMatchObject({
    "Browser time": "Not reported by Browserbase",
    "Proxy traffic": "Not reported by Browserbase",
    "Estimated cost": "Not known: Browserbase did not report the browser time",
  });
  expect(late?.warning).toBe(
    "Browserbase did not confirm that this browser ended. It may have kept running, and been billed, until its time limit.",
  );

  // Known time, unknown traffic: still no dollar figure.
  const noTraffic = scanOn(belgium({ proxyBytes: null }));
  expect(rowsOf(noTraffic, FINISHED + 61_000)["Estimated cost"]).toBe(
    "Not known: Browserbase did not report the proxy traffic",
  );
  // An end that was confirmed afterwards, and a scan without that line, carry no warning.
  expect(
    browserbaseLogFacts([browserLine("releaseUnconfirmed"), browserLine("released", { sessionSeconds: 41 })])
      .releaseUnconfirmed,
  ).toBe(false);
  expect(browserbaseVisitFacts(scanOn(belgium()), FINISHED + 61_000)?.warning).toBeNull();
});

test("Browserbase's own scripts are counted from the capture's line, and said without a number when there is none", () => {
  expect(browserbaseLogFacts(undefined)).toEqual({ serviceScripts: null, releaseUnconfirmed: false });
  expect(
    browserbaseLogFacts([makeLog({ message: "Navigating to https://www.example.com/" })]).serviceScripts,
  ).toBeNull();
  const log = browserbaseLogFacts([browserLine("serviceScripts", { count: 5, characters: 271_667 })]);
  expect(log.serviceScripts).toBe(5);
  const rows = Object.fromEntries(
    (browserbaseVisitFacts(scanOn(frankfurt()), FINISHED + 600_000, log)?.rows ?? []).map((row) => [
      row.name,
      row.value,
    ]),
  );
  expect(rows["Browserbase's own scripts"]).toBe(
    "5 scripts ran in the pages; left out of this scan's scripts and results",
  );
});

test("usage is read from the summary, which the late second read updates", () => {
  // The environment still holds the first read; the summary has the second.
  const detail = scanOn(belgium({ proxyBytes: 0 }), {
    remote: { region: "eu-central-1", sessionSeconds: 41, proxyBytes: 18_486_000 },
  });
  expect(rowsOf(detail, FINISHED + 5_000)["Proxy traffic"]).toBe("18 MB");
});

test("the visit line names the cloud region where the browser window would stand", () => {
  expect(visitFacts({ ...scanOn(belgium()), pages: [] })).toEqual([
    "Scroll",
    "1 page",
    "Browserbase, Frankfurt (eu-central-1)",
    "Fresh browser profile",
    "Belgium (Browserbase proxy)",
  ]);
});

test("a Browserbase visit is never said to have been recognisable as automated", () => {
  const notice = environmentNotice(scanOn(frankfurt()));
  // What was recorded of the browser is listed once, under Browser details: the notice does not repeat it.
  expect(notice).toEqual({ kind: "browserbase", manual: false, detectors: [] });
  const detail = scanOn(frankfurt());
  const watched = environmentNotice({
    environment: detail.environment && {
      ...detail.environment,
      visitor: "manual",
      detectors: [{ vendor: "DoubleVerify", kind: "invalidTrafficMeasurement", host: "cdn.doubleverify.com" }],
    },
  });
  expect(watched).toEqual({
    kind: "browserbase",
    manual: true,
    detectors: ["DoubleVerify (invalid-traffic measurement) was on the page"],
  });
});

test("a Browserbase browser that did show the flag is reported like any automated visit, without the headless sign", () => {
  const detail = scanOn(frankfurt());
  const shown = environmentNotice({ environment: detail.environment && { ...detail.environment, webdriver: true } });
  expect(shown).toEqual({ kind: "identifiable", signals: ["navigator.webdriver was true"], canShowBrowser: false });
});

test("the note states what was recorded, calmly, and offers the same scan on this Mac", async () => {
  const user = userEvent.setup();
  const detail = scanOn(belgium());
  const options = {
    ...detail.options,
    headless: true,
    seed: 99,
    browser: { kind: "browserbase", region: "eu-central-1" },
    proxy: { kind: "browserbaseLocation", country: "BE" },
  } as const;
  await renderWithApp(<EnvironmentNotice detail={{ ...detail, options }} />);
  const note = screen.getByRole("note", { name: "Scan environment" });
  // One sentence about where it ran, and the one fact that can change the result: Browserbase's own scripts.
  expect(note).toHaveTextContent(
    "This visit ran in a Browserbase browser; what the pages could read there is under Browser details. Browserbase ran its own scripts inside every page; the scanner leaves them out of its results, but they can change how a page behaves. To compare, repeat the scan on this Mac.",
  );
  // The details already say these: the note does not say them again.
  expect(note).not.toHaveTextContent(/navigator\.webdriver|headless|a video and a log|Frankfurt/);
  expect(note).not.toHaveTextContent(/the pages could tell|undetect|indistinguishable|like a (real|human)/i);
  expect(screen.queryByRole("alert")).toBeNull();

  const { api } = await import("@/lib/api");
  await api.deleteAllScans();
  await user.click(within(note).getByRole("button", { name: "on this Mac" }));
  await screen.findByText("scan screen");
  const [scan] = (await api.listScans({})).items;
  const started = await api.getScan(scan?.id ?? "");
  // The Browserbase location cannot come along; the proxy is left to the settings.
  expect(started.options).toMatchObject({ browser: { kind: "local" }, proxy: { kind: "system" } });
  expect(started.options.seed).not.toBe(99);
});

test("the overview shows the line once, the details on demand, and the late count when it arrives", async () => {
  const user = userEvent.setup();
  const early = scanOn(belgium({ proxyBytes: 0 }), { finishedAt: Date.now() });
  const view = await renderWithApp(<BrowserbaseVisitDetails detail={early} />);
  const group = screen.getByRole("group", { name: "Browserbase visit" });
  expect(group).toHaveTextContent("Browserbase · 41.0 s of browser time · proxy traffic still being counted");
  expect(within(group).queryByRole("definition")).toBeNull();

  await user.click(within(group).getByRole("button", { name: "Browser details" }));
  expect(within(group).getByText("Still being counted by Browserbase")).toBeInTheDocument();
  expect(within(group).getByText("d634f35c-3c1e-4f7a-9b2d-d634f35ca41c")).toBeInTheDocument();
  view.unmount();

  const settled = scanOn(belgium({ proxyBytes: 0 }), {
    finishedAt: Date.now(),
    remote: { region: "eu-central-1", sessionSeconds: 41, proxyBytes: 18_486_000 },
  });
  await renderWithApp(<ReportOverview detail={{ ...settled, overview: "Two auctions ran." }} />);
  const overview = within(screen.getByRole("region", { name: "Overview" }));
  expect(overview.getByRole("group", { name: "Browserbase visit" })).toHaveTextContent(
    "Browserbase · 41.0 s of browser time · 18 MB through the proxy · about $0.23 at list price",
  );
  expect(overview.getAllByRole("note", { name: "Scan environment" })).toHaveLength(1);
  // The region is named once, on the visit line.
  expect(overview.getAllByText(/Frankfurt/)).toHaveLength(1);
  expect(overview.getByText("Browserbase, Frankfurt (eu-central-1)")).toBeInTheDocument();
});

test("with the audit trail loaded, the overview counts Browserbase's scripts and warns about an unconfirmed end", async () => {
  const user = userEvent.setup();
  const detail = scanOn(frankfurt({ sessionSeconds: null }), { finishedAt: Date.now() - 120_000 });
  const environment = detail.environment && {
    ...detail.environment,
    detectors: [{ vendor: "DoubleVerify", kind: "invalidTrafficMeasurement" as const, host: "cdn.doubleverify.com" }],
  };
  const logs = [
    browserLine("sessionCreated", { region: "eu-central-1" }),
    browserLine("serviceScripts", { count: 5, characters: 271_667 }),
    browserLine("releaseUnconfirmed"),
  ];
  await renderWithApp(
    <ReportOverview detail={{ ...detail, environment, overview: "Two auctions ran." }} logs={logs} />,
  );
  const overview = within(screen.getByRole("region", { name: "Overview" }));
  expect(overview.getByRole("note", { name: "Scan environment" })).toHaveTextContent(
    "Browserbase ran its own scripts inside every page (5 scripts in this visit); the scanner leaves them out of its results, but they can change how a page behaves. DoubleVerify (invalid-traffic measurement) was on the page. To compare, repeat the scan on this Mac.",
  );
  const group = overview.getByRole("group", { name: "Browserbase visit" });
  expect(group).toHaveTextContent(
    "Browserbase did not confirm that this browser ended. It may have kept running, and been billed, until its time limit.",
  );
  await user.click(within(group).getByRole("button", { name: "Browser details" }));
  expect(
    within(group).getByText("5 scripts ran in the pages; left out of this scan's scripts and results"),
  ).toBeInTheDocument();
  expect(within(group).getAllByText("Not reported by Browserbase")).toHaveLength(1);
});
