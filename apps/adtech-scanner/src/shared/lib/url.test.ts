import { expect, test } from "vitest";
import { fileNameOf, hostnameOf, isLocalOrPrivateUrl, pathOf } from "./url";

test("hostnameOf never throws on a value that is not a URL", () => {
  expect(hostnameOf("https://www.example.com/a?b=1")).toBe("www.example.com");
  expect(hostnameOf("not a url")).toBe("not a url");
});

test("pathOf keeps the query and is empty for a bare origin", () => {
  expect(pathOf("https://www.example.com/a/b?x=1")).toBe("/a/b?x=1");
  expect(pathOf("https://www.example.com/")).toBe("");
  expect(pathOf("not a url")).toBe("");
});

test("fileNameOf names scripts by their last path segment, inline ones by their id", () => {
  expect(fileNameOf("https://cdn.example.com/ads/hb-wrapper.min.js?v=3")).toBe("hb-wrapper.min.js?v=3");
  expect(fileNameOf("https://cdn.example.com/")).toBe("cdn.example.com");
  expect(fileNameOf("inline://page/#script-4")).toBe("inline script #script-4");
});

test.each([
  ["http://localhost:3000/", true],
  ["http://app.localhost/", true],
  ["http://printer.local/status", true],
  ["http://PRINTER.LOCAL./", true],
  ["http://127.0.0.1:47615/demo-publisher/", true],
  ["http://127.8.9.1/", true],
  ["http://0.0.0.0/", true],
  ["http://10.1.2.3/", true],
  ["http://172.16.0.1/", true],
  ["http://172.31.255.255/", true],
  ["http://192.168.1.20:8080/", true],
  ["http://169.254.10.1/", true],
  ["http://100.64.0.1/", true],
  ["http://[::1]:8080/", true],
  ["http://[fd12:3456::1]/", true],
  ["http://[fe80::1]/", true],
  ["http://[::ffff:10.0.0.1]/", true],
  // Other spellings of 127.0.0.1 are normalised by the URL parser.
  ["http://2130706433/", true],
  ["https://www.example.com/", false],
  ["https://localhost.example.com/", false],
  ["https://local/", false],
  ["http://172.32.0.1/", false],
  ["http://100.128.0.1/", false],
  ["http://192.169.0.1/", false],
  ["http://8.8.8.8/", false],
  ["http://[2001:db8::1]/", false],
  ["http://[::ffff:8.8.8.8]/", false],
  ["not a url", false],
])("isLocalOrPrivateUrl(%s) is %s", (url, expected) => {
  expect(isLocalOrPrivateUrl(url)).toBe(expected);
});
