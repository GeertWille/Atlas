import { createContext, useContext } from "react";

/** A place in a script of the scan: lines of the formatted text, and the finding the link comes from. */
export type SourceTarget = { scriptId: string; line?: number; to?: number; finding?: string };

/**
 * Where the report's records lead outside the report: a script's source, a request of the network
 * record, the bid exchanges of an auction. The route supplies them; without them (and for scans that
 * have no network record) the report shows no such links.
 */
export type ReportLinks = {
  openSource: (target: SourceTarget) => void;
  /** Absent when the scan has no request list. `part`: the side of the exchange to show. */
  openRequest?: (requestId: string, part?: "response") => void;
  openExchanges?: (auctionId: string) => void;
};

const ReportLinksContext = createContext<ReportLinks | null>(null);

export const ReportLinksProvider = ReportLinksContext.Provider;

export function useReportLinks(): ReportLinks | null {
  return useContext(ReportLinksContext);
}
