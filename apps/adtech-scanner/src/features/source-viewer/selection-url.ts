/** The part of the URL that names a place in a file: `?line=118&to=124`. */
export type LineSearch = { line?: number; to?: number };

export type LineSpan = { from: number; to: number };

/** The lines a link asks for, inside the file and in order; `null` when it names none. */
export function searchToSpan(search: LineSearch, lineCount: number): LineSpan | null {
  if (search.line === undefined || lineCount < 1) return null;
  const clamp = (line: number) => Math.min(Math.max(1, line), lineCount);
  const first = clamp(search.line);
  const second = clamp(search.to ?? search.line);
  return { from: Math.min(first, second), to: Math.max(first, second) };
}

/**
 * What the URL says for a selection. A caret on one line is `line` alone; a selection that ends at the
 * start of a line does not include that line, as in every editor.
 */
export function spanToSearch(span: LineSpan): Required<Pick<LineSearch, "line">> & LineSearch {
  return span.to > span.from ? { line: span.from, to: span.to } : { line: span.from, to: undefined };
}

export function sameSpan(a: LineSpan | null, b: LineSpan | null): boolean {
  return a?.from === b?.from && a?.to === b?.to;
}

/** The lines a selection covers, given the line of its start and of its end and the end's column. */
export function selectionSpan(fromLine: number, toLine: number, toColumn: number): LineSpan {
  const lastLine = toLine > fromLine && toColumn === 1 ? toLine - 1 : toLine;
  return { from: fromLine, to: lastLine };
}
