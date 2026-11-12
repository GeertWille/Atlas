import { ChevronRightIcon, CircleHelpIcon, CircleOffIcon, GavelIcon, ServerIcon } from "lucide-react";
import type { Auction, BidStatus, DecodedBidResponse, ExchangeVerdict, NetworkRow, ScanDetail } from "@/bindings";
import { useNetworkRequests } from "@/lib/query/hooks";
import { verdictSentence } from "@/shared/domain/exchange-verdict";
import { BID_STATUS_META } from "@/shared/domain/vocabulary";
import { useScrollEdges } from "@/shared/hooks/use-scroll-edges";
import { cn } from "@/shared/lib/cn";
import { formatDuration, pluralize } from "@/shared/lib/format";
import { Empty } from "@/shared/ui/empty";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/shared/ui/select";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import type { IndexedRow } from "../query/match";
import { OUTCOME_STYLE } from "../table/cells";
import { buildBidMatrix, type MatrixBidder, type MatrixCell, summariseCell, summariseRequests } from "./bid-matrix";

const UNKNOWN_STYLE = { icon: CircleHelpIcon, tone: "text-muted-foreground", label: "Request unknown" };

/** The icon of a cell: the network's verdict where it says more than Prebid's status does. */
function cellStyle(cell: MatrixCell) {
  const byVerdict: Partial<Record<ExchangeVerdict, keyof typeof OUTCOME_STYLE>> = {
    lateOnNetwork: "late",
    atDeadline: "atDeadline",
    inTimeButTimedOut: "timedOutInPage",
    neverAnswered: "noAnswer",
    failed: "failed",
  };
  const byStatus: Record<BidStatus, keyof typeof OUTCOME_STYLE> = {
    rendered: "bid",
    won: "bid",
    bid: "bid",
    noBid: "noBid",
    timeout: "timedOutInPage",
    error: "failed",
    pending: "noAnswer",
  };
  if (cell.verdict === "notSent") return { icon: CircleOffIcon, tone: "text-orange-fg", label: "Not sent" };
  // Without a request there is nothing the network says: a timeout or an error is Prebid's word alone, and no warning.
  const priced = byStatus[cell.bid.status] === "bid" || byStatus[cell.bid.status] === "noBid";
  if (cell.verdict === null && !priced) return UNKNOWN_STYLE;
  return OUTCOME_STYLE[(cell.verdict && byVerdict[cell.verdict]) ?? byStatus[cell.bid.status]];
}

function Cell({
  cell,
  bidder,
  unit,
  viaServer,
  row,
  selected,
  onOpen,
}: {
  cell: MatrixCell;
  bidder: string;
  unit: string;
  viaServer: boolean;
  row: NetworkRow | undefined;
  selected: boolean;
  onOpen: (requestId: string) => void;
}) {
  const style = cellStyle(cell);
  const Icon = style.icon;
  const summary = summariseCell(cell, viaServer);
  const won = cell.bid.status === "won" || cell.bid.status === "rendered";
  const sentence = cell.verdict
    ? verdictSentence({ verdict: cell.verdict, lateByMs: cell.lateByMs, networkMs: cell.networkMs, error: row?.error })
    : `${BID_STATUS_META[cell.bid.status].description}. Which request carried this bid could not be told, so what the network did with it is unknown`;
  const content = (
    <span className="flex min-w-0 flex-1 items-center gap-1.5">
      <span className="sr-only">
        {bidder} for {unit}:
      </span>
      <Icon role="img" aria-label={style.label} className={cn("size-3.5 shrink-0", style.tone)} />
      <span className={cn("truncate tabular-nums", won && "font-semibold")}>{summary.primary}</span>
      {won ? (
        <span className="shrink-0 rounded-sm bg-violet-soft px-1 text-[10px] font-medium text-violet-fg">
          {BID_STATUS_META[cell.bid.status].label}
        </span>
      ) : null}
      {summary.secondary ? (
        <span className="ml-auto shrink-0 pl-2 text-muted-foreground tabular-nums">{summary.secondary}</span>
      ) : null}
    </span>
  );
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        {cell.requestId ? (
          <button
            type="button"
            aria-pressed={selected}
            onClick={() => onOpen(cell.requestId as string)}
            className={cn(
              "flex h-7 w-full items-center rounded-md px-2 text-left outline-offset-[-2px] hover:bg-accent",
              selected && "bg-primary-soft hover:bg-primary-soft",
            )}
          >
            {content}
          </button>
        ) : (
          // biome-ignore lint/a11y/noNoninteractiveTabindex: the cell explains itself on focus, like the ones that open an exchange
          <span tabIndex={0} className="flex h-7 w-full items-center rounded-md px-2 outline-offset-[-2px]">
            {content}
          </span>
        )}
      </TooltipTrigger>
      <TooltipContent side="top" className="max-w-72">
        <p className="font-medium">
          {bidder} · {unit}
        </p>
        <p className="text-muted-foreground">{sentence}.</p>
        {cell.verdict === "notSent" ? (
          <p className="text-muted-foreground">The bidder checks on the Auctions tab say why.</p>
        ) : null}
      </TooltipContent>
    </Tooltip>
  );
}

