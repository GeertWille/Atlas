import { describe, expect, it } from "vitest";
import { makeAuction, makeBid } from "@/test/fixtures";
import {
  classifyExchange,
  decodeExchange,
  isNotSent,
  openRtbRequestBody,
  openRtbResponseBody,
  timeoutStory,
  VERDICT_TOLERANCE_MS,
} from "./exchange";

describe("classifyExchange", () => {
  const base = { auctionEndedAtMs: 2000, failed: false, timedOutInPage: false };

  it("calls an early answer in time and says by how much", () => {
    expect(classifyExchange({ ...base, endedAtMs: 1962 })).toEqual({ verdict: "inTime", lateByMs: -38 });
  });

  it("calls a later answer late", () => {
    expect(classifyExchange({ ...base, endedAtMs: 2084 })).toEqual({ verdict: "lateOnNetwork", lateByMs: 84 });
  });

  it.each([-VERDICT_TOLERANCE_MS, -3, 0, 4, VERDICT_TOLERANCE_MS])(
    "gives no number %i ms from the close, where the two clocks cannot be told apart",
    (offset) => {
      expect(classifyExchange({ ...base, endedAtMs: 2000 + offset })).toEqual({
        verdict: "atDeadline",
        lateByMs: null,
      });
    },
  );

  it("is decided just outside the tolerance", () => {
    expect(classifyExchange({ ...base, endedAtMs: 2000 + VERDICT_TOLERANCE_MS + 1 }).verdict).toBe("lateOnNetwork");
    expect(classifyExchange({ ...base, endedAtMs: 2000 - VERDICT_TOLERANCE_MS - 1 }).verdict).toBe("inTime");
  });

  it("tells an answer that was in time but timed out in the page", () => {
    expect(classifyExchange({ ...base, endedAtMs: 1960, timedOutInPage: true })).toEqual({
      verdict: "inTimeButTimedOut",
      lateByMs: -40,
    });
  });

  it("has no number without an answer", () => {
    expect(classifyExchange({ ...base, endedAtMs: null })).toEqual({ verdict: "neverAnswered", lateByMs: null });
    expect(classifyExchange({ ...base, endedAtMs: 1041, failed: true })).toEqual({ verdict: "failed", lateByMs: null });
  });
});

describe("timeoutStory", () => {
  it("makes the reference site's slow partner late on the entry page", () => {
    expect(timeoutStory("ix", makeAuction([]), "desktop")).toEqual({ kind: "late", lateByMs: 84 });
    expect(timeoutStory("ix", makeAuction([]), "mobile")).toEqual({ kind: "neverAnswered" });
    expect(timeoutStory("ix", makeAuction([], { page: 2 }), "desktop")).toEqual({
      kind: "inTimeButTimedOut",
      earlyByMs: 40,
    });
  });
});

describe("isNotSent", () => {
  it("is the reference site's TripleLift bid that Prebid never sent", () => {
    expect(isNotSent(makeBid({ bidder: "triplelift", status: "noBid" }), true)).toBe(true);
    expect(isNotSent(makeBid({ bidder: "triplelift", status: "noBid" }), false)).toBe(false);
    expect(isNotSent(makeBid({ bidder: "triplelift", status: "bid" }), true)).toBe(false);
    expect(isNotSent(makeBid({ bidder: "rubicon", status: "noBid" }), true)).toBe(false);
  });
});

describe("decodeExchange", () => {
  const pageUrl = "https://www.example.com/news";
  const won = makeBid({ bidder: "openx", status: "won", cpm: 1.42, size: "970x250" });
  const declined = makeBid({ bidder: "openx", status: "noBid", cpm: null, size: null });

  it("reads the impressions and keeps consent and ids to presence flags", () => {
    const auction = makeAuction([won]);
    const decoded = decodeExchange({
      auction,
      bidder: "openx",
      units: [{ code: "div-gpt-top", sizes: ["970x250"], bid: won }],
      pageUrl,
      answered: true,
    });
    expect(decoded.protocol).toBe("openRtb");
    expect(decoded.request?.imps).toMatchObject([{ adUnit: "div-gpt-top", sizes: ["970x250"], floor: 0.01 }]);
    expect(decoded.request?.consentStringPresent).toBe(true);
    expect(decoded.request?.eidSources).toEqual(["id5-sync.com", "criteo.com"]);
    expect(JSON.stringify(decoded)).not.toContain("ID5*");
    expect(decoded.response?.seats).toMatchObject([{ seat: "openx", bids: [{ price: 1.42, size: "970x250" }] }]);
  });

  it("names the no-bid reason of an exchange that declines", () => {
    const decoded = decodeExchange({
      auction: makeAuction([declined]),
      bidder: "openx",
      units: [{ code: "div-gpt-top", sizes: ["970x250"], bid: declined }],
      pageUrl,
      answered: true,
    });
    expect(decoded.response).toMatchObject({ nbr: 7, nbrMeaning: "Blocked Publisher or Site", seats: [] });
    expect(decoded.response && JSON.parse(openRtbResponseBody(decoded.response, false))).toEqual({
      id: decoded.request?.id,
      nbr: 7,
    });
  });

  it("lists the server-side time and errors of a Prebid Server request per bidder", () => {
    const fast = makeBid({ bidder: "pubmatic", status: "bid", cpm: 0.88, responseTimeMs: 140 });
    const slow = makeBid({ bidder: "criteo", status: "timeout", cpm: null, responseTimeMs: null });
    const decoded = decodeExchange({
      auction: makeAuction([fast, slow]),
      bidder: null,
      units: [
        { code: "div-gpt-top", sizes: ["970x250"], bid: fast },
        { code: "div-gpt-top", sizes: ["970x250"], bid: slow },
      ],
      pageUrl,
      answered: true,
    });
    expect(decoded.protocol).toBe("prebidServer");
    expect(decoded.request?.imps.map((imp) => imp.bidders)).toEqual([["pubmatic"], ["criteo"]]);
    expect(decoded.response?.serverTimings).toEqual([{ bidder: "pubmatic", ms: 140 }]);
    expect(decoded.response?.serverErrors).toEqual([{ bidder: "criteo", code: 1, message: "timeout" }]);
  });

  it("has no response side without an answer", () => {
    const pending = makeBid({ bidder: "ix", status: "timeout", cpm: null, responseTimeMs: null });
    const decoded = decodeExchange({
      auction: makeAuction([pending]),
      bidder: "ix",
      units: [{ code: "div-gpt-top", sizes: ["970x250"], bid: pending }],
      pageUrl,
      answered: false,
    });
    expect(decoded.response).toBeNull();
    expect(decoded.request && JSON.parse(openRtbRequestBody(decoded.request, false)).user.ext.consent).toMatch(/^CQH0/);
  });
});
