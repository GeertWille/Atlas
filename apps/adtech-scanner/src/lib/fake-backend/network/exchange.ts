import type {
  Auction,
  Bid,
  DecodedBidRequest,
  DecodedBidResponse,
  DecodedExchange,
  DeviceProfile,
  ExchangeVerdict,
} from "@/bindings";
import { hashString } from "../rng";

/**
 * The page's clock and the network's can disagree by a few milliseconds. Inside this tolerance an
 * answer is "at the deadline" and gets no number. Mirrors the backend's `VERDICT_TOLERANCE_MS`,
 * which is a starting value and uncalibrated.
 */
export const VERDICT_TOLERANCE_MS = 10;

export type ExchangeTiming = {
  /** When the answer was complete; `null` when there was none. */
  endedAtMs: number | null;
  auctionEndedAtMs: number;
  failed: boolean;
  /** Prebid recorded a timeout for the bid. */
  timedOutInPage: boolean;
};

export function classifyExchange({ endedAtMs, auctionEndedAtMs, failed, timedOutInPage }: ExchangeTiming): {
  verdict: ExchangeVerdict;
  lateByMs: number | null;
} {
  if (failed) return { verdict: "failed", lateByMs: null };
  if (endedAtMs === null) return { verdict: "neverAnswered", lateByMs: null };
  const lateByMs = Math.round(endedAtMs - auctionEndedAtMs);
  if (Math.abs(lateByMs) <= VERDICT_TOLERANCE_MS) return { verdict: "atDeadline", lateByMs: null };
  if (lateByMs > 0) return { verdict: "lateOnNetwork", lateByMs };
  return { verdict: timedOutInPage ? "inTimeButTimedOut" : "inTime", lateByMs };
}

export type BidEndpoint = {
  method: "GET" | "POST";
  url: string;
  vendor: string;
  /** The exchange speaks OpenRTB, so its bodies can be decoded. */
  openRtb: boolean;
  /** A JSON POST to another origin: the browser asks first. */
  preflight?: boolean;
};

/** Where each simulated client-side bidder sends its bids, as the Prebid adapters do. */
export const BID_ENDPOINTS: Record<string, BidEndpoint> = {
  appnexus: {
    method: "POST",
    url: "https://ib.adnxs.com/ut/v3/prebid",
    vendor: "Xandr",
    openRtb: false,
    preflight: true,
  },
  rubicon: {
    method: "GET",
    url: "https://fastlane.rubiconproject.com/a/api/fastlane.json",
    vendor: "Magnite",
    openRtb: false,
  },
  ix: {
    method: "POST",
    url: "https://htlb.casalemedia.com/openrtb/pbjs?s=194730",
    vendor: "Index Exchange",
    openRtb: true,
  },
  openx: { method: "POST", url: "https://rtb.openx.net/openrtbb/prebidjs", vendor: "OpenX", openRtb: true },
  sovrn: {
    method: "POST",
    url: "https://ap.lijit.com/rtb/bid?src=prebid_prebid_8.52.0",
    vendor: "Sovrn",
    openRtb: true,
  },
  triplelift: {
    method: "POST",
    url: "https://tlx.3lift.com/header/auction?lib=prebid&v=8.52.0",
    vendor: "TripleLift",
    openRtb: false,
  },
};

/** These bidders are called through Prebid Server: one request carries all of them. */
export const SERVER_BIDDERS: readonly string[] = ["pubmatic", "criteo", "sharethrough"];
export const PREBID_SERVER: BidEndpoint = {
  method: "POST",
  url: "https://prebid.adnxs.com/pbs/v1/openrtb2/auction",
  vendor: "Prebid Server",
  openRtb: true,
};
/** What the server adds on top of its slowest bidder. */
export const SERVER_OVERHEAD_MS = 28;

