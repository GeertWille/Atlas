import { describe, expect, it } from "vitest";
import { parseQueryParams } from "./query-params";

describe("parseQueryParams", () => {
  it("lists the parameters in the order sent, decoded", () => {
    expect(parseQueryParams("https://x.example/a?b=1&c=two%20words&d=a+b#frag")).toEqual([
      { key: "b", value: "1", nested: [] },
      { key: "c", value: "two words", nested: [] },
      { key: "d", value: "a b", nested: [] },
    ]);
  });

  it("keeps repeated keys and keys without a value", () => {
    expect(parseQueryParams("https://x.example/a?x=1&x=2&flag&=odd").map((param) => [param.key, param.value])).toEqual([
      ["x", "1"],
      ["x", "2"],
      ["flag", ""],
      ["", "odd"],
    ]);
  });

  it("opens the key-values Google Ad Manager packs into one parameter", () => {
    const url = `https://securepubads.g.doubleclick.net/gampad/ads?gdfp_req=1&cust_params=${encodeURIComponent("section=football&paywall=0")}&prev_scp=${encodeURIComponent("hb_bidder=appnexus&hb_pb=1.20&pos=top")}`;
    const params = parseQueryParams(url);
    expect(params.find((param) => param.key === "cust_params")?.nested).toEqual([
      { key: "section", value: "football" },
      { key: "paywall", value: "0" },
    ]);
    expect(params.find((param) => param.key === "prev_scp")?.nested.map((pair) => pair.key)).toEqual([
      "hb_bidder",
      "hb_pb",
      "pos",
    ]);
    expect(params.find((param) => param.key === "gdfp_req")?.nested).toEqual([]);
  });

  it("shows a broken escape as it was sent instead of failing", () => {
    expect(parseQueryParams("https://x.example/a?bad=%E0%A4%A&ok=1")).toEqual([
      { key: "bad", value: "%E0%A4%A", nested: [] },
      { key: "ok", value: "1", nested: [] },
    ]);
  });

  it("is empty without a query string", () => {
    expect(parseQueryParams("https://x.example/a")).toEqual([]);
    expect(parseQueryParams("https://x.example/a?")).toEqual([]);
  });
});
