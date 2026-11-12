import { beforeEach, describe, expect, test } from "vitest";
import type { ScanEvent, ScanOptions, ScanStatus, ScriptRecord } from "@/bindings";
import { ApiError } from "@/lib/api/error";
import type { ApiPort } from "@/lib/api/port";
import { createFakeBackend } from "./index";

const options: ScanOptions = {
  url: "https://www.sportsdesk.example/football",
  device: "desktop",
  observeSeconds: 20,
  autoScroll: true,
  acceptConsent: true,
  aiEnabled: true,
};

const VALID_KEY = "sk-ant-api03-abcdefghijklmnopqrstuv";
const TYPESAFE_KEY = "typesafe-test-key";

/** Starts a scan and resolves with every event up to and including its final status. */
async function runToEnd(api: ApiPort, scanOptions: ScanOptions) {
  const events: ScanEvent[] = [];
  let finish: (status: ScanStatus) => void = () => {};
  const finished = new Promise<ScanStatus>((resolve) => {
    finish = resolve;
  });
  const unsubscribe = await api.subscribeScanEvents((event) => {
    events.push(event);
    if (event.type === "scanUpdated" && event.summary.status !== "running") finish(event.summary.status);
  });
  const summary = await api.startScan(scanOptions);
  const status = await finished;
  unsubscribe();
  return { scanId: summary.id, status, events };
}

beforeEach(() => {
  localStorage.clear();
});

describe("a full AI scan of the reference-bug site", () => {
  test("streams every pipeline node and ends in a complete report", async () => {
    const api = createFakeBackend({ profile: "fresh", speed: 5000 });
    await api.setApiKey(VALID_KEY);
    const { scanId, status, events } = await runToEnd(api, options);
    expect(status).toBe("completed");

    const detail = await api.getScan(scanId);
    expect(detail.steps.map((step) => step.status)).toEqual(Array(9).fill("completed"));
    expect(detail.summary).toMatchObject({ aiUsed: true, healthScore: 32, prebidVersion: "8.52.0", error: null });
    expect(detail.summary.costUsd).toBeGreaterThan(0.2);
    expect(detail.summary.costUsd).toBeLessThanOrEqual(0.5);
    expect(detail.summary.costUsd).toBeCloseTo(
      detail.steps.reduce((sum, step) => sum + step.costUsd, 0),
      10,
    );

    // The reference bug: lazy desktop slots never requested, reported as a critical AI finding.
    expect(detail.slots.filter((slot) => slot.outcome === "neverRequested")).toHaveLength(3);
    const critical = detail.findings.find((finding) => finding.severity === "critical");
    expect(critical).toMatchObject({ source: "ai", category: "logicBug" });
    expect(critical?.evidence.some((evidence) => evidence.kind === "code" && evidence.line !== null)).toBe(true);
    expect(detail.summary.findingCounts.critical).toBe(1);
    expect(detail.scripts.some((script) => script.analyzed)).toBe(true);
    expect(detail.metrics.scriptCount).toBe(detail.scripts.length);

    // Findings were pushed live, and every one of them is in the final report.
    const pushed = events.flatMap((event) => (event.type === "findingAdded" ? [event.finding.id] : []));
    expect(pushed).toEqual(detail.findings.map((finding) => finding.id));

    const logs = await api.getLogs(scanId);
    expect(new Set(logs.map((entry) => entry.kind))).toEqual(
      new Set(["message", "visitor", "llmRequest", "llmResponse", "thinking", "toolCall", "toolResult", "finding"]),
    );
    expect(logs.map((entry) => entry.ts)).toEqual([...logs.map((entry) => entry.ts)].sort((a, b) => a - b));
    const analyzeStep = detail.steps.find((step) => step.node === "analyzeLogic");
    expect(await api.getLogs(scanId, analyzeStep?.id)).toHaveLength(analyzeStep?.logCount ?? -1);
  });

  test("mobile takes the lazy path and has no critical finding", async () => {
    const api = createFakeBackend({ profile: "fresh", speed: 5000 });
    await api.setApiKey(VALID_KEY);
    const { scanId } = await runToEnd(api, { ...options, device: "mobile" });
    const detail = await api.getScan(scanId);
    expect(detail.auctions).toHaveLength(2);
    expect(detail.slots.some((slot) => slot.outcome === "neverRequested")).toBe(false);
    expect(detail.summary.findingCounts.critical).toBe(0);
  });
});

