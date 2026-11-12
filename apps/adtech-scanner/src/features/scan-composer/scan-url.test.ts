import { describe, expect, test } from "vitest";
import { normalizeScanUrl } from "./scan-url";

describe("normalizeScanUrl accepts", () => {
  test.each([
    ["example.com", "https://example.com/"],
    ["  Example.com/News?a=1#section  ", "https://example.com/News?a=1"],
    ["http://insecure.example.org/path", "http://insecure.example.org/path"],
    ["HTTPS://www.example.com", "https://www.example.com/"],
    ["sub.domain.co.uk/a/b", "https://sub.domain.co.uk/a/b"],
    ["localhost:3000/demo", "http://localhost:3000/demo"],
    ["127.0.0.1:47615/demo-publisher/", "http://127.0.0.1:47615/demo-publisher/"],
    ["https://localhost/x", "https://localhost/x"],
  ])("%s → %s", (input, expected) => {
    expect(normalizeScanUrl(input)).toEqual({ ok: true, url: expected });
  });
});

describe("normalizeScanUrl rejects", () => {
  test.each([
    ["", "Enter the address of a page to scan."],
    ["   ", "Enter the address of a page to scan."],
    ["foo bar.com", "A URL cannot contain spaces."],
    ["ftp://example.com/file", "Only http and https pages can be scanned."],
    ["chrome://settings", "Only http and https pages can be scanned."],
    ["example", "Enter a full domain, like example.com."],
    ["https://", "That does not look like a valid URL."],
    ["exa_mple.com", "Enter a full domain, like example.com."],
  ])("%j → %s", (input, error) => {
    expect(normalizeScanUrl(input)).toEqual({ ok: false, error });
  });
});
