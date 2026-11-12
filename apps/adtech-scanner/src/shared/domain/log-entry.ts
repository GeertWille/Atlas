import type { LogEntry, ScriptCategory, Severity } from "@/bindings";
import { SCRIPT_CATEGORIES, SEVERITIES } from "./vocabulary";

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
  /** Plain-text body (tool output, or the script excerpt a classification was made on), shown verbatim. */
  text: string | null;
  /** Remaining structured data, pretty-printed. */
  json: string | null;
};

/** Splits `data` into its plain-text body, if any, and everything else as JSON. */
export function logPayload(entry: LogEntry): LogPayload {
  const { data } = entry;
  if (data == null) return { text: null, json: null };
  const textKey = entry.kind === "classification" ? "excerpt" : "output";
  const text = isRecord(data) ? data[textKey] : null;
  if (isRecord(data) && typeof text === "string") {
    const rest = Object.entries(data).filter(([key]) => key !== textKey);
    return { text, json: rest.length > 0 ? JSON.stringify(Object.fromEntries(rest), null, 2) : null };
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

export type Threshold = { name: string; value: number };

export type ClassificationAnswer =
  /** A yes/no question: the probability that the answer is yes, and the cut-offs it was held against. */
  | { question: string; probability: number; thresholds: Threshold[] }
  /** A multiple-choice question: the option picked and how sure the classifier was. */
  | { question: string; selected: string; confidence: number | null };

export type ClassificationDetails = {
  answers: ClassificationAnswer[];
  /** Every named cut-off of the routing rules, e.g. `analyse 0.5`, `drop 0.15`. */
  thresholds: Threshold[];
  adRelated: boolean | null;
  category: ScriptCategory | null;
  /** The rule that routed the script, e.g. `jev-yes`. */
  rule: string | null;
};

const isProbability = (value: unknown): value is number => typeof value === "number" && value >= 0 && value <= 1;
const snakeToCamel = (id: string) => id.replace(/_(\w)/g, (_match, letter: string) => letter.toUpperCase());

function parseAnswer(answer: unknown): ClassificationAnswer[] {
  if (!isRecord(answer) || typeof answer.questionId !== "string") return [];
  if (isProbability(answer.pYes)) return [{ question: answer.questionId, probability: answer.pYes, thresholds: [] }];
  if (typeof answer.selected !== "string") return [];
  return [
    {
      question: answer.questionId,
      selected: answer.selected,
      confidence: isProbability(answer.confidence) ? answer.confidence : null,
    },
  ];
}

/**
 * A threshold named after a question (`publisherSpecific` for `publisher_specific`) is that question's
 * cut-off. The others (`analyse`, `drop`) route the script on the triage question, which is listed first.
 */
function assignThresholds(answers: ClassificationAnswer[], thresholds: Threshold[]) {
  const named = new Set(answers.map((answer) => snakeToCamel(answer.question)));
  let triageSeen = false;
  for (const answer of answers) {
    if (!("probability" in answer)) continue;
    answer.thresholds = thresholds.filter((threshold) =>
      named.has(threshold.name) ? threshold.name === snakeToCamel(answer.question) : !triageSeen,
    );
    triageSeen = true;
  }
}

/**
 * The structured part of a `classification` entry: what the classifier answered and how that was routed.
 * The audit data is not part of the typed contract, so every field is read defensively; whatever is
 * not recognised stays visible in the entry's JSON.
 */
export function classificationDetails(entry: LogEntry): ClassificationDetails | null {
  if (entry.kind !== "classification" || !isRecord(entry.data)) return null;
  const { answers, thresholds, decision } = entry.data;
  const verdict = isRecord(decision) ? decision : {};
  const details: ClassificationDetails = {
    answers: Array.isArray(answers) ? answers.flatMap(parseAnswer) : [],
    thresholds: isRecord(thresholds)
      ? Object.entries(thresholds).flatMap(([name, value]) => (isProbability(value) ? [{ name, value }] : []))
      : [],
    adRelated: typeof verdict.adRelated === "boolean" ? verdict.adRelated : null,
    category: SCRIPT_CATEGORIES.find((known) => known === verdict.category) ?? null,
    rule: typeof verdict.route === "string" && verdict.route !== "" ? verdict.route : null,
  };
  assignThresholds(details.answers, details.thresholds);
  const empty = details.answers.length === 0 && details.adRelated === null && details.rule === null;
  return empty ? null : details;
}
