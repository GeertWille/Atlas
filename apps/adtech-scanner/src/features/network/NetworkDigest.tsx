import { Link } from "@tanstack/react-router";
import { ArrowRightIcon, NetworkIcon } from "lucide-react";
import type { ReactNode } from "react";
import type { NetworkRow, ScanDetail } from "@/bindings";
import { useNetworkSummary } from "@/lib/query/hooks";
import { Panel, PanelHeader } from "@/shared/components/panel";
import { QueryError } from "@/shared/components/query-error";
import { formatBytes, formatDuration, formatInteger, pluralize } from "@/shared/lib/format";
import { Button } from "@/shared/ui/button";
import { Skeleton } from "@/shared/ui/skeleton";
import { CATEGORIES, CATEGORY_META, vendorLabel } from "./labels";
import { NetworkUnavailable, networkState } from "./NetworkUnavailable";
import type { NetworkSearch } from "./search";
import { buildAxis, lifecycleMarks } from "./strip/strip-layout";
import { AdOutcome, StatusMark } from "./table/cells";
import { splitName } from "./table/columns";

function WorkbenchLink({
  scanId,
  search,
  className,
  children,
  label,
}: {
  scanId: string;
  search: NetworkSearch;
  className?: string;
  children: ReactNode;
  label?: string;
}) {
  return (
    <Link to="/scans/$scanId/network" params={{ scanId }} search={search} aria-label={label} className={className}>
      {children}
    </Link>
  );
}

function Stat({
  scanId,
  search,
  label,
  value,
  hint,
}: {
  scanId: string;
  search: NetworkSearch;
  label: string;
  value: string;
  hint: string;
}) {
  return (
    <WorkbenchLink
      scanId={scanId}
      search={search}
      label={`${label}: ${value}. ${hint}`}
      className="flex min-w-0 flex-1 flex-col gap-0.5 rounded-lg px-3 py-2.5 hover:bg-accent"
    >
      <span className="truncate text-xs text-muted-foreground">{label}</span>
      <span className="text-lg leading-6 font-semibold tabular-nums">{value}</span>
    </WorkbenchLink>
  );
}

/** The visit on one line: a block per page view with its request count, and the auctions and renders that happened in it. */
function VisitStrip({
  detail,
  page,
  requestsByPage,
}: {
  detail: ScanDetail;
  page: number | undefined;
  requestsByPage: Map<number, number>;
}) {
  const scanId = detail.summary.id;
  const axis = buildAxis(detail.pages, undefined, []);
  const marks = lifecycleMarks(detail, detail.bidderDiagnostics?.consent.resolvedAtMs ?? null, axis);
  const percent = (ms: number) => `${Math.min(100, Math.max(0, (ms / axis.endMs) * 100))}%`;
  const blocks = detail.pages.map((view, index) => ({
    view,
    startMs: view.startedAtMs,
    endMs: detail.pages[index + 1]?.startedAtMs ?? axis.endMs,
  }));
  return (
    <div className="relative h-14 overflow-hidden rounded-lg border bg-background">
      {blocks.map(({ view, startMs, endMs }) => (
        <Link
          key={view.index}
          to="/scans/$scanId/network"
          params={{ scanId }}
          search={{ page: detail.pages.length > 1 ? view.index : undefined }}
          aria-label={`Page ${view.index + 1}: ${pluralize(requestsByPage.get(view.index) ?? 0, "request")} in ${formatDuration(endMs - startMs)}. Open it in the network view`}
          aria-current={page === view.index ? "true" : undefined}
          className="absolute inset-y-0 border-l outline-offset-[-2px] first:border-l-0 hover:bg-accent/70 aria-[current=true]:bg-primary-soft/60"
          style={{ left: percent(startMs), width: `calc(${percent(endMs)} - ${percent(startMs)})` }}
        >
          <span className="absolute top-1 left-2 flex items-baseline gap-1.5 text-[11px] whitespace-nowrap">
            <span className="font-medium">{detail.pages.length > 1 ? `Page ${view.index + 1}` : "Page"}</span>
            <span className="text-muted-foreground tabular-nums">
              {pluralize(requestsByPage.get(view.index) ?? 0, "request")}
            </span>
          </span>
        </Link>
      ))}
      <div aria-hidden="true" className="pointer-events-none absolute inset-x-0 bottom-1.5 h-4">
        {marks.auctions.map((auction) => (
          <span key={auction.id}>
            <span
              className="absolute top-[3px] h-[9px] rounded-[2px] bg-violet"
              style={{
                left: percent(auction.startMs),
                width: `max(3px, calc(${percent(auction.endMs)} - ${percent(auction.startMs)}))`,
              }}
            />
            {auction.timeoutAtMs !== null ? (
              <span className="absolute top-0 h-[15px] w-0.5 bg-amber" style={{ left: percent(auction.timeoutAtMs) }} />
            ) : null}
          </span>
        ))}
        {marks.renders.map((render, index) => (
          <span
            // biome-ignore lint/suspicious/noArrayIndexKey: renders are positions on the axis
            key={index}
            className="absolute top-[4px] size-0 -translate-x-1/2 border-x-[4px] border-t-[7px] border-x-transparent border-t-green"
            style={{ left: percent(render.atMs) }}
          />
        ))}
      </div>
    </div>
  );
}

