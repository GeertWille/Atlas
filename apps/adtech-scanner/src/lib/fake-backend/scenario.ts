import type {
  Finding,
  LogKind,
  LogLevel,
  PipelineNode,
  ScanDetail,
  ScanOptions,
  ScriptRecord,
  Settings,
  StepStatus,
} from "@/bindings";
import { aiAvailability } from "@/shared/domain/ai-availability";
import { costOf, isJevModel, JEV_MODEL } from "@/shared/domain/model-pricing";
import { buildDiagnostics } from "./diagnostics";
import { aiInvestigations, healthScore, heuristicFindings, overviewFor } from "./findings";
import {
  buildScripts,
  captureSite,
  computeMetrics,
  JEV_THRESHOLDS,
  jevParts,
  jevPublisherSpecific,
  jevRule,
} from "./site";

/** One scripted moment of a simulated scan, `at` ms after it started. */
export type Beat = { at: number } & (
  | { type: "stepStart"; node: PipelineNode; model: string | null }
  | { type: "stepEnd"; node: PipelineNode; status: StepStatus; summary: string }
  | { type: "log"; node: PipelineNode; level: LogLevel; kind: LogKind; message: string; data: unknown }
  | { type: "usage"; node: PipelineNode; model: string; inputTokens: number; outputTokens: number; costUsd: number }
  | { type: "finding"; finding: Finding }
  | { type: "fail"; node: PipelineNode; message: string }
);

export type ScanResult = Pick<
  ScanDetail,
  "metrics" | "findings" | "auctions" | "slots" | "scripts" | "overview" | "bidderDiagnostics"
> & {
  healthScore: number;
  prebidVersion: string | null;
  aiUsed: boolean;
};

export type Scenario = {
  beats: Beat[];
  /** `null` when the scenario ends in a failure. */
  result: ScanResult | null;
};

class Timeline {
  readonly beats: Beat[] = [];
  private now = 0;
  private node: PipelineNode = "launchBrowser";

  wait(ms: number) {
    this.now += ms;
  }

  start(node: PipelineNode, model: string | null = null) {
    this.node = node;
    this.beats.push({ at: this.now, type: "stepStart", node, model });
  }

  end(summary: string, status: StepStatus = "completed") {
    this.wait(60);
    this.beats.push({ at: this.now, type: "stepEnd", node: this.node, status, summary });
    this.wait(40);
  }

  skip(node: PipelineNode, reason: string) {
    this.beats.push({ at: this.now, type: "stepEnd", node, status: "skipped", summary: reason });
  }

  log(message: string, options: { after?: number; level?: LogLevel; kind?: LogKind; data?: unknown } = {}) {
    this.wait(options.after ?? 120);
    this.beats.push({
      at: this.now,
      type: "log",
      node: this.node,
      level: options.level ?? "info",
      kind: options.kind ?? "message",
      message,
      data: options.data ?? null,
    });
  }

  usage(model: string, inputTokens: number, outputTokens: number): number {
    const costUsd = costOf(model, { input: inputTokens, output: outputTokens }).usd;
    this.beats.push({ at: this.now, type: "usage", node: this.node, model, inputTokens, outputTokens, costUsd });
    return costUsd;
  }

  finding(finding: Finding) {
    this.beats.push({ at: this.now, type: "finding", finding });
    this.log(finding.title, {
      after: 0,
      kind: "finding",
      level: finding.severity === "critical" || finding.severity === "high" ? "warn" : "info",
      data: {
        findingId: finding.id,
        severity: finding.severity,
        category: finding.category,
        source: finding.source,
        confidence: finding.confidence,
      },
    });
  }

  fail(message: string) {
    this.wait(200);
    this.log(message, { after: 0, level: "error" });
    this.beats.push({ at: this.now, type: "fail", node: this.node, message });
  }
}

const VIEWPORT = { desktop: "1280×900", mobile: "390×844 @3x, touch" } as const;