describe("scans without the model", () => {
  test.each([
    ["no API key", false, true, "No Anthropic API key is configured"],
    ["AI disabled for the scan", true, false, "AI analysis is turned off for this scan"],
    // The backend checks the scan's own switch first, which is also what a composer without a key sends.
    ["AI disabled and no API key", false, false, "AI analysis is turned off for this scan"],
  ])("%s: only deterministic checks run", async (_name, hasKey, aiEnabled, reason) => {
    const api = createFakeBackend({ profile: "fresh", speed: 5000 });
    if (hasKey) await api.setApiKey(VALID_KEY);
    const { scanId, status } = await runToEnd(api, { ...options, aiEnabled });
    expect(status).toBe("completed");
    const detail = await api.getScan(scanId);
    expect(detail.summary).toMatchObject({ aiUsed: false, costUsd: 0 });
    expect(Object.fromEntries(detail.steps.map((step) => [step.node, step.status]))).toMatchObject({
      heuristics: "completed",
      filterScripts: "skipped",
      beautify: "skipped",
      analyzeLogic: "skipped",
      synthesize: "completed",
    });
    expect(detail.steps.find((step) => step.node === "analyzeLogic")?.summary).toBe(reason);
    expect(detail.findings.length).toBeGreaterThan(0);
    expect(detail.findings.every((finding) => finding.source === "heuristic")).toBe(true);
    expect(detail.scripts.every((script) => script.classifiedBy === "heuristic" && !script.analyzed)).toBe(true);
  });
});

test("a tiny budget stops the analysis early with a warning", async () => {
  const api = createFakeBackend({ profile: "fresh", speed: 5000 });
  await api.setApiKey(VALID_KEY);
  await api.updateSettings({ budgetUsd: 0.05 });
  const { scanId } = await runToEnd(api, options);
  const logs = await api.getLogs(scanId);
  expect(logs.some((entry) => entry.level === "warn" && entry.message.includes("Budget cap of $0.05 reached"))).toBe(
    true,
  );
  const detail = await api.getScan(scanId);
  expect(detail.findings.filter((finding) => finding.source === "ai")).toHaveLength(1);
});

test("an unreachable site fails in the capture step and keeps its audit trail", async () => {
  const api = createFakeBackend({ profile: "fresh", speed: 5000 });
  const { scanId, status } = await runToEnd(api, { ...options, url: "https://unreachable.example/" });
  expect(status).toBe("failed");
  const detail = await api.getScan(scanId);
  expect(detail.summary.error).toContain("ERR_NAME_NOT_RESOLVED");
  expect(detail.summary.healthScore).toBeNull();
  expect(detail.steps.map((step) => step.status).slice(0, 3)).toEqual(["completed", "failed", "pending"]);
  expect((await api.getLogs(scanId)).at(-1)).toMatchObject({ level: "error" });
});

test("cancelling stops the scan and no further events arrive", async () => {
  const api = createFakeBackend({ profile: "fresh", speed: 1 });
  const events: ScanEvent[] = [];
  await api.subscribeScanEvents((event) => events.push(event));
  const summary = await api.startScan(options);
  await new Promise((resolve) => setTimeout(resolve, 150));
  await api.cancelScan(summary.id);
  const countAfterCancel = events.length;
  await new Promise((resolve) => setTimeout(resolve, 300));

  expect(events.length).toBe(countAfterCancel);
  expect(events.at(-1)).toMatchObject({ type: "scanUpdated", summary: { status: "cancelled" } });
  const detail = await api.getScan(summary.id);
  expect(detail.summary.status).toBe("cancelled");
  expect(detail.steps.some((step) => step.status === "running")).toBe(false);
  expect(detail.steps.filter((step) => step.status === "cancelled")).toMatchObject([{ summary: "Cancelled" }]);
  expect(detail.steps.some((step) => step.status === "failed")).toBe(false);
  expect((await api.listScans({ status: "running" })).total).toBe(0);
});

