import type { PipelineNode } from "@/bindings";

/** Execution order of the scan graph. */
export const PIPELINE_NODES: readonly PipelineNode[] = [
  "launchBrowser",
  "capture",
  "telemetry",
  "heuristics",
  "filterScripts",
  "beautify",
  "analyzeLogic",
  "synthesize",
];

export const PIPELINE_NODE_META: Record<PipelineNode, { label: string; description: string; usesModel: boolean }> = {
  launchBrowser: { label: "Launch browser", description: "Start local Chrome over CDP", usesModel: false },
  capture: { label: "Capture page", description: "Load the page with the tracker injected and observe it", usesModel: false },
  telemetry: { label: "Derive telemetry", description: "Build auctions, slots and metrics", usesModel: false },
  heuristics: { label: "Deterministic checks", description: "Run revenue-leak rules", usesModel: false },
  filterScripts: { label: "Filter scripts", description: "Phase 1: classify which scripts are ad-related", usesModel: true },
  beautify: { label: "Beautify", description: "Format the selected scripts", usesModel: false },
  analyzeLogic: { label: "Analyse logic", description: "Phase 2: agentic hunt for logic bugs", usesModel: true },
  synthesize: { label: "Synthesise", description: "Merge findings, score health, write overview", usesModel: false },
};
