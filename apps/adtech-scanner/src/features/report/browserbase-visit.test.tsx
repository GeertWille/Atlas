import { screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, test } from "vitest";
import type { BrowserUse, ScanDetail } from "@/bindings";
import { makeDetail, makeEnvironment, makeSummary } from "@/test/fixtures";
import { renderWithApp } from "@/test/render";
import { BrowserbaseVisitDetails } from "./BrowserbaseVisitDetails";
import { browserbaseVisitFacts, proxyUsagePending } from "./browserbase-visit";
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

const rowsOf = (detail: ScanDetail, now = FINISHED + 600_000) =>
  Object.fromEntries((browserbaseVisitFacts(detail, now)?.rows ?? []).map((row) => [row.name, row.value]));

test("a visit on this Mac, and a scan without a recorded environment, have no Browserbase facts", () => {
  expect(browserbaseVisitFacts(makeDetail(), FINISHED)).toBeNull();
  expect(browserbaseVisitFacts(makeDetail({ environment: makeEnvironment() }), FINISHED)).toBeNull();
});

test("a visit from the region says where it ran, what it used and what the pages could read", () => {
  const detail = scanOn(frankfurt());
  expect(browserbaseVisitFacts(detail, FINISHED + 600_000)?.summary).toEqual([
    "Browserbase Frankfurt",
    "41.0 s of browser time",
    "about $0.002 at Browserbase",
  ]);
  expect(rowsOf(detail)).toEqual({
    "Browser ran in": "Browserbase, Frankfurt, Germany (eu-central-1)",
    "Left from": "Browserbase's Frankfurt region, a data centre address (no proxy)",
    "Browser time": "41.0 s, billed as 1 minute",
    "Proxy traffic": "None: the visit used no Browserbase proxy",
    "Estimated cost": "$0.002 at Browserbase's Developer plan overage prices of 2026-11-12; not part of the LLM cost",
    "Timezone the pages saw":
      "Europe/Berlin, set by the scanner to match the region (the browser reported America/Los_Angeles)",
    Language: "en-US, as the browser reported it",
    "navigator.webdriver": "false: the pages were not told the visit is automated",
    "User agent": "Mozilla/5.0 (X11; Linux x86_64) Chrome/157.0.0.0 Safari/537.36",
    Browser: "Chrome/157.0.8103.79, headless, which the user agent does not show",
    "Round trip":
      "119 ms per command between this Mac and the browser; frames and requests the scanner held for a moment waited that long",
    "Kept at Browserbase": "A video and a log",
    Session: "d634f35c-3c1e-4f7a-9b2d-d634f35ca41c",
  });
});

test("a Browserbase location names the country asked for, its traffic and the timezone Browserbase set", () => {
  const detail = scanOn(belgium({ sessionSeconds: 95 }));
  expect(browserbaseVisitFacts(detail, FINISHED + 600_000)?.summary).toEqual([
    "Browserbase Frankfurt",
    "1m 35s of browser time",
    "18 MB through the proxy",
    "about $0.23 at Browserbase",
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
    "Browser time": "Not reported",
    "Timezone the pages saw": "Not read",
    Language: "Not read",
    "navigator.webdriver": "Not read",
    "User agent": "Not recorded",
    Browser: "Version not reported, headless, which the user agent does not show",
    "Kept at Browserbase": "Nothing",
  });
  expect(rows).not.toHaveProperty("Round trip");
});

test("proxy traffic is still being counted until the second read, and never for a visit without a Browserbase proxy", () => {
  const proxied = { proxyKind: "browserbase" } as const;
  expect(proxyUsagePending({ proxyKind: "none" }, { proxyBytes: 0 }, FINISHED, FINISHED)).toBe(false);
  expect(proxyUsagePending({ proxyKind: "external" }, null, FINISHED, FINISHED)).toBe(false);
  expect(proxyUsagePending(proxied, null, FINISHED, FINISHED)).toBe(true);
  expect(proxyUsagePending(proxied, { proxyBytes: null }, FINISHED, FINISHED + 600_000)).toBe(true);
  // The first read after the end comes too early: a zero only counts once the second read is overdue.
  expect(proxyUsagePending(proxied, { proxyBytes: 0 }, FINISHED, FINISHED + 5_000)).toBe(true);
  expect(proxyUsagePending(proxied, { proxyBytes: 0 }, FINISHED, FINISHED + 61_000)).toBe(false);
  expect(proxyUsagePending(proxied, { proxyBytes: 82_715 }, FINISHED, FINISHED + 5_000)).toBe(false);

  const early = scanOn(belgium({ proxyBytes: 0 }));
  const facts = browserbaseVisitFacts(early, FINISHED + 5_000);
  expect(facts?.pending).toBe(true);
  expect(facts?.summary).toEqual([
    "Browserbase Frankfurt",
    "41.0 s of browser time",
    "proxy traffic still being counted",
  ]);
  expect(rowsOf(early, FINISHED + 5_000)).toMatchObject({
    "Proxy traffic": "Still being counted by Browserbase",
    "Estimated cost": "Known once the proxy traffic is counted",
  });
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
    "Browserbase Frankfurt",
    "Fresh browser profile",
    "Belgium (Browserbase proxy)",
  ]);
});

test("a Browserbase visit is never said to have been recognisable as automated", () => {
  const notice = environmentNotice(scanOn(frankfurt()));
  expect(notice).toEqual({
    kind: "browserbase",
    region: "eu-central-1",
    manual: false,
    facts: [
      "navigator.webdriver read false, so the pages were not told the visit is automated",
      "the browser ran headless, which its user agent does not show",
      "Browserbase solves CAPTCHAs on the pages automatically",
      "Browserbase kept a video and a log of the session",
    ],
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
  expect(note).toHaveTextContent(
    "This visit ran in a Browserbase browser in Frankfurt. Recorded: navigator.webdriver read false, so the pages were not told the visit is automated; the browser ran headless, which its user agent does not show; Browserbase solves CAPTCHAs on the pages automatically; Browserbase kept a video and a log of the session. To compare, repeat the scan on this Mac.",
  );
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
  expect(group).toHaveTextContent("Browserbase Frankfurt · 41.0 s of browser time · proxy traffic still being counted");
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
    "Browserbase Frankfurt · 41.0 s of browser time · 18 MB through the proxy · about $0.23 at Browserbase",
  );
  expect(overview.getAllByRole("note", { name: "Scan environment" })).toHaveLength(1);
});