describe("command errors", () => {
  test("starting a scan without a browser is browserNotFound", async () => {
    const api = createFakeBackend({ profile: "no-browser", speed: 5000 });
    expect(await api.detectBrowser()).toBeNull();
    const error = await api.startScan(options).catch((reason: unknown) => reason);
    expect(error).toBeInstanceOf(ApiError);
    expect(error).toMatchObject({ code: "browserNotFound" });

    await api.updateSettings({ chromePath: "/usr/bin/chromium" });
    expect(await api.detectBrowser()).toMatchObject({ path: "/usr/bin/chromium" });
    await api.updateSettings({ chromePath: null });
    expect(await api.detectBrowser()).toBeNull();
  });

  test("a relative URL is invalidInput and an unknown scan is notFound", async () => {
    const api = createFakeBackend({ profile: "fresh", speed: 5000 });
    await expect(api.startScan({ ...options, url: "example.com" })).rejects.toMatchObject({ code: "invalidInput" });
    await expect(api.getScan("nope")).rejects.toMatchObject({ code: "notFound" });
  });
});

describe("API key", () => {
  test("an invalid key is reported and not stored; a valid one is stored; clearing removes it", async () => {
    const api = createFakeBackend({ profile: "fresh", speed: 5000 });
    expect(await api.setApiKey("not-a-key")).toMatchObject({ valid: false, message: expect.stringContaining("401") });
    expect((await api.getSettings()).hasApiKey).toBe(false);
    expect(await api.setApiKey(VALID_KEY)).toEqual({ valid: true, message: null });
    expect((await api.getSettings()).hasApiKey).toBe(true);
    await api.clearApiKey();
    expect((await api.getSettings()).hasApiKey).toBe(false);
  });
});

describe("TypeSafe key and the Jev switch", () => {
  test("keys are stored per provider and a rejected key stores nothing", async () => {
    const api = createFakeBackend({ profile: "fresh", speed: 5000 });
    expect(await api.setApiKey("this-key-is-invalid", "typesafe")).toEqual({
      valid: false,
      message: "TypeSafe rejected this API key.",
    });
    expect(await api.getSettings()).toMatchObject({ hasApiKey: false, hasTypesafeKey: false });

    expect(await api.setApiKey("any-format-works", "typesafe")).toEqual({ valid: true, message: null });
    expect(await api.getSettings()).toMatchObject({ hasApiKey: false, hasTypesafeKey: true, jevEnabled: false });

    await api.setApiKey(VALID_KEY, "anthropic");
    await api.clearApiKey("anthropic");
    expect(await api.getSettings()).toMatchObject({ hasApiKey: false, hasTypesafeKey: true });
  });

  test("Jev cannot be enabled without a key, and removing the key turns it off", async () => {
    const api = createFakeBackend({ profile: "fresh", speed: 5000 });
    await expect(api.updateSettings({ jevEnabled: true })).rejects.toMatchObject({ code: "invalidInput" });
    expect((await api.getSettings()).jevEnabled).toBe(false);

    await api.setApiKey(TYPESAFE_KEY, "typesafe");
    expect((await api.updateSettings({ jevEnabled: true })).jevEnabled).toBe(true);
    await api.clearApiKey("typesafe");
    expect(await api.getSettings()).toMatchObject({ hasTypesafeKey: false, jevEnabled: false });
  });

  test("state saved before the Jev settings existed loads with them switched off", async () => {
    const { jevEnabled, hasTypesafeKey, ...legacySettings } = await createFakeBackend({
      profile: "fresh",
      speed: 5000,
    }).updateSettings({ budgetUsd: 1.25 });
    expect([jevEnabled, hasTypesafeKey]).toEqual([false, false]);
    localStorage.setItem(
      "adscan.fake.db.fresh",
      JSON.stringify({ settings: legacySettings, scans: [], deletedSeedIds: [] }),
    );
    expect(await createFakeBackend({ profile: "fresh", speed: 5000 }).getSettings()).toMatchObject({
      budgetUsd: 1.25,
      jevEnabled: false,
      hasTypesafeKey: false,
    });
  });
});

