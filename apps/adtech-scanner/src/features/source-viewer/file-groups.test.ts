import { describe, expect, it } from "vitest";
import type { Evidence } from "@/bindings";
import { makeFinding, makeScript } from "@/test/fixtures";
import { findingsCiting, flattenGroups, groupFiles } from "./file-groups";

const code = (overrides: Partial<Evidence>): Evidence => ({
  kind: "code",
  label: "Excerpt",
  body: "var a = 1;",
  scriptUrl: null,
  line: 10,
  endLine: 10,
  scriptId: null,
  requestId: null,
  ...overrides,
});

const wrapper = makeScript({ id: "s1", url: "https://site.example/ads/wrapper.js", analyzed: true, adRelated: true });
const config = makeScript({ id: "s2", url: "https://site.example/ads/config.js", analyzed: true, adRelated: true });
const gpt = makeScript({
  id: "s3",
  url: "https://securepubads.g.doubleclick.net/tag/js/gpt.js",
  hostname: "securepubads.g.doubleclick.net",
  vendor: "Google",
  adRelated: true,
});
const app = makeScript({ id: "s4", url: "https://site.example/static/app.js" });
const inline = makeScript({ id: "s5", url: "inline://site.example/#script-4", inline: true });

const findings = [
  makeFinding({ id: "f1", severity: "medium", evidence: [code({ scriptId: "s2" })] }),
  makeFinding({
    id: "f2",
    severity: "critical",
    evidence: [code({ scriptId: "s2" }), code({ scriptId: "s2", line: 40 })],
  }),
  makeFinding({ id: "f3", severity: "low", evidence: [code({ scriptUrl: wrapper.url })] }),
  makeFinding({ id: "f4", severity: "high", evidence: [{ ...code({ scriptId: "s1" }), kind: "metric" }] }),
];

describe("findingsCiting", () => {
  it("matches by script id, and by URL for evidence stored without one", () => {
    expect(findingsCiting(config, findings).map((finding) => finding.id)).toEqual(["f1", "f2"]);
    expect(findingsCiting(wrapper, findings).map((finding) => finding.id)).toEqual(["f3"]);
    expect(findingsCiting(app, findings)).toEqual([]);
  });
});

describe("groupFiles", () => {
  const groups = groupFiles([wrapper, config, gpt, app, inline], findings);

  it("puts analysed files first, then other ad code, then the rest", () => {
    expect(groups.map((group) => [group.key, group.files.map((file) => file.script.id)])).toEqual([
      ["analysed", ["s2", "s1"]],
      ["adRelated", ["s3"]],
      ["other", ["s4", "s5"]],
    ]);
  });

  it("counts a finding once per file and reports the worst severity", () => {
    const [first, second] = groups[0]?.files ?? [];
    expect(first).toMatchObject({ name: "config.js", findingCount: 2, worst: "critical" });
    expect(second).toMatchObject({ name: "wrapper.js", findingCount: 1, worst: "low" });
    expect(groups[1]?.files[0]).toMatchObject({ findingCount: 0, worst: null });
  });

  it("names where a file belongs: vendor, host, or inline", () => {
    expect(groups[1]?.files[0]?.origin).toBe("Google");
    expect(groups[2]?.files.map((file) => file.origin)).toEqual(["www.example.com", "inline"]);
  });

  it("filters by name, URL or vendor and drops empty groups", () => {
    expect(groupFiles([wrapper, config, gpt, app], findings, "GOOGLE").map((group) => group.key)).toEqual([
      "adRelated",
    ]);
    expect(groupFiles([wrapper, config, gpt, app], findings, "/ads/")[0]?.files).toHaveLength(2);
    expect(groupFiles([wrapper, config, gpt, app], findings, "nothing-matches")).toEqual([]);
  });
});

describe("flattenGroups", () => {
  it("keeps the heading of a collapsed group and hides its files", () => {
    const items = flattenGroups(groupFiles([wrapper, gpt, app], findings), new Set(["other"]));
    expect(
      items.map((item) =>
        item.kind === "group" ? `${item.key}:${item.count}:${item.collapsed}` : item.file.script.id,
      ),
    ).toEqual(["analysed:1:false", "s1", "adRelated:1:false", "s3", "other:1:true"]);
  });
});
