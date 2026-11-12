import type {
  AdLink,
  AdSlot,
  Auction,
  Bid,
  BidExchange,
  BodyState,
  DecodedExchange,
  NetworkCategory,
  PageView,
  RequestContext,
  RequestOutcome,
  ResourceKind,
  ScanDetail,
  ScriptCategory,
  ScriptRecord,
  ServedFrom,
  SlotRequest,
} from "@/bindings";
import { createRng, hashString, hexId, type Rng } from "../rng";
import {
  BID_ENDPOINTS,
  type BidEndpoint,
  classifyExchange,
  decodeExchange,
  type ExchangeSpec,
  type ExchangeUnit,
  isNotSent,
  noBidReason,
  openRtbRequestBody,
  openRtbResponseBody,
  PREBID_SERVER,
  proprietaryRequestBody,
  proprietaryResponseBody,
  SERVER_BIDDERS,
  SERVER_OVERHEAD_MS,
  timeoutStory,
} from "./exchange";
import { articleJson, commentsJson, creativeHtml, documentHtml, largeFeedJson } from "./payloads";

/** A frame of the stack that made a request: a place in one of the scan's scripts, or in code the scan did not keep. */
export type FrameDraft = {
  functionName: string;
  script: ScriptRecord | null;
  /** Used when the script is not one of the scan's. */
  url: string;
  /** An ad-stack call to find in the script; its line is the frame's. */
  call?: string;
};

export type BodyDraft = {
  state: BodyState;
  /** Size as the browser reported it; defaults to the text's length. */
  bytes?: number;
  /** Less than `bytes` for a truncated body. */
  storedBytes?: number;
  mime: string | null;
  scriptId?: string;
  text?: () => string;
};

/** One request of the simulated visit, before it has a number. */
export type RequestDraft = {
  id: string;
  seq: number;
  page: number;
  at: number;
  /** `null`: it never finished. */
  duration: number | null;
  method: string;
  url: string;
  kind: ResourceKind;
  status: number | null;
  outcome: RequestOutcome;
  error: string | null;
  category: NetworkCategory;
  bidder: string | null;
  context: RequestContext;
  servedFrom: ServedFrom;
  /** Decoded size of the answer. */
  size: number;
  /** Innermost first; empty when the parser made the request. */
  stack: FrameDraft[];
  preload: boolean;
  redirectOf: RequestDraft | null;
  preflightOf: RequestDraft | null;
  request: BodyDraft;
  response: BodyDraft;
  noHbTargeting: boolean;
  ad: AdLink | null;
  /** The browser attached cookies, the answer set some, the request carried a bearer token. */
  cookies: boolean;
  setsCookie: boolean;
  authorization: boolean;
  exchange: ExchangeSpec | null;
  decoded: DecodedExchange | null;
};

type DraftInput = Pick<RequestDraft, "page" | "at" | "duration" | "url" | "kind" | "category" | "size"> &
  Partial<Omit<RequestDraft, "id" | "seq">>;

/** A bid's side of the network: its request (none when Prebid never sent it) and how the answer related to the auction. */
export type BidLink = Omit<BidExchange, "requestId"> & { draft: RequestDraft | null };

/** What the plan says about the records of the report: which request each bid, slot request and script has. */
export type NetworkLinks = {
  bids: Map<Bid, BidLink>;
  slotRequests: Map<SlotRequest, RequestDraft>;
  scripts: Map<string, RequestDraft>;
};

export type NetworkPlan = { drafts: RequestDraft[]; links: NetworkLinks };

const NO_BODY: BodyDraft = { state: "empty", mime: null };
const METADATA_ONLY = (mime: string): BodyDraft => ({ state: "notKeptType", mime });
const json = (text: () => string): BodyDraft => ({ state: "kept", mime: "application/json", text });

const SCRIPT_CATEGORY: Record<ScriptCategory, NetworkCategory> = {
  prebid: "wrapper",
  wrapper: "wrapper",
  adServer: "wrapper",
  bidder: "wrapper",
  cmp: "consent",
  adQuality: "measurement",
  analytics: "measurement",
  application: "site",
  library: "otherThirdParty",
  unknown: "otherThirdParty",
};

const SYNC_PIXELS = [
  "https://ib.adnxs.com/getuid?https%3A%2F%2Fsync.sportsdesk-ads.example%2Fsetuid%3Fbidder%3Dadnxs%26uid%3D%24UID",
  "https://pixel.rubiconproject.com/tap.php?v=4222&nid=2149&put=usr_8Jk2pQ4mZ1xT7v",
  "https://image8.pubmatic.com/AdServer/ImgSync?p=164120&pu=https%3A%2F%2Fimage4.pubmatic.com%2FAdServer%2FSPug",
  "https://ssum.casalemedia.com/usermatchredir?s=194730&cb=https%3A%2F%2Fsync.sportsdesk-ads.example%2Fsetuid",
  "https://u.openx.net/w/1.0/cm?id=891039ac-a916-42bb-a651-4be9e3b201da&ph=a3aece0c",
  "https://eb2.3lift.com/getuid?gdpr=1&cmp_cs=CQH0zEAQH0zEAAcABBENBNFsAP&redir=%24UID",
  "https://ap.lijit.com/pixel?redir=https%3A%2F%2Fsync.sportsdesk-ads.example%2Fsetuid%3Fbidder%3Dsovrn",
  "https://gum.criteo.com/sync?c=72&r=2&j=cr_handle_data",
  "https://match.adsrvr.org/track/cmf/generic?ttd_pid=pubmatic&ttd_tpi=1&gdpr=1",
  "https://sync.1rx.io/usersync2/rmphb?gdpr=1&redir=https%3A%2F%2Fsync.sportsdesk-ads.example",
  "https://x.bidswitch.net/sync?ssp=themediagrid&gdpr=1",
  "https://cs.media.net/cksync?cs=1&type=pbs&ovsid=setstatuscode",
  "https://s.amazon-adsystem.com/iu3?cm3ppd=1&d=dtb-pub&csif=t&dl=n-index_rb",
  "https://sync.outbrain.com/cookie-sync?p=taboola&uid=a1f4d2c8",
  "https://ups.analytics.yahoo.com/ups/58301/sync?uid=8Jk2pQ4m&_origin=1",
  "https://pixel.tapad.com/idsync/ex/receive?partner_id=2499&partner_device_id=8Jk2pQ4m",
] as const;

