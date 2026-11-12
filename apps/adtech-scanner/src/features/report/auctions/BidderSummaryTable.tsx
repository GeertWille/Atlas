import type { Auction } from "@/bindings";
import { Panel, PanelHeader } from "@/shared/components/panel";
import { EMPTY, formatCpm, formatDuration, formatPercent } from "@/shared/lib/format";
import { summarizeBidders } from "./bidder-summary";

const NUMERIC = "px-3 py-2 text-right tabular-nums";

export function BidderSummaryTable({ auctions }: { auctions: Auction[] }) {
  const rows = summarizeBidders(auctions);
  return (
    <Panel className="overflow-hidden">
      <PanelHeader title="By bidder" description="Totals across every auction in this scan" />
      <table className="selectable w-full text-[13px]">
        <thead>
          <tr className="border-b text-xs text-muted-foreground">
            <th scope="col" className="px-4 py-2 text-left font-medium">
              Bidder
            </th>
            <th scope="col" className={`${NUMERIC} font-medium`}>
              Requests
            </th>
            <th scope="col" className={`${NUMERIC} font-medium`}>
              Bids
            </th>
            <th scope="col" className={`${NUMERIC} font-medium`}>
              Wins
            </th>
            <th scope="col" className={`${NUMERIC} font-medium`}>
              Timeouts
            </th>
            <th scope="col" className={`${NUMERIC} font-medium`}>
              Avg latency
            </th>
            <th scope="col" className={`${NUMERIC} font-medium`}>
              Avg CPM
            </th>
            <th scope="col" className={`${NUMERIC} pr-4 font-medium`}>
              Win rate
            </th>
          </tr>
        </thead>
        <tbody>
          {rows.map((row) => (
            <tr key={row.bidder} className="border-b last:border-b-0">
              <th scope="row" className="px-4 py-2 text-left font-medium">
                {row.bidder}
              </th>
              <td className={NUMERIC}>{row.requests}</td>
              <td className={NUMERIC}>{row.bids}</td>
              <td className={NUMERIC}>{row.wins}</td>
              <td className={NUMERIC}>
                {row.timeouts > 0 ? (
                  <span className="font-medium text-amber-fg">{row.timeouts}</span>
                ) : (
                  <span className="text-subtle-foreground">0</span>
                )}
              </td>
              <td className={NUMERIC}>{row.avgLatencyMs === null ? EMPTY : formatDuration(row.avgLatencyMs)}</td>
              <td className={NUMERIC}>
                {row.mixedCurrencies ? (
                  <span title="This bidder's bids are in more than one currency; they are not averaged">mixed</span>
                ) : (
                  formatCpm(row.avgCpm, row.currency)
                )}
              </td>
              <td className={`${NUMERIC} pr-4`}>{formatPercent(row.winRate)}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </Panel>
  );
}
