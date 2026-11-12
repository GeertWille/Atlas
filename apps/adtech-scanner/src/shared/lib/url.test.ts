import { expect, test } from "vitest";
import { fileNameOf, hostnameOf, pathOf } from "./url";

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
