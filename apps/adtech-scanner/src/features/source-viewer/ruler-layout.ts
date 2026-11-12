import type { RegionKind, Severity, SourceRegion } from "@/bindings";
import type { FindingMark } from "./anchors";

const MIN_TICK_PX = 3;
const MIN_BAND_PX = 2;

export type RulerBand = { top: number; height: number; kind: RegionKind; opacity: number };
export type RulerTick = { top: number; height: number; severity: Severity; line: number; findingId: string };
export type RulerHit = { top: number };

export type RulerLayout = {
  /** Left lane: what the AI read, and the reading guide's chunks, stronger for a better rank. */
  bands: RulerBand[];
  /** Right lane: findings. */
  ticks: RulerTick[];
  /** Right lane, under the findings: search matches, at most one per pixel row. */
  hits: RulerHit[];
};

/** Where a 1-based line sits on a ruler of `height` pixels that stands for the whole file. */
export function lineToY(line: number, lineCount: number, height: number): number {
  if (lineCount <= 1) return 0;
  return ((Math.min(Math.max(1, line), lineCount) - 1) / lineCount) * height;
}

/** The line a click at `y` asks for. */
export function yToLine(y: number, lineCount: number, height: number): number {
  if (height <= 0 || lineCount < 1) return 1;
  return Math.min(lineCount, Math.max(1, Math.floor((y / height) * lineCount) + 1));
}

/** Rank 1 is the reading guide's best chunk; lower ranks fade. Lines the model read itself are solid. */
function bandOpacity(region: SourceRegion): number {
  if (region.kind === "read") return 0.75;
  return Math.max(0.25, 0.6 - ((region.rank ?? 1) - 1) * 0.1);
}

export function layoutRuler(input: {
  marks: FindingMark[];
  regions: SourceRegion[];
  /** 1-based lines of the search matches. */
  hitLines: number[];
  lineCount: number;
  height: number;
}): RulerLayout {
  const { marks, regions, hitLines, lineCount, height } = input;
  if (lineCount < 1 || height <= 0) return { bands: [], ticks: [], hits: [] };
  const span = (startLine: number, endLine: number, minimum: number) => {
    const top = lineToY(startLine, lineCount, height);
    const bottom = (Math.min(endLine, lineCount) / lineCount) * height;
    const size = Math.max(minimum, bottom - top);
    // A mark on the last lines still fits on the ruler.
    return { top: Math.min(top, height - size), height: size };
  };
  const rows = new Set<number>();
  const hits: RulerHit[] = [];
  for (const line of hitLines) {
    const row = Math.min(height - MIN_BAND_PX, Math.round(lineToY(line, lineCount, height)));
    if (rows.has(row)) continue;
    rows.add(row);
    hits.push({ top: row });
  }
  return {
    bands: regions.map((region) => ({
      ...span(region.startLine, Math.min(region.endLine, lineCount), MIN_BAND_PX),
      kind: region.kind,
      opacity: bandOpacity(region),
    })),
    ticks: marks.map((mark) => ({
      ...span(mark.startLine, mark.endLine, MIN_TICK_PX),
      severity: mark.severity,
      line: mark.startLine,
      findingId: mark.findingId,
    })),
    hits,
  };
}
