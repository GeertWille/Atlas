import type { LogEntry, Severity } from "@/bindings";
import { SEVERITIES } from "./vocabulary";

/** Longer messages are cut off in the one-line summary, so the row has to open to show them. */
const HEADLINE_FITS = 80;

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** `{ url: "a.js", startLine: 3 }` → `url: "a.js", startLine: 3`, for one-line tool call previews. */
export function summarizeToolInput(data: unknown): string {
  if (!isRecord(data)) return data == null ? "" : JSON.stringify(data);
  return Object.entries(data)
    .map(([key, value]) => `${key}: ${JSON.stringify(value)}`)
    .join(", ");
}

export type LogPayload = {
  /** Plain-text body (tool output), shown verbatim. */
  text: string | null;
  /** Remaining structured data, pretty-printed. */
  json: string | null;
};

/** Splits `data` into the raw tool output, if any, and everything else as JSON. */
export function logPayload(entry: LogEntry): LogPayload {
  const { data } = entry;
  if (data == null) return { text: null, json: null };
  if (isRecord(data) && typeof data.output === "string") {
    const { output, ...rest } = data;
    return { text: output, json: Object.keys(rest).length > 0 ? JSON.stringify(rest, null, 2) : null };
  }
  return { text: null, json: JSON.stringify(data, null, 2) };
}

export function messageExceedsHeadline(entry: LogEntry): boolean {
  return entry.message.length > HEADLINE_FITS || entry.message.includes("\n");
}

export function isExpandable(entry: LogEntry): boolean {
  return entry.data != null || messageExceedsHeadline(entry);
}

/** Severity carried in the data of a `finding` log entry, when present. */
export function findingSeverity(entry: LogEntry): Severity | null {
  const severity = isRecord(entry.data) ? entry.data.severity : null;
  return SEVERITIES.find((known) => known === severity) ?? null;
}

export function logEntryAsText(entry: LogEntry): string {
  const { text, json } = logPayload(entry);
  return [entry.message, text, json].filter(Boolean).join("\n\n");
}

/** Id of the finding a `finding` log entry reported, when the backend recorded it. */
export function reportedFindingId(entry: LogEntry): string | null {
  const id = entry.kind === "finding" && isRecord(entry.data) ? entry.data.findingId : null;
  return typeof id === "string" ? id : null;
}
