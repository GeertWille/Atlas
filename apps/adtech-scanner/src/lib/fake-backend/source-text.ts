import { createRng, type Rng } from "./rng";

/** What a generated file reads like: an ad wrapper calls the ad stack, an application bundle carries long tables. */
export type CodeFlavor = "wrapper" | "application" | "library";

const VERBS = ["get", "set", "build", "resolve", "queue", "flush", "track", "render", "measure", "normalize"] as const;
const MORE_VERBS = [
  "schedule",
  "load",
  "parse",
  "merge",
  "bind",
  "collect",
  "apply",
  "reset",
  "observe",
  "emit",
] as const;
const NOUNS = [
  "Slot",
  "Unit",
  "Viewport",
  "Config",
  "Targeting",
  "Bid",
  "Queue",
  "Consent",
  "Timeout",
  "Refresh",
] as const;
const MORE_NOUNS = [
  "Sizes",
  "Placement",
  "Observer",
  "Batch",
  "Payload",
  "Route",
  "State",
  "Layout",
  "Session",
  "Cache",
] as const;
const LOCALS = ["node", "entry", "value", "index", "result", "options", "target", "state", "list", "item"] as const;
const WORDS = [
  "article",
  "gallery",
  "comments",
  "headline",
  "sidebar",
  "footer",
  "player",
  "newsletter",
  "search",
  "live",
] as const;

/** One-line calls into the ad stack, so a wrapper has something for the "Ad API calls" list. */
const AD_STATEMENTS = [
  "googletag.cmd.push(function () { googletag.pubads().enableSingleRequest(); });",
  "pbjs.setConfig({ bidderTimeout: PREBID_TIMEOUT, enableSendAllBids: true });",
  "pbjs.que.push(function () { pbjs.addAdUnits(adUnits); });",
  "googletag.defineSlot(unit.path, unit.sizes, unit.code).addService(googletag.pubads());",
  "window.__tcfapi('addEventListener', 2, onConsentChange);",
  "apstag.fetchBids({ slots: apsSlots, timeout: APS_TIMEOUT }, onApsBids);",
  "googletag.cmd.push(function () { googletag.enableServices(); });",
  "pbjs.setTargetingForGPTAsync(codes);",
] as const;

function name(rng: Rng): string {
  return `${rng.pick([...VERBS, ...MORE_VERBS])}${rng.pick([...NOUNS, ...MORE_NOUNS])}`;
}

function literal(rng: Rng): string {
  const kind = rng.int(0, 4);
  if (kind === 0) return String(rng.int(0, 2400));
  if (kind === 1) return `'${rng.pick(WORDS)}'`;
  if (kind === 2) return rng.chance(0.5) ? "true" : "false";
  if (kind === 3) return `${rng.pick(LOCALS)}.${rng.pick(WORDS)}`;
  return `${name(rng)}(${rng.pick(LOCALS)})`;
}

/** A long single statement: the string tables and route maps that make a minified bundle heavy. */
function table(rng: Rng): string {
  const entries = Array.from({ length: rng.int(110, 150) }, () => {
    const key = `${rng.pick(WORDS)}${rng.pick([...NOUNS, ...MORE_NOUNS])}`;
    return `${key}: '${rng.pick(WORDS)}/${rng.pick(WORDS)}-${rng.int(100, 999)}'`;
  });
  return `var ${rng.pick(WORDS)}${rng.pick(NOUNS)}Map = { ${entries.join(", ")} };`;
}

