import type {
  AdSlot,
  Auction,
  Bid,
  Finding,
  LogEntry,
  PageView,
  PipelineStep,
  ProxyProfile,
  ScanConfiguration,
  ScanDetail,
  ScanEnvironment,
  ScanSummary,
  ScriptRecord,
  Settings,
} from "@/bindings";

export const settingsFixture: Settings = {
  filterModel: "claude-haiku-5-5",
  analysisModel: "claude-sonnet-5-5",
  analysisEffort: "medium",
  budgetUsd: 0.5,
  chromePath: null,
  headless: true,
  defaultDevice: "desktop",
  defaultObserveSeconds: 20,
  theme: "system",
  hasApiKey: true,
  jevEnabled: false,
  hasTypesafeKey: false,
  checkSellersJson: true,
  defaultVisitor: "scroll",
  defaultExtraPages: 2,
  persistentProfile: false,
  proxies: [],
  defaultProxy: { kind: "system" },
  keepNetworkData: true,
  archiveLimitBytes: 5 * 1024 ** 3,
  storeFullHeaders: false,
  defaultBrowser: "local",
  browserbase: {
    region: "us-west-2",
    defaultProxy: { kind: "none" },
    persistentProfile: false,
    connected: false,
    projectName: null,
    concurrency: null,
  },
};

/** Settings with Browserbase set up, as after saving a key whose one project is "Local". */
export const browserbaseSettingsFixture: Settings = {
  ...settingsFixture,
  browserbase: { ...settingsFixture.browserbase, connected: true, projectName: "Local", concurrency: 3 },
};

export function makeSummary(overrides: Partial<ScanSummary> = {}): ScanSummary {
  return {
    id: "scan-1",
    url: "https://www.example.com/news",
    hostname: "www.example.com",
    device: "desktop",
    status: "completed",
    startedAt: 1_700_000_000_000,
    finishedAt: 1_700_000_040_000,
    durationMs: 40_000,
    costUsd: 0.3,
    findingCounts: { critical: 0, high: 0, medium: 0, low: 0, info: 0 },
    healthScore: 80,
    prebidVersion: "9.41.0",
    aiUsed: true,
    error: null,
    configuration: null,
    proxy: null,
    remote: null,
    ...overrides,
  };
}

export function makeFinding(overrides: Partial<Finding> = {}): Finding {
  return {
    id: "f1",
    scanId: "scan-1",
    severity: "medium",
    category: "timing",
    source: "heuristic",
    title: "A finding",
    description: "What is wrong.",
    recommendation: "How to fix it.",
    confidence: 0.8,
    impact: "Some impact",
    estimatedRevenueImpactPct: null,
    evidence: [],
    affectedSlots: [],
    affectedBidders: [],
    pages: [],
    ...overrides,
  };
}

export function makeBid(overrides: Partial<Bid> = {}): Bid {
  return {
    bidder: "appnexus",
    status: "bid",
    cpm: 1,
    currency: "USD",
    size: "300x250",
    requestedAtMs: 1000,
    responseTimeMs: 200,
    dealId: null,
    exchange: null,
    ...overrides,
  };
}

export function makeAuction(bids: Bid[], overrides: Partial<Auction> = {}): Auction {
  const winner = bids.find((bid) => bid.status === "won" || bid.status === "rendered");
  return {
    id: "a1",
    page: 0,
    startedAtMs: 1000,
    endedAtMs: 2500,
    durationMs: 1500,
    timeoutMs: 1500,
    adUnits: [
      {
        code: "div-gpt-top",
        sizes: ["728x90"],
        bids,
        winningBidder: winner?.bidder ?? null,
        winningCpm: winner?.cpm ?? null,
      },
    ],
    ...overrides,
  };
}

export function makeSlot(overrides: Partial<AdSlot> = {}): AdSlot {
  return {
    page: 0,
    elementId: "div-gpt-top",
    adUnitPath: "/123/site/top",
    sizes: ["728x90"],
    outcome: "filled",
    renderedSize: "728x90",
    definedAtMs: 400,
    requestedAtMs: 2000,
    responseAtMs: 2200,
    renderedAtMs: 2300,
    viewableAtMs: 3500,
    inInitialViewport: true,
    hadPrebidTargeting: true,
    requestedBeforeAuctionEnd: false,
    advertiserId: null,
    lineItemId: null,
    requests: [],
    ...overrides,
  };
}

