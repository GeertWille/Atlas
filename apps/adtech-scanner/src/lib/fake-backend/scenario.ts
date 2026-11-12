import type {
  Finding,
  LogKind,
  LogLevel,
  PipelineNode,
  ProxyUse,
  ScanDetail,
  ScanOptions,
  ScriptRecord,
  Settings,
  StepStatus,
  VisitorBehavior,
} from "@/bindings";
import { aiAvailability } from "@/shared/domain/ai-availability";
import { costOf, isJevModel, JEV_MODEL } from "@/shared/domain/model-pricing";
import { PAGE_REACH_LABEL, pagePath } from "@/shared/domain/page-views";
import { proxyUseLabel } from "@/shared/domain/proxy";
import { VISITOR_META, visitorOf } from "@/shared/domain/scan-options";
import { buildDiagnostics } from "./diagnostics";
import { aiInvestigations, type FindingDraft, healthScore, heuristicFindings, overviewFor } from "./findings";
import { buildScripts, computeMetrics, JEV_THRESHOLDS, jevParts, jevPublisherSpecific, jevRule } from "./site";
import { type CapturedVisit, captureVisit, type VisitedPage, visitMetrics } from "./visit";
import { AUTOMATED_TRAFFIC_CAVEAT, identifiableAsAutomated, visitFindings } from "./visit-findings";

/** One scripted moment of a simulated scan, `at` ms after it started. */
export type Beat = { at: number } & (
  | { type: "stepStart"; node: PipelineNode; model: string | null }
  | { type: "stepEnd"; node: PipelineNode; status: StepStatus; summary: string }
  | { type: "log"; node: PipelineNode; level: LogLevel; kind: LogKind; message: string; data: unknown }
  | { type: "usage"; node: PipelineNode; model: string; inputTokens: number; outputTokens: number; costUsd: number }
  | { type: "finding"; finding: Finding }
  | { type: "fail"; node: PipelineNode; message: string }
  /** A manual visit: the capture waits here until the user finishes it. */
  | { type: "awaitFinish" }
);

export type ScanResult = Pick<
  ScanDetail,
  "metrics" | "findings" | "auctions" | "slots" | "scripts" | "overview" | "bidderDiagnostics" | "pages" | "environment"
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

  /** A `visitor` audit entry, shaped like the ones the capture step stores for the real browser. */
  visitor(
    activity: "pageView" | "action" | "linkChoice" | "blocked" | "environment",
    message: string,
    page: number,
    detail: Record<string, unknown>,
    after = 120,
  ) {
    this.log(message, {
      after,
      kind: "visitor",
      level: activity === "blocked" ? "warn" : "info",
      data: { activity, page, detail },
    });
  }

  awaitFinish() {
    this.beats.push({ at: this.now, type: "awaitFinish" });
  }

  fail(message: string) {
    this.wait(200);
    this.log(message, { after: 0, level: "error" });
    this.beats.push({ at: this.now, type: "fail", node: this.node, message });
  }
}

const VIEWPORT = { desktop: "1280×900", mobile: "390×844 @3x, touch" } as const;

const pages = (count: number) => `${count} page${count === 1 ? "" : "s"}`;

type Gesture = { action: string; message: string; scrollDepth?: number; target?: string };

const SCROLL_STEPS: Gesture[] = [0.25, 0.5, 0.75, 1].map((scrollDepth) => ({
  action: "scroll",
  message: `Scrolled to ${scrollDepth * 100}%`,
  scrollDepth,
}));