/** A self-contained top-level block of exactly `lines` lines (1, 3, 5, 7 or 9). */
function block(rng: Rng, lines: number, flavor: CodeFlavor): string[] {
  const local = rng.pick(LOCALS);
  const other = rng.pick(LOCALS.filter((candidate) => candidate !== local));
  if (lines === 1) {
    if (flavor === "wrapper" && rng.chance(0.3)) return [rng.pick(AD_STATEMENTS)];
    if (flavor === "application" && rng.chance(0.55)) return [table(rng)];
    return [`var ${rng.pick(WORDS)}${rng.pick(NOUNS)} = ${literal(rng)};`];
  }
  const head = `function ${name(rng)}(${local}, ${other}) {`;
  if (lines === 3) return [head, `  return ${local} ? ${literal(rng)} : ${other};`, "}"];
  if (lines === 5) {
    return [
      head,
      `  var next = ${literal(rng)};`,
      `  if (!${local}) return ${other};`,
      `  return ${name(rng)}(next, ${local});`,
      "}",
    ];
  }
  if (lines === 7) {
    return [
      head,
      `  var pending = ${local}.filter(function (candidate) {`,
      `    return candidate.${rng.pick(WORDS)} !== ${literal(rng)};`,
      "  });",
      `  ${name(rng)}(pending, ${other});`,
      "  return pending.length;",
      "}",
    ];
  }
  return [
    head,
    `  var total = ${rng.int(0, 9)};`,
    `  for (var i = 0; i < ${local}.length; i += 1) {`,
    `    if (${local}[i].${rng.pick(WORDS)} === ${literal(rng)}) {`,
    `      total += ${name(rng)}(${local}[i], ${other});`,
    "    }",
    "  }",
    `  return total > ${rng.int(1, 40)} ? total : ${literal(rng)};`,
    "}",
  ];
}

const BLOCK_SIZES = [9, 7, 5, 3, 1] as const;

/** Exactly `count` lines of plausible, syntactically complete JavaScript, blocks separated by blank lines. */
export function fillerLines(rng: Rng, count: number, flavor: CodeFlavor): string[] {
  const lines: string[] = [];
  while (lines.length < count) {
    const remaining = count - lines.length;
    // A block needs its own lines and, unless it ends the stretch, a blank line and more code after it.
    const fitting = BLOCK_SIZES.filter((size) => size === remaining || size + 2 <= remaining);
    const size = fitting.length > 0 ? rng.pick(fitting) : 0;
    if (size === 0) {
      lines.push("");
      continue;
    }
    lines.push(...block(rng, size, flavor));
    if (lines.length < count) lines.push("");
  }
  return lines;
}

export type PlantedExcerpt = { line: number; body: string };

/**
 * A file of `lineCount` lines in which every excerpt sits at the line it is cited at. Excerpts that
 * would overlap an earlier one are dropped: two findings citing the same lines share the text.
 */
export function generateFile(
  seed: number,
  lineCount: number,
  flavor: CodeFlavor,
  excerpts: PlantedExcerpt[],
): string[] {
  const rng = createRng(seed);
  const lines: string[] = [];
  for (const excerpt of [...excerpts].sort((a, b) => a.line - b.line)) {
    const start = excerpt.line - 1;
    if (start < lines.length) continue;
    // The lines around an excerpt stay blank, so the excerpt reads as its own block.
    const gap = start - lines.length;
    const after = lines.length > 0 && gap > 1 ? 1 : 0;
    if (gap > 0) lines.push(...Array<string>(after).fill(""), ...fillerLines(rng, gap - 1 - after, flavor), "");
    lines.push(...excerpt.body.split("\n"));
  }
  if (lines.length < lineCount) {
    if (lines.length > 0) lines.push("");
    lines.push(...fillerLines(rng, Math.max(0, lineCount - lines.length), flavor));
  }
  return lines;
}

/** The file as a minifier would have shipped it: one line, no indentation, line comments made safe. */
export function minify(lines: string[]): { text: string; lineStarts: number[] } {
  const lineStarts: number[] = [];
  let text = "";
  for (const line of lines) {
    lineStarts.push(text.length);
    const trimmed = line.trim();
    const comment = /^\/\/\s?(.*)$/.exec(trimmed);
    text += comment ? `/* ${comment[1]} */` : trimmed.replace(/\s\/\/\s?(.*)$/, " /* $1 */");
  }
  return { text, lineStarts };
}

/** The 1-based formatted line that holds 0-based `offset` of the minified text. */
export function lineAtOffset(lineStarts: number[], offset: number): number {
  let low = 0;
  let high = lineStarts.length - 1;
  while (low < high) {
    const middle = (low + high + 1) >> 1;
    if ((lineStarts[middle] ?? 0) <= offset) low = middle;
    else high = middle - 1;
  }
  return low + 1;
}