const BEACONS = [
  "https://www.google-analytics.com/g/collect?v=2&tid=G-7H2M4XW1PQ&en=page_view&_s=1",
  "https://www.google-analytics.com/g/collect?v=2&tid=G-7H2M4XW1PQ&en=scroll&epn.percent_scrolled=50",
  "https://ping.chartbeat.net/ping?h=sportsdesk.example&p=%2Ffootball%2Ftransfer-news&d=sportsdesk.example&g=64121",
  "https://sb.scorecardresearch.com/b?c1=2&c2=18120612&cs_ucfr=1&rn=1731402812",
  "https://tps.doubleverify.com/visit.jpg?ctx=741233&cmp=DV112&sid=sportsdesk&dvtagver=6.1.src",
  "https://pixel.adsafeprotected.com/services/pub?anId=926841&slot=%7Bid%3Adiv-gpt-top-leaderboard%7D",
  "https://api.segment.io/v1/p",
  "https://pagead2.googlesyndication.com/pagead/gen_204?id=av-js&type=reach&bin=7",
  "https://www.googletagmanager.com/a?id=GTM-K4T8XQ&cv=41&v=3&t=t&pid=1620948112&rv=b9s",
  "https://stats.g.doubleclick.net/g/collect?v=2&tid=G-7H2M4XW1PQ&aip=1",
] as const;

const SITE_IMAGES = [
  "hero-1600",
  "thumb-transfer-1",
  "thumb-transfer-2",
  "thumb-derby",
  "author-m-keller",
  "logo",
] as const;

class Planner {
  readonly drafts: RequestDraft[] = [];
  readonly links: NetworkLinks = { bids: new Map(), slotRequests: new Map(), scripts: new Map() };
  private readonly rng: Rng;
  private readonly origin: string;
  private readonly rootDomain: string;
  private readonly leak: boolean;

  constructor(private readonly detail: ScanDetail) {
    const { hostname, origin } = new URL(detail.summary.url);
    this.origin = origin;
    this.rootDomain = hostname.replace(/^www\./, "");
    this.rng = createRng(hashString(`${hostname}|${detail.summary.device}|network`));
    // The reference site (and the demo page on this machine) is the one with the planted bugs.
    this.leak = detail.metrics.prebidTimeoutMs === 1500;
  }

  /** The catalogue's URLs name the reference site; other sites get their own name in them. */
  private local(url: string): string {
    return url
      .replaceAll("sportsdesk.example", this.rootDomain)
      .replaceAll("sportsdesk", this.rootDomain.split(".")[0] ?? "site");
  }

  private script(match: string): ScriptRecord | null {
    return this.detail.scripts.find((script) => script.url.includes(match)) ?? null;
  }

  private frame(match: string, functionName: string, call?: string): FrameDraft[] {
    const script = this.script(match);
    return script ? [{ functionName, script, url: script.url, call }] : [];
  }

  add(input: DraftInput): RequestDraft {
    const draft: RequestDraft = {
      id: "",
      seq: 0,
      method: "GET",
      status: 200,
      outcome: "ok",
      error: null,
      bidder: null,
      context: "page",
      servedFrom: "network",
      stack: [],
      preload: false,
      redirectOf: null,
      preflightOf: null,
      request: NO_BODY,
      response: NO_BODY,
      noHbTargeting: false,
      ad: null,
      cookies: false,
      setsCookie: false,
      authorization: false,
      exchange: null,
      decoded: null,
      ...input,
      at: Math.max(0, Math.round(input.at)),
      duration: input.duration === null ? null : Math.max(1, Math.round(input.duration)),
    };
    this.drafts.push(draft);
    return draft;
  }

  page(page: PageView, next: PageView | undefined) {
    const loadsDocument = page.reachedBy !== "routeChange";
    const end = (page.loadMs ?? 0) + page.dwellMs;
    if (loadsDocument) this.document(page);
    this.scripts(page, loadsDocument);
    this.siteAssets(page, loadsDocument);
    this.siteData(page, end);
    if (loadsDocument) this.consent(page);
    this.auctions(page);
    this.adServer(page);
    this.sync(page);
    this.measurement(page, end);
    this.openEnded(page, end, next !== undefined);
  }

