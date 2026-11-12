import { describe, expect, it } from "vitest";
import { verdictSentence, verdictShort } from "./exchange-verdict";

describe("verdictSentence", () => {
  it("says how early an answer in time was", () => {
    expect(verdictSentence({ verdict: "inTime", lateByMs: -38, networkMs: 212 })).toBe(
      "Answered in 212 ms, 38 ms before the auction closed",
    );
  });

  it("gives the lateness of a late answer", () => {
    expect(verdictSentence({ verdict: "lateOnNetwork", lateByMs: 84, networkMs: 334 })).toBe(
      "Response arrived after the auction closed (+84 ms)",
    );
  });

  it("gives no number inside the clock tolerance", () => {
    const sentence = verdictSentence({ verdict: "atDeadline", lateByMs: null, networkMs: 251 });
    expect(sentence).toBe("The response arrived as the auction closed");
    expect(sentence).not.toMatch(/\d/);
  });

  it("explains an answer that was in time on the network and still timed out", () => {
    expect(verdictSentence({ verdict: "inTimeButTimedOut", lateByMs: -40, networkMs: 210 })).toBe(
      "The response arrived 40 ms before the timeout, but Prebid recorded a timeout: the page was busy or the adapter was slow to parse",
    );
  });

  it("covers the outcomes without an answer", () => {
    expect(verdictSentence({ verdict: "neverAnswered", lateByMs: null, networkMs: null })).toBe(
      "No response before the visit ended",
    );
    expect(verdictSentence({ verdict: "notSent", lateByMs: null, networkMs: null })).toBe(
      "Prebid announced this bid but no request left the page",
    );
  });

  it("names the error of a failed request when there is one", () => {
    expect(
      verdictSentence({ verdict: "failed", lateByMs: null, networkMs: 41, error: "net::ERR_CONNECTION_RESET" }),
    ).toBe("The request failed: net::ERR_CONNECTION_RESET");
    expect(verdictSentence({ verdict: "failed", lateByMs: null, networkMs: null })).toBe("The request failed");
  });

  it("still reads without the numbers", () => {
    expect(verdictSentence({ verdict: "inTime", lateByMs: null, networkMs: null })).toBe(
      "Answered, before the auction closed",
    );
    expect(verdictSentence({ verdict: "lateOnNetwork", lateByMs: null, networkMs: null })).toBe(
      "Response arrived after the auction closed",
    );
  });
});

describe("verdictShort", () => {
  it("fits a table cell", () => {
    expect(verdictShort({ verdict: "lateOnNetwork", lateByMs: 84 })).toBe("late +84 ms");
    expect(verdictShort({ verdict: "atDeadline", lateByMs: null })).toBe("at deadline");
    expect(verdictShort({ verdict: "notSent", lateByMs: null })).toBe("not sent");
  });
});