describe("scans with Jev System One", () => {
  async function withJev(config: { anthropicKey: boolean; jevFails?: boolean }) {
    const api = createFakeBackend({ profile: "fresh", speed: 5000, jevFails: config.jevFails });
    if (config.anthropicKey) await api.setApiKey(VALID_KEY);
    await api.setApiKey(TYPESAFE_KEY, "typesafe");
    await api.updateSettings({ jevEnabled: true });
    const { scanId, status } = await runToEnd(api, options);
    expect(status).toBe("completed");
    const detail = await api.getScan(scanId);
    const step = (node: string) => detail.steps.find((candidate) => candidate.node === node);
    return { detail, step, logs: await api.getLogs(scanId) };
  }
  const byModel = (scripts: ScriptRecord[], model: string) =>
    scripts.filter((script) => script.classifierModel === model);

  test("Jev classifies every undecided script, logs each decision, and guides the logic analysis", async () => {
    const { detail, step, logs } = await withJev({ anthropicKey: true });
    const judged = byModel(detail.scripts, "jev-1.14.0");
    expect(judged.length).toBeGreaterThan(10);
    expect(byModel(detail.scripts, "claude-haiku-5-5")).toHaveLength(0);
    for (const script of judged) {
      expect(script.classifiedBy).toBe("ai");
      expect(script.adProbability).toBeGreaterThanOrEqual(0);
      expect(script.adProbability).toBeLessThanOrEqual(1);
      // Under the drop threshold a script is out; under the analyse threshold only the publisher's own code stays.
      const p = script.adProbability ?? 0;
      expect(script.adRelated).toBe(p >= 0.5 || (p >= 0.15 && script.firstParty));
    }
    const byRule = detail.scripts.filter((script) => script.classifiedBy === "heuristic");
    expect(byRule.every((script) => script.classifierModel === null && script.adProbability === null)).toBe(true);

    const decisions = logs.filter((entry) => entry.kind === "classification");
    expect(decisions.map((entry) => (entry.data as { scriptId: string }).scriptId)).toEqual(
      judged.map((script) => script.id),
    );
    expect(new Set(decisions.map((entry) => (entry.data as { decision: { route: string } }).decision.route))).toEqual(
      new Set(["jev-yes", "jev-unsure", "jev-generic", "jev-no"]),
    );
    for (const entry of decisions) {
      expect((entry.data as { excerpt: string }).excerpt.length).toBeLessThanOrEqual(300);
    }

    expect(step("filterScripts")).toMatchObject({ status: "completed", model: "jev-1.14.0", outputTokens: 0 });
    expect(step("filterScripts")?.costUsd).toBeLessThan(0.01);
    expect(step("analyzeLogic")).toMatchObject({ status: "completed", model: "claude-sonnet-5-5, jev-1.14.0" });
    expect(logs.some((entry) => entry.message.startsWith("Jev System One reading guide:"))).toBe(true);
    expect(logs.some((entry) => entry.message.includes("Jev System One failed"))).toBe(false);
    expect(detail.summary.costUsd).toBeCloseTo(
      detail.steps.reduce((sum, candidate) => sum + candidate.costUsd, 0),
      10,
    );
  });

  test("with only a TypeSafe key, scripts are classified and the logic analysis is skipped", async () => {
    const { detail, step } = await withJev({ anthropicKey: false });
    expect(byModel(detail.scripts, "jev-1.14.0").length).toBeGreaterThan(10);
    expect(step("filterScripts")).toMatchObject({ status: "completed", model: "jev-1.14.0" });
    expect(step("beautify")).toMatchObject({ status: "skipped", summary: "No Anthropic API key is configured" });
    expect(step("analyzeLogic")).toMatchObject({ status: "skipped", summary: "No Anthropic API key is configured" });
    expect(step("synthesize")).toMatchObject({ status: "completed", model: null });
    expect(detail.scripts.some((script) => script.analyzed)).toBe(false);
    expect(detail.findings.every((finding) => finding.source === "heuristic")).toBe(true);
    expect(detail.summary.aiUsed).toBe(true);
    expect(detail.summary.costUsd).toBeGreaterThan(0);
    expect(detail.summary.costUsd).toBeLessThan(0.01);
    expect(detail.overview).toContain("Scripts were classified by AI, but wrapper logic was not analysed");
    expect(detail.overview).not.toContain("Only deterministic checks ran");
  });

  test("when Jev fails mid-scan the remaining scripts go to Claude Haiku, with a warning and a caveat", async () => {
    const { detail, step, logs } = await withJev({ anthropicKey: true, jevFails: true });
    const byJev = byModel(detail.scripts, "jev-1.14.0");
    const byHaiku = byModel(detail.scripts, "claude-haiku-5-5");
    expect(byJev).toHaveLength(4);
    expect(byHaiku.length).toBeGreaterThan(5);
    expect(byHaiku.every((script) => script.adProbability === null)).toBe(true);

    const warning = logs.find((entry) => entry.level === "warn" && entry.message.includes("Jev System One failed"));
    expect(warning?.message).toBe(
      `Jev System One failed during script triage (rate limited by the API after 3 attempts); ${byHaiku.length} scripts fall back to claude-haiku-5-5`,
    );
    expect(warning?.data).toEqual({
      from: "jev-1.14.0",
      to: "claude-haiku-5-5",
      reason: "rate limited by the API after 3 attempts",
      remainingScripts: byHaiku.length,
    });
    expect(step("filterScripts")).toMatchObject({ status: "completed", model: "jev-1.14.0, claude-haiku-5-5" });
    expect(detail.overview).toContain(`\n\n**Note:** ${warning?.message}.`);
    // Jev is done for this scan: no reading guide either.
    expect(step("analyzeLogic")).toMatchObject({ status: "completed", model: "claude-sonnet-5-5" });
    expect(logs.some((entry) => entry.message.includes("reading guide"))).toBe(false);
    expect(detail.findings.some((finding) => finding.source === "ai")).toBe(true);
  });

  test("when Jev fails and there is no Anthropic key, the remaining scripts stay with the knowledge base", async () => {
    const { detail, step, logs } = await withJev({ anthropicKey: false, jevFails: true });
    expect(byModel(detail.scripts, "jev-1.14.0")).toHaveLength(4);
    expect(detail.scripts.filter((script) => script.classifiedBy === "ai")).toHaveLength(4);
    const warning = logs.find((entry) => entry.message.includes("Jev System One failed"));
    expect(warning).toMatchObject({ level: "warn", data: { to: "the knowledge base" } });
    expect(warning?.message).toMatch(/\d+ scripts stay as the knowledge base left them$/);
    expect(step("filterScripts")).toMatchObject({ status: "completed", model: "jev-1.14.0" });
    expect(step("analyzeLogic")?.status).toBe("skipped");
  });

  test("a stored key alone changes nothing: Jev is only used while the switch is on", async () => {
    const api = createFakeBackend({ profile: "fresh", speed: 5000 });
    await api.setApiKey(VALID_KEY);
    await api.setApiKey(TYPESAFE_KEY, "typesafe");
    const { scanId } = await runToEnd(api, options);
    const detail = await api.getScan(scanId);
    expect(detail.scripts.some((script) => script.adProbability !== null)).toBe(false);
    expect(detail.steps.find((step) => step.node === "filterScripts")?.model).toBe("claude-haiku-5-5");
  });

  test("history seeded before Jev was switched on keeps its Claude classification", async () => {
    const api = createFakeBackend({ profile: "seeded", speed: 5000 });
    await api.setApiKey(TYPESAFE_KEY, "typesafe");
    await api.updateSettings({ jevEnabled: true });
    const reloaded = createFakeBackend({ profile: "seeded", speed: 5000 });
    const detail = await reloaded.getScan("seed-sportsdesk-desktop");
    expect(detail.scripts.some((script) => script.adProbability !== null)).toBe(false);
  });
});