  private document(page: PageView) {
    const www = new URL(page.url).hostname.startsWith("www.");
    // People type the bare domain: the entry page is reached through one redirect.
    const redirect =
      page.index === 0 && www
        ? this.add({
            page: page.index,
            at: 0,
            duration: 48,
            url: page.url.replace("//www.", "//"),
            kind: "document",
            category: "site",
            status: 301,
            size: 0,
            response: { state: "unavailable", mime: null },
          })
        : null;
    const at = redirect ? 50 : 0;
    this.add({
      page: page.index,
      at,
      duration: Math.min((page.loadMs ?? 900) * 0.12, 420) + 90,
      url: page.url,
      kind: "document",
      category: "site",
      size: 86_400,
      redirectOf: redirect,
      cookies: true,
      setsCookie: true,
      response: { state: "kept", mime: "text/html", text: () => documentHtml(page, this.detail.scripts) },
    });
  }

  private scripts(page: PageView, loadsDocument: boolean) {
    if (!loadsDocument) return;
    const gtm = this.script("googletagmanager.com/gtm.js");
    for (const script of this.detail.scripts) {
      if (script.inline || !script.pages.includes(page.index)) continue;
      // Tags that the tag manager injects have a script as their initiator; the rest are in the HTML.
      const injected = script.category === "analytics" && script !== gtm && gtm !== null;
      const cached = page.index > 0;
      const duration = cached ? 4 : 30 + Math.min(260, script.sizeBytes / 2600) + (hashString(script.url) % 60);
      const draft = this.add({
        page: page.index,
        at: (script.loadedAtMs ?? 400) - duration,
        duration,
        url: script.url,
        kind: "script",
        category: SCRIPT_CATEGORY[script.category],
        size: script.sizeBytes,
        servedFrom: cached ? "diskCache" : "network",
        stack: injected ? this.frame("googletagmanager.com/gtm.js", "injectTag") : [],
        cookies: script.firstParty,
        response: { state: "kept", mime: "application/javascript", scriptId: script.id },
      });
      if (!this.links.scripts.has(script.id)) this.links.scripts.set(script.id, draft);
    }
  }

  private siteAssets(page: PageView, loadsDocument: boolean) {
    const cached = page.index > 0;
    const cdn = `https://cdn.${this.rootDomain}`;
    if (loadsDocument) {
      for (const [index, sheet] of ["base.3f1c9a", "article.77b0e2", "ads.0a41de"].entries()) {
        this.add({
          page: page.index,
          at: 70 + index * 14,
          duration: cached ? 3 : 60 + index * 22,
          url: `${cdn}/static/css/${sheet}.css`,
          kind: "stylesheet",
          category: "site",
          size: 18_400 + index * 9_100,
          servedFrom: cached ? "diskCache" : "network",
          response: { state: "kept", mime: "text/css", text: () => `/* ${sheet}.css */\n.ad-slot{min-height:250px}\n` },
        });
      }
      for (const [index, font] of ["inter-var", "source-serif-4", "source-serif-4-italic"].entries()) {
        this.add({
          page: page.index,
          at: 190 + index * 9,
          duration: cached ? 2 : 80 + index * 30,
          url: `${cdn}/static/fonts/${font}.woff2`,
          kind: "font",
          category: "site",
          size: 48_200 + index * 7_700,
          servedFrom: cached ? "memoryCache" : "network",
          preload: index === 0,
          response: METADATA_ONLY("font/woff2"),
        });
      }
    }
    const images = loadsDocument ? 30 : 20;
    for (let index = 0; index < images; index += 1) {
      const name = SITE_IMAGES[index % SITE_IMAGES.length];
      // One author portrait was deleted from the image service a while ago.
      const missing = loadsDocument && index === 4;
      this.add({
        page: page.index,
        at: 260 + index * 46 + this.rng.int(0, 30),
        duration: 70 + this.rng.int(0, 240),
        url: `https://img.${this.rootDomain}/${640 + (index % 3) * 320}/${name}-${page.index}${index}.${missing ? "jpg" : "webp"}`,
        kind: "image",
        category: "site",
        status: missing ? 404 : 200,
        outcome: missing ? "httpError" : "ok",
        size: missing ? 310 : 24_000 + this.rng.int(0, 190_000),
        response: missing
          ? { state: "kept", mime: "text/plain", text: () => "Not found" }
          : METADATA_ONLY("image/webp"),
      });
    }
  }

