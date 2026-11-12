import type { NetworkRow, ScanDetail } from "@/bindings";
import {
  CATEGORY_VALUES,
  type IsValue,
  parseDuration,
  parseSize,
  parseStatus,
  type QueryKey,
  TYPE_VALUES,
} from "./keys";
import type { ParsedQuery, QueryTerm } from "./parse";

/** A request slower than this is "slow"; so is a bid call that used most of its auction's timeout. */
export const SLOW_MS = 1000;
export const SLOW_BID_SHARE = 0.8;

/** A row with what filtering needs worked out once: lower-cased text to search and when it ended. */
export type IndexedRow = {
  row: NetworkRow;
  /** URL, vendor and bidder, lower case: what bare text matches. */
  haystack: string;
  urlLower: string;
  pathLower: string;
  /** End on the page's clock; `null` when the request never finished. */
  endMs: number | null;
};

export function indexRow(row: NetworkRow): IndexedRow {
  const urlLower = row.url.toLowerCase();
  let pathLower = urlLower;
  try {
    pathLower = new URL(row.url).pathname.toLowerCase();
  } catch {
    // Not a parsable URL: the whole text is searched as the path.
  }
  return {
    row,
    haystack: `${urlLower} ${row.vendor?.toLowerCase() ?? ""} ${row.bidder?.toLowerCase() ?? ""}`,
    urlLower,
    pathLower,
    endMs: row.durationMs === null ? null : row.startedAtMs + row.durationMs,
  };
}

type AuctionWindow = { id: string; page: number; startMs: number; endMs: number; timeoutMs: number | null };

/** What a filter needs to know about the scan beyond the rows: its auctions and which requests carried a winner. */
export type MatchContext = {
  /** By auction id, and by "1", "2", … as the report numbers them. */
  auctions: Map<string, AuctionWindow>;
  /** Start of the first auction of each page view. A page without one has no entry. */
  firstAuctionStart: Map<number, number>;
  wonRequestIds: Set<string>;
};

export function buildMatchContext(detail: Pick<ScanDetail, "auctions">): MatchContext {
  const auctions = new Map<string, AuctionWindow>();
  const firstAuctionStart = new Map<number, number>();
  const wonRequestIds = new Set<string>();
  detail.auctions.forEach((auction, index) => {
    const window: AuctionWindow = {
      id: auction.id,
      page: auction.page,
      startMs: auction.startedAtMs,
      endMs: auction.endedAtMs ?? auction.startedAtMs + (auction.timeoutMs ?? 0),
      timeoutMs: auction.timeoutMs,
    };
    auctions.set(auction.id.toLowerCase(), window);
    auctions.set(String(index + 1), window);
    const first = firstAuctionStart.get(auction.page);
    if (first === undefined || auction.startedAtMs < first) firstAuctionStart.set(auction.page, auction.startedAtMs);
    for (const unit of auction.adUnits) {
      for (const bid of unit.bids) {
        const won = bid.status === "won" || bid.status === "rendered";
        if (won && bid.exchange?.requestId) wonRequestIds.add(bid.exchange.requestId);
      }
    }
  });
  return { auctions, firstAuctionStart, wonRequestIds };
}

type RowTest = (indexed: IndexedRow, context: MatchContext) => boolean;

export function isSlow({ row }: IndexedRow, context: MatchContext): boolean {
  if (row.durationMs === null) return false;
  if (row.durationMs > SLOW_MS) return true;
  const timeout =
    row.category === "bid" && row.ad ? context.auctions.get(row.ad.auctionId.toLowerCase())?.timeoutMs : null;
  return timeout != null && row.durationMs > timeout * SLOW_BID_SHARE;
}

