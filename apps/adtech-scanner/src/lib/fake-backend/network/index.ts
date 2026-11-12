import type {
  AdContext,
  BodyRef,
  BodySide,
  DiagnosticCheck,
  Header,
  InitiatorRef,
  NetworkArchiveInfo,
  NetworkBodyChunk,
  NetworkCategory,
  NetworkRequestDetail,
  NetworkRow,
  NetworkRowChunk,
  NetworkSummary,
  RowPhases,
  ScanDetail,
  StackFrame,
  TimingDetail,
} from "@/bindings";
import { ApiError } from "@/lib/api/error";
import type { NetworkBodyQuery, NetworkRowsQuery } from "@/lib/api/port";
import { hashString } from "../rng";
import { adCallLine, capturedPositionOf, capturedText, sourceLineCount } from "../sources";
import { NO_BID_REASONS } from "./exchange";
import { type BodyDraft, type FrameDraft, type NetworkPlan, planNetwork, type RequestDraft } from "./plan";

const DEFAULT_CHUNK_ROWS = 2000;
const MAX_CHUNK_ROWS = 5000;
const DEFAULT_BODY_BYTES = 2 * 1024 * 1024;
const MAX_BODY_BYTES = 8 * 1024 * 1024;

const VENDORS: [suffix: string, name: string][] = [
  ["doubleclick.net", "Google"],
  ["googlesyndication.com", "Google"],
  ["2mdn.net", "Google"],
  ["google-analytics.com", "Google"],
  ["googletagmanager.com", "Google"],
  ["adnxs.com", "Xandr"],
  ["rubiconproject.com", "Magnite"],
  ["pubmatic.com", "PubMatic"],
  ["casalemedia.com", "Index Exchange"],
  ["indexww.com", "Index Exchange"],
  ["criteo.com", "Criteo"],
  ["criteo.net", "Criteo"],
  ["openx.net", "OpenX"],
  ["3lift.com", "TripleLift"],
  ["lijit.com", "Sovrn"],
  ["sharethrough.com", "Sharethrough"],
  ["amazon-adsystem.com", "Amazon"],
  ["cookielaw.org", "OneTrust"],
  ["onetrust.com", "OneTrust"],
  ["id5-sync.com", "ID5"],
  ["doubleverify.com", "DoubleVerify"],
  ["adsafeprotected.com", "Integral Ad Science"],
  ["scorecardresearch.com", "Comscore"],
  ["chartbeat.com", "Chartbeat"],
  ["chartbeat.net", "Chartbeat"],
  ["segment.com", "Segment"],
  ["segment.io", "Segment"],
  ["taboola.com", "Taboola"],
  ["adsrvr.org", "The Trade Desk"],
  ["crwdcntrl.net", "Lotame"],
];

function vendorOf(host: string, draft: RequestDraft): string | null {
  if (draft.ad?.viaServer) return "Prebid Server";
  return VENDORS.find(([suffix]) => host === suffix || host.endsWith(`.${suffix}`))?.[1] ?? null;
}

const stored = (body: BodyDraft) => body.state === "kept" || body.state === "truncated";

/** How a request's time splits into the phases of the waterfall; `null` where the browser reports none. */
function phasesOf(draft: RequestDraft, firstOnHost: boolean): RowPhases | null {
  if (
    draft.duration === null ||
    draft.servedFrom !== "network" ||
    draft.outcome === "failed" ||
    draft.outcome === "blocked"
  ) {
    return null;
  }
  const seed = hashString(`${draft.url}|${draft.at}`);
  const queue = Math.min(draft.duration * 0.1, 1 + (seed % 6));
  const connect = firstOnHost ? Math.min(draft.duration * 0.35, 24 + (seed % 70)) : 0;
  const receive = Math.min((draft.duration - queue - connect) * 0.6, 2 + draft.size / 40_000);
  return { queue, connect, wait: Math.max(0, draft.duration - queue - connect - receive), receive };
}

export type FakeNetwork = {
  rows: NetworkRow[];
  drafts: Map<string, RequestDraft>;
  /** Where each stack frame lies in its script as captured. */
  frames: Map<FrameDraft, StackFrame>;
};

