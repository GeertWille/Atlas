import type { Auction } from "@/bindings";
import { Panel } from "@/shared/components/panel";
import { BID_STATUS_META } from "@/shared/domain/vocabulary";
import { cn } from "@/shared/lib/cn";
import { formatCpm, formatDuration, formatOffset, pluralize } from "@/shared/lib/format";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import { BID_STATUS_STYLE } from "./bid-status-style";
import { layoutAuction, type WaterfallBar, type WaterfallGroup } from "./waterfall-layout";

/** Label column and value column widths; the plot fills the space between them. */
const GRID = "grid grid-cols-[168px_minmax(0,1fr)_76px]";
const PLOT_INSET = "left-[168px] right-[76px]";

function BidTooltip({ bar, auction }: { bar: WaterfallBar; auction: Auction }) {
  const { bid } = bar;
  const rows: [string, string][] = [
    ["Status", BID_STATUS_META[bid.status].label],
    ["Request sent", `+${formatOffset(bar.offsetMs)} (page ${formatOffset(bid.requestedAtMs)})`],
    [
      "Response time",
      bid.responseTimeMs === null
        ? `none within ${formatDuration(auction.timeoutMs)}`
        : formatDuration(bid.responseTimeMs),
    ],
  ];
  if (bid.cpm !== null) rows.push(["CPM", formatCpm(bid.cpm, bid.currency)]);
  if (bid.size) rows.push(["Size", bid.size]);
  if (bid.dealId) rows.push(["Deal", bid.dealId]);
  return (
    <div className="flex flex-col gap-1.5">
      <p className="font-medium">{bid.bidder}</p>
      <dl className="grid grid-cols-[auto_auto] gap-x-4 gap-y-0.5 tabular-nums">
        {rows.map(([label, value]) => (
          <div key={label} className="contents">
            <dt className="text-muted-foreground">{label}</dt>
            <dd className="text-right">{value}</dd>
          </div>
        ))}
      </dl>
      <p className="text-muted-foreground">{BID_STATUS_META[bid.status].description}</p>
    </div>
  );
}

function BidRow({ bar, auction }: { bar: WaterfallBar; auction: Auction }) {
  const { bid } = bar;
  const style = BID_STATUS_STYLE[bid.status];
  const Icon = style.icon;
  const meta = BID_STATUS_META[bid.status];
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <li
          // biome-ignore lint/a11y/noNoninteractiveTabindex: rows expose exact values on focus
          tabIndex={0}
          aria-label={`${bid.bidder}: ${meta.label}${bid.cpm === null ? "" : `, ${formatCpm(bid.cpm, bid.currency)}`}${bid.responseTimeMs === null ? "" : `, ${formatDuration(bid.responseTimeMs)}`}`}
          className={cn(
            GRID,
            "h-7 items-center rounded-md outline-offset-[-2px] hover:bg-accent/70",
            bar.isWinner && "bg-primary-soft/50",
          )}
        >
          <span className="flex min-w-0 items-center gap-2 pr-3 pl-3 text-[13px]">
            <Icon aria-hidden="true" className={cn("size-3.5 shrink-0", style.iconTone)} />
            <span className={cn("truncate", bar.isWinner ? "font-semibold" : "text-muted-foreground")}>
              {bid.bidder}
            </span>
          </span>
          <span className="relative h-full">
            <span
              className={cn(
                "absolute top-1/2 h-3.5 -translate-y-1/2 rounded-[4px]",
                style.bar,
                bar.openEnded && "rounded-r-none",
              )}
              style={{ left: `${bar.startPercent}%`, width: `${bar.widthPercent}%` }}
            />
          </span>
          <span
            className={cn(
              "pr-3 text-right text-xs tabular-nums",
              bid.cpm === null ? "text-subtle-foreground" : "font-medium",
            )}
          >
            {bid.cpm === null ? meta.label : formatCpm(bid.cpm, bid.currency)}
          </span>
        </li>
      </TooltipTrigger>
      <TooltipContent side="top" align="center">
        <BidTooltip bar={bar} auction={auction} />
      </TooltipContent>
    </Tooltip>
  );
}

function AdUnitGroup({ group, auction }: { group: WaterfallGroup; auction: Auction }) {
  return (
    <div className="flex flex-col gap-px">
      <div className={cn(GRID, "h-8 items-end pb-1")}>
        <span className="selectable col-span-2 flex min-w-0 items-baseline gap-2 pl-3">
          <span className="truncate font-mono text-xs font-medium">{group.code}</span>
          <span className="shrink-0 text-xs text-subtle-foreground">{group.sizes.join(", ")}</span>
        </span>
        <span className="pr-3 text-right text-[11px] text-subtle-foreground">
          {group.winningBidder ? "CPM" : "no winner"}
        </span>
      </div>
      <ul aria-label={`Bids for ${group.code}`} className="flex flex-col gap-px">
        {group.bars.map((bar) => (
          <BidRow key={bar.bid.bidder} bar={bar} auction={auction} />
        ))}
      </ul>
    </div>
  );
}

export function AuctionWaterfall({ auction, index }: { auction: Auction; index: number }) {
  const layout = layoutAuction(auction);
  const bidCount = auction.adUnits.reduce((sum, unit) => sum + unit.bids.length, 0);
  const hitTimeout = auction.timeoutMs !== null && (auction.durationMs ?? 0) >= auction.timeoutMs;

  return (
    <Panel aria-label={`Auction ${index + 1}`} className="overflow-hidden">
      <header className="flex flex-wrap items-baseline justify-between gap-x-4 gap-y-1 border-b px-4 py-3">
        <h3 className="flex items-baseline gap-2 text-sm font-semibold">
          Auction {index + 1}
          <span className="selectable font-mono text-xs font-normal text-subtle-foreground">{auction.id}</span>
        </h3>
        <p className="text-xs text-muted-foreground tabular-nums">
          started at {formatOffset(auction.startedAtMs)} · ran {formatDuration(auction.durationMs)}
          {hitTimeout ? " (hit timeout)" : ""} · {pluralize(auction.adUnits.length, "ad unit")} ·{" "}
          {pluralize(bidCount, "bid request")}
        </p>
      </header>

      <div className="relative px-1 pt-7 pb-2">
        {/* Shared axis: tick labels, gridlines and the timeout marker span every row. */}
        <div aria-hidden="true" className={cn("pointer-events-none absolute inset-y-0", PLOT_INSET)}>
          {layout.scale.ticks.map((tick) => (
            <div key={tick} className="absolute inset-y-0" style={{ left: `${layout.scale.toPercent(tick)}%` }}>
              <span className="absolute top-2 -translate-x-1/2 text-[11px] whitespace-nowrap text-subtle-foreground tabular-nums">
                {tick === 0 ? "0" : formatDuration(tick)}
              </span>
              <span className="absolute top-7 bottom-0 w-px bg-border" />
            </div>
          ))}
          {layout.timeoutPercent !== null ? (
            <div className="absolute inset-y-0" style={{ left: `${layout.timeoutPercent}%` }}>
              <span className="absolute top-7 bottom-0 w-0.5 -translate-x-1/2 bg-amber" />
              <span className="absolute bottom-1.5 left-1.5 rounded-sm bg-card px-1 text-[11px] font-medium whitespace-nowrap text-amber-fg">
                timeout {formatDuration(auction.timeoutMs)}
              </span>
            </div>
          ) : null}
        </div>
        <div className="relative flex flex-col gap-1 pb-6">
          {layout.groups.map((group) => (
            <AdUnitGroup key={group.code} group={group} auction={auction} />
          ))}
        </div>
      </div>
    </Panel>
  );
}