const IS_TESTS: Record<IsValue, RowTest> = {
  failed: ({ row }) => row.outcome === "failed" || row.outcome === "httpError",
  blocked: ({ row }) => row.outcome === "blocked",
  cancelled: ({ row }) => row.outcome === "cancelled",
  pending: ({ row }) => row.outcome === "pending",
  slow: isSlow,
  bid: ({ row }) => row.category === "bid",
  late: ({ row }) => row.category === "bid" && row.ad?.verdict === "lateOnNetwork",
  // An answer that arrived in time and carried no bid. A late or missing answer is not a "no bid".
  nobid: ({ row }) =>
    row.category === "bid" &&
    row.ad !== null &&
    row.ad.cpm === null &&
    (row.ad.verdict === "inTime" || row.ad.verdict === "atDeadline"),
  won: ({ row }, context) => context.wonRequestIds.has(row.id),
  cached: ({ row }) =>
    row.servedFrom === "diskCache" || row.servedFrom === "memoryCache" || row.servedFrom === "prefetch",
  sw: ({ row }) => row.servedFrom === "serviceWorker",
  redirect: ({ row }) => row.redirectOf !== null || (row.status !== null && row.status >= 300 && row.status < 400),
  post: ({ row }) => row.method === "POST",
  "first-party": ({ row }) => row.firstParty,
  "third-party": ({ row }) => !row.firstParty,
  // On a page view that never auctioned, every request is "before the first auction": the page waited for one.
  "before-auction": ({ row }, context) => {
    const first = context.firstAuctionStart.get(row.page);
    return first === undefined || row.startedAtMs < first;
  },
  "no-hb-targeting": ({ row }) => row.noHbTargeting,
};

/** The test for one value of a key. Values were validated when the query was parsed. */
function valueTest(key: QueryKey, value: string): RowTest {
  const lower = value.toLowerCase();
  switch (key) {
    case "host":
      return ({ row }) => row.host === lower || row.host.endsWith(`.${lower}`);
    case "url":
      return ({ urlLower }) => urlLower.includes(lower);
    case "path":
      return ({ pathLower }) => pathLower.includes(lower);
    case "method":
      return ({ row }) => row.method.toLowerCase() === lower;
    case "status": {
      const range = parseStatus(value);
      return ({ row }) => range !== null && row.status !== null && row.status >= range.min && row.status <= range.max;
    }
    case "type": {
      const kinds = TYPE_VALUES[lower] ?? [];
      return ({ row }) => kinds.includes(row.kind);
    }
    case "cat":
      return ({ row }) => row.category === CATEGORY_VALUES[lower];
    case "vendor":
      return ({ row }) => row.vendor?.toLowerCase().includes(lower) ?? false;
    case "bidder":
      return ({ row }) => row.bidder?.toLowerCase() === lower;
    case "unit":
      return ({ row }) => row.ad?.adUnits.some((unit) => unit.toLowerCase() === lower) ?? false;
    case "page":
      return ({ row }) => row.page === Number(value) - 1;
    case "auction":
      // Its own bid and ad-server calls, and everything else under way while it ran.
      return ({ row, endMs }, context) => {
        const auction = context.auctions.get(lower);
        if (!auction) return false;
        if (row.ad?.auctionId === auction.id) return true;
        return (
          row.page === auction.page &&
          row.startedAtMs <= auction.endMs &&
          (endMs ?? Number.POSITIVE_INFINITY) >= auction.startMs
        );
      };
    case "initiator":
      return ({ row }) => row.initiator?.url?.toLowerCase().includes(lower) ?? false;
    case "mime":
      return ({ row }) => row.mime?.toLowerCase().includes(lower) ?? false;
    case "larger-than": {
      const bytes = parseSize(value) ?? Number.POSITIVE_INFINITY;
      return ({ row }) => row.transferredBytes > bytes;
    }
    case "slower-than": {
      const ms = parseDuration(value) ?? Number.POSITIVE_INFINITY;
      return ({ row }) => row.durationMs !== null && row.durationMs > ms;
    }
    case "is":
      return IS_TESTS[lower as IsValue];
  }
}

function termTest(term: QueryTerm): RowTest {
  let test: RowTest;
  if (term.kind === "text") test = ({ haystack }) => haystack.includes(term.needle);
  else if (term.kind === "regex") test = ({ row }) => term.pattern.test(row.url);
  else {
    const tests = term.values.map((value) => valueTest(term.key, value));
    test = (indexed, context) => tests.some((candidate) => candidate(indexed, context));
  }
  return term.negated ? (indexed, context) => !test(indexed, context) : test;
}

/** One function for a whole filter: a row passes when every term does. */
export function compileQuery(parsed: ParsedQuery, context: MatchContext): (indexed: IndexedRow) => boolean {
  const tests = parsed.terms.map(termTest);
  return (indexed) => tests.every((test) => test(indexed, context));
}