function BidderRow({
  bidder,
  units,
  viaServer,
  rows,
  selectedId,
  onOpen,
}: {
  bidder: MatrixBidder;
  units: { code: string }[];
  viaServer: boolean;
  rows: ReadonlyMap<string, IndexedRow>;
  selectedId: string | undefined;
  onOpen: (requestId: string) => void;
}) {
  return (
    <tr className="border-b border-border/60">
      <th
        scope="row"
        className={cn(
          "sticky left-0 z-10 bg-background py-0.5 pr-3 text-left font-medium group-data-[scrolled=true]/matrix:shadow-[6px_0_6px_-6px_var(--color-border-strong)]",
          viaServer ? "pl-7" : "pl-3",
        )}
      >
        {bidder.bidder}
      </th>
      {bidder.cells.map((cell, index) => (
        // biome-ignore lint/suspicious/noArrayIndexKey: cells follow the fixed ad unit columns
        <td key={index} className="px-1 py-0.5">
          {cell ? (
            <Cell
              cell={cell}
              bidder={bidder.bidder}
              unit={units[index]?.code ?? ""}
              viaServer={viaServer}
              row={cell.requestId ? rows.get(cell.requestId)?.row : undefined}
              selected={cell.requestId !== null && cell.requestId === selectedId}
              onOpen={onOpen}
            />
          ) : null}
        </td>
      ))}
      <td className="px-3 py-0.5 whitespace-nowrap text-muted-foreground tabular-nums">
        {summariseRequests(bidder, viaServer)}
      </td>
    </tr>
  );
}

const LEGEND: (keyof typeof OUTCOME_STYLE)[] = ["bid", "noBid", "late", "timedOutInPage", "noAnswer", "failed"];

/**
 * One auction as bidders × ad units, with what the network says about every bid. Cells that one
 * request answers open the same exchange; the bidders of a Prebid Server request stand together.
 */