function NotableRow({ scanId, row, showPage }: { scanId: string; row: NetworkRow; showPage: boolean }) {
  const { name, query } = splitName(row.url);
  return (
    <li>
      <WorkbenchLink
        scanId={scanId}
        search={{ req: row.id, page: showPage ? row.page : undefined }}
        className="grid grid-cols-[72px_minmax(0,1fr)_124px_128px_64px] items-center gap-x-3 px-4 py-1.5 text-xs hover:bg-accent/60"
      >
        <StatusMark row={row} />
        <span className="min-w-0 truncate">
          <span className="font-medium">{name}</span>
          <span className="text-subtle-foreground">{query}</span>
        </span>
        <span className="truncate text-muted-foreground">{vendorLabel(row)}</span>
        <AdOutcome row={row} />
        <span className="text-right text-muted-foreground tabular-nums">{formatDuration(row.durationMs)}</span>
      </WorkbenchLink>
    </li>
  );
}

/**
 * The report's Network tab: what the visit's traffic looked like and what is worth opening, with
 * every element a link into the network view filtered to it. It is a digest, not the list.
 */
export function NetworkDigest({ detail, page }: { detail: ScanDetail; page: number | undefined }) {
  const scanId = detail.summary.id;
  const state = networkState(detail);
  const available = state === "available" || state === "listOnly";
  const summary = useNetworkSummary(scanId, available);
  if (!available) return <NetworkUnavailable detail={detail} availability={state} />;
  if (summary.isError)
    return (
      <QueryError
        title="Could not load the network summary"
        error={summary.error}
        onRetry={() => void summary.refetch()}
      />
    );
  if (!summary.data) {
    return (
      <div className="flex flex-col gap-3">
        <Skeleton className="h-20 rounded-xl" />
        <Skeleton className="h-48 rounded-xl" />
      </div>
    );
  }
  const data = summary.data;
  const multiPage = detail.pages.length > 1;
  const scope: NetworkSearch = { page: multiPage ? page : undefined };
  const notable = data.notable.filter((row) => page === undefined || row.page === page);
  const requestsByPage = new Map(data.pages.map((entry) => [entry.page, entry.requests]));
  const categories = CATEGORIES.flatMap((category) => {
    const count = data.categories.find((entry) => entry.category === category);
    return count && count.requests > 0 ? [count] : [];
  });

  return (
    <div className="flex flex-col gap-4">
      <Panel className="overflow-hidden">
        <PanelHeader
          title="Network"
          description={
            state === "listOnly"
              ? "The network data of this scan was removed to stay under the storage limit. The request list remains, without query strings, headers and bodies."
              : `Every request of the visit, with headers and the text it received${data.archive.headersRedacted ? "; cookie and authorization values are replaced by their length" : ""}.`
          }
          actions={
            <Button asChild size="sm">
              <Link to="/scans/$scanId/network" params={{ scanId }} search={scope}>
                <NetworkIcon />
                Open the network view
                <ArrowRightIcon />
              </Link>
            </Button>
          }
        />
        <div className="flex divide-x border-b p-1.5 [&>a]:mx-1 [&>a:first-child]:ml-0">
          <Stat
            scanId={scanId}
            search={scope}
            label="Requests"
            value={formatInteger(data.totalRequests)}
            hint="Open them all"
          />
          <Stat
            scanId={scanId}
            search={scope}
            label="Transferred"
            value={formatBytes(data.transferredBytes)}
            hint="Open the list"
          />
          <Stat
            scanId={scanId}
            search={{ ...scope, q: "is:failed" }}
            label="Failed"
            value={formatInteger(data.failed)}
            hint="Open the failed requests"
          />
          <Stat
            scanId={scanId}
            search={{ ...scope, q: "cat:bid is:late" }}
            label="Late bids"
            value={formatInteger(data.lateBids)}
            hint="Open the bid calls that came late"
          />
          <Stat
            scanId={scanId}
            search={{ ...scope, q: "cat:bid is:pending" }}
            label="Unanswered bids"
            value={formatInteger(data.unansweredBids)}
            hint="Open the bid calls without an answer"
          />
          <Stat
            scanId={scanId}
            search={{ ...scope, q: "is:cached,sw" }}
            label="Not from the network"
            value={formatInteger(data.fromCache)}
            hint="Open what a cache or a service worker answered"
          />
        </div>
        <div className="flex flex-col gap-3 p-4">
          <VisitStrip detail={detail} page={page} requestsByPage={requestsByPage} />
          <ul aria-label="Requests by category" className="flex flex-wrap gap-1.5">
            {categories.map((entry) => (
              <li key={entry.category}>
                <WorkbenchLink
                  scanId={scanId}
                  search={{ ...scope, q: `cat:${CATEGORY_META[entry.category].token}` }}
                  className="inline-flex h-6 items-center gap-1.5 rounded-full border border-border-strong px-2.5 text-xs text-muted-foreground hover:bg-accent hover:text-foreground"
                >
                  {CATEGORY_META[entry.category].label}
                  <span className="font-medium text-foreground tabular-nums">{formatInteger(entry.requests)}</span>
                </WorkbenchLink>
              </li>
            ))}
          </ul>
          {multiPage && page !== undefined ? (
            <p className="text-xs text-muted-foreground">
              The counts are for the whole visit. The links open page {page + 1}.
            </p>
          ) : null}
        </div>
      </Panel>

      <Panel className="overflow-hidden">
        <PanelHeader
          title="Worth opening"
          description="Bid calls that failed, came late or were never answered, blocked requests and the slowest request before the first auction"
        />
        {notable.length === 0 ? (
          <p className="px-4 py-3 text-[13px] text-muted-foreground">
            Nothing stands out on {page === undefined ? "this visit" : "this page view"}.
          </p>
        ) : (
          <ul aria-label="Notable requests" className="divide-y">
            {notable.map((row) => (
              <NotableRow key={row.id} scanId={scanId} row={row} showPage={multiPage} />
            ))}
          </ul>
        )}
      </Panel>
    </div>
  );
}
