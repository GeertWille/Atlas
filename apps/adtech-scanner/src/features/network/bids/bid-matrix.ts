import type { Auction, Bid, DecodedBidResponse, ExchangeVerdict, NetworkRow } from "@/bindings";
import { verdictShort } from "@/shared/domain/exchange-verdict";
import { currenciesOf } from "@/shared/domain/prices";
import { BID_STATUSES } from "@/shared/domain/vocabulary";
import { formatCpm, formatDuration, pluralize } from "@/shared/lib/format";

export type MatrixCell = {
  bid: Bid;
  /** `null` when no request left the page for the bid, and on scans without a network record. */
  requestId: string | null;
  verdict: ExchangeVerdict | null;
  lateByMs: number | null;
  networkMs: number | null;
  /** OpenRTB no-bid reason of the answer. */
  nbr: number | null;
  /** Prebid Server: how long this bidder took on the server, and what the server said went wrong. */
  serverMs: number | null;
  serverError: string | null;
};

export type MatrixRequest = {
  id: string;
  row: NetworkRow | undefined;
  /** Duration of the CORS preflight that went before it. */
  preflightMs: number | null;
  /** How many cells this one request answers. */
  cells: number;
};

export type MatrixBidder = {
  bidder: string;
  /** One per ad unit column; `null` where the bidder was not asked for that ad unit. */
  cells: (MatrixCell | null)[];
  requests: MatrixRequest[];
};

/** Bidders called from the page, or the bidders that one Prebid Server request fanned out to. */
export type MatrixSection =
  | { kind: "client"; bidders: MatrixBidder[] }
  | { kind: "server"; request: MatrixRequest; bidders: MatrixBidder[] };

export type BidMatrix = {
  /** The columns. Every bidder's `cells` has one entry per column, in this order. */
  adUnits: { code: string; sizes: string[] }[];
  sections: MatrixSection[];
  /** The currencies the bids are priced in. With more than one, every price says which it is in. */
  currencies: (string | null)[];
};

const rank = (bid: Bid) => BID_STATUSES.indexOf(bid.status);

export type MatrixSources = {
  rows: ReadonlyMap<string, NetworkRow>;
  /** Preflight rows by the id of the request they cleared the way for. */
  preflights: ReadonlyMap<string, NetworkRow>;
  /** Decoded answers of the Prebid Server requests that are loaded, by request id. */
  serverResponses: ReadonlyMap<string, DecodedBidResponse>;
};

/**
 * An auction as bidders × ad units. Most adapters send one request for all their ad units, and
 * Prebid Server sends one for several bidders: cells that share a request are tied through it.
 */
export function buildBidMatrix(auction: Auction, sources: MatrixSources): BidMatrix {
  const adUnits = auction.adUnits.map((unit) => ({ code: unit.code, sizes: unit.sizes }));
  const names = [...new Set(auction.adUnits.flatMap((unit) => unit.bids.map((bid) => bid.bidder)))];

  const bidders = names.map((bidder): MatrixBidder => {
    const cells = auction.adUnits.map((unit): MatrixCell | null => {
      // A bidder can answer one ad unit with several bids: the best one stands for the cell.
      const [bid] = unit.bids.filter((candidate) => candidate.bidder === bidder).sort((a, b) => rank(a) - rank(b));
      if (!bid) return null;
      const requestId = bid.exchange?.requestId ?? null;
      const response = requestId ? sources.serverResponses.get(requestId) : undefined;
      return {
        bid,
        requestId,
        verdict: bid.exchange?.verdict ?? null,
        lateByMs: bid.exchange?.lateByMs ?? null,
        networkMs: bid.exchange?.networkMs ?? null,
        nbr: (requestId ? sources.rows.get(requestId)?.ad?.nbr : null) ?? null,
        serverMs: response?.serverTimings.find((timing) => timing.bidder === bidder)?.ms ?? null,
        serverError: response?.serverErrors.find((error) => error.bidder === bidder)?.message ?? null,
      };
    });
    const ids = [...new Set(cells.flatMap((cell) => cell?.requestId ?? []))];
    return {
      bidder,
      cells,
      requests: ids.map((id) => ({
        id,
        row: sources.rows.get(id),
        preflightMs: sources.preflights.get(id)?.durationMs ?? null,
        cells: cells.filter((cell) => cell?.requestId === id).length,
      })),
    };
  });

  const best = (bidder: MatrixBidder) =>
    Math.min(...bidder.cells.map((cell) => (cell ? rank(cell.bid) : BID_STATUSES.length)));
  bidders.sort((a, b) => best(a) - best(b) || a.bidder.localeCompare(b.bidder));

  const viaServer = (bidder: MatrixBidder) => bidder.requests.find((request) => request.row?.ad?.viaServer);
  const client = bidders.filter((bidder) => !viaServer(bidder));
  const serverIds = [...new Set(bidders.flatMap((bidder) => viaServer(bidder)?.id ?? []))];
  const sections: MatrixSection[] = client.length > 0 ? [{ kind: "client", bidders: client }] : [];
  for (const id of serverIds) {
    const members = bidders.filter((bidder) => viaServer(bidder)?.id === id);
    sections.push({
      kind: "server",
      request: {
        id,
        row: sources.rows.get(id),
        preflightMs: sources.preflights.get(id)?.durationMs ?? null,
        cells: members.reduce((sum, bidder) => sum + bidder.cells.filter((cell) => cell?.requestId === id).length, 0),
      },
      bidders: members,
    });
  }
  return { adUnits, sections, currencies: currenciesOf(auction.adUnits.flatMap((unit) => unit.bids)) };
}

