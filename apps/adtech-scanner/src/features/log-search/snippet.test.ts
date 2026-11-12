import { expect, test } from "vitest";
import { makeLog } from "@/test/fixtures";
import { groupHitsByScan } from "./group-hits";
import { parseSnippet } from "./snippet";

test("splits a snippet into marked and unmarked parts and decodes entities", () => {
  expect(parseSnippet("a &lt;b&gt; <mark>time&amp;out</mark> c")).toEqual([
    { text: "a <b> ", marked: false },
    { text: "time&out", marked: true },
    { text: " c", marked: false },
  ]);
});

test("escaped markup stays text: nothing but <mark> is interpreted", () => {
  expect(parseSnippet("&lt;script&gt;alert(1)&lt;/script&gt;")).toEqual([
    { text: "<script>alert(1)</script>", marked: false },
  ]);
  // Even an unescaped tag is carried as plain text for React to render literally.
  expect(parseSnippet('<img src=x onerror="x()"> <mark>hit</mark>')).toEqual([
    { text: '<img src=x onerror="x()"> ', marked: false },
    { text: "hit", marked: true },
  ]);
});

test("handles adjacent marks, quotes and double-escaped entities", () => {
  expect(parseSnippet("<mark>a</mark><mark>b</mark> &quot;q&quot; &#39;s&#39; &amp;lt;")).toEqual([
    { text: "a", marked: true },
    { text: "b", marked: true },
    { text: " \"q\" 's' &lt;", marked: false },
  ]);
  expect(parseSnippet("")).toEqual([]);
});

test("groups hits by scan in first-seen order and tracks the earliest entry", () => {
  const hit = (id: string, scanId: string, ts: number) => ({
    entry: makeLog({ id, scanId, ts }),
    scanUrl: `https://${scanId}.example/`,
    snippet: "",
  });
  const groups = groupHitsByScan([hit("l1", "b", 200), hit("l2", "a", 50), hit("l3", "b", 100)]);
  expect(groups.map((group) => [group.scanId, group.firstEntryAt, group.hits.map((item) => item.entry.id)])).toEqual([
    ["b", 100, ["l1", "l3"]],
    ["a", 50, ["l2"]],
  ]);
});
