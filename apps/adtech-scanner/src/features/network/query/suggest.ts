import { CATEGORY_VALUES, IS_VALUES, QUERY_KEYS, TYPE_VALUES } from "./keys";
import type { IndexedRow } from "./match";

export type Suggestion = {
  /** What is shown in the list. */
  label: string;
  /** The filter with the suggestion in place of the word being typed. */
  query: string;
  /** Where the caret goes afterwards. */
  caret: number;
};

const MAX_SUGGESTIONS = 8;

/** Values worth offering for the keys whose values come from the scan itself. */
export type Facets = { vendors: string[]; bidders: string[]; hosts: string[] };

/** The vendors, bidders and hosts of a list, most frequent first. */
export function buildFacets(rows: IndexedRow[]): Facets {
  const tally = (pick: (indexed: IndexedRow) => string | null) => {
    const counts = new Map<string, number>();
    for (const indexed of rows) {
      const value = pick(indexed);
      if (value) counts.set(value, (counts.get(value) ?? 0) + 1);
    }
    return [...counts.entries()].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0])).map(([value]) => value);
  };
  return {
    vendors: tally(({ row }) => row.vendor),
    bidders: tally(({ row }) => row.bidder),
    hosts: tally(({ row }) => row.host),
  };
}

function valuesFor(key: string, facets: Facets): readonly string[] | null {
  switch (key) {
    case "is":
      return IS_VALUES;
    case "type":
      return Object.keys(TYPE_VALUES);
    case "cat":
      return Object.keys(CATEGORY_VALUES);
    case "vendor":
      return facets.vendors;
    case "bidder":
      return facets.bidders;
    case "host":
      return facets.hosts;
    case "method":
      return ["GET", "POST", "OPTIONS"];
    case "status":
      return ["2xx", "3xx", "4xx", "5xx"];
    default:
      return null;
  }
}

const quoted = (value: string) => (/[\s,]/.test(value) ? `"${value}"` : value);

/**
 * What could complete the word the caret is in: a filter key while a bare word is typed, or the
 * values of a key after its colon (after the last comma, for a list of alternatives).
 */
export function suggest(query: string, caret: number, facets: Facets): Suggestion[] {
  const before = query.slice(0, caret);
  const wordStart = before.search(/\S*$/);
  const word = before.slice(wordStart);
  const after = query.slice(caret);
  // In the middle of a word, or with nothing typed, there is nothing to complete.
  if (word === "" || /^\S/.test(after) || word.startsWith("/")) return [];
  const sign = word.startsWith("-") ? "-" : "";
  const body = word.slice(sign.length);
  const place = (replacement: string): Pick<Suggestion, "query" | "caret"> => ({
    query: `${query.slice(0, wordStart)}${replacement}${after}`,
    caret: wordStart + replacement.length,
  });

  const colon = body.indexOf(":");
  if (colon === -1) {
    const lower = body.toLowerCase();
    if (lower === "") return [];
    return QUERY_KEYS.filter((key) => key.startsWith(lower) && key !== lower)
      .slice(0, MAX_SUGGESTIONS)
      .map((key) => ({ label: `${key}:`, ...place(`${sign}${key}:`) }));
  }
  const key = body.slice(0, colon).toLowerCase();
  const values = valuesFor(key, facets);
  if (!values) return [];
  const typed = body.slice(colon + 1);
  const lastComma = typed.lastIndexOf(",");
  const kept = typed.slice(0, lastComma + 1);
  const partial = typed
    .slice(lastComma + 1)
    .replaceAll('"', "")
    .toLowerCase();
  return values
    .filter((value) => value.toLowerCase().includes(partial) && value.toLowerCase() !== partial)
    .slice(0, MAX_SUGGESTIONS)
    .map((value) => ({ label: value, ...place(`${sign}${key}:${kept}${quoted(value)}`) }));
}