function resolveFrames(detail: ScanDetail, drafts: RequestDraft[]): Map<FrameDraft, StackFrame> {
  const unique = new Set(drafts.flatMap((draft) => draft.stack));
  const frames = new Map<FrameDraft, StackFrame>();
  // One script at a time: its text is generated once for all the frames that point into it.
  const ordered = [...unique].sort((a, b) => (a.script?.id ?? "").localeCompare(b.script?.id ?? ""));
  for (const frame of ordered) {
    const { script } = frame;
    if (!script) {
      frames.set(frame, { function: frame.functionName, url: frame.url, scriptId: null, line: 1, column: 1 });
      continue;
    }
    const line =
      (frame.call ? adCallLine(detail, script, frame.call) : null) ??
      1 + (hashString(`${script.id}|${frame.functionName}`) % sourceLineCount(detail, script));
    const position = capturedPositionOf(detail, script, line);
    frames.set(frame, {
      function: frame.functionName,
      url: script.url,
      scriptId: script.id,
      line: position.line,
      column: position.column ?? 1,
    });
  }
  return frames;
}

function initiatorOf(draft: RequestDraft, frames: Map<FrameDraft, StackFrame>, pageUrl: string): InitiatorRef | null {
  if (draft.preflightOf)
    return { kind: "preflight", url: draft.preflightOf.url, scriptId: null, line: null, column: null };
  const [innermost] = draft.stack;
  const frame = innermost ? frames.get(innermost) : undefined;
  if (frame)
    return { kind: "script", url: frame.url, scriptId: frame.scriptId, line: frame.line, column: frame.column };
  // A navigation has no initiator; everything else on the page starts with its HTML.
  if (draft.kind === "document" && draft.context === "page") return null;
  if (draft.redirectOf) return { kind: "other", url: draft.redirectOf.url, scriptId: null, line: null, column: null };
  return { kind: draft.preload ? "preload" : "parser", url: pageUrl, scriptId: null, line: null, column: null };
}

function withoutQuery(url: string): string {
  const { origin, pathname } = new URL(url);
  return `${origin}${pathname}`;
}

/** The request list of a scan, as the backend serves it. `evicted`: only what the list itself keeps is left. */
export function materializeNetwork(detail: ScanDetail, padTo = 0): FakeNetwork {
  const { drafts } = planNetwork(detail, padTo);
  const evicted = detail.network?.availability === "evicted";
  const rootDomain = new URL(detail.summary.url).hostname.replace(/^www\./, "");
  const frames = resolveFrames(detail, drafts);
  const seenHosts = new Set<string>();
  const rows = drafts.map((draft): NetworkRow => {
    const { host } = new URL(draft.url);
    const firstOnHost = !seenHosts.has(`${draft.page}|${host}`);
    seenHosts.add(`${draft.page}|${host}`);
    const fromNetwork = draft.servedFrom === "network";
    const compressible = stored(draft.response) || draft.kind === "script";
    return {
      id: draft.id,
      seq: draft.seq,
      page: draft.page,
      context: draft.context,
      method: draft.method,
      url: evicted ? withoutQuery(draft.url) : draft.url,
      host,
      kind: draft.kind,
      mime: draft.response.mime,
      status: draft.status,
      outcome: draft.outcome,
      error: draft.error,
      category: draft.category,
      vendor: vendorOf(host, draft),
      bidder: draft.bidder,
      firstParty: host === rootDomain || host.endsWith(`.${rootDomain}`),
      initiator: initiatorOf(draft, frames, detail.pages[draft.page]?.url ?? detail.summary.url),
      startedAtMs: draft.at,
      durationMs: draft.duration,
      phases: evicted ? null : phasesOf(draft, firstOnHost),
      transferredBytes:
        fromNetwork && draft.duration !== null ? Math.round(draft.size * (compressible ? 0.32 : 1)) + 310 : 0,
      decodedBytes: draft.size,
      servedFrom: draft.servedFrom,
      redirectOf: draft.redirectOf?.id ?? null,
      preflightOf: draft.preflightOf?.id ?? null,
      hasRequestBody: !evicted && stored(draft.request),
      hasResponseBody: !evicted && stored(draft.response),
      noHbTargeting: draft.noHbTargeting,
      ad: draft.ad,
    };
  });
  return { rows, drafts: new Map(drafts.map((draft) => [draft.id, draft])), frames };
}