/** What each behaviour does on a page, one audit entry per gesture. A manual session records none: the person acts. */
const GESTURES: Record<VisitorBehavior, Gesture[]> = {
  none: [],
  scroll: SCROLL_STEPS,
  browse: [
    { action: "pointerMove", message: "Moved the pointer over a heading", target: "heading" },
    { action: "scroll", message: "Scrolled to 30%", scrollDepth: 0.3 },
    { action: "hover", message: "Rested the pointer on a paragraph", target: "paragraph" },
    { action: "scroll", message: "Scrolled to 60%", scrollDepth: 0.6 },
    { action: "scrollBack", message: "Scrolled back to 45%", scrollDepth: 0.45 },
    { action: "scroll", message: "Scrolled to 85%", scrollDepth: 0.85 },
  ],
  manual: [],
};

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
  "jev-generic": "not ad-related, generic library",
  "jev-no": "not ad-related",
} as const;
/** Stand-ins for the (at most 300-character) excerpt of the best-scoring part that an audit entry keeps. */
const JEV_EXCERPT = {
  "jev-yes":
    "var IS_MOBILE = window.screen.width < 1440; pbjs.que.push(function () { pbjs.addAdUnits(adUnits); if (!IS_MOBILE) { pbjs.requestBids({ timeout: PREBID_TIMEOUT, bidsBackHandler: sendAdServerRequest }); } });",
  "jev-unsure":
    "window.dataLayer = window.dataLayer || []; dataLayer.push({ event: 'page_view', adSlotsOnPage: document.querySelectorAll('[id^=div-gpt-]').length });",
  "jev-generic":
    "!function(w,d,t){w.TrackerObject=t;var q=w[t]=w[t]||[];q.methods=['page','track','identify'];q.load=function(id){var s=d.createElement('script');s.async=!0;s.src='https://analytics.example/events.js?id='+id;d.head.appendChild(s)}}(window,document,'tq');",
  "jev-no":
    "function hydrate(root, routes) { var match = routes.find(function (route) { return route.test(location.pathname); }); return match ? match.render(root) : null; }",
} as const;
/** A simulated outage lets Jev answer for this many scripts before it starts failing. */
const JEV_ANSWERS_BEFORE_FAILURE = 4;
const JEV_FAILURE = "rate limited by the API after 3 attempts";

// The backend's own skip reasons, and its order: a scan with AI turned off says so whatever keys exist.
const AI_TURNED_OFF = "AI analysis is turned off for this scan";
const NO_ANTHROPIC_KEY = "No Anthropic API key is configured";

/** Estimated extra telemetry the logic analysis reads for each page view after the first. */
const TOKENS_PER_EXTRA_PAGE = 1_500;
/** A manual session visits this many pages unless the user finishes it earlier. */
const MANUAL_SESSION_PAGES = 3;
/** Simulated time the person spends on a page of a manual session before moving on. */
const MANUAL_PAGE_MS = 5_000;

export type ScenarioContext = {
  scanId: string;
  /** As the backend stores them: visitor, pages, seed, browser window, profile and budget are resolved. */
  options: ScanOptions;
  /** The proxy the scan runs through, resolved from the options and the settings. */
  proxy: ProxyUse;
  settings: Settings;
  browserVersion: string;
  /** Jev stops answering part-way through script triage. */
  jevFails?: boolean;
  /** A manual session that the user finished after this many page views. */
  manualPages?: number;
};

