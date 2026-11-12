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
    environment: null,
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
  expect(visit.environment).toBe("This visit is identifiable as automated: navigator.webdriver is true");
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
