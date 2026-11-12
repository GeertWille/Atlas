import { describe, expect, it } from "vitest";
import { makeRow } from "@/test/network-fixtures";
import { indexRow } from "./match";
import { buildFacets, suggest } from "./suggest";

const facets = buildFacets(
  [
    makeRow({ vendor: "Google", url: "https://a.doubleclick.net/x" }),
    makeRow({ vendor: "Google", url: "https://a.doubleclick.net/y" }),
    makeRow({ vendor: "Index Exchange", bidder: "ix", url: "https://htlb.casalemedia.com/z" }),
    makeRow({ vendor: null, url: "https://www.example.com/" }),
  ].map(indexRow),
);
const labels = (query: string, caret = query.length) => suggest(query, caret, facets).map((item) => item.label);

describe("buildFacets", () => {
  it("lists what the scan has, most frequent first", () => {
    expect(facets.vendors).toEqual(["Google", "Index Exchange"]);
    expect(facets.bidders).toEqual(["ix"]);
    expect(facets.hosts[0]).toBe("a.doubleclick.net");
  });
});

describe("suggest", () => {
  it("offers keys for a bare word", () => {
    expect(labels("ho")).toEqual(["host:"]);
    expect(labels("cat:bid s")).toEqual(["status:", "slower-than:"]);
    expect(labels("")).toEqual([]);
    expect(labels("cat:bid ")).toEqual([]);
  });

  it("offers the fixed values of a key", () => {
    expect(labels("is:la")).toEqual(["late"]);
    expect(labels("is:")).toHaveLength(8);
    expect(labels("cat:ad")).toEqual(["adserver"]);
    expect(labels("type:xh")).toEqual(["xhr"]);
  });

  it("offers the scan's own vendors, bidders and hosts", () => {
    expect(labels("vendor:")).toEqual(["Google", "Index Exchange"]);
    expect(labels("bidder:i")).toEqual(["ix"]);
    expect(labels("host:casale")).toEqual(["htlb.casalemedia.com"]);
  });

  it("completes the last alternative of a list and keeps the others", () => {
    const [item] = suggest("is:failed,la", 12, facets);
    expect(item).toEqual({ label: "late", query: "is:failed,late", caret: 14 });
  });

  it("quotes a value with a space, and keeps the minus", () => {
    const [, exchange] = suggest("x -vendor:", 10, facets);
    expect(exchange).toMatchObject({ query: 'x -vendor:"Index Exchange"', caret: 26 });
    expect(suggest("-ho", 3, facets)[0]).toMatchObject({ query: "-host:" });
  });

  it("replaces only the word at the caret", () => {
    expect(suggest("is:la cat:bid", 5, facets)[0]).toMatchObject({ query: "is:late cat:bid", caret: 7 });
  });

  it("stays quiet in the middle of a word, for a finished value and for keys with free values", () => {
    expect(labels("is:late", 3)).toEqual([]);
    expect(labels("is:late")).toEqual([]);
    expect(labels("url:abc")).toEqual([]);
    expect(labels("/reg")).toEqual([]);
  });
});