/** As the backend names them: the list OpenRTB 2.6 refers to (OpenRTB 3.0, "No-Bid Reason Codes"). */
export const NO_BID_REASONS: Record<number, string> = {
  0: "Unknown Error",
  1: "Technical Error",
  2: "Invalid Request",
  3: "Known Web Crawler",
  4: "Suspected Non-Human Traffic",
  5: "Cloud, Data Center, or Proxy IP",
  6: "Unsupported Device",
  7: "Blocked Publisher or Site",
  8: "Unmatched User",
  9: "Daily User Cap Met",
  10: "Daily Domain Cap Met",
  11: "Ads.txt Authorization Unavailable",
  12: "Ads.txt Authorization Violation",
  13: "Ads.cert Authentication Unavailable",
  14: "Ads.cert Authentication Violation",
  15: "Insufficient Auction Time",
  16: "Incomplete SupplyChain",
  17: "Blocked SupplyChain Node",
};

/** The reason these exchanges give when they decline: the site is blocked at one, the visitor looks hosted to another. */
const NO_BID_REASON_OF: Record<string, number> = { openx: 7, sovrn: 5 };

export function noBidReason(bidder: string): number | null {
  return NO_BID_REASON_OF[bidder] ?? null;
}

/** How an unanswered bid looks on the network. Each simulated bidder fails in its own way. */
export type TimeoutStory =
  | { kind: "late"; lateByMs: number }
  | { kind: "inTimeButTimedOut"; earlyByMs: number }
  | { kind: "neverAnswered" };

export function timeoutStory(bidder: string, auction: Auction, device: DeviceProfile): TimeoutStory {
  if (bidder === "ix") {
    // The reference site's slow partner: late on the entry page, never back on a phone, and on a
    // later page view its answer is in before the deadline while the page is too busy to read it.
    if (auction.page > 0) return { kind: "inTimeButTimedOut", earlyByMs: 40 };
    return device === "mobile" ? { kind: "neverAnswered" } : { kind: "late", lateByMs: 84 };
  }
  if (bidder === "triplelift") return { kind: "neverAnswered" };
  if (bidder === "openx") return { kind: "inTimeButTimedOut", earlyByMs: 40 };
  return { kind: "late", lateByMs: 30 + (hashString(`${bidder}|${auction.id}`) % 200) };
}

/**
 * On the reference site TripleLift's ad units lack `inventoryCode`: Prebid announces the bid, sends
 * nothing and reports "no bid".
 */
export function isNotSent(bid: Bid, leak: boolean): boolean {
  return leak && bid.bidder === "triplelift" && bid.status === "noBid";
}

export type ExchangeUnit = { code: string; sizes: string[]; bid: Bid };

export type ExchangeSpec = {
  auction: Auction;
  /** The bidder of a client-side request; `null` for the Prebid Server request. */
  bidder: string | null;
  units: ExchangeUnit[];
  pageUrl: string;
  /** Whether an answer came back at all. */
  answered: boolean;
};

const impId = (auction: Auction, unit: ExchangeUnit) =>
  hashString(`${auction.id}|${unit.code}|${unit.bid.bidder}`).toString(16).padStart(8, "0");

const hasPrice = (bid: Bid) =>
  bid.cpm !== null && (bid.status === "bid" || bid.status === "won" || bid.status === "rendered");

/** A consent string and ids that look the part. The decoded view never shows them; the raw body does. */
const FAKE_TC_STRING =
  "CQH0zEAQH0zEAAcABBENBNFsAP_gAEPgAAAALgNf_X__b2_r-_7_f_t0eY1P9_7__-0zjhfdl-8N3f_X_L8X_2M7vF36tq4KuR4ku3LBIUdlHPHcTVmw6okVryPsbk2cr7NKJ7PEmnMbOydYGH9_n1_z-ZKY7___f__z_v-v___9____7-3f3__5___-__e_V__9zfn9_____9vP___9v-_9__________3_79_7_H9-AAAA";
const EIDS = [
  { source: "id5-sync.com", uids: [{ id: "ID5*kT2x9aQeW4f0m1Zr8sBvQ1y7pLhJcN3d", atype: 1 }] },
  { source: "criteo.com", uids: [{ id: "8Jk2-pQ4mZ1xT7vR0bLwYc3nHs6dFa9e", atype: 1 }] },
];