  private siteData(page: PageView, end: number) {
    const api = `${this.origin}/api/v2`;
    const app = this.frame("chunk-article", "loadArticle");
    const slug = new URL(page.url).pathname.split("/").filter(Boolean).pop() ?? "front";
    this.add({
      page: page.index,
      at: 420,
      duration: 132,
      url: `${api}/article/${slug}.json?fields=body,related,tags`,
      kind: "fetch",
      category: "site",
      size: 14_800,
      stack: app,
      cookies: true,
      response: json(() => articleJson(page)),
    });
    this.add({
      page: page.index,
      at: 610,
      duration: 96,
      url: `${api}/comments/latest.json?article=${slug}&limit=20`,
      kind: "fetch",
      category: "site",
      size: 3_900,
      stack: this.frame("chunk-comments", "fetchComments"),
      cookies: true,
      response: json(() => commentsJson()),
    });
    this.add({
      page: page.index,
      at: 1_180,
      duration: 64,
      url: `${api}/metrics/vitals`,
      method: "POST",
      kind: "ping",
      category: "site",
      status: 204,
      size: 0,
      stack: app,
      cookies: true,
      request: json(() => JSON.stringify({ lcp: 1840, cls: 0.04, inp: 96, page: page.url })),
    });
    if (page.index === 0) {
      // The session endpoint wants a bearer token the page still holds from an expired login.
      this.add({
        page: 0,
        at: 380,
        duration: 58,
        url: `${this.origin}/api/session`,
        kind: "xhr",
        category: "site",
        status: 401,
        outcome: "httpError",
        size: 74,
        stack: this.frame("paywall", "restoreSession"),
        cookies: true,
        authorization: true,
        response: json(() => JSON.stringify({ error: "token_expired", message: "Sign in again" })),
      });
      this.add({
        page: 0,
        at: 2_300,
        duration: 940,
        url: `${api}/feed/archive.json?season=2026`,
        kind: "fetch",
        category: "site",
        size: 3_250_000,
        stack: app,
        response: json(() => largeFeedJson(3_250_000)),
      });
      this.add({
        page: 0,
        at: 2_900,
        duration: 1_860,
        url: `${api}/feed/full-season.json?season=2025`,
        kind: "fetch",
        category: "site",
        size: 7_759_462,
        stack: app,
        response: {
          state: "truncated",
          bytes: 7_759_462,
          storedBytes: 5 * 1024 * 1024,
          mime: "application/json",
          text: () => largeFeedJson(7_759_462, 5 * 1024 * 1024),
        },
      });
      // The archive writer fell behind while those two were stored: this one was left out.
      this.add({
        page: 0,
        at: 3_300,
        duration: 310,
        url: `${api}/feed/standings.json`,
        kind: "fetch",
        category: "site",
        size: 212_480,
        stack: app,
        response: { state: "dropped", bytes: 212_480, mime: "application/json" },
      });
      this.add({
        page: 0,
        at: 1_400,
        duration: end - 1_400,
        url: `wss://live.${this.rootDomain}/updates?channel=football`,
        kind: "webSocket",
        category: "site",
        status: 101,
        size: 0,
        stack: app,
      });
    } else {
      this.add({
        page: page.index,
        at: 340,
        duration: 6,
        url: `${this.origin}/static/offline.json`,
        kind: "fetch",
        category: "site",
        size: 1_240,
        servedFrom: "serviceWorker",
        context: "serviceWorker",
        response: json(() => JSON.stringify({ version: "sw-41", precache: ["/", "/offline"] })),
      });
    }
  }

  private consent(page: PageView) {
    const sdk = this.frame("otSDKStub", "loadBanner");
    this.add({
      page: page.index,
      at: 300,
      duration: 88,
      url: "https://cdn.cookielaw.org/consent/0193a7c2-58f1-7d44-a1e0-4c1d5b7f9a31/0193a7c2-58f1-7d44-a1e0-4c1d5b7f9a31.json",
      kind: "xhr",
      category: "consent",
      size: 6_200,
      servedFrom: page.index > 0 ? "diskCache" : "network",
      stack: sdk,
      response: json(() => JSON.stringify({ CookieSPAEnabled: false, RuleSet: [{ Type: "IAB2V2", Default: true }] })),
    });
    this.add({
      page: page.index,
      at: 395,
      duration: 71,
      url: "https://geolocation.onetrust.com/cookieconsentpub/v1/geo/location",
      kind: "script",
      category: "consent",
      size: 96,
      stack: sdk,
      response: {
        state: "kept",
        mime: "application/javascript",
        text: () => 'jsonFeed({"country":"BE","state":"VLG","stateName":"Flanders","continent":"EU"});',
      },
    });
    if (page.index === 0) {
      this.add({
        page: 0,
        at: 760,
        duration: 142,
        url: "https://privacyportal.onetrust.com/request/v1/consentreceipts",
        method: "POST",
        kind: "xhr",
        category: "consent",
        size: 62,
        stack: this.frame("otBannerSdk", "saveConsent"),
        request: json(() =>
          JSON.stringify({ requestInformation: "eyJhbGciOiJSUzUxMiJ9", identifier: "a41f", test: false }),
        ),
        response: json(() => JSON.stringify({ receipt: "0193a7c2", status: "CONFIRMED" })),
      });
    }
  }

