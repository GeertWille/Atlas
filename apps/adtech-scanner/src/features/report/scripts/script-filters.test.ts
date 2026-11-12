import { expect, test } from "vitest";
import { makeScript } from "@/test/fixtures";
import { filterScripts, NO_SCRIPT_FILTERS, scriptFunnel, sortScripts } from "./script-filters";

const scripts = [
  makeScript({ id: "app", sizeBytes: 6000, loadedAtMs: 300 }),
  makeScript({
    id: "wrapper",
    url: "https://www.example.com/ads/hb-wrapper.js",
    category: "wrapper",
    adRelated: true,
    analyzed: true,
    sizeBytes: 3000,
    loadedAtMs: 900,
    classificationReason: "Calls pbjs.requestBids",
  }),
  makeScript({
    id: "gpt",
    url: "https://securepubads.g.doubleclick.net/tag/js/gpt.js",
    category: "adServer",
    adRelated: true,
    vendor: "Google",
    sizeBytes: 1000,
    loadedAtMs: 600,
  }),
];

const ids = (list: typeof scripts) => list.map((script) => script.id);

test("search matches URL, vendor and classification reason, case-insensitively", () => {
  expect(ids(filterScripts(scripts, { ...NO_SCRIPT_FILTERS, search: "GOOGLE" }))).toEqual(["gpt"]);
  expect(ids(filterScripts(scripts, { ...NO_SCRIPT_FILTERS, search: "requestbids" }))).toEqual(["wrapper"]);
  expect(ids(filterScripts(scripts, { ...NO_SCRIPT_FILTERS, search: " example.com " }))).toEqual(["app", "wrapper"]);
});

test("category and scope narrow the list", () => {
  expect(ids(filterScripts(scripts, { ...NO_SCRIPT_FILTERS, categories: ["adServer", "wrapper"] }))).toEqual([
    "wrapper",
    "gpt",
  ]);
  expect(ids(filterScripts(scripts, { ...NO_SCRIPT_FILTERS, scope: "adRelated" }))).toEqual(["wrapper", "gpt"]);
  expect(ids(filterScripts(scripts, { ...NO_SCRIPT_FILTERS, scope: "analyzed" }))).toEqual(["wrapper"]);
});

test("sorts by load order or by size", () => {
  expect(ids(sortScripts(scripts, "loadOrder"))).toEqual(["app", "gpt", "wrapper"]);
  expect(ids(sortScripts(scripts, "sizeDesc"))).toEqual(["app", "wrapper", "gpt"]);
});

test("the funnel reports count, bytes and share of bytes per stage", () => {
  expect(scriptFunnel(scripts)).toEqual([
    { key: "all", label: "Scripts loaded", count: 3, bytes: 10_000, shareOfBytes: 1 },
    { key: "adRelated", label: "Ad-related", count: 2, bytes: 4000, shareOfBytes: 0.4 },
    { key: "analyzed", label: "Analysed in Phase 2", count: 1, bytes: 3000, shareOfBytes: 0.3 },
  ]);
  expect(scriptFunnel([]).map((stage) => stage.shareOfBytes)).toEqual([0, 0, 0]);
});
