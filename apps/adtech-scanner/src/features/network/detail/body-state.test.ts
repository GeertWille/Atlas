import { describe, expect, it } from "vitest";
import type { BodyState } from "@/bindings";
import { makeBodyRef } from "@/test/network-fixtures";
import { bodyNotice, mediaWord } from "./body-state";

describe("bodyNotice", () => {
  it("has nothing to say about a body stored whole", () => {
    expect(bodyNotice(makeBodyRef(), "response")).toBeNull();
  });

  it("says how much of a truncated body was kept", () => {
    const notice = bodyNotice(
      makeBodyRef({ state: "truncated", bytes: 7_759_462, storedBytes: 5 * 1024 * 1024 }),
      "response",
    );
    expect(notice?.title).toBe("First 5.0 MB of 7.4 MB kept");
  });

  it("tells an empty answer from a request without a body", () => {
    const empty = makeBodyRef({ state: "empty", bytes: 0, storedBytes: 0, mime: null });
    expect(bodyNotice(empty, "response")?.title).toBe("No content");
    expect(bodyNotice(empty, "request")?.title).toBe("No request body");
  });

  it("names the kind and size of a body kept as metadata only", () => {
    const image = makeBodyRef({ state: "notKeptType", bytes: 49_152, storedBytes: 0, mime: "image/webp" });
    expect(bodyNotice(image, "response")).toMatchObject({
      title: "Body not kept (image, 48 KB)",
      detail: "Images, video, audio and fonts are recorded as metadata only.",
    });
  });

  it("has a different sentence for every reason a body is missing", () => {
    const states: BodyState[] = ["scanLimit", "dropped", "unavailable", "pending", "evicted"];
    const notices = states.map((state) =>
      bodyNotice(makeBodyRef({ state, bytes: 217_088, storedBytes: 0 }), "response"),
    );
    const sentences = notices.map((notice) => `${notice?.title} ${notice?.detail ?? ""}`);
    expect(new Set(sentences).size).toBe(states.length);
    expect(notices.every((notice) => notice !== null)).toBe(true);
    expect(notices[4]).toMatchObject({
      title: "Body removed to stay under the storage limit (was 212 KB)",
      storage: true,
    });
    expect(notices.slice(0, 4).every((notice) => notice?.storage === false)).toBe(true);
  });

  it("says of a body a Browserbase browser could not hand over that it was not read, and why", () => {
    const notice = bodyNotice(
      makeBodyRef({ state: "tooLargeForRemote", bytes: 12_163_481, storedBytes: 0, mime: "application/json" }),
      "response",
    );
    expect(notice).toEqual({
      title: "Body not read (was 11.6 MB)",
      detail:
        "It was too large to bring over the connection to the Browserbase browser in one piece, so the scanner left it there.",
      storage: false,
    });
    // Not the sentence of any other missing body.
    expect(notice?.title).not.toBe(bodyNotice(makeBodyRef({ state: "dropped", bytes: 12_163_481 }), "response")?.title);
  });

  it("leaves the size out when it is unknown", () => {
    expect(bodyNotice(makeBodyRef({ state: "dropped", bytes: 0, storedBytes: 0 }), "response")?.title).toBe(
      "Body not kept",
    );
  });
});

describe("mediaWord", () => {
  it.each([
    ["image/png", "image"],
    ["video/mp4", "video"],
    ["audio/mpeg", "audio"],
    ["font/woff2", "font"],
    ["application/font-woff", "font"],
    ["application/octet-stream", "binary data"],
    [null, "binary data"],
  ])("%s is %s", (mime, word) => {
    expect(mediaWord(mime)).toBe(word);
  });
});
