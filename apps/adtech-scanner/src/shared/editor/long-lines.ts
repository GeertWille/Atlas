/** Above this, a line is costlier to tokenise and lay out than it is worth: highlighting goes off and wrap goes on. */
export const LONG_LINE_LIMIT = 10_000;

/** Whether any line of `text` is longer than `limit` characters. Stops at the first one. */
export function hasLongLine(text: string, limit: number = LONG_LINE_LIMIT): boolean {
  let lineStart = 0;
  while (lineStart <= text.length) {
    const lineEnd = text.indexOf("\n", lineStart);
    const end = lineEnd === -1 ? text.length : lineEnd;
    if (end - lineStart > limit) return true;
    if (lineEnd === -1) return false;
    lineStart = lineEnd + 1;
  }
  return false;
}
