import { GavelIcon } from "lucide-react";
import type { Auction, BidderDiagnostics, PageView } from "@/bindings";
import { BID_STATUS_META, BID_STATUSES } from "@/shared/domain/vocabulary";
import { cn } from "@/shared/lib/cn";
import { pluralize } from "@/shared/lib/format";
import { Empty } from "@/shared/ui/empty";
import { PageSectionHeader } from "../pages/PageSectionHeader";
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

/** The auctions of one page view under its heading; a page that never auctioned says so. */
function PageAuctions({ page, auctions }: { page: PageView; auctions: { auction: Auction; index: number }[] }) {
  return (
    <section aria-label={`Auctions on page ${page.index + 1}`} className="flex flex-col gap-3">
      <PageSectionHeader page={page} summary={pluralize(auctions.length, "auction")} />
      {auctions.length === 0 ? (
        <p className="rounded-xl border border-dashed px-4 py-3 text-[13px] text-muted-foreground">
          No Prebid auction started on this page view.
        </p>
      ) : (
        auctions.map(({ auction, index }) => <AuctionWaterfall key={auction.id} auction={auction} index={index} />)
      )}
    </section>
  );
}

export function AuctionsTab({
  auctions,
  diagnostics,
  pages = [],
}: {
  auctions: Auction[];
  /** The page views to group the auctions under, when a multi-page visit is shown whole. */
  pages?: PageView[];
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
      {pages.length > 0
        ? pages.map((page) => (
            <PageAuctions
              key={page.index}
              page={page}
              // Auctions keep one numbering across the visit.
              auctions={auctions.flatMap((auction, index) => (auction.page === page.index ? [{ auction, index }] : []))}
            />
          ))
        : auctions.map((auction, index) => <AuctionWaterfall key={auction.id} auction={auction} index={index} />)}
      <BidderSummaryTable auctions={auctions} />
      {diagnostics ? <BidderChecks diagnostics={diagnostics} /> : null}
    </div>
  );
}