/** The questions Jev answers about each part of an undecided script; the first one routes the script. */
const JEV_TRIAGE_QUESTIONS = [
  {
    id: "ad_orchestration",
    instructions: "Does the JavaScript in `code` call or configure an advertising stack for a web page?",
  },
  {
    id: "publisher_specific",
    instructions:
      "Is the JavaScript in `code` written or configured for one specific website, rather than being a generic library?",
  },
];
const JEV_GUIDE_QUESTIONS = [
  "ad_orchestration",
  "publisher_specific",
  "auction_gating",
  "device_detection",
  "timing_config",
  "lazy_load",
  "slot_definition",
  "consent_gating",
];
const JEV_QUESTION_SET = "jev-questions-v1";
const JEV_OUTCOME = {
  "jev-yes": "ad-related, Phase 2 candidate",
  "jev-unsure": "kept for analysis, ranked last",
  "jev-no": "not ad-related",
} as const;
/** Stand-ins for the (at most 300-character) excerpt of the best-scoring part that an audit entry keeps. */
const JEV_EXCERPT = {
  "jev-yes":
    "var IS_MOBILE = window.screen.width < 1440; pbjs.que.push(function () { pbjs.addAdUnits(adUnits); if (!IS_MOBILE) { pbjs.requestBids({ timeout: PREBID_TIMEOUT, bidsBackHandler: sendAdServerRequest }); } });",
  "jev-unsure":
    "window.dataLayer = window.dataLayer || []; dataLayer.push({ event: 'page_view', adSlotsOnPage: document.querySelectorAll('[id^=div-gpt-]').length });",
  "jev-no":
    "function hydrate(root, routes) { var match = routes.find(function (route) { return route.test(location.pathname); }); return match ? match.render(root) : null; }",
} as const;
/** A simulated outage lets Jev answer for this many scripts before it starts failing. */
const JEV_ANSWERS_BEFORE_FAILURE = 4;
const JEV_FAILURE = "rate limited by the API after 3 attempts";

// The backend's own skip reasons, and its order: a scan with AI turned off says so whatever keys exist.
const AI_TURNED_OFF = "AI analysis is turned off for this scan";
const NO_ANTHROPIC_KEY = "No Anthropic API key is configured";

export type ScenarioContext = {
  scanId: string;
  options: ScanOptions;
  settings: Settings;
  browserVersion: string;
  /** Jev stops answering part-way through script triage. */
  jevFails?: boolean;
};