export function listRows(network: FakeNetwork, query: NetworkRowsQuery = {}): NetworkRowChunk {
  const pages = query.pages ?? [];
  const selected = pages.length > 0 ? network.rows.filter((row) => pages.includes(row.page)) : network.rows;
  const from = query.cursor ? Number(query.cursor) : 0;
  const limit = Math.min(MAX_CHUNK_ROWS, Math.max(1, query.limit ?? DEFAULT_CHUNK_ROWS));
  const to = from + limit;
  return {
    rows: selected.slice(from, to),
    nextCursor: to < selected.length ? String(to) : null,
    total: selected.length,
  };
}

const EMPTY_ARCHIVE: NetworkArchiveInfo = {
  availability: "notRecorded",
  requestCount: 0,
  bodyCount: 0,
  storedBytes: 0,
  headersRedacted: true,
  truncated: false,
};

/** As the backend picks them: failed bid calls, late and unanswered ones, blocked requests, the slowest before the first auction. */
function notableRows(rows: NetworkRow[], detail: ScanDetail): NetworkRow[] {
  const auctionStarts = detail.auctions.filter((auction) => auction.page === 0).map((auction) => auction.startedAtMs);
  const firstAuction = auctionStarts.length > 0 ? Math.min(...auctionStarts) : undefined;
  let slowest: NetworkRow | undefined;
  if (firstAuction !== undefined) {
    for (const row of rows) {
      if (row.page !== 0 || row.startedAtMs >= firstAuction || row.durationMs === null) continue;
      if (!slowest || row.durationMs > (slowest.durationMs ?? 0)) slowest = row;
    }
  }
  const picks = [
    ...rows.filter((row) => row.category === "bid" && ["failed", "httpError", "blocked"].includes(row.outcome)),
    ...rows.filter((row) => row.ad?.verdict === "lateOnNetwork" || row.ad?.verdict === "neverAnswered"),
    ...rows.filter((row) => row.outcome === "blocked"),
    ...(slowest ? [slowest] : []),
  ];
  return [...new Set(picks)].slice(0, 8);
}

export function summarise(detail: ScanDetail, network: FakeNetwork | null): NetworkSummary {
  const rows = network?.rows ?? [];
  const sum = (items: NetworkRow[]) => items.reduce((total, row) => total + row.transferredBytes, 0);
  const tally = <K extends string | number>(key: (row: NetworkRow) => K | null) => {
    const groups = new Map<K, NetworkRow[]>();
    for (const row of rows) {
      const value = key(row);
      if (value !== null) groups.set(value, [...(groups.get(value) ?? []), row]);
    }
    return [...groups.entries()];
  };
  return {
    archive: detail.network ?? EMPTY_ARCHIVE,
    totalRequests: rows.length,
    transferredBytes: sum(rows),
    failed: rows.filter((row) => row.outcome === "failed" || row.outcome === "httpError").length,
    blockedOrCancelled: rows.filter((row) => row.outcome === "blocked" || row.outcome === "cancelled").length,
    lateBids: rows.filter((row) => row.category === "bid" && row.ad?.verdict === "lateOnNetwork").length,
    unansweredBids: rows.filter((row) => row.category === "bid" && row.ad?.verdict === "neverAnswered").length,
    fromCache: rows.filter((row) => row.servedFrom !== "network").length,
    categories: tally<NetworkCategory>((row) => row.category).map(([category, items]) => ({
      category,
      requests: items.length,
      transferredBytes: sum(items),
    })),
    vendors: tally((row) => row.vendor)
      .map(([vendor, items]) => ({ vendor, requests: items.length }))
      .sort((a, b) => b.requests - a.requests)
      .slice(0, 10),
    pages: tally((row) => row.page).map(([page, items]) => ({
      page,
      requests: items.length,
      transferredBytes: sum(items),
    })),
    notable: notableRows(rows, detail),
  };
}