export function BidInspector({
  scan,
  auctions,
  auctionId,
  onAuctionChange,
  rows,
  selectedId,
  onOpen,
}: {
  scan: ScanDetail;
  /** The auctions of the page views shown. */
  auctions: Auction[];
  auctionId: string | undefined;
  onAuctionChange: (auctionId: string) => void;
  rows: ReadonlyMap<string, IndexedRow>;
  selectedId: string | undefined;
  onOpen: (requestId: string) => void;
}) {
  const auction = auctions.find((candidate) => candidate.id === auctionId) ?? auctions[0];
  const plainRows = new Map([...rows].map(([id, indexed]) => [id, indexed.row]));
  const preflights = new Map(
    [...rows.values()].flatMap(({ row }) => (row.preflightOf ? [[row.preflightOf, row] as const] : [])),
  );
  const serverIds = auction
    ? [
        ...new Set(
          auction.adUnits.flatMap((unit) =>
            unit.bids.flatMap((bid) => {
              const id = bid.exchange?.requestId;
              return id && rows.get(id)?.row.ad?.viaServer ? [id] : [];
            }),
          ),
        ),
      ]
    : [];
  const serverRequests = useNetworkRequests(scan.summary.id, serverIds);
  const [scroller, edges] = useScrollEdges<HTMLDivElement>();
  if (!auction) {
    return (
      <Empty
        icon={GavelIcon}
        title="No Prebid auction on this page view"
        description="Prebid never started an auction here, so there are no bid exchanges to lay out. The requests view still shows what the page did instead."
      />
    );
  }
  const serverResponses = new Map<string, DecodedBidResponse>();
  serverIds.forEach((id, index) => {
    const response = serverRequests[index]?.data?.decoded?.response;
    if (response) serverResponses.set(id, response);
  });
  const matrix = buildBidMatrix(auction, { rows: plainRows, preflights, serverResponses });
  const number = scan.auctions.findIndex((candidate) => candidate.id === auction.id) + 1;
  const bidderCount = matrix.sections.reduce((sum, section) => sum + section.bidders.length, 0);
  const end = auction.endedAtMs ?? auction.startedAtMs + (auction.durationMs ?? 0);

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="flex shrink-0 flex-wrap items-center gap-x-3 gap-y-1 border-b px-3 py-2">
        <Select value={auction.id} onValueChange={onAuctionChange}>
          <SelectTrigger aria-label="Auction" className="h-7 w-auto gap-2 text-[13px]">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {auctions.map((candidate) => (
              <SelectItem key={candidate.id} value={candidate.id}>
                Auction {scan.auctions.findIndex((other) => other.id === candidate.id) + 1}
                {scan.pages.length > 1 ? ` · page ${candidate.page + 1}` : ""}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        <p className="text-xs text-muted-foreground tabular-nums">
          started {formatDuration(auction.startedAtMs)} · timeout{" "}
          {auction.timeoutMs === null ? "not set" : formatDuration(auction.timeoutMs)} · ended {formatDuration(end)} ·{" "}
          {pluralize(bidderCount, "bidder")} · {pluralize(matrix.adUnits.length, "ad unit")}
        </p>
      </div>
      <div className="relative min-h-0 flex-1">
        {/* More ad units than fit: the edges say so, and the bidder column stays in place. */}
        <span
          aria-hidden="true"
          data-shown={edges.end}
          className="pointer-events-none absolute inset-y-0 right-0 z-30 flex w-10 items-center justify-end bg-linear-to-l from-background via-background/80 to-transparent pr-1 opacity-0 transition-opacity data-[shown=true]:opacity-100"
        >
          <ChevronRightIcon className="size-4 text-muted-foreground" />
        </span>
        <div ref={scroller} data-scrolled={edges.start} className="group/matrix h-full overflow-auto">
          <table
            aria-label={`Bid exchanges of auction ${number}`}
            className="w-full min-w-max border-separate border-spacing-0 text-xs"
          >
            <thead>
              <tr className="text-[11px] text-muted-foreground">
                <th
                  scope="col"
                  className="sticky top-0 left-0 z-20 border-b bg-background py-1.5 pr-3 pl-3 text-left align-bottom font-medium group-data-[scrolled=true]/matrix:shadow-[6px_0_6px_-6px_var(--color-border-strong)]"
                >
                  Bidder
                </th>
                {matrix.adUnits.map((unit) => (
                  <th
                    key={unit.code}
                    scope="col"
                    className="sticky top-0 z-10 min-w-40 border-b bg-background px-3 py-1.5 text-left font-medium"
                  >
                    <span className="selectable block font-mono text-foreground">{unit.code}</span>
                    <span className="block font-normal tabular-nums">{unit.sizes.join(", ") || "no sizes"}</span>
                  </th>
                ))}
                <th
                  scope="col"
                  className="sticky top-0 z-10 border-b bg-background px-3 py-1.5 text-left align-bottom font-medium"
                >
                  Request
                </th>
              </tr>
            </thead>
            {matrix.sections.map((section) => (
              <tbody
                key={section.kind === "server" ? section.request.id : "client"}
                className="[&_td]:border-b [&_td]:border-border/60 [&_th]:border-b [&_th]:border-border/60"
              >
                {section.kind === "server" ? (
                  <tr>
                    <th
                      scope="rowgroup"
                      colSpan={matrix.adUnits.length + 2}
                      className="bg-muted/50 px-3 py-1 text-left font-medium"
                    >
                      <span className="sticky left-3 flex w-fit items-center gap-2">
                        <ServerIcon aria-hidden="true" className="size-3.5 text-subtle-foreground" />
                        Prebid Server: 1 request for {pluralize(section.bidders.length, "bidder")}
                        {section.request.row?.durationMs != null
                          ? `, ${formatDuration(section.request.row.durationMs)}`
                          : ""}
                        <button
                          type="button"
                          onClick={() => onOpen(section.request.id)}
                          className="rounded-sm font-normal text-primary-text underline-offset-4 hover:underline"
                        >
                          Open the exchange
                        </button>
                      </span>
                    </th>
                  </tr>
                ) : null}
                {section.bidders.map((bidder) => (
                  <BidderRow
                    key={bidder.bidder}
                    bidder={bidder}
                    units={matrix.adUnits}
                    viaServer={section.kind === "server"}
                    rows={rows}
                    selectedId={selectedId}
                    onOpen={onOpen}
                  />
                ))}
              </tbody>
            ))}
          </table>
        </div>
      </div>
      <ul
        aria-label="Legend"
        className="flex shrink-0 flex-wrap items-center gap-x-4 gap-y-1 border-t bg-sidebar px-3 py-1.5 text-[11px] text-muted-foreground"
      >
        {LEGEND.map((kind) => {
          const Icon = OUTCOME_STYLE[kind].icon;
          return (
            <li key={kind} className="flex items-center gap-1">
              <Icon aria-hidden="true" className={cn("size-3.5", OUTCOME_STYLE[kind].tone)} />
              {OUTCOME_STYLE[kind].label}
            </li>
          );
        })}
        <li className="flex items-center gap-1">
          <CircleOffIcon aria-hidden="true" className="size-3.5 text-orange-fg" />
          Not sent
        </li>
        <li className="flex items-center gap-1">
          <CircleHelpIcon aria-hidden="true" className="size-3.5 text-muted-foreground" />
          Request unknown
        </li>
        <li className="ml-auto">A cell opens its exchange. Cells with the same request open the same one.</li>
      </ul>
    </div>
  );
}