  private auctions(page: PageView) {
    const prebid = [...this.frame("/prebid", "ajax"), ...this.frame("/prebid", "callBids")];
    const wrapper = this.frame("hb-wrapper", "requestBids", "requestBids");
    const stack = [...prebid, ...wrapper];
    for (const auction of this.detail.auctions.filter((candidate) => candidate.page === page.index)) {
      // "The auction closed" is its start plus its timeout: an auction whose bidders all answered ends
      // early, and nobody was late for it.
      const end =
        auction.timeoutMs === null
          ? (auction.endedAtMs ?? auction.startedAtMs)
          : auction.startedAtMs + auction.timeoutMs;
      const units: ExchangeUnit[] = auction.adUnits.flatMap((unit) =>
        unit.bids.map((bid) => ({ code: unit.code, sizes: unit.sizes, bid })),
      );
      const sent = units.filter((unit) => {
        if (!isNotSent(unit.bid, this.leak)) return true;
        this.links.bids.set(unit.bid, {
          draft: null,
          verdict: "notSent",
          lateByMs: null,
          networkMs: null,
          exact: false,
        });
        return false;
      });
      const server = sent.filter((unit) => SERVER_BIDDERS.includes(unit.bid.bidder));
      if (server.length > 0) this.exchange(auction, end, null, PREBID_SERVER, server, page, stack);
      // A bidder sends one request for all its ad units. Bids that went differently get their own.
      const groups = new Map<string, ExchangeUnit[]>();
      for (const unit of sent.filter((candidate) => !server.includes(candidate))) {
        const fate = unit.bid.status === "timeout" || unit.bid.status === "error" ? unit.bid.status : "answered";
        const key = `${unit.bid.bidder}|${fate}`;
        groups.set(key, [...(groups.get(key) ?? []), unit]);
      }
      for (const group of groups.values()) {
        const bidder = group[0]?.bid.bidder ?? "";
        const endpoint = BID_ENDPOINTS[bidder];
        if (endpoint) this.exchange(auction, end, bidder, endpoint, group, page, stack);
      }
    }
    // Amazon's own auction runs beside Prebid's; nothing ties its call to a Prebid auction.
    if (page.metrics.auctionCount > 0) {
      this.add({
        page: page.index,
        at: (page.metrics.timeToFirstAuctionMs ?? 800) + 6,
        duration: 164,
        url: `https://c.amazon-adsystem.com/e/dtb/bid?src=3121&u=${encodeURIComponent(page.url)}&pid=4f1ba2&cb=0&ws=1280x900&v=7.63.00&t=1200&slots=%5B%7B%22sd%22%3A%22div-gpt-top-leaderboard%22%7D%5D`,
        kind: "xhr",
        category: "bid",
        size: 1_180,
        stack: this.frame("apstag", "fetchBids"),
        cookies: true,
        response: json(() =>
          JSON.stringify({ slots: [{ slotID: "div-gpt-top-leaderboard", amzniid: "", size: "0x0" }] }),
        ),
      });
    }
  }

  private exchange(
    auction: Auction,
    auctionEnd: number,
    bidder: string | null,
    endpoint: BidEndpoint,
    units: ExchangeUnit[],
    page: PageView,
    stack: FrameDraft[],
  ) {
    const at = Math.min(...units.map((unit) => unit.bid.requestedAtMs ?? auction.startedAtMs));
    const answeredTimes = units.flatMap((unit) => (unit.bid.responseTimeMs === null ? [] : [unit.bid.responseTimeMs]));
    const fate = units[0]?.bid.status;
    let duration: number | null = Math.max(40, ...answeredTimes) + (bidder === null ? SERVER_OVERHEAD_MS : 0);
    let failed = false;
    if (fate === "error") {
      failed = true;
      duration = 41;
    } else if (fate === "timeout" && bidder !== null) {
      const story = timeoutStory(bidder, auction, this.detail.summary.device);
      if (story.kind === "neverAnswered") duration = null;
      else duration = auctionEnd + (story.kind === "late" ? story.lateByMs : -story.earlyByMs) - at;
    } else if (at + duration > auctionEnd - 12) {
      // An answer Prebid counted cannot have arrived after the auction closed.
      duration = Math.max(20, auctionEnd - 12 - at);
    }
    const answered = duration !== null && !failed;
    const spec: ExchangeSpec = { auction, bidder, units, pageUrl: page.url, answered };
    const decoded = endpoint.openRtb ? decodeExchange(spec) : null;
    const { verdict, lateByMs } = classifyExchange({
      endedAtMs: duration === null ? null : at + duration,
      auctionEndedAtMs: auctionEnd,
      failed,
      timedOutInPage: fate === "timeout",
    });
    // The request's price is the best of its bids in the currency of the first: nothing converts between two.
    const priced = units.flatMap((unit) => (answered && unit.bid.cpm !== null ? [unit.bid] : []));
    const currency = priced[0]?.currency ?? null;
    const prices = priced.flatMap((bid) => (bid.currency === currency ? (bid.cpm ?? []) : []));
    const reason = answered && prices.length === 0 && bidder !== null ? noBidReason(bidder) : null;
    const requestText = decoded?.request
      ? () => openRtbRequestBody(decoded.request as NonNullable<typeof decoded.request>, bidder === null)
      : () => proprietaryRequestBody(spec);
    const responseText = decoded?.response
      ? () => openRtbResponseBody(decoded.response as NonNullable<typeof decoded.response>, bidder === null)
      : () => proprietaryResponseBody(spec);
    const get = endpoint.method === "GET";
    const url = get
      ? `${endpoint.url}?account_id=26014&site_id=581204&zone_id=${3_100_000 + (hashString(auction.id) % 900_000)}&size_id=2&alt_size_ids=57&rp_floor=0.01&gdpr=1&tk_flint=pbjs_lite_v8.52.0&p_pos=atf&rf=${encodeURIComponent(page.url)}`
      : endpoint.url;
    let response: BodyDraft;
    if (failed) response = { state: "unavailable", mime: null };
    else if (duration === null) response = { state: "pending", mime: null };
    else response = { state: "kept", mime: "application/json", text: responseText };
    const draft = this.add({
      page: page.index,
      at,
      duration,
      url,
      method: endpoint.method,
      kind: "xhr",
      category: "bid",
      bidder,
      status: answered ? 200 : null,
      outcome: failed ? "failed" : duration === null ? "pending" : "ok",
      error: failed ? "net::ERR_CONNECTION_RESET" : null,
      size: answered ? 600 + units.length * 820 : 0,
      stack,
      cookies: true,
      setsCookie: answered,
      request: get ? NO_BODY : { state: "kept", mime: "text/plain", text: requestText },
      response,
      ad: {
        auctionId: auction.id,
        adUnits: [...new Set(units.map((unit) => unit.code))],
        viaServer: bidder === null,
        exact: endpoint.openRtb,
        verdict,
        lateByMs,
        cpm: prices.length > 0 ? Math.max(...prices) : null,
        currency: prices.length > 0 ? currency : null,
        nbr: reason,
      },
      exchange: spec,
      decoded,
    });
    if (endpoint.preflight) {
      this.add({
        page: page.index,
        at: at - 45,
        duration: 45,
        url,
        method: "OPTIONS",
        kind: "preflight",
        category: "bid",
        bidder,
        status: 204,
        size: 0,
        preflightOf: draft,
        response: { state: "unavailable", mime: null },
      });
    }
    for (const unit of units) {
      this.links.bids.set(unit.bid, {
        draft,
        verdict,
        lateByMs,
        networkMs: duration,
        exact: endpoint.openRtb,
      });
    }
    return draft;
  }

