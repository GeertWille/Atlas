import { expect, test } from "vitest";
import type { LogEntry } from "@/bindings";
import { makeLog } from "@/test/fixtures";
import { deriveVisit, formatTimer } from "./visit";

let sequence = 0;
const visitor = (activity: string, message: string, detail: unknown, page = 0): LogEntry => {
  sequence += 1;
  return makeLog({ id: `l${sequence}`, kind: "visitor", message, data: { activity, page, detail } });
};
const pageView = (index: number, path: string, reachedBy: string, link: unknown = null) =>
  visitor(
    "pageView",
    `Page ${index + 1}: ${path}`,
    { index, url: `https://site.example${path}`, title: `Title ${index + 1}`, reachedBy, link },
    index,
  );

test("nothing is known about a visit before its first visitor entry", () => {
  expect(deriveVisit([makeLog({ message: "Navigating" })])).toEqual({
    pages: [],
    current: null,
    actions: [],
    skipped: null,
    remote: null,
    remoteNotice: null,
  });
});

test("pages, the current page, the latest actions and the skip summary come from the audit trail alone", () => {
  const visit = deriveVisit([
    makeLog({ message: "Navigating to the page" }),
    pageView(0, "/", "entry"),
    visitor("environment", "This visit is identifiable as automated: navigator.webdriver is true", { webdriver: true }),
    visitor("action", "Clicked the consent banner", { action: "consent" }),
    visitor("action", "Moved the pointer over a heading", { action: "pointerMove" }),
    visitor("action", "Scrolled to 30%", { action: "scroll", scrollDepth: 0.3 }),
    visitor("action", "Scrolled to 60%", { action: "scroll", scrollDepth: 0.6 }),
    visitor("linkChoice", "Following link: /sport/final (best of 5)", {
      chosen: { text: "Final", href: "https://site.example/sport/final", score: 6, reasons: [] },
      alternatives: [],
      skipped: { "off-site": 9, "inside-ad": 3, sensitive: 2 },
    }),
    visitor("blocked", "Closed a pop-up window (partner.example): not recorded", { reason: "popup" }),
    pageView(1, "/sport/final", "linkClick", { text: "Final", href: "https://site.example/sport/final", reason: "" }),
  ]);

  expect(visit.pages.map((page) => [page.index, page.url, page.reachedBy, page.linkText])).toEqual([
    [0, "https://site.example/", "entry", null],
    [1, "https://site.example/sport/final", "linkClick", "Final"],
  ]);
  expect(visit.current?.index).toBe(1);
  // The three newest, oldest first; the earlier gestures have scrolled out.
  expect(visit.actions.map((action) => action.message)).toEqual([
    "Scrolled to 60%",
    "Following link: /sport/final (best of 5)",
    "Closed a pop-up window (partner.example): not recorded",
  ]);
  expect(visit.actions.map((action) => action.warning)).toEqual([false, false, true]);
  expect(visit.skipped).toBe("Skipped 14 links: 9 off-site, 3 inside ads, 2 sign-in, checkout and the like");
  // The environment entry is no action of the visitor: it is not listed among them.
  expect(visit.actions.map((action) => action.message)).not.toContain(
    "This visit is identifiable as automated: navigator.webdriver is true",
  );
});

test("a page view announced again keeps its place, and a later choice without skips clears the summary", () => {
  const visit = deriveVisit([
    pageView(0, "/", "entry"),
    visitor("linkChoice", "Following link", { chosen: null, alternatives: [], skipped: { hidden: 1 } }),
    pageView(1, "/a", "linkClick"),
    pageView(1, "/a", "reload"),
    visitor("linkChoice", "No link to follow", { chosen: null, alternatives: [], skipped: {} }, 1),
  ]);
  expect(visit.pages).toHaveLength(2);
  expect(visit.pages[1]?.reachedBy).toBe("reload");
  expect(visit.skipped).toBeNull();
});

test("the session timer reads as minutes and seconds and never goes below zero", () => {
  expect(formatTimer(0)).toBe("00:00");
  expect(formatTimer(134_900)).toBe("02:14");
  expect(formatTimer(15 * 60_000)).toBe("15:00");
  expect(formatTimer(-5_000)).toBe("00:00");
});

const remote = (event: string, message: string, detail: Record<string, unknown> = {}, ts = 1_000): LogEntry => {
  sequence += 1;
  return makeLog({
    id: `l${sequence}`,
    ts,
    kind: "visitor",
    level: event === "connectionLost" ? "warn" : "info",
    message,
    data: { activity: "browser", page: null, detail: { event, ...detail } },
  });
};

test("a Browserbase browser is known from the moment Browserbase starts it, with its region and its clock", () => {
  expect(deriveVisit([makeLog({ message: "Using Browserbase in eu-central-1" })]).remote).toBeNull();
  const started = deriveVisit([
    remote("sessionCreated", "Browserbase started a browser in eu-central-1", { region: "eu-central-1" }, 5_000),
    remote("connected", "Connected to Chrome/157.0.8103.79, round trip 119 ms", {}, 5_400),
  ]);
  expect(started.remote).toEqual({ region: "eu-central-1", startedAt: 5_000, endedAt: null, liveViewReady: false });
  // No page yet: the browser is there before the visit is anywhere.
  expect(started.current).toBeNull();
});

test("the live view is ready once the capture says so, and not after the browser has ended", () => {
  const session = remote("sessionCreated", "started", { region: "us-west-2" }, 5_000);
  const ready = remote("liveView", "The browser is ready for you in Browserbase's live view", {}, 7_000);
  expect(deriveVisit([session]).remote?.liveViewReady).toBe(false);
  expect(deriveVisit([session, ready]).remote?.liveViewReady).toBe(true);
  expect(deriveVisit([session, ready, remote("released", "ended after 41 s", {}, 46_000)]).remote).toEqual({
    region: "us-west-2",
    startedAt: 5_000,
    endedAt: 46_000,
    liveViewReady: false,
  });
});

test("waiting at the limit and a lost connection are passed on in the backend's words", () => {
  const waiting = deriveVisit([remote("rateLimited", "Browserbase is at its limit; trying again in 5 s")]);
  expect(waiting).toMatchObject({ remote: null, remoteNotice: "Browserbase is at its limit; trying again in 5 s" });
  // Once the browser starts, the wait is over.
  const started = deriveVisit([
    remote("rateLimited", "Browserbase is at its limit; trying again in 5 s"),
    remote("sessionCreated", "started", { region: "us-west-2" }, 9_000),
  ]);
  expect(started.remoteNotice).toBeNull();
  const lost = deriveVisit([
    remote("sessionCreated", "started", { region: "us-west-2" }, 9_000),
    remote("connectionLost", "The connection to the Browserbase browser was lost", {}, 30_000),
  ]);
  expect(lost).toMatchObject({
    remote: { endedAt: 30_000, liveViewReady: false },
    remoteNotice: "The connection to the Browserbase browser was lost",
  });
});

test("an entry about the browser that names no known event is just a message", () => {
  expect(deriveVisit([remote("somethingNew", "Browserbase did something")]).remote).toBeNull();
});