function decodedRequest(spec: ExchangeSpec): DecodedBidRequest {
  const { hostname, href } = new URL(spec.pageUrl);
  return {
    id: hashString(`${spec.auction.id}|${spec.bidder ?? "pbs"}`)
      .toString(16)
      .padStart(8, "0"),
    tmaxMs: spec.auction.timeoutMs,
    imps: spec.units.map((unit) => ({
      id: impId(spec.auction, unit),
      adUnit: unit.code,
      tagId: unit.code,
      mediaTypes: ["banner"],
      sizes: unit.sizes,
      floor: 0.01,
      floorCurrency: "USD",
      bidders: spec.bidder === null ? [unit.bid.bidder] : [],
    })),
    siteDomain: hostname.replace(/^www\./, ""),
    sitePage: href,
    currencies: ["USD"],
    schain: { complete: true, nodes: [{ asi: hostname.replace(/^www\./, ""), sid: "pub-58217594", hp: true }] },
    gdpr: true,
    coppa: null,
    consentStringPresent: true,
    usPrivacyPresent: false,
    gppPresent: false,
    eidSources: EIDS.map((eid) => eid.source),
    test: false,
  };
}

const ADVERTISERS = [
  "acme-cars.example",
  "northwind-travel.example",
  "harbour-bank.example",
  "trailhead-shoes.example",
];

function decodedResponse(spec: ExchangeSpec, request: DecodedBidRequest): DecodedBidResponse {
  const bidders = [...new Set(spec.units.map((unit) => unit.bid.bidder))];
  const seats = bidders.flatMap((bidder) => {
    const bids = spec.units
      .filter((unit) => unit.bid.bidder === bidder && hasPrice(unit.bid))
      .map((unit) => {
        const seed = hashString(`${impId(spec.auction, unit)}|creative`);
        return {
          impId: impId(spec.auction, unit),
          adUnit: unit.code,
          price: unit.bid.cpm ?? 0,
          size: unit.bid.size,
          advertiserDomains: [ADVERTISERS[seed % ADVERTISERS.length] ?? "acme-cars.example"],
          creativeId: `cr-${(seed % 900_000) + 100_000}`,
          dealId: unit.bid.dealId,
          markupBytes: 900 + (seed % 2400),
        };
      });
    return bids.length > 0 ? [{ seat: bidder, bids }] : [];
  });
  const reason = spec.bidder !== null && seats.length === 0 ? noBidReason(spec.bidder) : null;
  const viaServer = spec.bidder === null;
  return {
    id: request.id,
    currency: seats.length > 0 ? "USD" : null,
    nbr: reason,
    nbrMeaning: reason === null ? null : (NO_BID_REASONS[reason] ?? null),
    seats,
    serverTimings: viaServer
      ? bidders.flatMap((bidder) => {
          const times = spec.units.flatMap((unit) =>
            unit.bid.bidder === bidder && unit.bid.responseTimeMs !== null ? [unit.bid.responseTimeMs] : [],
          );
          return times.length > 0 ? [{ bidder, ms: Math.max(...times) }] : [];
        })
      : [],
    serverErrors: viaServer
      ? bidders
          .filter((bidder) => spec.units.some((unit) => unit.bid.bidder === bidder && unit.bid.status === "timeout"))
          .map((bidder) => ({ bidder, code: 1, message: "timeout" }))
      : [],
  };
}

export function decodeExchange(spec: ExchangeSpec): DecodedExchange {
  const request = decodedRequest(spec);
  return {
    protocol: spec.bidder === null ? "prebidServer" : "openRtb",
    request,
    response: spec.answered ? decodedResponse(spec, request) : null,
  };
}

