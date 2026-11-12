import { describe, expect, it } from "vitest";
import { languageForMime } from "./languages";

describe("languageForMime", () => {
  it.each([
    ["application/json", "json"],
    ["application/json; charset=utf-8", "json"],
    ["application/vnd.api+json", "json"],
    ["text/javascript", "javascript"],
    ["application/x-javascript", "javascript"],
    ["application/xml", "xml"],
    ["text/xml", "xml"],
    ["application/vast+xml", "xml"],
    ["text/html", "text"],
    ["application/xhtml+xml", "text"],
    ["text/plain", "text"],
    [null, "text"],
  ] as const)("%s is shown as %s", (mime, language) => {
    expect(languageForMime(mime)).toBe(language);
  });
});