const FAKE_COOKIES: [name: string, value: string][] = [
  ["_ga", "GA1.1.1620948112.1731402812"],
  ["_sd_session", "s%3A8Jk2-pQ4mZ1xT7vR0bLwYc3nHs6dFa9e"],
  ["OptanonConsent", "isGpcEnabled=0&datestamp=Tue+Nov+12+2026&version=202410.1.0&groups=C0001%3A1%2CC0002%3A1"],
];

const bytes = (text: string) => new TextEncoder().encode(text).length;

function cookieHeader(redacted: boolean): Header {
  return redacted
    ? {
        name: "Cookie",
        value: FAKE_COOKIES.map(([name, value]) => `${name}=[${bytes(value)} bytes]`).join("; "),
        redaction: "cookie",
      }
    : { name: "Cookie", value: FAKE_COOKIES.map(([name, value]) => `${name}=${value}`).join("; "), redaction: null };
}

function setCookieHeader(draft: RequestDraft, redacted: boolean): Header {
  const value = `u${hashString(draft.url).toString(16)}${hashString(draft.id).toString(16)}`;
  const attributes = "Path=/; Max-Age=31536000; Secure; SameSite=None";
  return redacted
    ? { name: "Set-Cookie", value: `uid=[${bytes(value)} bytes]; ${attributes}`, redaction: "cookie" }
    : { name: "Set-Cookie", value: `uid=${value}; ${attributes}`, redaction: null };
}

const FAKE_TOKEN = `eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9.${"c3BvcnRzZGVzay1zZXNzaW9u".repeat(7)}.sig`;

function requestHeaders(draft: RequestDraft, detail: ScanDetail, redacted: boolean): Header[] {
  const plain = (name: string, value: string): Header => ({ name, value, redaction: null });
  const pageUrl = detail.pages[draft.page]?.url ?? detail.summary.url;
  const headers = [
    plain("Accept", draft.kind === "document" ? "text/html,application/xhtml+xml,*/*;q=0.8" : "*/*"),
    plain("Accept-Encoding", "gzip, deflate, br, zstd"),
    plain("Accept-Language", "en-GB,en;q=0.9"),
    plain("Referer", pageUrl),
    plain(
      "User-Agent",
      "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36",
    ),
  ];
  if (draft.method === "POST") {
    headers.push(plain("Content-Type", draft.request.mime ?? "text/plain"), plain("Origin", new URL(pageUrl).origin));
  }
  if (draft.method === "OPTIONS") {
    headers.push(
      plain("Access-Control-Request-Method", "POST"),
      plain("Access-Control-Request-Headers", "content-type"),
    );
  }
  if (draft.authorization) {
    headers.push(
      redacted
        ? { name: "Authorization", value: `Bearer [${bytes(FAKE_TOKEN)} bytes]`, redaction: "credential" }
        : plain("Authorization", `Bearer ${FAKE_TOKEN}`),
    );
  }
  if (draft.cookies) headers.push(cookieHeader(redacted));
  return headers;
}

function responseHeaders(draft: RequestDraft, redacted: boolean): Header[] {
  if (draft.status === null) return [];
  const plain = (name: string, value: string): Header => ({ name, value, redaction: null });
  const headers = [
    plain("Date", "Thu, 12 Nov 2026 14:13:32 GMT"),
    plain(
      "Cache-Control",
      draft.kind === "script" || draft.kind === "stylesheet" ? "public, max-age=31536000, immutable" : "no-store",
    ),
    plain("Timing-Allow-Origin", "*"),
  ];
  if (draft.response.mime) headers.unshift(plain("Content-Type", draft.response.mime));
  if (draft.status >= 300 && draft.status < 400) headers.push(plain("Location", "(the next hop)"));
  if (draft.category !== "site") headers.push(plain("Access-Control-Allow-Origin", "*"));
  if (draft.method === "OPTIONS") headers.push(plain("Access-Control-Allow-Methods", "POST, GET, OPTIONS"));
  if (stored(draft.response)) headers.push(plain("Content-Encoding", "br"));
  if (draft.setsCookie) headers.push(setCookieHeader(draft, redacted));
  return headers;
}

