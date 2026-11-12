import type { AdLink, BodyRef, NetworkRequestDetail, NetworkRow } from "@/bindings";

export function makeAdLink(overrides: Partial<AdLink> = {}): AdLink {
  return {
    auctionId: "a1",
    adUnits: ["div-gpt-top"],
    viaServer: false,
    exact: true,
    verdict: "inTime",
    lateByMs: -200,
    cpm: null,
    nbr: null,
    ...overrides,
  };
}

export function makeRow(overrides: Partial<NetworkRow> = {}): NetworkRow {
  const url = overrides.url ?? "https://www.example.com/static/app.js";
  return {
    id: "r1",
    seq: 1,
    page: 0,
    context: "page",
    method: "GET",
    url,
    host: new URL(url).host,
    kind: "script",
    mime: "application/javascript",
    status: 200,
    outcome: "ok",
    error: null,
    category: "site",
    vendor: null,
    bidder: null,
    firstParty: true,
    initiator: null,
    startedAtMs: 100,
    durationMs: 200,
    phases: { queue: 2, connect: 40, wait: 130, receive: 28 },
    transferredBytes: 12_000,
    decodedBytes: 40_000,
    servedFrom: "network",
    redirectOf: null,
    preflightOf: null,
    hasRequestBody: false,
    hasResponseBody: true,
    noHbTargeting: false,
    ad: null,
    ...overrides,
  };
}

export function makeBodyRef(overrides: Partial<BodyRef> = {}): BodyRef {
  return { state: "kept", bytes: 1200, storedBytes: 1200, mime: "application/json", scriptId: null, ...overrides };
}

export function makeRequestDetail(overrides: Partial<NetworkRequestDetail> = {}): NetworkRequestDetail {
  return {
    row: makeRow(),
    available: true,
    requestHeaders: [],
    responseHeaders: [],
    protocol: "h2",
    priority: "High",
    remoteAddress: "203.0.113.7:443",
    timing: null,
    redirectChain: [],
    preflightMs: null,
    stack: [],
    requestBody: makeBodyRef({ state: "empty", bytes: 0, storedBytes: 0, mime: null }),
    responseBody: makeBodyRef(),
    adContext: null,
    decoded: null,
    ...overrides,
  };
}
