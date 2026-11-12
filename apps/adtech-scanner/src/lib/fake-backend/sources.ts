import type {
  AdCall,
  ScanDetail,
  ScriptRecord,
  ScriptSource,
  SourceAnchor,
  SourceAvailability,
  SourceFormat,
  SourceRegion,
  SourceView,
} from "@/bindings";
import { ApiError } from "@/lib/api/error";
import type { SourcePosition } from "@/lib/api/port";
import { isJevModel } from "@/shared/domain/model-pricing";
import { hashString } from "./rng";
import { type CodeFlavor, generateFile, lineAtOffset, minify } from "./source-text";

/** The application bundle is the large file of every simulated site: the viewer's 20,000-line case. */
const LARGE_FILE_LINES = 20_000;
const MAX_GENERATED_LINES = 6_000;
const BYTES_PER_LINE = 38;
/** Lines of the reading guide's chunks, as the backend cuts them. */
const GUIDE_CHUNK_LINES = 160;

const AD_CALL_PATTERNS: [name: string, pattern: RegExp][] = [
  ["requestBids", /\.requestBids\s*\(/],
  ["addAdUnits", /\.addAdUnits\s*\(/],
  ["setConfig", /\bpbjs\.setConfig\s*\(/],
  ["setTargetingForGPTAsync", /\.setTargetingForGPTAsync\s*\(/],
  ["defineSlot", /\.defineSlot\s*\(/],
  ["refresh", /\.refresh\s*\(/],
  ["enableServices", /\.enableServices\s*\(/],
  ["googletag.cmd", /\bgoogletag\.cmd\.push\s*\(/],
  ["apstag.fetchBids", /\bapstag\.fetchBids\s*\(/],
  ["__tcfapi", /__tcfapi\s*\(/],
];

type BuiltSource = {
  formatted: string[];
  format: SourceFormat;
  /** As captured: one minified line, or the same lines for a file that shipped readable. */
  original: string;
  /** Start offset of each formatted line in a minified original; `null` when the original kept its lines. */
  lineStarts: number[] | null;
};

function isLargeFile(script: ScriptRecord): boolean {
  return script.url.includes("/static/js/main.");
}

/** Shipped minified: everything but the small hand-written files. */
function isMinified(script: ScriptRecord): boolean {
  return !script.inline && !script.url.endsWith("/ad-config.js");
}

function formatOf(script: ScriptRecord): SourceFormat {
  // One file the parser rejects, and one the formatter gives up on, so both notes can be seen.
  if (script.url.includes("chunk-comments")) return "reflow";
  if (script.url.includes("pubads_impl")) return "unformatted";
  return "parser";
}

function flavorOf(script: ScriptRecord): CodeFlavor {
  if (script.category === "wrapper" || script.category === "prebid" || script.category === "adServer") return "wrapper";
  return script.category === "application" ? "application" : "library";
}

function build(detail: ScanDetail, script: ScriptRecord): BuiltSource {
  const excerpts = detail.findings.flatMap((finding) =>
    finding.evidence.flatMap((evidence) =>
      evidence.kind === "code" && evidence.scriptId === script.id && evidence.line !== null
        ? [{ line: evidence.line, body: evidence.body }]
        : [],
    ),
  );
  const lineCount = isLargeFile(script)
    ? LARGE_FILE_LINES
    : Math.min(MAX_GENERATED_LINES, Math.max(12, Math.round(script.sizeBytes / BYTES_PER_LINE)));
  const lines = generateFile(hashString(`${script.url}|source`), lineCount, flavorOf(script), excerpts);
  const format = formatOf(script);
  if (!isMinified(script)) return { formatted: lines, format, original: lines.join("\n"), lineStarts: null };
  const { text, lineStarts } = minify(lines);
  // A file the formatter gave up on is shown as it was captured, in both views.
  return format === "unformatted"
    ? { formatted: [text], format, original: text, lineStarts: null }
    : { formatted: lines, format, original: text, lineStarts };
}

function adCalls(lines: string[]): AdCall[] {
  const calls: AdCall[] = [];
  lines.forEach((line, index) => {
    for (const [name, pattern] of AD_CALL_PATTERNS) if (pattern.test(line)) calls.push({ name, line: index + 1 });
  });
  return calls;
}

function anchors(detail: ScanDetail, script: ScriptRecord): SourceAnchor[] {
  return detail.findings.flatMap((finding) =>
    finding.evidence.flatMap((evidence, evidenceIndex) =>
      evidence.kind === "code" && evidence.scriptId === script.id && evidence.line !== null
        ? [
            {
              findingId: finding.id,
              evidenceIndex,
              startLine: evidence.line,
              endLine: evidence.endLine ?? evidence.line,
              located: true,
            },
          ]
        : [],
    ),
  );
}

/** What Phase 2 read of a file it was handed: its head, and the stretch around everything it cited. */
function regions(detail: ScanDetail, script: ScriptRecord, lineCount: number): SourceRegion[] {
  if (!script.analyzed) return [];
  const clamp = (line: number) => Math.min(Math.max(1, line), lineCount);
  const read: SourceRegion[] = [
    { kind: "read", startLine: 1, endLine: clamp(60), rank: null },
    ...anchors(detail, script).map(
      (anchor): SourceRegion => ({
        kind: "read",
        startLine: clamp(anchor.startLine - 3),
        endLine: clamp(anchor.endLine + 18),
        rank: null,
      }),
    ),
  ];
  const guided = detail.scripts.some((other) => other.classifierModel !== null && isJevModel(other.classifierModel));
  if (!guided) return read;
  // The guide ranks the chunks that hold evidence first, then the head of the file.
  const cited = anchors(detail, script).map((anchor) => Math.floor((anchor.startLine - 1) / GUIDE_CHUNK_LINES));
  const chunks = [...new Set([...cited, 0])].slice(0, 4);
  return [
    ...read,
    ...chunks.map(
      (chunk, index): SourceRegion => ({
        kind: "guide",
        startLine: clamp(chunk * GUIDE_CHUNK_LINES + 1),
        endLine: clamp((chunk + 1) * GUIDE_CHUNK_LINES),
        rank: index + 1,
      }),
    ),
  ];
}

function availabilityOf(detail: ScanDetail): SourceAvailability {
  switch (detail.network?.availability) {
    case "available":
      return "available";
    case "disabled":
      return "notRetained";
    case "evicted":
      return "evicted";
    default:
      return "notRecorded";
  }
}

/** The generated file is megabytes for the large bundle: only the one being read is kept. */
let lastBuilt: { key: string; source: BuiltSource } | null = null;

function builtSource(detail: ScanDetail, script: ScriptRecord): BuiltSource {
  const key = `${detail.summary.id}|${script.id}`;
  if (lastBuilt?.key !== key) lastBuilt = { key, source: build(detail, script) };
  return lastBuilt.source;
}

/** Where a position in the captured source lies in the formatted text. */
export function formattedLineOf(detail: ScanDetail, script: ScriptRecord, position: SourcePosition): number {
  const source = builtSource(detail, script);
  if (!source.lineStarts) return Math.min(Math.max(1, position.line), source.formatted.length);
  return lineAtOffset(source.lineStarts, Math.max(0, (position.column ?? 1) - 1));
}

/** The position in the captured source of a formatted line, as an initiator or a stack frame reports it. */
export function capturedPositionOf(detail: ScanDetail, script: ScriptRecord, formattedLine: number): SourcePosition {
  const source = builtSource(detail, script);
  if (!source.lineStarts) return { line: formattedLine, column: 1 };
  return { line: 1, column: (source.lineStarts[formattedLine - 1] ?? 0) + 1 };
}

/** The 1-based formatted line of the first call of `name` in a script, for request initiators. */
export function adCallLine(detail: ScanDetail, script: ScriptRecord, name: string): number | null {
  return adCalls(builtSource(detail, script).formatted).find((call) => call.name === name)?.line ?? null;
}

export function fakeScriptSource(
  detail: ScanDetail,
  scriptId: string,
  view: SourceView,
  position?: SourcePosition,
): ScriptSource {
  const script = detail.scripts.find((candidate) => candidate.id === scriptId);
  if (!script) throw new ApiError("notFound", `Script ${scriptId} does not exist in scan ${detail.summary.id}`);
  const availability = availabilityOf(detail);
  const base = { scanId: detail.summary.id, scriptId, view, availability };
  if (availability !== "available") {
    return {
      ...base,
      text: null,
      lineCount: 0,
      format: "unformatted",
      anchors: [],
      regions: [],
      adCalls: [],
      focusLine: null,
    };
  }
  const source = builtSource(detail, script);
  if (view === "original") {
    const lines = source.original.split("\n");
    return {
      ...base,
      text: source.original,
      lineCount: lines.length,
      format: "unformatted",
      anchors: [],
      regions: [],
      adCalls: adCalls(lines),
      focusLine: position ? Math.min(Math.max(1, position.line), lines.length) : null,
    };
  }
  const located = source.format !== "unformatted";
  return {
    ...base,
    text: source.formatted.join("\n"),
    lineCount: source.formatted.length,
    format: source.format,
    anchors: located ? anchors(detail, script) : [],
    regions: located ? regions(detail, script, source.formatted.length) : [],
    adCalls: adCalls(source.formatted),
    focusLine: position ? formattedLineOf(detail, script, position) : null,
  };
}
