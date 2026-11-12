import { beforeEach, describe, expect, it } from "vitest";
import type { ApiPort } from "@/lib/api/port";
import { createFakeBackend } from ".";

const DESKTOP = "seed-sportsdesk-desktop";

describe("the simulated script sources", () => {
  let api: ApiPort;

  beforeEach(() => {
    localStorage.clear();
    api = createFakeBackend({ profile: "seeded", speed: 1000 });
  });

  it("has every code excerpt of a finding at the lines the finding cites", async () => {
    const detail = await api.getScan(DESKTOP);
    const cited = detail.findings.flatMap((finding) =>
      finding.evidence.flatMap((evidence) => (evidence.kind === "code" && evidence.scriptId ? [evidence] : [])),
    );
    expect(cited.length).toBeGreaterThanOrEqual(4);
    for (const evidence of cited) {
      const source = await api.getScriptSource(DESKTOP, evidence.scriptId ?? "", "formatted");
      const lines = (source.text ?? "").split("\n");
      expect(lines.slice((evidence.line ?? 0) - 1, evidence.endLine ?? 0).join("\n")).toBe(evidence.body);
    }
  });

  it("shows the reference bug in the wrapper at line 211", async () => {
    const detail = await api.getScan(DESKTOP);
    const wrapper = detail.scripts.find((script) => script.url.endsWith("hb-wrapper.min.js"));
    const source = await api.getScriptSource(DESKTOP, wrapper?.id ?? "", "formatted");
    expect(source).toMatchObject({ availability: "available", format: "parser" });
    expect((source.text ?? "").split("\n")[210]).toBe("var IS_MOBILE = window.screen.width < 1440;");
    expect(source.lineCount).toBe((source.text ?? "").split("\n").length);
    expect(source.anchors).toEqual([
      expect.objectContaining({ startLine: 208, endLine: 215, located: true, evidenceIndex: 0 }),
      expect.objectContaining({ startLine: 231, endLine: 245, located: true, evidenceIndex: 1 }),
      expect.objectContaining({ startLine: 318, endLine: 332, located: true, evidenceIndex: 0 }),
    ]);
    expect(source.regions.every((region) => region.kind === "read")).toBe(true);
    expect(source.regions.some((region) => region.startLine <= 208 && region.endLine >= 215)).toBe(true);
    expect(source.adCalls).toContainEqual({ name: "requestBids", line: 326 });
    expect(source.adCalls.some((call) => call.name === "refresh")).toBe(true);
  });

  it("serves the original as one minified line and maps a position in it to the formatted line", async () => {
    const detail = await api.getScan(DESKTOP);
    const wrapper = detail.scripts.find((script) => script.url.endsWith("hb-wrapper.min.js"));
    const original = await api.getScriptSource(DESKTOP, wrapper?.id ?? "", "original");
    expect(original).toMatchObject({ format: "unformatted", lineCount: 1, anchors: [], regions: [] });
    const column = (original.text ?? "").indexOf("var IS_MOBILE = window.screen.width < 1440;") + 1;
    expect(column).toBeGreaterThan(0);
    const mapped = await api.getScriptSource(DESKTOP, wrapper?.id ?? "", "formatted", { line: 1, column });
    expect(mapped.focusLine).toBe(211);
  });

  it("has a 20,000-line bundle whose original is one line of over 2 MB", async () => {
    const detail = await api.getScan(DESKTOP);
    const bundle = detail.scripts.find((script) => script.url.includes("/static/js/main."));
    const formatted = await api.getScriptSource(DESKTOP, bundle?.id ?? "", "formatted");
    expect(formatted.lineCount).toBe(20_000);
    const original = await api.getScriptSource(DESKTOP, bundle?.id ?? "", "original");
    expect(original.lineCount).toBe(1);
    expect(original.text?.length).toBeGreaterThan(2_000_000);
  });

  it("says how each file was formatted", async () => {
    const detail = await api.getScan(DESKTOP);
    const format = async (match: string) => {
      const script = detail.scripts.find((candidate) => candidate.url.includes(match));
      return (await api.getScriptSource(DESKTOP, script?.id ?? "", "formatted")).format;
    };
    expect(await format("chunk-comments")).toBe("reflow");
    expect(await format("pubads_impl")).toBe("unformatted");
    expect(await format("ad-config")).toBe("parser");
  });

  it("answers without text when the scan kept none, and says why", async () => {
    for (const [scanId, availability] of [
      ["seed-morningledger-desktop", "notRetained"],
      ["seed-techpulse-desktop", "evicted"],
      ["seed-morningledger-mobile", "notRecorded"],
    ] as const) {
      const source = await api.getScriptSource(scanId, "s7", "formatted");
      expect(source).toMatchObject({ availability, text: null, lineCount: 0, anchors: [] });
    }
    await expect(api.getScriptSource(DESKTOP, "s999", "formatted")).rejects.toMatchObject({ code: "notFound" });
  });
});