  private adServer(page: PageView) {
    const gpt = [
      ...this.frame("pubads_impl", "sendAdRequest"),
      ...this.frame("hb-wrapper", "sendAdServerRequest", "refresh"),
    ];
    const slots = this.detail.slots.filter((slot) => slot.page === page.index);
    for (const slot of slots) {
      for (const [index, request] of slot.requests.entries()) {
        const refresh = index > 0;
        const duration =
          !refresh && slot.responseAtMs !== null
            ? slot.responseAtMs - request.atMs
            : slot.outcome === "pending"
              ? null
              : 180;
        const draft = this.add({
          page: page.index,
          at: request.atMs,
          duration,
          url: gamUrl(slot, request, page, this.rng),
          kind: "xhr",
          category: "adServer",
          status: duration === null ? null : 200,
          outcome: duration === null ? "pending" : "ok",
          size: duration === null ? 0 : slot.outcome === "empty" ? 420 : 9_800 + this.rng.int(0, 2_400),
          stack: gpt,
          cookies: true,
          noHbTargeting: !request.hadPrebidTargeting,
          response:
            duration === null
              ? { state: "pending", mime: null }
              : {
                  state: "kept",
                  mime: "text/plain",
                  text: () =>
                    JSON.stringify({
                      [slot.adUnitPath ?? slot.elementId]: ["html", 0, null, null, 0, 90, 728, 1, 0, null, null, 1],
                    }),
                },
        });
        this.links.slotRequests.set(request, draft);
      }
      if (slot.renderedAtMs !== null) this.creative(page, slot, slot.renderedAtMs);
    }
  }

  private creative(page: PageView, slot: AdSlot, renderedAt: number) {
    const frame = hexId(this.rng, 26);
    const host = `https://${frame}.safeframe.googlesyndication.com`;
    this.add({
      page: page.index,
      at: renderedAt - 46,
      duration: 38,
      url: `${host}/safeframe/1-0-40/html/container.html`,
      kind: "document",
      category: "creative",
      context: "iframe",
      size: 5_300,
      response: { state: "kept", mime: "text/html", text: () => creativeHtml(slot) },
    });
    const assets: [string, ResourceKind, string, number][] = [
      [`https://tpc.googlesyndication.com/simgad/${this.rng.int(10 ** 15, 10 ** 16)}`, "image", "image/png", 64_000],
      [
        `https://s0.2mdn.net/ads/richmedia/studio/pv2/${this.rng.int(6e7, 7e7)}/logo.svg`,
        "image",
        "image/svg+xml",
        4_100,
      ],
      ["https://tpc.googlesyndication.com/sodar/sodar2.js", "script", "application/javascript", 17_600],
      [
        `https://cdn.doubleverify.com/dv-measurements${this.rng.int(100, 999)}.js`,
        "script",
        "application/javascript",
        41_200,
      ],
    ];
    for (const [index, [url, kind, mime, size]] of assets.entries()) {
      this.add({
        page: page.index,
        at: renderedAt - 6 + index * 22,
        duration: 44 + this.rng.int(0, 120),
        url,
        kind,
        category: index === 3 ? "measurement" : "creative",
        context: "iframe",
        size,
        response:
          kind === "image"
            ? METADATA_ONLY(mime)
            : { state: "kept", mime, text: () => `/* ${url.split("/").pop()} */\n(function(){var w=window;})();` },
      });
    }
    this.add({
      page: page.index,
      at: renderedAt + 30,
      duration: 52,
      url: `https://securepubads.g.doubleclick.net/pcs/view?xai=AKAOjs${hexId(this.rng, 24)}&sig=Cg0ArKJSzE&uach_m=%5BUACH%5D&adurl=`,
      kind: "ping",
      category: "measurement",
      context: "iframe",
      status: 204,
      size: 0,
    });
  }

