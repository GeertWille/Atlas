import { GavelIcon } from "lucide-react";
import type { Auction, BidderDiagnostics } from "@/bindings";
import { BID_STATUS_META, BID_STATUSES } from "@/shared/domain/vocabulary";
import { cn } from "@/shared/lib/cn";
import { Empty } from "@/shared/ui/empty";
import { AuctionWaterfall } from "./AuctionWaterfall";
import { BidderChecks } from "./BidderChecks";
import { BidderSummaryTable } from "./BidderSummaryTable";
import { BID_STATUS_STYLE } from "./bid-status-style";

function Legend() {
  return (
    <ul
      aria-label="Bid status legend"
      className="flex flex-wrap items-center gap-x-4 gap-y-1.5 text-xs text-muted-foreground"
    >
      {BID_STATUSES.map((status) => {
        const style = BID_STATUS_STYLE[status];
        const Icon = style.icon;
        return (
          <li key={status} className="flex items-center gap-1.5" title={BID_STATUS_META[status].description}>
            <span className={cn("h-2.5 w-4 rounded-[3px]", style.bar)} />
            <Icon aria-hidden="true" className={cn("size-3.5", style.iconTone)} />
            {BID_STATUS_META[status].label}
          </li>
        );
      })}
      <li className="flex items-center gap-1.5">
        <span className="h-3 w-0.5 bg-amber" />
        Auction timeout
      </li>
    </ul>
  );
}

export function AuctionsTab({
  auctions,
  diagnostics,
}: {
  auctions: Auction[];
  /** `null` for scans whose bidder checks did not run. */
  diagnostics: BidderDiagnostics | null;
}) {
  if (auctions.length === 0) {
    return (
      <Empty
        icon={GavelIcon}
        title="No Prebid auctions captured"
        description="Prebid never fired auctionInit during the observation window. Either the page has no header bidding, or the wrapper never started an auction."
      />
    );
  }
  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-col gap-1.5">
        <p className="text-[13px] text-muted-foreground">
          Each bar runs from bid request to response on a time axis that starts at auctionInit.
        </p>
        <Legend />
      </div>
      {auctions.map((auction, index) => (
        <AuctionWaterfall key={auction.id} auction={auction} index={index} />
      ))}
      <BidderSummaryTable auctions={auctions} />
      {diagnostics ? <BidderChecks diagnostics={diagnostics} /> : null}
    </div>
  );
}
