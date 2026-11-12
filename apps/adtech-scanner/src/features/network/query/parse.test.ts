import { describe, expect, it } from "vitest";
import { parseDuration, parseSize, parseStatus } from "./keys";
import { parseQuery, tokenize } from "./parse";

const shape = (query: string) =>
  parseQuery(query).terms.map((term) => {
    const sign = term.negated ? "-" : "";
    if (term.kind === "text") return `${sign}text(${term.needle})`;
    if (term.kind === "regex") return `${sign}regex(${term.pattern.source})`;
    return `${sign}${term.key}(${term.values.join("|")})`;
  });

describe("tokenize", () => {
  it("splits at spaces and records where each word sits", () => {
    expect(tokenize("  cat:bid   is:late ")).toEqual([
      { raw: "cat:bid", start: 2, end: 9 },
      { raw: "is:late", start: 12, end: 19 },
    ]);
  });

  it("keeps a quoted phrase and a regular expression whole", () => {
    expect(tokenize('vendor:"Index Exchange" /ads\\?gdfp req/ x').map((token) => token.raw)).toEqual([
      'vendor:"Index Exchange"',
      "/ads\\?gdfp req/",
      "x",
    ]);
  });

  it("treats an unclosed slash as an ordinary word", () => {
    expect(tokenize("/gampad ads").map((token) => token.raw)).toEqual(["/gampad", "ads"]);
  });
});

describe("parseQuery", () => {
  it("reads keys, bare text and negation", () => {
    expect(shape("cat:bid rubicon -is:cached")).toEqual(["cat(bid)", "text(rubicon)", "-is(cached)"]);
  });

  it("reads commas as alternatives within one key", () => {
    expect(shape("is:failed,late status:4xx,503")).toEqual(["is(failed|late)", "status(4xx|503)"]);
  });

  it("reads quotes as one value or one phrase", () => {
    expect(shape('vendor:"Index Exchange",Google "no bid"')).toEqual(["vendor(Index Exchange|Google)", "text(no bid)"]);
  });

  it("reads a regular expression on the URL, case-insensitively", () => {
    const [term] = parseQuery("-/Gampad\\/ads/").terms;
    expect(term).toMatchObject({ kind: "regex", negated: true });
    expect(term?.kind === "regex" && term.pattern.test("https://x/gampad/ads?a=1")).toBe(true);
  });

  it("is case-insensitive in keys and keeps the case of values", () => {
    expect(shape("HOST:Example.COM Method:post")).toEqual(["host(Example.COM)", "method(post)"]);
  });

  it("takes a URL for text instead of an unknown key", () => {
    const parsed = parseQuery("https://ib.adnxs.com/ut/v3 localhost:8080");
    expect(shape("https://ib.adnxs.com/ut/v3")).toEqual(["text(https://ib.adnxs.com/ut/v3)"]);
    expect(parsed.notes).toEqual([{ start: 27, end: 41, message: "Unknown filter “localhost:”; searched as text" }]);
  });

  it("falls back to text for an unknown key and says where", () => {
    const parsed = parseQuery("cat:bid foo:bar");
    expect(shape("cat:bid foo:bar")).toEqual(["cat(bid)", "text(foo:bar)"]);
    expect(parsed.notes).toEqual([{ start: 8, end: 15, message: "Unknown filter “foo:”; searched as text" }]);
  });

  it.each([
    ["is:nope", "Unknown state “nope”"],
    ["type:video", "Unknown type “video”"],
    ["cat:ads", "Unknown category “ads”"],
    ["status:ok", "“ok” is not a status: use 204 or 4xx"],
    ["page:0", "“0” is not a page number"],
    ["larger-than:big", "“big” is not a size: use 50k or 1.5m"],
    ["slower-than:1min", "“1min” is not a duration: use 500ms or 2s"],
    ["host:", "“host:” needs a value"],
    ["/(unclosed/", "Not a valid regular expression"],
  ])("explains what is wrong with %s", (query, message) => {
    const parsed = parseQuery(`x ${query}`);
    expect(parsed.notes).toEqual([{ start: 2, end: 2 + query.length, message: `${message}; searched as text` }]);
    expect(parsed.terms).toHaveLength(2);
    expect(parsed.terms[1]?.kind).toBe("text");
  });

  it("rejects the whole word when one alternative is wrong", () => {
    expect(parseQuery("is:failed,nope").notes).toHaveLength(1);
  });

  it("ignores a lone dash and empty quotes", () => {
    expect(shape('- "" -')).toEqual(["text(-)", "text(-)"]);
    expect(shape("")).toEqual([]);
  });
});

describe("units", () => {
  it("reads sizes in bytes, kilobytes and megabytes", () => {
    expect(parseSize("300")).toBe(300);
    expect(parseSize("50k")).toBe(51_200);
    expect(parseSize("1.5MB")).toBe(1_572_864);
    expect(parseSize("k")).toBeNull();
  });

  it("reads durations in milliseconds and seconds", () => {
    expect(parseDuration("500ms")).toBe(500);
    expect(parseDuration("2s")).toBe(2000);
    expect(parseDuration("800")).toBe(800);
    expect(parseDuration("fast")).toBeNull();
  });

  it("reads a status or a class of statuses", () => {
    expect(parseStatus("204")).toEqual({ min: 204, max: 204 });
    expect(parseStatus("4xx")).toEqual({ min: 400, max: 499 });
    expect(parseStatus("6xx")).toBeNull();
    expect(parseStatus("20")).toBeNull();
  });
});
