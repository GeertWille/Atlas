import type { Finding, Severity, SourceAnchor, SourceRegion } from "@/bindings";
import { SEVERITIES } from "@/shared/domain/vocabulary";

/** A finding's code evidence placed in the file: where it is, and what to say about it. */
export type FindingMark = {
  findingId: string;
  evidenceIndex: number;
  title: string;
  severity: Severity;
  /** 1-based, inclusive, inside the file. */
  startLine: number;
  endLine: number;
  /** False when the excerpt was not found again and the stored lines are shown as they are. */
  located: boolean;
};

const rank = (severity: Severity) => SEVERITIES.indexOf(severity);

export function worstSeverity(severities: Severity[]): Severity | null {
  return [...severities].sort((a, b) => rank(a) - rank(b))[0] ?? null;
}

/**
 * The anchors a file's findings can be drawn at, top to bottom. An anchor of a finding the report no
 * longer has is left out; lines are clamped to the file, so a stale anchor never points past its end.
 */
export function findingMarks(anchors: SourceAnchor[], findings: Finding[], lineCount: number): FindingMark[] {
  if (lineCount < 1) return [];
  const clamp = (line: number) => Math.min(Math.max(1, line), lineCount);
  return anchors
    .flatMap((anchor): FindingMark[] => {
      const finding = findings.find((candidate) => candidate.id === anchor.findingId);
      if (!finding) return [];
      const startLine = clamp(anchor.startLine);
      return [
        {
          findingId: finding.id,
          evidenceIndex: anchor.evidenceIndex,
          title: finding.title,
          severity: finding.severity,
          startLine,
          endLine: Math.max(startLine, clamp(anchor.endLine)),
          located: anchor.located,
        },
      ];
    })
    .sort((a, b) => a.startLine - b.startLine || rank(a.severity) - rank(b.severity));
}

export type LineTint = {
  line: number;
  /** The most severe finding on the line decides its colour. */
  severity: Severity;
  /** First and last line of a marked range, for its top and bottom edge. */
  first: boolean;
  last: boolean;
};

/** One entry per marked line, in line order: what the editor tints. Overlapping ranges merge into the worse severity. */
export function lineTints(marks: FindingMark[]): LineTint[] {
  const tints = new Map<number, LineTint>();
  for (const mark of marks) {
    for (let line = mark.startLine; line <= mark.endLine; line += 1) {
      const existing = tints.get(line);
      const first = line === mark.startLine;
      const last = line === mark.endLine;
      if (!existing) {
        tints.set(line, { line, severity: mark.severity, first, last });
        continue;
      }
      if (rank(mark.severity) < rank(existing.severity)) existing.severity = mark.severity;
      existing.first ||= first;
      existing.last ||= last;
    }
  }
  return [...tints.values()].sort((a, b) => a.line - b.line);
}

export type GutterMark = {
  line: number;
  severity: Severity;
  located: boolean;
  /** Every finding that starts on this line, most severe first. */
  marks: FindingMark[];
};

/** One gutter marker per line a finding starts on. */
export function gutterMarks(marks: FindingMark[]): GutterMark[] {
  const byLine = new Map<number, FindingMark[]>();
  for (const mark of marks) byLine.set(mark.startLine, [...(byLine.get(mark.startLine) ?? []), mark]);
  return [...byLine.entries()]
    .sort(([a], [b]) => a - b)
    .map(([line, group]) => {
      const sorted = [...group].sort((a, b) => rank(a.severity) - rank(b.severity));
      return {
        line,
        severity: sorted[0]?.severity ?? "info",
        located: sorted.every((mark) => mark.located),
        marks: sorted,
      };
    });
}

/**
 * The finding to go to from `line` with F8 (forward) or Shift+F8: the next one that starts after the
 * caret, wrapping round at the ends. `null` when the file has none.
 */
export function adjacentMark(marks: FindingMark[], line: number, direction: 1 | -1): FindingMark | null {
  if (marks.length === 0) return null;
  if (direction === 1) return marks.find((mark) => mark.startLine > line) ?? marks[0] ?? null;
  return marks.findLast((mark) => mark.startLine < line) ?? marks[marks.length - 1] ?? null;
}

export type LineRange = { startLine: number; endLine: number };

/** The lines the AI was shown, as non-overlapping ranges in order. */
export function readRanges(regions: SourceRegion[], lineCount: number): LineRange[] {
  const sorted = regions
    .map((region) => ({
      startLine: Math.max(1, region.startLine),
      endLine: Math.min(lineCount, region.endLine),
    }))
    .filter((range) => range.endLine >= range.startLine)
    .sort((a, b) => a.startLine - b.startLine);
  const merged: LineRange[] = [];
  for (const range of sorted) {
    const last = merged[merged.length - 1];
    if (last && range.startLine <= last.endLine + 1) last.endLine = Math.max(last.endLine, range.endLine);
    else merged.push({ ...range });
  }
  return merged;
}

/** Share of the file's lines that the AI was shown, 0 to 1. */
export function readShare(regions: SourceRegion[], lineCount: number): number {
  if (lineCount < 1) return 0;
  const read = readRanges(regions, lineCount).reduce((sum, range) => sum + range.endLine - range.startLine + 1, 0);
  return read / lineCount;
}
