import type {
  LogEntry,
  PipelineStep,
  ScanConfigurationRef,
  ScanDetail,
  ScanEvent,
  ScanOptions,
  ScanSummary,
} from "@/bindings";
import { PIPELINE_NODES } from "@/shared/domain/pipeline";
import type { Beat, ScanResult } from "./scenario";

export type StoredScan = { detail: ScanDetail; logs: LogEntry[] };

const EMPTY_METRICS: ScanDetail["metrics"] = {
  pageLoadMs: null,
  auctionCount: 0,
  adUnitCount: 0,
  bidderCount: 0,
  bidRequestCount: 0,
  bidResponseCount: 0,
  bidTimeoutCount: 0,
  noBidCount: 0,
  timeoutRate: 0,
  slotCount: 0,
  emptySlotCount: 0,
  fillRate: 0,
  avgWinningCpm: null,
  totalWinningCpm: 0,
  timeToFirstAuctionMs: null,
  timeToFirstAdMs: null,
  scriptCount: 0,
  adScriptCount: 0,
  totalScriptBytes: 0,
  prebidTimeoutMs: null,
  hasPrebid: false,
  hasGpt: false,
  hasAmazonTam: false,
};

export function createRunningScan(
  id: string,
  options: ScanOptions,
  startedAt: number,
  configuration: ScanConfigurationRef | null = null,
): StoredScan {
  const steps: PipelineStep[] = PIPELINE_NODES.map((node) => ({
    id: `${id}-${node}`,
    scanId: id,
    node,
    status: "pending",
    startedAt: null,
    durationMs: null,
    summary: null,
    model: null,
    inputTokens: 0,
    outputTokens: 0,
    costUsd: 0,
    logCount: 0,
  }));
  const summary: ScanSummary = {
    id,
    url: options.url,
    hostname: new URL(options.url).hostname,
    device: options.device,
    status: "running",
    startedAt,
    finishedAt: null,
    durationMs: null,
    costUsd: 0,
    findingCounts: { critical: 0, high: 0, medium: 0, low: 0, info: 0 },
    healthScore: null,
    prebidVersion: null,
    aiUsed: false,
    error: null,
    configuration,
  };
  return {
    detail: {
      summary,
      options,
      metrics: EMPTY_METRICS,
      findings: [],
      auctions: [],
      slots: [],
      scripts: [],
      steps,
      overview: null,
      bidderDiagnostics: null,
      pages: [],
      environment: null,
    },
    logs: [],
  };
}

function stepOf(scan: StoredScan, node: PipelineStep["node"]): PipelineStep {
  const step = scan.detail.steps.find((candidate) => candidate.node === node);
  if (!step) throw new Error(`Scan has no step for node ${node}`);
  return step;
}

function finishScan(scan: StoredScan, status: ScanSummary["status"], now: number) {
  const { summary } = scan.detail;
  summary.status = status;
  summary.finishedAt = now;
  summary.durationMs = now - summary.startedAt;
}

const snapshot = <T>(value: T): T => structuredClone(value);

/** Applies one beat to the scan in place and returns the events a backend would emit for it. */
export function applyBeat(scan: StoredScan, beat: Beat, now: number): ScanEvent[] {
  const { summary } = scan.detail;
  switch (beat.type) {
    case "awaitFinish":
      return [];
    case "stepStart": {
      const step = stepOf(scan, beat.node);
      step.status = "running";
      step.startedAt = now;
      step.model = beat.model;
      return [{ type: "stepUpdated", step: snapshot(step) }];
    }
    case "stepEnd": {
      const step = stepOf(scan, beat.node);
      step.status = beat.status;
      step.summary = beat.summary;
      step.durationMs = step.startedAt === null ? null : now - step.startedAt;
      return [{ type: "stepUpdated", step: snapshot(step) }];
    }
    case "log": {
      const step = stepOf(scan, beat.node);
      step.logCount += 1;
      const entry: LogEntry = {
        id: `${summary.id}-l${scan.logs.length + 1}`,
        scanId: summary.id,
        stepId: step.id,
        node: beat.node,
        ts: now,
        level: beat.level,
        kind: beat.kind,
        message: beat.message,
        data: beat.data,
      };
      scan.logs.push(entry);
      return [{ type: "log", entry: snapshot(entry) }];
    }
    case "usage": {
      const step = stepOf(scan, beat.node);
      step.inputTokens += beat.inputTokens;
      step.outputTokens += beat.outputTokens;
      step.costUsd += beat.costUsd;
      // Like the backend: a step that called a second model (a fallback) names both.
      if (step.model === null) step.model = beat.model;
      else if (!step.model.split(", ").includes(beat.model)) step.model = `${step.model}, ${beat.model}`;
      summary.costUsd += beat.costUsd;
      summary.aiUsed = true;
      return [
        { type: "stepUpdated", step: snapshot(step) },
        { type: "scanUpdated", summary: snapshot(summary) },
      ];
    }
    case "finding": {
      scan.detail.findings.push(beat.finding);
      summary.findingCounts[beat.finding.severity] += 1;
      return [
        { type: "findingAdded", finding: snapshot(beat.finding) },
        { type: "scanUpdated", summary: snapshot(summary) },
      ];
    }
    case "fail": {
      const step = stepOf(scan, beat.node);
      step.status = "failed";
      step.summary = beat.message;
      step.durationMs = step.startedAt === null ? null : now - step.startedAt;
      summary.error = beat.message;
      finishScan(scan, "failed", now);
      return [
        { type: "stepUpdated", step: snapshot(step) },
        { type: "scanUpdated", summary: snapshot(summary) },
      ];
    }
  }
}

export function completeScan(scan: StoredScan, result: ScanResult, now: number): ScanEvent[] {
  const { healthScore, prebidVersion, aiUsed, ...analysis } = result;
  Object.assign(scan.detail, analysis);
  Object.assign(scan.detail.summary, { healthScore, prebidVersion, aiUsed });
  finishScan(scan, "completed", now);
  return [{ type: "scanUpdated", summary: snapshot(scan.detail.summary) }];
}

export function cancelScan(scan: StoredScan, now: number): ScanEvent[] {
  const { steps } = scan.detail;
  const running = steps.find((step) => step.status === "running");
  // Between two steps nothing is running; the note then belongs to the step that just finished.
  const current = running ?? steps.findLast((step) => step.status !== "pending") ?? steps[0];
  const events: ScanEvent[] = [];
  if (current) {
    events.push(
      ...applyBeat(
        scan,
        {
          at: 0,
          type: "log",
          node: current.node,
          level: "warn",
          kind: "message",
          message: "Scan cancelled by user",
          data: null,
        },
        now,
      ),
    );
  }
  if (running) {
    events.push(
      ...applyBeat(scan, { at: 0, type: "stepEnd", node: running.node, status: "failed", summary: "Cancelled" }, now),
    );
  }
  finishScan(scan, "cancelled", now);
  events.push({ type: "scanUpdated", summary: snapshot(scan.detail.summary) });
  return events;
}
