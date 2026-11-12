import { expect, test } from "vitest";
import { makeLog } from "@/test/fixtures";
import { describeSkipped, visitorActivity } from "./visitor-log";

const visitor = (activity: string, detail: unknown, page: number | null = 0) =>
  makeLog({ kind: "visitor", data: { activity, page, detail } });

test("a page view carries where the visit is and how it got there", () => {
  const entry = visitor(
    "pageView",
    {
      index: 1,
      url: "https://site.example/climate/kelp",
      title: "Kelp",
      reachedBy: "linkClick",
      startedAtMs: 12850,
      link: { text: "Kelp farms", href: "https://site.example/climate/kelp", reason: "content link" },
    },
    1,
  );
  expect(visitorActivity(entry)).toEqual({
    activity: "pageView",
    page: 1,
    index: 1,
    url: "https://site.example/climate/kelp",
    title: "Kelp",
    reachedBy: "linkClick",
    linkText: "Kelp farms",
  });
});

test("a link choice lists the pick, the runners-up and the skip counts, most frequent first", () => {
  const choice = visitorActivity(
    visitor("linkChoice", {
      chosen: { text: "Final", href: "https://site.example/sport/final", score: 6, reasons: ["content link", 3] },
      alternatives: [{ text: "Other", href: "https://site.example/other", score: 4 }, { text: "No address" }],
      skipped: { "inside-ad": 3, "off-site": 9, sensitive: 2, visited: 0 },
    }),
  );
  expect(choice).toEqual({
    activity: "linkChoice",
    page: 0,
    chosen: { text: "Final", href: "https://site.example/sport/final", score: 6, reasons: ["content link"] },
    alternatives: [{ text: "Other", href: "https://site.example/other", score: 4 }],
    skipped: [
      { reason: "off-site", count: 9 },
      { reason: "inside-ad", count: 3 },
      { reason: "sensitive", count: 2 },
    ],
  });
  expect(choice?.activity === "linkChoice" && describeSkipped(choice.skipped)).toBe(
    "9 off-site, 3 inside ads, 2 sign-in, checkout and the like",
  );
  expect(describeSkipped([{ reason: "new-reason", count: 1 }])).toBe("1 new-reason");
});

test("a page with no link worth following has no pick", () => {
  expect(visitorActivity(visitor("linkChoice", { chosen: null, alternatives: [], skipped: {} }))).toMatchObject({
    chosen: null,
    alternatives: [],
    skipped: [],
  });
});

test("actions, blocks and the environment keep their few facts", () => {
  expect(visitorActivity(visitor("action", { action: "scroll", scrollDepth: 0.6 }))).toEqual({
    activity: "action",
    page: 0,
    action: "scroll",
    scrollDepth: 0.6,
  });
  expect(visitorActivity(visitor("blocked", { reason: "off-site", host: "partner.example" }))).toMatchObject({
    reason: "off-site",
    host: "partner.example",
  });
  expect(visitorActivity(visitor("environment", { webdriver: true, headless: false }, null))).toEqual({
    activity: "environment",
    page: null,
    webdriver: true,
    headless: false,
  });
});

test("anything that is not a well-formed visitor entry is just a message", () => {
  expect(visitorActivity(makeLog({ kind: "message", data: { activity: "pageView" } }))).toBeNull();
  expect(visitorActivity(makeLog({ kind: "visitor", data: null }))).toBeNull();
  expect(visitorActivity(visitor("somethingNew", {}))).toBeNull();
  // A page view without a URL or with an unknown way of getting there cannot be placed.
  expect(visitorActivity(visitor("pageView", { index: 0, reachedBy: "entry" }))).toBeNull();
  expect(
    visitorActivity(visitor("pageView", { index: 0, url: "https://a.example/", reachedBy: "teleport" })),
  ).toBeNull();
  // Missing details degrade to "unknown", they do not throw.
  expect(visitorActivity(visitor("action", null))).toEqual({
    activity: "action",
    page: 0,
    action: null,
    scrollDepth: null,
  });
});