  private sync(page: PageView) {
    if (page.metrics.auctionCount === 0) return;
    const from = (page.metrics.timeToFirstAuctionMs ?? 900) + 1_700;
    const count = page.reachedBy === "routeChange" ? 10 : 28;
    for (let index = 0; index < count; index += 1) {
      // Partners sync in rounds: the second round is the same pixels with the ids they just learned.
      const round = index < SYNC_PIXELS.length ? "" : "&rnd=2";
      const url = `${this.local(SYNC_PIXELS[index % SYNC_PIXELS.length] ?? SYNC_PIXELS[0])}${round}`;
      // One exchange's sync endpoint is having a bad day.
      const broken = index === 9;
      this.add({
        page: page.index,
        at: from + index * 74 + this.rng.int(0, 40),
        duration: 60 + this.rng.int(0, 190),
        url,
        kind: "image",
        category: "identity",
        context: index % 3 === 0 ? "iframe" : "page",
        status: broken ? 503 : 200,
        outcome: broken ? "httpError" : "ok",
        size: broken ? 190 : 43,
        cookies: true,
        setsCookie: !broken,
        response: broken
          ? { state: "kept", mime: "text/html", text: () => "<h1>503 Service Unavailable</h1>" }
          : METADATA_ONLY("image/gif"),
      });
    }
    if (page.index === 0) {
      // A sync that bounces through two partners before it lands.
      const first = this.add({
        page: 0,
        at: from + 420,
        duration: 62,
        url: "https://sync.crwdcntrl.net/map/c=9828/tp=PUBM/gdpr=1?https%3A%2F%2Fib.adnxs.com%2Fsetuid%3Fentity%3D315",
        kind: "image",
        category: "identity",
        status: 302,
        size: 0,
        cookies: true,
        setsCookie: true,
        response: { state: "unavailable", mime: null },
      });
      const second = this.add({
        page: 0,
        at: from + 484,
        duration: 71,
        url: "https://ib.adnxs.com/setuid?entity=315&code=8Jk2pQ4mZ1xT7vR0bLwY",
        kind: "image",
        category: "identity",
        status: 302,
        size: 0,
        redirectOf: first,
        cookies: true,
        setsCookie: true,
        response: { state: "unavailable", mime: null },
      });
      this.add({
        page: 0,
        at: from + 557,
        duration: 48,
        url: "https://cm.g.doubleclick.net/pixel?google_nid=appnexus&google_cm&google_hm=OEprMnBRNG1aMXhUN3ZSMGJMd1k",
        kind: "image",
        category: "identity",
        size: 43,
        redirectOf: second,
        cookies: true,
        setsCookie: true,
        response: METADATA_ONLY("image/gif"),
      });
      this.add({
        page: 0,
        at: from - 900,
        duration: 118,
        url: "https://id5-sync.com/g/v2/1204.json",
        method: "POST",
        kind: "xhr",
        category: "identity",
        size: 412,
        stack: this.frame("id5-api", "fetchId"),
        cookies: true,
        request: json(() => JSON.stringify({ partner: 1204, v: "1.0.62", o: "api", gdpr: 1, rf: page.url, top: 1 })),
        response: json(() =>
          JSON.stringify({ universal_uid: "ID5*kT2x9aQeW4f0m1Zr8sBvQ1y7pLhJcN3d", cascade_needed: false }),
        ),
      });
    }
  }

  private measurement(page: PageView, end: number) {
    const gtm = this.frame("googletagmanager.com/gtm.js", "sendBeacon");
    const count = page.reachedBy === "routeChange" ? 30 : 46;
    for (let index = 0; index < count; index += 1) {
      const url = this.local(BEACONS[index % BEACONS.length] ?? BEACONS[0]);
      const segment = url.includes("segment.io");
      // The segment endpoint answers without the CORS header the page's fetch needs.
      const blocked = segment && index === 6;
      this.add({
        page: page.index,
        at: 500 + (index / count) * Math.max(2_000, end - 1_500) + this.rng.int(0, 120),
        duration: 34 + this.rng.int(0, 140),
        url,
        method: segment || url.includes("/g/collect") ? "POST" : "GET",
        kind: segment ? "fetch" : url.includes("collect") || url.includes("gen_204") ? "ping" : "image",
        category: "measurement",
        status: blocked ? null : url.includes("collect") || url.includes("gen_204") ? 204 : 200,
        outcome: blocked ? "blocked" : "ok",
        error: blocked ? "Blocked by CORS policy: no Access-Control-Allow-Origin header" : null,
        size: blocked ? 0 : 35,
        stack: gtm,
        cookies: !segment,
        request: segment
          ? json(() => JSON.stringify({ type: "page", anonymousId: "a41f-22c1", name: page.title }))
          : NO_BODY,
        response: blocked
          ? { state: "unavailable", mime: null }
          : url.includes("collect") || url.includes("gen_204")
            ? NO_BODY
            : METADATA_ONLY("image/gif"),
      });
    }
  }

