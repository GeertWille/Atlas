export type QueryParam = {
  key: string;
  value: string;
  /** The parameters packed inside this one, for the few that carry a query string of their own. */
  nested: { key: string; value: string }[];
};

/** Google Ad Manager packs key-values into single parameters: custom targeting, and one set per slot. */
const NESTED_PARAMS = new Set(["cust_params", "prev_scp", "scp"]);

function decode(text: string): string {
  try {
    return decodeURIComponent(text.replaceAll("+", " "));
  } catch {
    // Not valid percent-encoding: shown as it was sent.
    return text;
  }
}

function pairs(query: string): { key: string; value: string }[] {
  return query
    .split("&")
    .filter((part) => part !== "")
    .map((part) => {
      const equals = part.indexOf("=");
      return equals === -1
        ? { key: decode(part), value: "" }
        : { key: decode(part.slice(0, equals)), value: decode(part.slice(equals + 1)) };
    });
}

/** The query string of a URL as a table, in the order sent. Repeated keys stay repeated. */
export function parseQueryParams(url: string): QueryParam[] {
  const start = url.indexOf("?");
  if (start === -1) return [];
  const hash = url.indexOf("#", start);
  const query = url.slice(start + 1, hash === -1 ? undefined : hash);
  return pairs(query).map(({ key, value }) => ({
    key,
    value,
    nested: NESTED_PARAMS.has(key) && value.includes("=") ? pairs(value) : [],
  }));
}