function timingOf(row: NetworkRow): TimingDetail | null {
  const { phases } = row;
  if (!phases || row.durationMs === null) return null;
  const connecting = phases.connect > 0;
  return {
    queuedMs: phases.queue,
    dnsMs: connecting ? phases.connect * 0.2 : null,
    connectMs: connecting ? phases.connect * 0.35 : null,
    tlsMs: connecting ? phases.connect * 0.45 : null,
    sendMs: 1,
    waitMs: phases.wait,
    receiveMs: phases.receive,
    responseAtMs: row.startedAtMs + row.durationMs - phases.receive,
    finishedAtMs: row.startedAtMs + row.durationMs,
  };
}

function bodyText(detail: ScanDetail, body: BodyDraft): string | null {
  if (!stored(body)) return null;
  const script = body.scriptId ? detail.scripts.find((candidate) => candidate.id === body.scriptId) : undefined;
  if (script) return capturedText(detail, script);
  return body.text?.() ?? "";
}

function bodyRef(detail: ScanDetail, draft: RequestDraft, side: BodySide, evicted: boolean): BodyRef {
  const body = side === "request" ? draft.request : draft.response;
  if (evicted) {
    return {
      state: body.state === "empty" ? "empty" : "evicted",
      bytes: body.bytes ?? draft.size,
      storedBytes: 0,
      mime: body.mime,
      scriptId: null,
    };
  }
  const reported = body.bytes ?? (side === "response" ? draft.size : bytes(bodyText(detail, body) ?? ""));
  let storedBytes = 0;
  if (body.state === "kept") storedBytes = reported;
  else if (body.state === "truncated") storedBytes = body.storedBytes ?? reported;
  return { state: body.state, bytes: reported, storedBytes, mime: body.mime, scriptId: body.scriptId ?? null };
}

function redirectChain(network: FakeNetwork, draft: RequestDraft) {
  let first = draft;
  while (first.redirectOf) first = first.redirectOf;
  const chain = [first];
  for (;;) {
    const last = chain[chain.length - 1];
    const next = [...network.drafts.values()].find((candidate) => candidate.redirectOf === last);
    if (!next) break;
    chain.push(next);
  }
  return chain.length > 1 ? chain.map((hop) => ({ requestId: hop.id, url: hop.url, status: hop.status })) : [];
}

/** Bid requests only: an ad-server call is tied to its slot by the slot's own request id. */
function adContext(detail: ScanDetail, draft: RequestDraft): AdContext | null {
  if (!draft.ad || !draft.exchange) return null;
  const auction = detail.auctions.find((candidate) => candidate.id === draft.ad?.auctionId);
  const bidders = [...new Set(draft.exchange.units.map((unit) => unit.bid.bidder))];
  const findings = detail.findings.filter((finding) =>
    finding.affectedBidders.some((bidder) => bidders.includes(bidder)),
  );
  const checks = (detail.bidderDiagnostics?.bidders ?? [])
    .filter((diagnosis) => bidders.includes(diagnosis.bidder))
    .flatMap((diagnosis) => diagnosis.checks)
    .filter((check) => check.status === "problem" || check.status === "notice");
  return {
    link: draft.ad,
    bidder: draft.bidder,
    units: draft.exchange.units.map((unit) => ({
      adUnit: unit.code,
      status: unit.bid.status,
      cpm: unit.bid.cpm,
      verdict: draft.ad?.verdict ?? "inTime",
    })),
    auctionStartedAtMs: auction?.startedAtMs ?? null,
    auctionEndedAtMs: auction?.endedAtMs ?? null,
    timeoutMs: auction?.timeoutMs ?? null,
    slotElementId: null,
    findingIds: findings.map((finding) => finding.id),
    checkCodes: [...new Set(checks.map((check) => check.code))],
  };
}

function draftOf(network: FakeNetwork, requestId: string): RequestDraft {
  const draft = network.drafts.get(requestId);
  if (!draft) throw new ApiError("notFound", `Request ${requestId} does not exist in this scan`);
  return draft;
}