export function buildScenario({
  scanId,
  options,
  proxy,
  settings,
  browserVersion,
  jevFails = false,
  manualPages = MANUAL_SESSION_PAGES,
}: ScenarioContext): Scenario {
  const hostname = new URL(options.url).hostname;
  const timeline = new Timeline();
  const visitor = visitorOf(options);
  const headless = options.headless ?? settings.headless;
  const budgetUsd = options.budgetUsd ?? settings.budgetUsd;
  const pageCount = visitor === "manual" ? manualPages : 1 + (options.extraPages ?? 0);
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
  timeline.log(`Using ${browserVersion}${headless ? " (headless)" : ""}`, { after: 80 });
  timeline.log("Started browser with remote debugging on 127.0.0.1:9222", {
    after: 700,
    level: "debug",
    data: {
      args: [
        ...(headless ? ["--headless=new"] : []),
        "--remote-debugging-port=9222",
        "--disable-background-networking",
        "--no-first-run",
      ],
    },
  });
  timeline.log("CDP session attached; Network, Page and Runtime domains enabled", { after: 260, level: "debug" });
  timeline.end("Browser ready");

  timeline.start("capture");
  timeline.log(
    `Visit: ${VISITOR_META[visitor].label.toLowerCase()}${visitor === "browse" ? `, up to ${pageCount} pages` : ""}, seed ${options.seed ?? 0}, ${headless ? "headless" : "browser shown"}, ${options.persistentProfile ? "kept scan profile" : "fresh profile"}, ${proxyUseLabel(proxy).toLowerCase()}`,
    { after: 40 },
  );
  timeline.log(`Emulating ${options.device} (${VIEWPORT[options.device]})`, { after: 80 });
  timeline.log("Tracker injected with Page.addScriptToEvaluateOnNewDocument", { level: "debug" });
  timeline.log(`Navigating to ${options.url}`, { after: 100 });

  if (/unreachable|offline|invalid/.test(hostname)) {
    timeline.wait(1400);
    timeline.fail(`Navigation failed: net::ERR_NAME_NOT_RESOLVED for ${hostname}`);
    return { beats: timeline.beats, result: null };
  }

  const visit = captureVisit({
    url: options.url,
    device: options.device,
    visitor,
    pageCount,
    observeSeconds: options.observeSeconds,
    seed: options.seed ?? 0,
    headless,
    persistentProfile: options.persistentProfile ?? false,
    proxy,
  });
  const site = visit.entry;
  const documentPages = visit.pages.filter((page) => page.view.loadMs !== null).map((page) => page.view.index);
  const scripts = buildScripts(
    site,
    {
      jev: jevTriage ? { model: JEV_MODEL, answers: jevFailed ? JEV_ANSWERS_BEFORE_FAILURE : Infinity } : null,
      claudeModel: triageRan && settings.hasApiKey ? settings.filterModel : null,
      logicAnalysis: analysisRan,
    },
    documentPages,
  );
  const metrics = visitMetrics(visit, scripts);

  for (const page of visit.pages) visitPage(page);
  if (visitor === "manual") {
    timeline.awaitFinish();
    timeline.log(`Capture finished by you after ${pages(visit.pages.length)}`, { after: 0 });
  } else {
    timeline.log(`Observation window closed after ${options.observeSeconds} s`, { after: 200 });
  }
  const auctionCount = `${visit.auctions.length} auction${visit.auctions.length === 1 ? "" : "s"}`;
  timeline.log(
    `Captured ${scripts.length} scripts (${(metrics.totalScriptBytes / 1_048_576).toFixed(1)} MB), ${auctionCount}, ${visit.slots.length} GPT slots`,
    {
      data: {
        scripts: scripts.length,
        auctions: visit.auctions.length,
        slots: visit.slots.length,
        bidRequests: metrics.bidRequestCount,
        pages: visit.pages.length,
      },
    },
  );
  timeline.end(
    `${visit.pages.length > 1 ? `${pages(visit.pages.length)}, ` : ""}${scripts.length} scripts, ${auctionCount}, ${visit.slots.length} slots`,
  );

  timeline.start("telemetry");
  timeline.log(
    `Derived ${metrics.bidRequestCount} bid requests from ${metrics.bidderCount} bidders: ${metrics.bidResponseCount} bids, ${metrics.noBidCount} no-bids, ${metrics.bidTimeoutCount} timeouts`,
    { data: metrics },
  );
  timeline.log(`Slot outcomes: ${visit.slots.map((slot) => `${slot.elementId}=${slot.outcome}`).join(", ")}`, {
    level: "debug",
  });
  timeline.end(
    `Fill rate ${Math.round(metrics.fillRate * 100)}%, timeout rate ${Math.round(metrics.timeoutRate * 100)}%`,
  );

  timeline.start("bidderDiagnostics");
  const diagnostics = buildDiagnostics({ ...site, auctions: visit.auctions }, options.url, settings.checkSellersJson);
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
  const addFinding = (draft: FindingDraft) => {
    // Findings about what one page did belong to the entry page unless they say otherwise.
    const finding = { pages: [0], ...draft, id: `${scanId}-f${findings.length + 1}`, scanId };
    findings.push(finding);
    timeline.wait(140);
    timeline.finding(finding);
  };

  timeline.start("heuristics");
  timeline.log("Evaluating 14 deterministic rules against telemetry", { level: "debug" });
  for (const draft of heuristicFindings(site, computeMetrics(site, scripts))) addFinding(draft);
  for (const draft of visitFindings(visit)) addFinding({ pages: [], ...draft });
  if (identifiableAsAutomated(visit)) caveats.push(AUTOMATED_TRAFFIC_CAVEAT);
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
  const visitNote =
    visit.pages.length > 1
      ? ` The visit covered ${pages(visit.pages.length)}; the slot and auction counts in this summary are the entry page's.`
      : "";
  const overview = [
    `${overviewFor(site, findings, analysisRan ? "analysed" : triageRan ? "triaged" : "deterministic")}${visitNote}`,
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
      auctions: visit.auctions,
      slots: visit.slots,
      scripts,
      overview,
      bidderDiagnostics: diagnostics,
      pages: visit.pages.map((page) => page.view),
      environment: visit.environment,
      healthScore: score,
      prebidVersion: site.prebidVersion,
      aiUsed: triageRan,
    },
  };

  /** One page view of the visit: what the browser did there and what the page did in return. */
  function visitPage({ view, site: pageSite, story }: VisitedPage) {
    const { index } = view;
    const path = pagePath(view.url);
    timeline.visitor(
      "pageView",
      `Page ${index + 1}: ${path} (${PAGE_REACH_LABEL[view.reachedBy].toLowerCase()})`,
      index,
      {
        index,
        url: view.url,
        // The title is read after load: a page view is announced without one.
        title: null,
        reachedBy: view.reachedBy,
        startedAtMs: view.startedAtMs,
        link: view.link,
      },
      index === 0 ? 0 : visitor === "manual" ? MANUAL_PAGE_MS : 900,
    );
    if (view.loadMs !== null) {
      timeline.log(`DOMContentLoaded at ${Math.round(view.loadMs * 0.38)} ms`, { after: 600, level: "debug" });
    }
    if (index === 0) announceEnvironment(visit);
    if (index === 0 || story === "consentAskedAgain") handleConsent(index);
    if (index === 0) {
      timeline.log(`Prebid.js v${site.prebidVersion} detected (window.pbjs); GPT and apstag present`, { after: 200 });
    }
    const [firstAuction, ...laterAuctions] = pageSite.auctions;
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
    if (view.loadMs !== null) timeline.log(`Page load event at ${view.loadMs} ms`, { after: 700 });

    const slice = view.dwellMs / 4;
    if (visitor === "scroll") {
      timeline.log("Scrolling to the bottom of the page in 4 steps", { after: slice });
    }
    for (const [step, auction] of laterAuctions.entries()) {
      act(index, step);
      timeline.log(`auctionInit ${auction.id}: ${auction.adUnits.length} ad units at ${auction.startedAtMs} ms`, {
        after: slice / 2,
        level: "debug",
        data: { auctionId: auction.id, adUnitCodes: auction.adUnits.map((unit) => unit.code) },
      });
    }
    for (let step = laterAuctions.length; step < GESTURES[visitor].length; step += 1) act(index, step);
    timeline.wait(visitor === "manual" ? 0 : slice);

    const next = visit.pages[index + 1];
    if (visitor === "browse" && next) chooseLink(index, next);
  }

  function announceEnvironment({ environment }: CapturedVisit) {
    const signals = [
      environment.webdriver ? "navigator.webdriver is true" : null,
      environment.headless ? "headless" : null,
    ].filter((signal) => signal !== null);
    timeline.visitor(
      "environment",
      signals.length > 0
        ? `This visit is identifiable as automated: ${signals.join(", ")}`
        : "This visit is not flagged as automated: navigator.webdriver is false and the browser is visible",
      0,
      {
        webdriver: environment.webdriver,
        headless: environment.headless,
        persistentProfile: environment.persistentProfile,
        userAgentOverridden: environment.userAgentOverridden,
        viewportOverridden: environment.viewportOverridden,
      },
    );
  }

  function handleConsent(page: number) {
    if (!options.acceptConsent) {
      timeline.log(
        visitor === "manual"
          ? "Consent banner detected (OneTrust); left to you"
          : "Consent banner detected (OneTrust); left untouched",
        { after: 300, level: visitor === "manual" ? "info" : "warn" },
      );
      return;
    }
    timeline.log("Consent banner detected (OneTrust); clicked “Accept all”", {
      after: 300,
      data: { cmp: "OneTrust", tcfVersion: 2.2, selector: "#onetrust-accept-btn-handler" },
    });
    timeline.visitor("action", "Clicked the consent banner", page, { action: "consent" }, 0);
  }

  /** The visitor's next gesture on a page, spread over its dwell time. */
  function act(page: number, step: number) {
    const gesture = GESTURES[visitor][step];
    if (!gesture) return;
    const touch = options.device === "mobile";
    const action = touch && gesture.action === "scroll" ? "swipe" : gesture.action;
    if (touch && (action === "pointerMove" || action === "hover")) return;
    const dwellMs = visit.pages[page]?.view.dwellMs ?? 0;
    timeline.visitor(
      "action",
      touch && action === "swipe" ? gesture.message.replace("Scrolled", "Swiped") : gesture.message,
      page,
      { action, scrollDepth: gesture.scrollDepth, target: gesture.target },
      dwellMs / (GESTURES[visitor].length + 4),
    );
  }

  function chooseLink(page: number, next: VisitedPage) {
    const href = next.view.url;
    const origin = new URL(href).origin;
    timeline.visitor("linkChoice", `Following link: ${pagePath(href)} (best of 5)`, page, {
      chosen: {
        text: next.view.title,
        href,
        score: 6,
        reasons: ["content link", "article-like path", "text of 25-120 characters"],
      },
      alternatives: [
        { text: "Rail strike called off after late talks", href: `${origin}/2026/11/rail-strike-called-off`, score: 5 },
        { text: "Sport", href: `${origin}/sport`, score: 2 },
        { text: "Opinion: the case for slower news", href: `${origin}/opinion/slower-news`, score: 2 },
        { text: "Newsletters", href: `${origin}/newsletters`, score: 1 },
      ],
      skipped: { "off-site": 9, "inside-ad": 3, sensitive: 2, file: 1, visited: page },
    });
    if (page === 0) {
      timeline.visitor(
        "blocked",
        "Closed a pop-up window (partner-offers.example): not recorded",
        page,
        { reason: "popup", host: "partner-offers.example" },
        200,
      );
    }
    timeline.visitor("action", `Clicked “${next.view.title}”`, page, { action: "click", target: "link" }, 400);
  }

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
      const publisherSpecific = jevPublisherSpecific(script.firstParty, adProbability);
      const rule = jevRule(adProbability, publisherSpecific);
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
              { questionId: "publisher_specific", pYes: publisherSpecific },
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
      `Starting agentic analysis with ${settings.analysisModel} (effort ${settings.analysisEffort}, budget $${budgetUsd.toFixed(2)})`,
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
      if (spent >= budgetUsd) {
        timeline.log(
          `Budget cap of $${budgetUsd.toFixed(2)} reached after ${turn} turn${turn === 1 ? "" : "s"}; stopping analysis with partial results`,
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
      const pageTokens = turn === 0 ? (visit.pages.length - 1) * TOKENS_PER_EXTRA_PAGE : 0;
      const inputTokens = Math.round((34_000 + turn * 14_000) * effortScale) + pageTokens;
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
