import { getSearchQuery, searchPanelOpen, setSearchQuery } from "@codemirror/search";
import { type EditorState, StateField } from "@codemirror/state";

/** Counting stops here; beyond it the panel says "10,000+" and the ruler shows the first ones. */
const MATCH_CAP = 10_000;

export type SearchSummary = {
  /** Start offsets of the matches, in document order. */
  starts: readonly number[];
  /** There are more matches than were counted. */
  capped: boolean;
  /** The search panel is open with a query that can run. */
  active: boolean;
};

const IDLE: SearchSummary = { starts: [], capped: false, active: false };

function summarise(state: EditorState): SearchSummary {
  const query = getSearchQuery(state);
  if (!searchPanelOpen(state) || !query.valid) return IDLE;
  const cursor = query.getCursor(state);
  const starts: number[] = [];
  for (let match = cursor.next(); !match.done; match = cursor.next()) {
    if (starts.length === MATCH_CAP) return { starts, capped: true, active: true };
    starts.push(match.value.from);
  }
  return { starts, capped: false, active: true };
}

/** Where the current query matches, kept next to the search state so the panel, status bar and ruler agree. */
export const searchSummaryField = StateField.define<SearchSummary>({
  create: () => IDLE,
  update(value, transaction) {
    const queryChanged = transaction.effects.some((effect) => effect.is(setSearchQuery));
    const panelToggled = searchPanelOpen(transaction.startState) !== searchPanelOpen(transaction.state);
    return queryChanged || panelToggled || transaction.docChanged ? summarise(transaction.state) : value;
  },
});

/** 1-based position of the match the selection sits on, or `null` when the selection is not a match. */
export function currentMatch(summary: SearchSummary, selectionFrom: number): number | null {
  let low = 0;
  let high = summary.starts.length - 1;
  while (low <= high) {
    const middle = (low + high) >> 1;
    const start = summary.starts[middle] ?? 0;
    if (start === selectionFrom) return middle + 1;
    if (start < selectionFrom) low = middle + 1;
    else high = middle - 1;
  }
  return null;
}

/** "3 of 41", "41 results", "10,000+ results", "No results". */
export function describeMatches(summary: SearchSummary, selectionFrom: number): string {
  const total = summary.starts.length;
  if (total === 0) return "No results";
  const count = `${total.toLocaleString("en-US")}${summary.capped ? "+" : ""}`;
  const current = currentMatch(summary, selectionFrom);
  if (current !== null) return `${current.toLocaleString("en-US")} of ${count}`;
  return `${count} ${total === 1 && !summary.capped ? "result" : "results"}`;
}