  private openEnded(page: PageView, end: number, navigatesAway: boolean) {
    // A long poll that the page still had open when the visit ended.
    if (page.index === 0) {
      this.add({
        page: 0,
        at: Math.max(4_000, end - 9_000),
        duration: null,
        url: `${this.origin}/api/live/poll?since=1731402812&channel=transfer-news`,
        kind: "fetch",
        category: "site",
        status: null,
        outcome: "pending",
        size: 0,
        stack: this.frame("chunk-article", "pollLiveBlog"),
        cookies: true,
        response: { state: "pending", mime: null },
      });
    }
    if (navigatesAway) {
      this.add({
        page: page.index,
        at: end - 140,
        duration: 138,
        url: `https://img.${this.rootDomain}/960/related-story-${page.index}.webp`,
        kind: "image",
        category: "site",
        status: null,
        outcome: "cancelled",
        error: "net::ERR_ABORTED",
        size: 0,
        response: { state: "unavailable", mime: null },
      });
    }
  }

  /** Creative and beacon traffic to reach `total` rows, for the large-list cases. */
  pad(total: number, pages: PageView[]) {
    let index = 0;
    while (this.drafts.length < total) {
      const page = pages[index % pages.length] as PageView;
      const end = (page.loadMs ?? 0) + page.dwellMs;
      const beacon = index % 3 !== 0;
      const url = beacon
        ? `${this.local(BEACONS[index % BEACONS.length] ?? BEACONS[0])}&seq=${index}`
        : `https://tpc.googlesyndication.com/simgad/${10 ** 15 + index * 7919}`;
      this.add({
        page: page.index,
        at: 300 + ((index * 7919) % Math.max(1, end - 400)),
        duration: 30 + ((index * 131) % 380),
        url,
        kind: beacon ? "ping" : "image",
        category: beacon ? "measurement" : "creative",
        context: beacon ? "page" : "iframe",
        status: beacon ? 204 : 200,
        size: beacon ? 0 : 20_000 + ((index * 977) % 90_000),
        response: beacon ? NO_BODY : METADATA_ONLY("image/png"),
      });
      index += 1;
    }
  }
}

function gamUrl(slot: AdSlot, request: SlotRequest, page: PageView, rng: Rng): string {
  const [, network = "21000000000", site = "site", unit = "unit"] = (slot.adUnitPath ?? "").split("/");
  const targeting = request.hadPrebidTargeting
    ? `hb_bidder=appnexus&hb_pb=1.20&hb_size=${slot.sizes[0] ?? "300x250"}&hb_adid=${request.prebidAdId ?? "none"}&hb_format=banner&pos=${unit}`
    : `pos=${unit}`;
  const params = new URLSearchParams({
    gdfp_req: "1",
    pvsid: String(rng.int(10 ** 14, 10 ** 15)),
    correlator: String(rng.int(10 ** 14, 10 ** 15)),
    output: "ldjh",
    impl: "fifs",
    iu_parts: `${network},${site},${unit}`,
    enc_prev_ius: "/0/1/2",
    prev_iu_szs: slot.sizes.join("|"),
    cust_params: "section=football&article_type=news&paywall=0",
    prev_scp: targeting,
    gdpr: "1",
    gdpr_consent: "CQH0zEAQH0zEAAcABBENBNFsAP_gAEPgAAAALgNf_X__b2_r-_7_f_t0eY1P9_7__-0zjhfdl-8N3f",
    url: page.url,
    vis: "1",
    psts: "AOrYGsl",
  });
  return `https://securepubads.g.doubleclick.net/gampad/ads?${params}`;
}

/** A visit of this many page views reaches the scan's cap for stored bodies on its last one. */
const CAPPED_FROM_PAGES = 3;

/**
 * A long visit fills the scan's allowance for bodies: what the last page view fetched after it was
 * reached is listed with its size and without its body. The scripts of that page view were stored
 * with the first page that loaded them.
 */
function capBodies(drafts: RequestDraft[], pages: PageView[]) {
  const last = pages[pages.length - 1];
  if (pages.length < CAPPED_FROM_PAGES || !last) return;
  for (const draft of drafts) {
    const data = draft.kind === "fetch" || draft.kind === "xhr";
    if (draft.page !== last.index || !data || draft.response.state !== "kept" || draft.exchange) continue;
    draft.response = { state: "scanLimit", bytes: draft.response.bytes ?? draft.size, mime: draft.response.mime };
  }
}

/**
 * Every request of a simulated visit, derived from the scan's own auctions, slots, scripts and page
 * views so that each of them can name its request. Depends on nothing but those records: planning
 * the same scan twice gives the same requests with the same ids.
 */
export function planNetwork(detail: ScanDetail, padTo = 0): NetworkPlan {
  const planner = new Planner(detail);
  for (const [index, page] of detail.pages.entries()) planner.page(page, detail.pages[index + 1]);
  const byStart = (a: RequestDraft, b: RequestDraft) => a.page - b.page || a.at - b.at || a.seq - b.seq;
  // The visit's own requests are numbered first, so padding the list never renumbers them.
  const own = planner.drafts.length;
  [...planner.drafts].sort(byStart).forEach((draft, seq) => {
    draft.seq = seq;
  });
  planner.pad(padTo, detail.pages);
  planner.drafts.slice(own).forEach((draft, index) => {
    draft.seq = own + index;
  });
  for (const draft of planner.drafts) draft.id = `r${draft.seq}`;
  capBodies(planner.drafts.slice(0, own), detail.pages);
  return { drafts: [...planner.drafts].sort(byStart), links: planner.links };
}
