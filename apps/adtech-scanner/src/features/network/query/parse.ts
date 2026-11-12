import { isQueryKey, type QueryKey, valueProblem } from "./keys";

/** One word of a filter as it was typed, with where it sits in the text. */
export type QueryToken = { raw: string; start: number; end: number };

export type QueryTerm = { token: QueryToken; negated: boolean } & (
  | { kind: "text"; needle: string }
  | { kind: "regex"; pattern: RegExp }
  /** Any of `values` satisfies the term. */
  | { kind: "filter"; key: QueryKey; values: string[] }
);

/** Something the filter could not use as written. The words it covers are searched as plain text instead. */
export type QueryNote = { start: number; end: number; message: string };

export type ParsedQuery = { terms: QueryTerm[]; notes: QueryNote[] };

/**
 * Splits a filter into its words: separated by spaces, except inside double quotes and inside a
 * `/regular expression/`.
 */
export function tokenize(query: string): QueryToken[] {
  const tokens: QueryToken[] = [];
  let index = 0;
  while (index < query.length) {
    if (/\s/.test(query[index] ?? "")) {
      index += 1;
      continue;
    }
    const start = index;
    const body = query[index] === "-" ? index + 1 : index;
    if (query[body] === "/") {
      // A regular expression runs to the next unescaped slash, spaces included.
      let end = body + 1;
      while (end < query.length && !(query[end] === "/" && query[end - 1] !== "\\")) end += 1;
      if (end < query.length) {
        index = end + 1;
        tokens.push({ raw: query.slice(start, index), start, end: index });
        continue;
      }
    }
    let quoted = false;
    while (index < query.length && (quoted || !/\s/.test(query[index] ?? ""))) {
      if (query[index] === '"') quoted = !quoted;
      index += 1;
    }
    tokens.push({ raw: query.slice(start, index), start, end: index });
  }
  return tokens;
}

const unquote = (text: string) => text.replaceAll('"', "");

/** Splits `a,"b c",d` at the commas outside quotes. */
function splitValues(text: string): string[] {
  const values: string[] = [];
  let current = "";
  let quoted = false;
  for (const character of text) {
    if (character === '"') quoted = !quoted;
    else if (character === "," && !quoted) {
      values.push(current);
      current = "";
    } else current += character;
  }
  values.push(current);
  return values.filter((value) => value !== "");
}

function parseToken(token: QueryToken, notes: QueryNote[]): QueryTerm | null {
  const negated = token.raw.startsWith("-") && token.raw.length > 1;
  const body = negated ? token.raw.slice(1) : token.raw;
  const asText = (): QueryTerm | null => {
    const needle = unquote(body).toLowerCase();
    return needle === "" ? null : { token, negated, kind: "text", needle };
  };
  const note = (message: string) => {
    notes.push({ start: token.start, end: token.end, message: `${message}; searched as text` });
    return asText();
  };

  if (body.length > 2 && body.startsWith("/") && body.endsWith("/")) {
    try {
      return { token, negated, kind: "regex", pattern: new RegExp(body.slice(1, -1), "i") };
    } catch {
      return note("Not a valid regular expression");
    }
  }

  const colon = body.indexOf(":");
  const quote = body.indexOf('"');
  if (colon <= 0 || (quote !== -1 && quote < colon)) return asText();
  const key = body.slice(0, colon).toLowerCase();
  // A word with a colon that is not a filter key is most often a URL: `https://…` or `host:443`.
  if (!/^[a-z-]+$/.test(key)) return asText();
  if (!isQueryKey(key)) return /^https?$/.test(key) ? asText() : note(`Unknown filter “${key}:”`);
  const values = splitValues(body.slice(colon + 1));
  if (values.length === 0) return note(`“${key}:” needs a value`);
  for (const value of values) {
    const problem = valueProblem(key, value);
    if (problem) return note(problem);
  }
  return { token, negated, kind: "filter", key, values };
}

/** Reads a filter. Never fails: what cannot be used as a filter is searched as text, with a note saying so. */
export function parseQuery(query: string): ParsedQuery {
  const notes: QueryNote[] = [];
  const terms = tokenize(query).flatMap((token) => parseToken(token, notes) ?? []);
  return { terms, notes };
}