export function requestDetail(detail: ScanDetail, network: FakeNetwork, requestId: string): NetworkRequestDetail {
  const draft = draftOf(network, requestId);
  const row = network.rows.find((candidate) => candidate.id === requestId) as NetworkRow;
  const evicted = detail.network?.availability === "evicted";
  const redacted = detail.network?.headersRedacted ?? true;
  const preflight = [...network.drafts.values()].find((candidate) => candidate.preflightOf === draft);
  const base = {
    row,
    requestBody: bodyRef(detail, draft, "request", evicted),
    responseBody: bodyRef(detail, draft, "response", evicted),
  };
  if (evicted) {
    return {
      ...base,
      available: false,
      requestHeaders: [],
      responseHeaders: [],
      protocol: null,
      priority: null,
      remoteAddress: null,
      timing: null,
      redirectChain: [],
      preflightMs: null,
      stack: [],
      adContext: null,
      decoded: null,
    };
  }
  return {
    ...base,
    available: true,
    requestHeaders: requestHeaders(draft, detail, redacted),
    responseHeaders: responseHeaders(draft, redacted),
    protocol: draft.kind === "webSocket" ? "websocket" : draft.status === null ? null : "h2",
    priority:
      draft.kind === "document" ? "VeryHigh" : draft.kind === "script" || draft.kind === "stylesheet" ? "High" : "Low",
    remoteAddress:
      draft.servedFrom === "network" && draft.status !== null ? `203.0.113.${hashString(row.host) % 250}:443` : null,
    timing: timingOf(row),
    redirectChain: redirectChain(network, draft),
    preflightMs: preflight?.duration ?? null,
    stack: draft.stack.flatMap((frame) => network.frames.get(frame) ?? []),
    adContext: adContext(detail, draft),
    decoded: draft.decoded,
  };
}

export function bodyChunk(
  detail: ScanDetail,
  network: FakeNetwork,
  requestId: string,
  query: NetworkBodyQuery,
): NetworkBodyChunk {
  const draft = draftOf(network, requestId);
  const evicted = detail.network?.availability === "evicted";
  const ref = bodyRef(detail, draft, query.side, evicted);
  const text = evicted ? null : bodyText(detail, query.side === "request" ? draft.request : draft.response);
  if (text === null) return { state: ref.state, text: null, offset: 0, nextOffset: null, totalBytes: 0 };
  // The simulated bodies are close to one byte per character; offsets count characters here.
  const offset = Math.max(0, query.offset ?? 0);
  const end = offset + Math.min(MAX_BODY_BYTES, Math.max(1, query.maxBytes ?? DEFAULT_BODY_BYTES));
  return {
    state: ref.state,
    text: text.slice(offset, end),
    offset,
    nextOffset: end < text.length ? end : null,
    totalBytes: text.length,
  };
}

/** How a scan's archive is kept: not at all, or with cookie and authorization values replaced. */
export type ArchiveMode = { keep: boolean; redactHeaders: boolean; evicted?: boolean; padTo?: number };

function responseCheck(drafts: RequestDraft[]): DiagnosticCheck | null {
  if (drafts.length === 0) return null;
  const check = (status: DiagnosticCheck["status"], code: string, summary: string, excerpt: string | null) => ({
    area: "response" as const,
    status,
    code,
    summary,
    excerpt,
    adUnits: [],
    pages: [...new Set(drafts.map((draft) => draft.page))],
    requestIds: drafts.map((draft) => draft.id),
  });
  const failed = drafts.filter((draft) => draft.outcome === "failed");
  if (failed.length > 0) {
    return check("problem", "response-failed", `The bid endpoint could not be reached (${failed[0]?.error})`, null);
  }
  const reason = drafts.find((draft) => draft.ad?.nbr !== null && draft.ad?.nbr !== undefined)?.ad?.nbr;
  if (reason !== undefined && reason !== null) {
    return check(
      "notice",
      "response-no-bid-reason",
      `The endpoint answered with no-bid reason ${reason} (${NO_BID_REASONS[reason]})`,
      `{"nbr":${reason}}`,
    );
  }
  const requests = `${drafts.length} ${drafts.length === 1 ? "request" : "requests"}`;
  return check("pass", "response-ok", `The endpoint answered ${requests}`, null);
}