export function buildScenario({
  scanId,
  options,
  settings,
  browserVersion,
  jevFails = false,
}: ScenarioContext): Scenario {
  const hostname = new URL(options.url).hostname;
  const timeline = new Timeline();
  const ai = aiAvailability(settings);
  const skipReason = (available: boolean) => (!options.aiEnabled ? AI_TURNED_OFF : available ? null : NO_ANTHROPIC_KEY);
  const triageSkipReason = skipReason(ai.triage !== null);
  const analysisSkipReason = skipReason(ai.logicAnalysis);
  const triageRan = triageSkipReason === null;
  const analysisRan = analysisSkipReason === null;
  const jevTriage = triageRan && ai.triage === "jev";
  const jevFailed = jevTriage && jevFails;
  const caveats: string[] = [];

  timeline.start("launchBrowser");
  timeline.log(`Using ${browserVersion}${settings.headless ? " (headless)" : ""}`, { after: 80 });
  timeline.log("Started browser with remote debugging on 127.0.0.1:9222", {
    after: 700,
    level: "debug",
    data: {
      args: ["--headless=new", "--remote-debugging-port=9222", "--disable-background-networking", "--no-first-run"],
    },
  });
  timeline.log("CDP session attached; Network, Page and Runtime domains enabled", { after: 260, level: "debug" });
  timeline.end("Browser ready");

  timeline.start("capture");
  timeline.log(`Emulating ${options.device} (${VIEWPORT[options.device]})`, { after: 80 });
  timeline.log("Tracker injected with Page.addScriptToEvaluateOnNewDocument", { level: "debug" });
  timeline.log(`Navigating to ${options.url}`, { after: 100 });

  if (/unreachable|offline|invalid/.test(hostname)) {
    timeline.wait(1400);
    timeline.fail(`Navigation failed: net::ERR_NAME_NOT_RESOLVED for ${hostname}`);
    return { beats: timeline.beats, result: null };
  }

  const site = captureSite(hostname, options.device);
  const scripts = buildScripts(site, {
    jev: jevTriage ? { model: JEV_MODEL, answers: jevFailed ? JEV_ANSWERS_BEFORE_FAILURE : Infinity } : null,
    claudeModel: triageRan && settings.hasApiKey ? settings.filterModel : null,
    logicAnalysis: analysisRan,
  });
  const metrics = computeMetrics(site, scripts);
  const firstAuction = site.auctions[0];
  const observeMs = options.observeSeconds * 1000;

  timeline.log(`DOMContentLoaded at ${Math.round(site.pageLoadMs * 0.38)} ms`, { after: 600, level: "debug" });
  if (options.acceptConsent) {
    timeline.log("Consent banner detected (OneTrust); clicked “Accept all”", {
      after: 300,
      data: { cmp: "OneTrust", tcfVersion: 2.2, selector: "#onetrust-accept-btn-handler" },
    });
  } else {
    timeline.log("Consent banner detected (OneTrust); left untouched", { after: 300, level: "warn" });
  }
  timeline.log(`Prebid.js v${site.prebidVersion} detected (window.pbjs); GPT and apstag present`, { after: 200 });
  if (firstAuction) {
    timeline.log(
      `auctionInit ${firstAuction.id}: ${firstAuction.adUnits.length} ad units, timeout ${firstAuction.timeoutMs} ms`,
      {
        after: 300,
        level: "debug",
        data: { auctionId: firstAuction.id, adUnitCodes: firstAuction.adUnits.map((unit) => unit.code) },
      },
    );
  }
  timeline.log(`Page load event at ${site.pageLoadMs} ms`, { after: 700 });
  const observeSlice = observeMs / 4;
  if (options.autoScroll) {
    timeline.log("Auto-scroll started: 4 steps to the bottom of the page", { after: observeSlice });
  }
  for (const auction of site.auctions.slice(1)) {
    timeline.log(`auctionInit ${auction.id}: ${auction.adUnits.length} ad units at ${auction.startedAtMs} ms`, {
      after: observeSlice / 2,
      level: "debug",
      data: { auctionId: auction.id, adUnitCodes: auction.adUnits.map((unit) => unit.code) },
    });
  }
  timeline.log(`Observation window closed after ${options.observeSeconds} s`, { after: observeSlice * 2 });
  const auctionCount = `${site.auctions.length} auction${site.auctions.length === 1 ? "" : "s"}`;
  timeline.log(
    `Captured ${scripts.length} scripts (${(metrics.totalScriptBytes / 1_048_576).toFixed(1)} MB), ${auctionCount}, ${site.slots.length} GPT slots`,
    {
      data: {
        scripts: scripts.length,
        auctions: site.auctions.length,
        slots: site.slots.length,
        bidRequests: metrics.bidRequestCount,
      },
    },
  );
  timeline.end(`${scripts.length} scripts, ${auctionCount}, ${site.slots.length} slots`);

  timeline.start("telemetry");
  timeline.log(
    `Derived ${metrics.bidRequestCount} bid requests from ${metrics.bidderCount} bidders: ${metrics.bidResponseCount} bids, ${metrics.noBidCount} no-bids, ${metrics.bidTimeoutCount} timeouts`,
    { data: metrics },
  );
  timeline.log(`Slot outcomes: ${site.slots.map((slot) => `${slot.elementId}=${slot.outcome}`).join(", ")}`, {
    level: "debug",
  });
  timeline.end(
    `Fill rate ${Math.round(metrics.fillRate * 100)}%, timeout rate ${Math.round(metrics.timeoutRate * 100)}%`,
  );

  timeline.start("bidderDiagnostics");
  const diagnostics = buildDiagnostics(site, options.url, settings.checkSellersJson);
  timeline.log(`Requesting ${diagnostics.adsTxt.url}`, { after: 60 });
  timeline.log(`${diagnostics.adsTxt.url}: ${diagnostics.adsTxt.recordCount} seller records, 0 invalid lines`, {
    after: 260,
  });
  if (settings.checkSellersJson) {
    timeline.log("Requesting sellers.json from the exchanges that ads.txt names", { after: 420 });
  }
  const flaggedChecks = diagnostics.bidders
    .flatMap((bidder) => bidder.checks)
    .filter((check) => check.status === "problem" || check.status === "notice").length;
  timeline.end(`${diagnostics.bidders.length} bidders checked, ${flaggedChecks} need attention`);

  const findings: Finding[] = [];
  const addFinding = (draft: Omit<Finding, "id" | "scanId">) => {
    const finding = { ...draft, id: `${scanId}-f${findings.length + 1}`, scanId };
    findings.push(finding);
    timeline.wait(140);
    timeline.finding(finding);
  };

  timeline.start("heuristics");
  timeline.log("Evaluating 14 deterministic rules against telemetry", { level: "debug" });
  for (const draft of heuristicFindings(site, metrics)) addFinding(draft);
  const heuristicCount = findings.length;
  timeline.end(`${heuristicCount} of 14 rules matched`);

  let spent = 0;
  if (triageRan) {
    runScriptTriage();
  } else {
    timeline.log(`Skipping AI phases: ${triageSkipReason}`, { after: 0, level: "warn" });
    timeline.skip("filterScripts", triageSkipReason);
  }
  if (analysisRan) {
    runLogicAnalysis();
  } else {
    if (triageRan) timeline.log(`Skipping logic analysis: ${analysisSkipReason}`, { after: 0, level: "warn" });
    timeline.skip("beautify", analysisSkipReason);
    timeline.skip("analyzeLogic", analysisSkipReason);
  }

  timeline.start("synthesize", analysisRan ? settings.filterModel : null);
  const score = healthScore(findings);
  const overview = [
    overviewFor(site, findings, analysisRan ? "analysed" : triageRan ? "triaged" : "deterministic"),
    ...caveats.map((caveat) => `**Note:** ${caveat}.`),
  ].join("\n\n");
  timeline.log(`Ranked ${findings.length} findings; no duplicates between rule and model findings`, { after: 200 });
  if (analysisRan) {
    timeline.log(`Writing overview with ${settings.filterModel}`, {
      after: 150,
      kind: "llmRequest",
      data: { model: settings.filterModel, maxTokens: 300, findings: findings.map((finding) => finding.title) },
    });
    timeline.wait(900);
    timeline.usage(settings.filterModel, 2_140, 96);
    timeline.log(overview, {
      after: 0,
      kind: "llmResponse",
      data: { stopReason: "end_turn", usage: { inputTokens: 2_140, outputTokens: 96 } },
    });
  }
  timeline.log(`Ad stack health score: ${score}/100`, { after: 150 });
  timeline.end(`Health ${score}/100, ${findings.length} findings`);

  return {
    beats: timeline.beats,
    result: {
      metrics,
      findings,
      auctions: site.auctions,
      slots: site.slots,
      scripts,
      overview,
      bidderDiagnostics: diagnostics,
      healthScore: score,
      prebidVersion: site.prebidVersion,
      aiUsed: triageRan,
    },
  };

  function classifyWithClaude(toClassify: ScriptRecord[]) {
    timeline.log(`Classify ${toClassify.length} scripts as ad-related or not`, {
      kind: "llmRequest",
      data: {
        model: settings.filterModel,
        system:
          "You triage scripts for an ad-tech audit. For each script decide its category and whether it takes part in ad delivery. You see the URL, size and the first 1,200 characters only.",
        scripts: toClassify.map((script) => ({ id: script.id, url: script.url, bytes: script.sizeBytes })),
      },
    });
    timeline.wait(2600);
    spent += timeline.usage(settings.filterModel, 14_820, 1_260);
    timeline.log(
      `Classified ${toClassify.length} scripts: ${toClassify.filter((script) => script.adRelated).length} ad-related`,
      {
        after: 0,
        kind: "llmResponse",
        data: {
          stopReason: "end_turn",
          usage: { inputTokens: 14_820, outputTokens: 1_260 },
          classifications: toClassify.map((script) => ({
            id: script.id,
            category: script.category,
            adRelated: script.adRelated,
            confidence: script.classificationConfidence,
          })),
        },
      },
    );
  }

  function classifyWithJev(toClassify: ScriptRecord[]) {
    const parts = toClassify.map((script) => jevParts(script.sizeBytes));
    const requests = parts.reduce((sum, { judged }) => sum + judged, 0);
    timeline.log(
      `Jev System One: ${toClassify.length} scripts, up to ${requests} parts to judge, ${JEV_TRIAGE_QUESTIONS.length} questions each. Each request carries one part of one script; script source is not stored.`,
      {
        level: "debug",
        kind: "llmRequest",
        data: {
          model: JEV_MODEL,
          questionSet: JEV_QUESTION_SET,
          questions: JEV_TRIAGE_QUESTIONS,
          thresholds: JEV_THRESHOLDS,
        },
      },
    );
    for (const [index, script] of toClassify.entries()) {
      const adProbability = script.adProbability ?? 0;
      const rule = jevRule(adProbability);
      const { total, judged } = parts[index] ?? { total: 1, judged: 1 };
      timeline.log(
        `${script.id} ${script.url}: ad orchestration ${adProbability.toFixed(2)} (analyse >= 0.50, drop < 0.15) over ${judged} of ${total} parts -> ${JEV_OUTCOME[rule]} [${rule}]`,
        {
          after: 90,
          kind: "classification",
          data: {
            scriptId: script.id,
            classifier: "jev",
            model: JEV_MODEL,
            modelAnswered: JEV_MODEL,
            questionSet: JEV_QUESTION_SET,
            answers: [
              { questionId: "ad_orchestration", pYes: adProbability },
              { questionId: "publisher_specific", pYes: jevPublisherSpecific(script.firstParty, adProbability) },
            ],
            thresholds: JEV_THRESHOLDS,
            decision: { adRelated: script.adRelated, category: script.category, route: rule },
            parts: { total, chosen: judged },
            excerpt: JEV_EXCERPT[rule],
          },
        },
      );
    }
    spent += timeline.usage(JEV_MODEL, requests * 2_400, 0);
  }

  function runScriptTriage() {
    const byJev = scripts.filter((script) => script.classifierModel !== null && isJevModel(script.classifierModel));
    const byClaude = scripts.filter((script) => script.classifiedBy === "ai" && !byJev.includes(script));
    // Only a script that neither a rule nor a model decided has no confidence.
    const unclassified = scripts.filter((script) => script.classificationConfidence === null);
    const undecided = byJev.length + byClaude.length + unclassified.length;

    timeline.start("filterScripts", jevTriage ? JEV_MODEL : settings.filterModel);
    timeline.log(
      `${scripts.length - undecided} scripts matched known-vendor rules; ${undecided} sent to ${jevTriage ? "Jev System One" : "the model"}`,
      { after: 100 },
    );
    if (jevTriage) classifyWithJev(byJev);
    if (jevFailed) {
      const remaining = byClaude.length + unclassified.length;
      const fallback = settings.hasApiKey ? settings.filterModel : "the knowledge base";
      const outcome = settings.hasApiKey ? `fall back to ${fallback}` : "stay as the knowledge base left them";
      const caveat = `Jev System One failed during script triage (${JEV_FAILURE}); ${remaining} scripts ${outcome}`;
      caveats.push(caveat);
      timeline.log(caveat, {
        level: "warn",
        data: { from: JEV_MODEL, to: fallback, reason: JEV_FAILURE, remainingScripts: remaining },
      });
    }
    if (byClaude.length > 0) classifyWithClaude(byClaude);
    timeline.end(
      `Classified ${byJev.length + byClaude.length} of ${undecided} undecided scripts; ${metrics.adScriptCount} scripts are ad-related`,
    );
  }

  /** Jev marks where in the selected scripts the logic worth reading is, before the analysis model starts. */
  function buildReadingGuide(analyzed: ScriptRecord[]) {
    const chunks = analyzed.flatMap((script, index) =>
      Array.from({ length: Math.min(4, jevParts(script.sizeBytes).total) }, (_, part) => ({
        scriptId: script.id,
        lines: [1 + part * 160, (part + 1) * 160],
        rank: Math.round((0.94 - index * 0.09 - part * 0.21) * 100) / 100,
      })),
    );
    timeline.log(
      `Jev System One reading guide: ${chunks.length} chunks of the selected scripts, ${JEV_GUIDE_QUESTIONS.length} questions each. Each request carries one chunk; script source is not stored.`,
      {
        after: 200,
        level: "debug",
        kind: "llmRequest",
        data: { model: JEV_MODEL, questionSet: JEV_QUESTION_SET, questions: JEV_GUIDE_QUESTIONS, minRank: 0.3 },
      },
    );
    timeline.wait(600);
    spent += timeline.usage(JEV_MODEL, chunks.length * 2_600, 0);
    timeline.log(
      `Jev System One judged ${chunks.length} chunks; ${chunks.filter((chunk) => chunk.rank >= 0.3).length} ranked high enough for the reading guide`,
      { after: 0, level: "debug", data: { model: JEV_MODEL, chunks } },
    );
  }

  function runLogicAnalysis() {
    const analyzed = scripts.filter((script) => script.analyzed);

    timeline.start("beautify");
    for (const script of analyzed) {
      const name = script.inline ? "inline bootstrap" : script.url.split("/").pop();
      timeline.log(
        `Beautified ${name}: ${(script.sizeBytes / 1024).toFixed(1)} KB → ${Math.round(script.sizeBytes / 23).toLocaleString("en-US")} lines`,
        { after: 160, level: "debug" },
      );
    }
    timeline.end(`${analyzed.length} scripts ready for analysis`);

    timeline.start("analyzeLogic", settings.analysisModel);
    timeline.log(
      `Starting agentic analysis with ${settings.analysisModel} (effort ${settings.analysisEffort}, budget $${settings.budgetUsd.toFixed(2)})`,
      { after: 100 },
    );
    timeline.log("Find logic bugs in the wrapper that directly cause revenue loss", {
      kind: "llmRequest",
      data: {
        model: settings.analysisModel,
        effort: settings.analysisEffort,
        system:
          "You are an ad-tech reliability engineer. Use the tools to read beautified scripts and telemetry. Report only defects that lose revenue, each with code evidence.",
        tools: [
          "read_script",
          "search_scripts",
          "get_slot_timeline",
          "get_page_context",
          "query_bids",
          "report_finding",
        ],
        context: {
          scripts: analyzed.map((script) => script.url),
          telemetrySummary: `${metrics.slotCount} slots, ${metrics.emptySlotCount} empty, timeout rate ${Math.round(metrics.timeoutRate * 100)}%`,
        },
      },
    });
    if (jevTriage && !jevFailed) buildReadingGuide(analyzed);

    const effortScale = { low: 0.7, medium: 1, high: 1.45 }[settings.analysisEffort];
    let reported = 0;
    for (const [turn, investigation] of aiInvestigations(site).entries()) {
      if (spent >= settings.budgetUsd) {
        timeline.log(
          `Budget cap of $${settings.budgetUsd.toFixed(2)} reached after ${turn} turn${turn === 1 ? "" : "s"}; stopping analysis with partial results`,
          { level: "warn" },
        );
        break;
      }
      timeline.log(investigation.thinking, { after: 1800 * effortScale, kind: "thinking" });
      for (const tool of investigation.tools) {
        timeline.log(tool.name, { after: 900, kind: "toolCall", data: tool.input });
        timeline.log(`${tool.name} returned ${tool.result.split("\n").length} lines`, {
          after: 350,
          kind: "toolResult",
          data: { output: tool.result },
        });
      }
      timeline.log("report_finding", {
        after: 2200 * effortScale,
        kind: "toolCall",
        data: {
          title: investigation.finding.title,
          severity: investigation.finding.severity,
          category: investigation.finding.category,
          confidence: investigation.finding.confidence,
        },
      });
      addFinding(investigation.finding);
      reported += 1;
      const inputTokens = Math.round((34_000 + turn * 14_000) * effortScale);
      const outputTokens = Math.round((1_850 + turn * 240) * effortScale);
      spent += timeline.usage(settings.analysisModel, inputTokens, outputTokens);
      timeline.log(`Turn ${turn + 1} complete`, {
        after: 0,
        kind: "llmResponse",
        level: "debug",
        data: { stopReason: "tool_use", usage: { inputTokens, outputTokens } },
      });
    }
    timeline.end(`${reported} finding${reported === 1 ? "" : "s"} reported`);
  }
}
