import { describe, expect, it } from "vitest";
import type { Header } from "@/bindings";
import { makeBodyRef, makeRequestDetail, makeRow } from "@/test/network-fixtures";
import { carriesIdentifiers, toCurl } from "./curl";

const header = (name: string, value: string, redaction: Header["redaction"] = null): Header => ({
  name,
  value,
  redaction,
});

describe("toCurl", () => {
  it("writes a GET with its headers", () => {
    const detail = makeRequestDetail({
      row: makeRow({ url: "https://x.example/a?b=1" }),
      requestHeaders: [header("Accept", "*/*"), header("Referer", "https://www.example.com/")],
    });
    expect(toCurl(detail, null)).toEqual({
      command: "curl 'https://x.example/a?b=1' \\\n  -H 'Accept: */*' \\\n  -H 'Referer: https://www.example.com/'",
      redactedHeaders: [],
      bodyIncomplete: false,
    });
  });

  it("sends a POST body as data, without -X", () => {
    const detail = makeRequestDetail({
      row: makeRow({ url: "https://rtb.example/bid", method: "POST" }),
      requestHeaders: [header("Content-Type", "text/plain")],
      requestBody: makeBodyRef({ bytes: 9 }),
    });
    expect(toCurl(detail, '{"id":1}').command).toBe(
      "curl 'https://rtb.example/bid' \\\n  -H 'Content-Type: text/plain' \\\n  --data-raw '{\"id\":1}'",
    );
  });

  it("names any other method", () => {
    const detail = makeRequestDetail({ row: makeRow({ url: "https://rtb.example/bid", method: "OPTIONS" }) });
    expect(toCurl(detail, null).command).toBe("curl 'https://rtb.example/bid' \\\n  -X OPTIONS");
    const bodiless = makeRequestDetail({ row: makeRow({ url: "https://rtb.example/ping", method: "POST" }) });
    expect(toCurl(bodiless, null).command).toBe("curl 'https://rtb.example/ping' \\\n  -X POST");
  });

  it("puts a placeholder where a value was not stored, and says which headers", () => {
    const detail = makeRequestDetail({
      requestHeaders: [
        header("Cookie", "_ga=[27 bytes]; id=[36 bytes]", "cookie"),
        header("Authorization", "Bearer [212 bytes]", "credential"),
        header("Accept", "*/*"),
      ],
    });
    const { command, redactedHeaders } = toCurl(detail, null);
    expect(redactedHeaders).toEqual(["Cookie", "Authorization"]);
    expect(command).toContain("-H 'Cookie: <redacted: not stored>'");
    expect(command).toContain("-H 'Authorization: <redacted: not stored>'");
    // The stored stand-in is never passed off as the value.
    expect(command).not.toContain("[27 bytes]");
    expect(command).not.toContain("Bearer");
  });

  it("keeps a credential that was stored in full", () => {
    const detail = makeRequestDetail({ requestHeaders: [header("Cookie", "_ga=GA1.1.1")] });
    expect(toCurl(detail, null)).toMatchObject({
      redactedHeaders: [],
      command: expect.stringContaining("Cookie: _ga=GA1.1.1"),
    });
  });

  it("leaves out what curl sets itself and asks for decompression instead", () => {
    const detail = makeRequestDetail({
      requestHeaders: [
        header(":authority", "x.example"),
        header("Host", "x.example"),
        header("Content-Length", "12"),
        header("Accept-Encoding", "gzip, br"),
        header("User-Agent", "Mozilla/5.0"),
      ],
    });
    expect(toCurl(detail, null).command).toBe(
      "curl 'https://www.example.com/static/app.js' \\\n  -H 'User-Agent: Mozilla/5.0' \\\n  --compressed",
    );
  });

  it("quotes for the shell, so nothing in a URL or body can run", () => {
    const detail = makeRequestDetail({
      row: makeRow({ url: "https://x.example/a?q=it's&b=$(id)`id`", method: "POST" }),
      requestBody: makeBodyRef({ bytes: 20 }),
    });
    const { command } = toCurl(detail, "a'; rm -rf ~; echo '");
    expect(command).toContain("curl 'https://x.example/a?q=it'\\''s&b=$(id)`id`'");
    expect(command).toContain("--data-raw 'a'\\''; rm -rf ~; echo '\\'''");
  });

  it("says when the body is missing or cut short", () => {
    const post = makeRow({ url: "https://rtb.example/bid", method: "POST" });
    const lost = makeRequestDetail({
      row: post,
      requestBody: makeBodyRef({ state: "dropped", bytes: 5000, storedBytes: 0 }),
    });
    expect(toCurl(lost, null)).toMatchObject({
      bodyIncomplete: true,
      command: expect.stringContaining("--data-raw '<request body not stored>'"),
    });
    const cut = makeRequestDetail({
      row: post,
      requestBody: makeBodyRef({ state: "truncated", bytes: 5000, storedBytes: 100 }),
    });
    expect(toCurl(cut, "abc").bodyIncomplete).toBe(true);
    const whole = makeRequestDetail({ row: post, requestBody: makeBodyRef({ bytes: 3 }) });
    expect(toCurl(whole, "abc").bodyIncomplete).toBe(false);
  });
});

describe("carriesIdentifiers", () => {
  it("is true for bid requests and for credentials stored in full", () => {
    expect(carriesIdentifiers(makeRequestDetail({ row: makeRow({ category: "bid" }) }))).toBe(true);
    expect(carriesIdentifiers(makeRequestDetail({ requestHeaders: [header("Cookie", "sid=abc")] }))).toBe(true);
    expect(
      carriesIdentifiers(makeRequestDetail({ requestHeaders: [header("Cookie", "sid=[3 bytes]", "cookie")] })),
    ).toBe(false);
    expect(carriesIdentifiers(makeRequestDetail())).toBe(false);
  });
});