describe("seeded history", () => {
  const seeded = () => createFakeBackend({ profile: "seeded", speed: 5000 });

  test("lists newest first, pages, and filters by search, status and device", async () => {
    const api = seeded();
    const all = await api.listScans({ limit: 100 });
    expect(all.total).toBe(26);
    expect(all.items.map((scan) => scan.startedAt)).toEqual(
      [...all.items.map((scan) => scan.startedAt)].sort((a, b) => b - a),
    );
    expect(new Set(all.items.map((scan) => scan.status))).toEqual(new Set(["completed", "failed", "cancelled"]));

    const page = await api.listScans({ limit: 10, offset: 20 });
    expect(page.items).toHaveLength(6);
    expect(page.total).toBe(26);

    const sportsdesk = await api.listScans({ search: "SPORTSDESK" });
    expect(sportsdesk.items.every((scan) => scan.hostname === "www.sportsdesk.example")).toBe(true);
    expect((await api.listScans({ status: "failed" })).items.map((scan) => scan.id)).toEqual(["seed-unreachable"]);
    const mobile = await api.listScans({ device: "mobile", limit: 100 });
    expect(mobile.items.every((scan) => scan.device === "mobile")).toBe(true);
  });

  test("dashboard stats aggregate the history", async () => {
    const api = seeded();
    const stats = await api.getDashboardStats();
    const all = await api.listScans({ limit: 100 });
    expect(stats.totalScans).toBe(26);
    expect(stats.sitesScanned).toBe(new Set(all.items.map((scan) => scan.hostname)).size);
    expect(stats.totalCostUsd).toBeCloseTo(
      all.items.reduce((sum, scan) => sum + scan.costUsd, 0),
      10,
    );
    expect(stats.openCriticalFindings).toBeGreaterThan(0);
    expect(stats.avgHealthScore).toBeGreaterThan(0);
  });

  test("log search escapes text, marks matches and honours the limit", async () => {
    const api = seeded();
    const hits = await api.searchLogs("IS_MOBILE", 5);
    expect(hits.length).toBeGreaterThan(0);
    expect(hits.length).toBeLessThanOrEqual(5);
    for (const hit of hits) {
      expect(hit.snippet).toContain("<mark>IS_MOBILE</mark>");
      expect(hit.snippet.replaceAll(/<\/?mark>/g, "")).not.toMatch(/[<>]/);
    }
    expect(await api.searchLogs("   ")).toEqual([]);
    expect(await api.searchLogs("zzz-no-such-token")).toEqual([]);
  });

  test("deleting one scan and then all scans survives a restart", async () => {
    const api = seeded();
    await api.deleteScan("seed-unreachable");
    expect((await api.listScans({ limit: 100 })).total).toBe(25);
    expect((await seeded().listScans({ limit: 100 })).total).toBe(25);

    await api.deleteAllScans();
    expect((await api.listScans({})).total).toBe(0);
    expect((await api.getDashboardStats()).totalScans).toBe(0);
    expect((await seeded().listScans({})).total).toBe(0);
  });

  test("exports Markdown and JSON", async () => {
    const api = seeded();
    const markdown = await api.exportScan("seed-sportsdesk-desktop", "markdown");
    expect(markdown).toContain("# Ad stack scan: www.sportsdesk.example");
    expect(markdown).toContain("```js");
    const json = JSON.parse(await api.exportScan("seed-sportsdesk-desktop", "json"));
    expect(json.summary.id).toBe("seed-sportsdesk-desktop");
  });
});

test("settings and finished scans persist across restarts; running scans do not", async () => {
  const first = createFakeBackend({ profile: "fresh", speed: 5000 });
  await first.updateSettings({ theme: "dark", budgetUsd: 1.25 });
  const { scanId } = await runToEnd(first, options);
  const slow = createFakeBackend({ profile: "fresh", speed: 0.001 });
  const unfinished = await slow.startScan(options);

  const restarted = createFakeBackend({ profile: "fresh", speed: 5000 });
  expect(await restarted.getSettings()).toMatchObject({ theme: "dark", budgetUsd: 1.25 });
  expect((await restarted.listScans({})).items.map((scan) => scan.id)).toEqual([scanId]);
  await expect(restarted.updateSettings({ budgetUsd: 0 })).rejects.toMatchObject({ code: "invalidInput" });
  expect((await restarted.getSettings()).budgetUsd).toBe(1.25);
  await slow.cancelScan(unfinished.id);
});