/**
 * Records a finished scan's network data: gives the scan its archive, and every bid, slot request,
 * script, bidder check and piece of evidence the id of its request.
 */
export function recordNetwork(detail: ScanDetail, mode: ArchiveMode): void {
  const plan: NetworkPlan = planNetwork(detail, mode.padTo);
  const { links, drafts } = plan;
  for (const auction of detail.auctions) {
    for (const unit of auction.adUnits) {
      for (const bid of unit.bids) {
        const link = links.bids.get(bid);
        if (!link) continue;
        const { draft, ...facts } = link;
        // Bid calls are read while they pass even when nothing is stored: the verdict is known, the request is not kept.
        bid.exchange = { ...facts, requestId: mode.keep ? (draft?.id ?? null) : null };
      }
    }
  }
  if (!mode.keep) {
    detail.network = { ...EMPTY_ARCHIVE, availability: "disabled", headersRedacted: mode.redactHeaders };
    return;
  }
  for (const slot of detail.slots) {
    for (const request of slot.requests) request.requestId = links.slotRequests.get(request)?.id ?? null;
  }
  for (const script of detail.scripts) {
    const draft = links.scripts.get(script.id);
    script.requestId = draft?.id ?? null;
    script.initiator = draft?.stack[0]?.url ?? detail.pages[0]?.url ?? null;
    script.sha256 = script.hash.repeat(4);
  }
  linkChecks(detail, plan);
  for (const finding of detail.findings) {
    for (const evidence of finding.evidence) {
      // Timeline evidence about a slot is labelled with it: its ad-server call is the request to show.
      const slot = detail.slots.find(
        (candidate) => candidate.elementId === evidence.label && finding.pages.includes(candidate.page),
      );
      if (evidence.kind === "timeline" && slot) evidence.requestId = slot.requests[0]?.requestId ?? null;
    }
  }
  detail.network = {
    availability: mode.evicted ? "evicted" : "available",
    requestCount: drafts.length,
    bodyCount: mode.evicted ? 0 : drafts.filter((draft) => stored(draft.response) || stored(draft.request)).length,
    storedBytes: mode.evicted ? 0 : Math.round(drafts.reduce((sum, draft) => sum + draft.size, 0) * 0.24),
    headersRedacted: mode.redactHeaders,
    truncated: drafts.some((draft) => draft.response.state === "dropped"),
  };
}

function linkChecks(detail: ScanDetail, { drafts, links }: NetworkPlan) {
  for (const diagnosis of detail.bidderDiagnostics?.bidders ?? []) {
    const { bidder } = diagnosis;
    const requests = drafts.filter(
      (draft) => draft.exchange?.units.some((unit) => unit.bid.bidder === bidder) && draft.method !== "OPTIONS",
    );
    const answered = responseCheck(requests);
    if (answered) {
      diagnosis.checks = diagnosis.checks.filter((check) => check.code !== "response-not-seen");
      if (answered.status === "pass") diagnosis.checks.push(answered);
      else diagnosis.checks.unshift(answered);
    }
    const unsent = detail.auctions.flatMap((auction) =>
      auction.adUnits.filter((unit) =>
        unit.bids.some((bid) => bid.bidder === bidder && links.bids.get(bid)?.verdict === "notSent"),
      ),
    );
    if (unsent.length === 0) continue;
    const units = [...new Set(unsent.map((unit) => unit.code))];
    diagnosis.checks = diagnosis.checks.filter((check) => check.code !== "params-ok");
    diagnosis.checks.unshift({
      area: "params",
      status: "problem",
      code: "params-missing-required",
      summary: `Required param inventoryCode is missing on ${units.length} ${units.length === 1 ? "ad unit" : "ad units"}: Prebid announces the bid and sends nothing`,
      excerpt: `{ bidder: "${bidder}", params: {} }`,
      adUnits: units,
      pages: [],
      requestIds: [],
    });
  }
}
