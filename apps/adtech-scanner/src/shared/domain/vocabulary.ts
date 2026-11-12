import type {
  BidStatus,
  DeviceProfile,
  FindingCategory,
  FindingSource,
  LogKind,
  LogLevel,
  ScanStatus,
  ScriptCategory,
  Severity,
  SeverityCounts,
  SlotOutcome,
  StepStatus,
} from "@/bindings";
import type { Tone } from "@/shared/ui/badge";

type Meta = { label: string; tone: Tone };

/** Most severe first. */
export const SEVERITIES: readonly Severity[] = ["critical", "high", "medium", "low", "info"];

export const SEVERITY_META: Record<Severity, Meta> = {
  critical: { label: "Critical", tone: "red" },
  high: { label: "High", tone: "orange" },
  medium: { label: "Medium", tone: "amber" },
  low: { label: "Low", tone: "blue" },
  info: { label: "Info", tone: "gray" },
};

export function totalFindings(counts: SeverityCounts): number {
  return counts.critical + counts.high + counts.medium + counts.low + counts.info;
}

export const SCAN_STATUS_META: Record<ScanStatus, Meta> = {
  queued: { label: "Queued", tone: "gray" },
  running: { label: "Running", tone: "violet" },
  completed: { label: "Completed", tone: "green" },
  failed: { label: "Failed", tone: "red" },
  cancelled: { label: "Cancelled", tone: "gray" },
};

export function isActive(status: ScanStatus): boolean {
  return status === "queued" || status === "running";
}

export const STEP_STATUS_LABEL: Record<StepStatus, string> = {
  pending: "Pending",
  running: "Running",
  completed: "Completed",
  failed: "Failed",
  skipped: "Skipped",
};

export const DEVICE_LABEL: Record<DeviceProfile, string> = {
  desktop: "Desktop",
  mobile: "Mobile",
};

export function otherDevice(device: DeviceProfile): DeviceProfile {
  return device === "desktop" ? "mobile" : "desktop";
}

export const FINDING_CATEGORIES: readonly FindingCategory[] = [
  "logicBug",
  "timing",
  "configuration",
  "fill",
  "lazyLoad",
  "pricing",
  "hygiene",
  "consent",
];

export const FINDING_CATEGORY_LABEL: Record<FindingCategory, string> = {
  logicBug: "Logic bug",
  timing: "Timing",
  configuration: "Configuration",
  fill: "Fill",
  lazyLoad: "Lazy load",
  pricing: "Pricing",
  hygiene: "Hygiene",
  consent: "Consent",
};

export const FINDING_SOURCE_LABEL: Record<FindingSource, string> = {
  ai: "AI analysis",
  heuristic: "Deterministic rule",
};

/** Best outcome first: winners, then live bids, then the ways a bidder can fail. Legends and bid rows use this order. */
export const BID_STATUSES: readonly BidStatus[] = ["rendered", "won", "bid", "noBid", "timeout", "error", "pending"];

export const BID_STATUS_META: Record<BidStatus, Meta & { description: string }> = {
  rendered: { label: "Rendered", tone: "green", description: "Won the auction and its creative rendered" },
  won: { label: "Won", tone: "violet", description: "Highest bid in the Prebid auction" },
  bid: { label: "Bid", tone: "blue", description: "Bid received in time" },
  noBid: { label: "No bid", tone: "gray", description: "Bidder answered without a bid" },
  timeout: { label: "Timeout", tone: "amber", description: "No answer before the auction timeout" },
  error: { label: "Error", tone: "red", description: "Bidder request failed" },
  pending: { label: "Pending", tone: "outline", description: "Never answered during observation" },
};

export const SLOT_OUTCOME_META: Record<SlotOutcome, Meta & { description: string }> = {
  filled: { label: "Filled", tone: "green", description: "A creative rendered" },
  empty: { label: "Empty", tone: "red", description: "Ad server answered with no creative" },
  neverRequested: { label: "Never requested", tone: "orange", description: "Defined, but no ad request was made" },
  pending: { label: "Pending", tone: "gray", description: "Requested, no response during observation" },
};

export const SCRIPT_CATEGORIES: readonly ScriptCategory[] = [
  "prebid",
  "wrapper",
  "adServer",
  "bidder",
  "cmp",
  "adQuality",
  "analytics",
  "application",
  "library",
  "unknown",
];

export const SCRIPT_CATEGORY_META: Record<ScriptCategory, Meta> = {
  prebid: { label: "Prebid", tone: "violet" },
  wrapper: { label: "Wrapper", tone: "violet" },
  adServer: { label: "Ad server", tone: "blue" },
  bidder: { label: "Bidder", tone: "blue" },
  cmp: { label: "CMP", tone: "amber" },
  adQuality: { label: "Ad quality", tone: "green" },
  analytics: { label: "Analytics", tone: "gray" },
  application: { label: "Application", tone: "gray" },
  library: { label: "Library", tone: "gray" },
  unknown: { label: "Unknown", tone: "outline" },
};

export const LOG_KINDS: readonly LogKind[] = [
  "message",
  "llmRequest",
  "llmResponse",
  "thinking",
  "toolCall",
  "toolResult",
  "finding",
];

export const LOG_KIND_LABEL: Record<LogKind, string> = {
  message: "Message",
  llmRequest: "LLM request",
  llmResponse: "LLM response",
  thinking: "Thinking",
  toolCall: "Tool call",
  toolResult: "Tool result",
  finding: "Finding",
};

export const LOG_LEVELS: readonly LogLevel[] = ["debug", "info", "warn", "error"];

export const LOG_LEVEL_LABEL: Record<LogLevel, string> = {
  debug: "Debug",
  info: "Info",
  warn: "Warning",
  error: "Error",
};