export type CellSummary = {
  /** What happened: a price, a reason, or a verdict. */
  primary: string;
  /** How long it took, on the network or on the server. */
  secondary: string | null;
};

/** The two things a cell says. The network's verdict wins over Prebid's status where they differ. */
export function summariseCell(cell: MatrixCell, viaServer: boolean, withCurrencyCode = false): CellSummary {
  // No request could be tied to the bid: Prebid's own record stands, and the network's side is unknown.
  if (cell.verdict === null) return { primary: prebidOutcome(cell, withCurrencyCode), secondary: UNKNOWN_REQUEST };
  const time = viaServer
    ? cell.serverMs === null
      ? null
      : `server ${formatDuration(cell.serverMs)}`
    : cell.networkMs === null
      ? null
      : formatDuration(cell.networkMs);
  if (cell.serverError) return { primary: `server ${cell.serverError}`, secondary: time };
  switch (cell.verdict) {
    case "notSent":
      return { primary: "not sent", secondary: null };
    case "failed":
      return { primary: "failed", secondary: time };
    case "neverAnswered":
      return { primary: "no answer", secondary: null };
    case "lateOnNetwork":
    case "atDeadline":
    case "inTimeButTimedOut":
      return { primary: verdictShort({ verdict: cell.verdict, lateByMs: cell.lateByMs }), secondary: time };
    default:
      break;
  }
  return { primary: prebidOutcome(cell, withCurrencyCode), secondary: time };
}

/** What stands in a cell in place of a time when the bid's request is not known. */
export const UNKNOWN_REQUEST = "request unknown";

/** What Prebid recorded for the bid: a price, a timeout, an error or no bid. */
function prebidOutcome(cell: MatrixCell, withCurrencyCode: boolean): string {
  if (cell.bid.cpm !== null) return formatCpm(cell.bid.cpm, cell.bid.currency, withCurrencyCode);
  if (cell.bid.status === "timeout") return "timeout";
  if (cell.bid.status === "error") return "error";
  return cell.nbr === null ? "no bid" : `no-bid ${cell.nbr}`;
}

/** The Request column of a bidder's row: how its cells were asked for. */
export function summariseRequests(bidder: MatrixBidder, viaServer: boolean): string {
  if (viaServer) return "shared";
  const sent = bidder.cells.filter((cell) => cell !== null);
  if (bidder.requests.length === 0) return sent.length > 0 ? "no request" : "";
  const [only] = bidder.requests;
  if (bidder.requests.length > 1 || !only) return pluralize(bidder.requests.length, "request");
  const parts = [only.cells > 1 ? `1 request for ${only.cells} ad units` : "1 request"];
  if (only.row?.outcome === "failed") parts[0] = "failed";
  if (only.row?.durationMs != null) parts.push(formatDuration(only.row.durationMs));
  // Scans recorded before the capture corrected a preflight's late announcement can hold a negative duration.
  if (only.preflightMs !== null) parts.push(`preflight +${formatDuration(Math.max(0, only.preflightMs))}`);
  return parts.join(", ");
}