export function makeScript(overrides: Partial<ScriptRecord> = {}): ScriptRecord {
  return {
    id: "s1",
    url: "https://www.example.com/app.js",
    hostname: "www.example.com",
    inline: false,
    sizeBytes: 1000,
    hash: "0123456789abcdef",
    category: "application",
    adRelated: false,
    classificationReason: "Application bundle.",
    classificationConfidence: 0.9,
    classifiedBy: "ai",
    classifierModel: "claude-haiku-5-5",
    adProbability: null,
    analyzed: false,
    vendor: null,
    loadedAtMs: 500,
    firstParty: true,
    pages: [],
    initiator: null,
    requestId: null,
    sha256: null,
    ...overrides,
  };
}

export function makeStep(overrides: Partial<PipelineStep> = {}): PipelineStep {
  return {
    id: "scan-1-capture",
    scanId: "scan-1",
    node: "capture",
    status: "completed",
    startedAt: 1_700_000_000_000,
    durationMs: 1000,
    summary: null,
    model: null,
    inputTokens: 0,
    outputTokens: 0,
    costUsd: 0,
    logCount: 0,
    ...overrides,
  };
}

export function makeLog(overrides: Partial<LogEntry> = {}): LogEntry {
  return {
    id: "l1",
    scanId: "scan-1",
    stepId: "scan-1-capture",
    node: "capture",
    ts: 1_700_000_001_000,
    level: "info",
    kind: "message",
    message: "A log line",
    data: null,
    ...overrides,
  };
}

export function makeDetail(overrides: Partial<ScanDetail> = {}): ScanDetail {
  return {
    summary: makeSummary(),
    options: {
      url: "https://www.example.com/news",
      device: "desktop",
      observeSeconds: 20,
      autoScroll: true,
      acceptConsent: true,
      aiEnabled: true,
    },
    metrics: {
      pageLoadMs: 2100,
      auctionCount: 1,
      adUnitCount: 1,
      bidderCount: 2,
      bidRequestCount: 2,
      bidResponseCount: 1,
      bidTimeoutCount: 1,
      noBidCount: 0,
      timeoutRate: 0.5,
      slotCount: 2,
      emptySlotCount: 1,
      fillRate: 1,
      avgWinningCpm: 2,
      totalWinningCpm: 2,
      timeToFirstAuctionMs: 800,
      timeToFirstAdMs: 2300,
      scriptCount: 3,
      adScriptCount: 2,
      totalScriptBytes: 3000,
      prebidTimeoutMs: 1500,
      hasPrebid: true,
      hasGpt: true,
      hasAmazonTam: false,
    },
    findings: [],
    auctions: [],
    slots: [],
    scripts: [],
    steps: [],
    overview: null,
    bidderDiagnostics: null,
    pages: [],
    environment: null,
    network: null,
    ...overrides,
  };
}

export function makePage(overrides: Partial<PageView> = {}): PageView {
  return {
    index: 0,
    url: "https://www.example.com/news",
    title: "News",
    startedAtMs: 0,
    dwellMs: 20_000,
    reachedBy: "entry",
    link: null,
    loadMs: 2100,
    maxScrollDepth: 0.8,
    metrics: {
      auctionCount: 1,
      bidRequestCount: 2,
      bidResponseCount: 1,
      slotCount: 1,
      emptySlotCount: 0,
      fillRate: 1,
      timeToFirstAuctionMs: 800,
      timeToFirstAdMs: 2300,
    },
    ...overrides,
  };
}

export function makeEnvironment(overrides: Partial<ScanEnvironment> = {}): ScanEnvironment {
  return {
    visitor: "scroll",
    seed: 4711,
    headless: true,
    persistentProfile: false,
    // What Chrome reports in every mode: a local scripted visit shows only by running headless.
    webdriver: false,
    userAgentOverridden: false,
    viewportOverridden: true,
    emulatedMobile: false,
    detectors: [],
    proxy: { kind: "system", name: null, host: null },
    browser: { kind: "local" },
    ...overrides,
  };
}

export function makeProxy(overrides: Partial<ProxyProfile> = {}): ProxyProfile {
  return {
    id: "proxy-be",
    name: "Belgium",
    scheme: "http",
    host: "be.proxy.example",
    port: 8080,
    username: null,
    hasPassword: false,
    ...overrides,
  };
}

export function makeConfiguration(overrides: Partial<ScanConfiguration> = {}): ScanConfiguration {
  return {
    id: "config-1",
    name: "Example from Belgium",
    options: {
      url: "https://www.example.com/news",
      device: "desktop",
      observeSeconds: 20,
      autoScroll: true,
      acceptConsent: true,
      aiEnabled: true,
      visitor: "scroll",
      extraPages: 0,
      headless: true,
    },
    createdAt: 1_700_000_000_000,
    updatedAt: 1_700_000_000_000,
    lastScanId: null,
    lastRunAt: null,
    ...overrides,
  };
}