/** The request body the decoded view was read from, with the consent string and ids it leaves out. */
export function openRtbRequestBody(decoded: DecodedBidRequest, viaServer: boolean): string {
  return JSON.stringify(
    {
      id: decoded.id,
      tmax: decoded.tmaxMs,
      cur: decoded.currencies,
      imp: decoded.imps.map((imp) => ({
        id: imp.id,
        tagid: imp.tagId,
        banner: { format: imp.sizes.map((size) => ({ w: Number(size.split("x")[0]), h: Number(size.split("x")[1]) })) },
        bidfloor: imp.floor,
        bidfloorcur: imp.floorCurrency,
        ext: viaServer
          ? {
              prebid: { bidder: Object.fromEntries(imp.bidders.map((bidder) => [bidder, { placementId: imp.tagId }])) },
            }
          : { tid: `${imp.id}-0000-4000-8000-${imp.id}0000`, gpid: `/${imp.adUnit}` },
      })),
      site: { domain: decoded.siteDomain, page: decoded.sitePage, publisher: { id: "58217594" } },
      device: { w: 1280, h: 900, language: "en", dnt: 0 },
      user: { ext: { consent: FAKE_TC_STRING, eids: EIDS } },
      regs: { ext: { gdpr: decoded.gdpr ? 1 : 0 } },
      source: {
        ext: {
          schain: {
            ver: "1.0",
            complete: decoded.schain?.complete ? 1 : 0,
            nodes: decoded.schain?.nodes.map((node) => ({ asi: node.asi, sid: node.sid, hp: node.hp ? 1 : 0 })),
          },
        },
      },
    },
    null,
    0,
  );
}

export function openRtbResponseBody(decoded: DecodedBidResponse, viaServer: boolean): string {
  if (decoded.nbr !== null) return JSON.stringify({ id: decoded.id, nbr: decoded.nbr });
  const ext = viaServer
    ? {
        ext: {
          responsetimemillis: Object.fromEntries(decoded.serverTimings.map((timing) => [timing.bidder, timing.ms])),
          errors: Object.fromEntries(
            decoded.serverErrors.map((error) => [error.bidder, [{ code: error.code, message: error.message }]]),
          ),
        },
      }
    : {};
  return JSON.stringify({
    id: decoded.id,
    cur: decoded.currency ?? undefined,
    seatbid: decoded.seats.map((seat) => ({
      seat: seat.seat,
      bid: seat.bids.map((bid) => ({
        id: `${bid.impId}-1`,
        impid: bid.impId,
        price: bid.price,
        w: bid.size ? Number(bid.size.split("x")[0]) : undefined,
        h: bid.size ? Number(bid.size.split("x")[1]) : undefined,
        adomain: bid.advertiserDomains,
        crid: bid.creativeId,
        dealid: bid.dealId ?? undefined,
        adm: `<div data-creative="${bid.creativeId}">${"·".repeat(40)}</div>`,
      })),
    })),
    ...ext,
  });
}

/** Bodies of the exchanges that have their own format: readable raw, no decoder. */
export function proprietaryRequestBody(spec: ExchangeSpec): string {
  return JSON.stringify({
    tags: spec.units.map((unit) => ({
      uuid: impId(spec.auction, unit),
      code: unit.code,
      sizes: unit.sizes.map((size) => ({ width: Number(size.split("x")[0]), height: Number(size.split("x")[1]) })),
      ad_types: ["banner"],
    })),
    sdk: { source: "pbjs", version: "8.52.0" },
    gdpr_consent: { consent_string: FAKE_TC_STRING, consent_required: true },
    referrer_detection: { rd_ref: spec.pageUrl, rd_top: true },
  });
}

export function proprietaryResponseBody(spec: ExchangeSpec): string {
  return JSON.stringify({
    version: "3.0.0",
    tags: spec.units.map((unit) => ({
      uuid: impId(spec.auction, unit),
      nobid: !hasPrice(unit.bid),
      ...(hasPrice(unit.bid)
        ? { ads: [{ cpm: unit.bid.cpm, creative_id: hashString(unit.code) % 1_000_000, ad_type: "banner" }] }
        : {}),
    })),
  });
}
