import { InboxIcon, InfoIcon, SearchXIcon } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import type { ScanDetail } from "@/bindings";
import { QueryError } from "@/shared/components/query-error";
import { pagePath } from "@/shared/domain/page-views";
import { useCopy } from "@/shared/hooks/use-copy";
import { useElementSize } from "@/shared/hooks/use-element-size";
import { isBoolean, isFiniteNumber, useStoredState } from "@/shared/hooks/use-stored-state";
import { formatBytes, formatInteger, formatPercent } from "@/shared/lib/format";
import { hasModifier } from "@/shared/lib/platform";
import { Button } from "@/shared/ui/button";
import { Empty } from "@/shared/ui/empty";
import { Skeleton } from "@/shared/ui/skeleton";
import { SplitPane } from "@/shared/ui/split-pane";
import { ToggleGroup, ToggleGroupItem } from "@/shared/ui/toggle-group";
import { BidInspector } from "./bids/BidInspector";
import { type DetailDock, DetailPanel } from "./detail/DetailPanel";
import { FilterBar } from "./FilterBar";
import { NetworkUnavailable, networkState } from "./NetworkUnavailable";
import { categoryCounts, stateCounts, summariseList } from "./query/facets";
import { buildMatchContext, compileQuery } from "./query/match";
import { parseQuery } from "./query/parse";
import { isStringList, pushRecent } from "./query/recipes";
import { buildFacets } from "./query/suggest";
import type { DetailTab, NetworkGrouping, NetworkSearch, NetworkView } from "./search";
import { OverviewStrip } from "./strip/OverviewStrip";
import { axisStart, buildAxis, inWindow, lifecycleMarks, normalizeWindow, type TimeWindow } from "./strip/strip-layout";
import { CategoryChips, ColumnsMenu, GroupMenu, LifecycleMenu, RecipesMenu, StateChips } from "./Toolbar";
import { formatSort, isColumnChoices, nextSort, parseSort, visibleColumns } from "./table/columns";
import { buildGrid, initiallyCollapsed } from "./table/grouping";
import { itemKey, RequestGrid, ROW_HEIGHT } from "./table/RequestGrid";
import { buildWaterfallScale } from "./table/waterfall-scale";
import { useNetworkRows } from "./use-network-rows";

/** Typing filters at once; the URL follows when the typing pauses. */
const URL_WRITE_DELAY_MS = 300;
/** A list at least this wide docks the details beside it; a narrower one below. */
const DOCK_RIGHT_FROM = 1000;
const COMPACT_BELOW = 900;

const isDock = (value: unknown): value is DetailDock | null =>
  value === null || value === "right" || value === "bottom";

function Footer({ summary, loading }: { summary: ReturnType<typeof summariseList>; loading: boolean }) {
  const vendors = summary.vendors.map((entry) => `${entry.vendor} ${formatInteger(entry.requests)}`).join(", ");
  return (
    <footer
      aria-live="polite"
      className="flex h-6 shrink-0 items-center gap-1.5 overflow-hidden border-t bg-sidebar px-3 text-[11px] whitespace-nowrap text-muted-foreground tabular-nums"
    >
      <span data-testid="request-count" className="text-foreground">
        {summary.shown === summary.total
          ? `${formatInteger(summary.total)} requests`
          : `${formatInteger(summary.shown)} of ${formatInteger(summary.total)} requests`}
      </span>
      {loading ? <span>· loading more…</span> : null}
      <span>
        ·{" "}
        {summary.shown === summary.total
          ? formatBytes(summary.totalBytes)
          : `${formatBytes(summary.shownBytes)} of ${formatBytes(summary.totalBytes)}`}{" "}
        transferred
      </span>
      {summary.failed > 0 ? <span>· {formatInteger(summary.failed)} failed</span> : null}
      {summary.lateBids > 0 ? (
        <span>
          · {formatInteger(summary.lateBids)} late {summary.lateBids === 1 ? "bid" : "bids"}
        </span>
      ) : null}
      {summary.cachedShare >= 0.2 ? (
        <span>· {formatPercent(summary.cachedShare)} from cache: not a cold load</span>
      ) : null}
      {vendors ? <span className="truncate">· {vendors}</span> : null}
    </footer>
  );
}

export function NetworkWorkbench({
  detail,
  search,
  onSearchChange,
}: {
  detail: ScanDetail;
  search: NetworkSearch;
  /**
   * Changes the named parts of the URL's state and leaves the rest as it is at that moment; `undefined`
   * removes a part. Every change replaces the URL: filter, selection and window are not steps to go Back through.
   */
  onSearchChange: (patch: NetworkSearch) => void;
}) {
  const scanId = detail.summary.id;
  const state = networkState(detail);
  const loaded = useNetworkRows(scanId, state === "available" || state === "listOnly");
  const multiPage = detail.pages.length > 1;
  const page = multiPage && detail.pages.some((candidate) => candidate.index === search.page) ? search.page : undefined;
  const view: NetworkView = search.view ?? "requests";

  const [measure, size] = useElementSize();
  const width = size?.width ?? 1040;
  const compact = width < COMPACT_BELOW;
  const [dockChoice, setDockChoice] = useStoredState<DetailDock | null>("adscan.network.dock", null, isDock);
  const [sideWidth, setSideWidth] = useStoredState("adscan.network.detail-width", 500, isFiniteNumber);
  const [bottomHeight, setBottomHeight] = useStoredState("adscan.network.detail-height", 300, isFiniteNumber);
  const [choices, setChoices] = useStoredState("adscan.network.columns", {}, isColumnChoices);
  const [comfortable, setComfortable] = useStoredState("adscan.network.comfortable", false, isBoolean);
  const [recents, setRecents] = useStoredState<string[]>("adscan.network.recents", [], isStringList);
  const [maximized, setMaximized] = useState(false);
  const [collapsedChoice, setCollapsedChoice] = useState<ReadonlySet<string> | null>(null);
  const [activeChoice, setActiveChoice] = useState<string | null>(null);
  const filterInput = useRef<HTMLInputElement>(null);
  const gridHost = useRef<HTMLDivElement>(null);
  const { copy } = useCopy();
  const dock: DetailDock = dockChoice ?? (width >= DOCK_RIGHT_FROM ? "right" : "bottom");

  // The filter as typed. It applies at once; the URL is written when the typing pauses.
  const [draft, setDraft] = useState(search.q ?? "");
  const writtenQuery = useRef(search.q ?? "");
  const urlTimer = useRef<ReturnType<typeof setTimeout>>(undefined);
  useEffect(() => {
    // A filter that arrives from outside (a link, Back) replaces what is typed; our own echo does not.
    if ((search.q ?? "") !== writtenQuery.current) {
      writtenQuery.current = search.q ?? "";
      setDraft(search.q ?? "");
    }
  }, [search.q]);
  useEffect(() => () => clearTimeout(urlTimer.current), []);

  const setQuery = (next: string) => {
    setDraft(next);
    clearTimeout(urlTimer.current);
    urlTimer.current = setTimeout(() => {
      writtenQuery.current = next;
      onSearchChange({ q: next === "" ? undefined : next });
      setRecents(pushRecent(recents, next));
    }, URL_WRITE_DELAY_MS);
  };

  const context = useMemo(() => buildMatchContext(detail), [detail]);
  const pageRows = useMemo(
    () => (page === undefined ? loaded.rows : loaded.rows.filter(({ row }) => row.page === page)),
    [loaded.rows, page],
  );
  const axis = useMemo(() => buildAxis(detail.pages, page, pageRows), [detail.pages, page, pageRows]);
  const timeWindow = normalizeWindow(search.from, search.to, axis);
  const windowFrom = timeWindow?.fromMs;
  const windowTo = timeWindow?.toMs;
  const windowRows = useMemo(() => {
    if (windowFrom === undefined || windowTo === undefined) return pageRows;
    const range = { fromMs: windowFrom, toMs: windowTo };
    return pageRows.filter((indexed) => inWindow(indexed, axis, range));
  }, [pageRows, axis, windowFrom, windowTo]);
  const parsed = useMemo(() => parseQuery(draft), [draft]);
  const matching = useMemo(() => pageRows.filter(compileQuery(parsed, context)), [pageRows, parsed, context]);
  const shown = useMemo(() => {
    if (windowFrom === undefined || windowTo === undefined) return matching;
    const range = { fromMs: windowFrom, toMs: windowTo };
    return matching.filter((indexed) => inWindow(indexed, axis, range));
  }, [matching, axis, windowFrom, windowTo]);
  const counts = useMemo(() => categoryCounts(windowRows, draft, context), [windowRows, draft, context]);
  const states = useMemo(() => stateCounts(windowRows, draft, context), [windowRows, draft, context]);
  const facets = useMemo(() => buildFacets(pageRows), [pageRows]);
  const marks = useMemo(
    () => lifecycleMarks(detail, detail.bidderDiagnostics?.consent.resolvedAtMs ?? null, axis),
    [detail, axis],
  );

  const grouping: NetworkGrouping = search.group ?? (multiPage && page === undefined ? "page" : "none");
  const sort = parseSort(search.sort);
  const collapsed = collapsedChoice ?? initiallyCollapsed(loaded.rows.length, detail.pages, page);
  const items = useMemo(
    () => buildGrid(shown, { grouping, sort, collapsed, pages: detail.pages }),
    [shown, grouping, sort, collapsed, detail.pages],
  );

  const selected = search.req ? loaded.byId.get(search.req) : undefined;
  const detailOpen = selected !== undefined;
  const detailBeside = detailOpen && dock === "right" && !maximized;
  const listWidth = detailBeside ? width - sideWidth : width;
  const columns = visibleColumns({ choices, listWidth, detailBeside });
  const activeKey = activeChoice ?? selected?.row.id ?? (items[0] ? itemKey(items[0]) : null);

  const scale = useMemo(
    () => buildWaterfallScale({ axis, context, marks, window: windowFrom === undefined ? null : timeWindow }),
    // biome-ignore lint/correctness/useExhaustiveDependencies: the window is its two numbers
    [axis, context, marks, windowFrom, windowTo],
  );

  const openRequest = (requestId: string | undefined, dt?: DetailTab) => {
    if (requestId) setActiveChoice(requestId);
    else setMaximized(false);
    onSearchChange(requestId ? { req: requestId, ...(dt ? { dt } : {}) } : { req: undefined, dt: undefined });
  };
  const setWindow = (next: TimeWindow | null) =>
    onSearchChange({ from: next ? Math.round(next.fromMs) : undefined, to: next ? Math.round(next.toMs) : undefined });
  const setPage = (next: number | undefined) =>
    // Another page view has its own clock: a window and a selection of this one mean nothing there.
    onSearchChange({ page: next, from: undefined, to: undefined, req: undefined, dt: undefined, auction: undefined });
  const toggleGroup = (key: string, fold?: boolean) => {
    const next = new Set(collapsed);
    if (fold ?? !next.has(key)) next.add(key);
    else next.delete(key);
    setCollapsedChoice(next);
  };

  // With the details open, the arrow keys move the selection and the details follow, as in DevTools.
  const moveActive = (key: string) => {
    setActiveChoice(key);
    if (detailOpen && !key.startsWith("group:")) onSearchChange({ req: key });
  };

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      const target = event.target as HTMLElement;
      const typing = target.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(target.tagName);
      if (
        (hasModifier(event) &&
          event.code === "KeyF" &&
          !event.altKey &&
          !event.shiftKey &&
          !target.closest(".cm-editor")) ||
        (event.key === "/" && !typing)
      ) {
        event.preventDefault();
        filterInput.current?.focus();
        filterInput.current?.select();
        return;
      }
      if (hasModifier(event) && event.altKey && event.code === "KeyB") {
        event.preventDefault();
        openRequest(detailOpen ? undefined : activeKey && !activeKey.startsWith("group:") ? activeKey : undefined);
        return;
      }
      if (event.metaKey || event.ctrlKey || event.altKey) return;
      // Dialogs, menus, lists and the editor's find bar take their own Escape. A tooltip that closes on the
      // same key marks the event as handled, so that mark cannot be the test.
      if (
        event.key === "Escape" &&
        !target.closest("[role='dialog'], [role='alertdialog'], [role='menu'], [role='listbox'], .cm-panels")
      ) {
        if (maximized) setMaximized(false);
        else if (detailOpen) {
          openRequest(undefined);
          gridHost.current?.querySelector<HTMLElement>("[role='treegrid']")?.focus();
        } else if (timeWindow) setWindow(null);
        else return;
        event.preventDefault();
        return;
      }
      if (typing || !multiPage) return;
      // `[` and `]` step through the page views; before the first and after the last comes the whole visit.
      if (event.key === "[" || event.key === "]") {
        const order = [undefined, ...detail.pages.map((candidate) => candidate.index)];
        const at = order.indexOf(page);
        const next = order[(at + (event.key === "]" ? 1 : -1) + order.length) % order.length];
        event.preventDefault();
        setPage(next);
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  });

  if (state !== "available" && state !== "listOnly") {
    return (
      <div className="flex h-full items-center justify-center">
        <NetworkUnavailable detail={detail} availability={state} />
      </div>
    );
  }

  const pageAuctions = detail.auctions.filter((auction) => page === undefined || auction.page === page);
  const summary = summariseList(shown, pageRows);
  const selection = selected
    ? {
        startMs: axisStart(selected, axis),
        endMs: selected.row.durationMs === null ? null : axisStart(selected, axis) + selected.row.durationMs,
      }
    : null;

  let list: React.ReactNode;
  if (loaded.isPending) {
    list = (
      <div aria-busy="true" className="flex flex-col gap-2 p-3">
        {Array.from({ length: 12 }, (_, index) => (
          // biome-ignore lint/suspicious/noArrayIndexKey: a fixed decorative list
          <Skeleton key={index} className="h-4" style={{ width: `${92 - ((index * 13) % 40)}%` }} />
        ))}
      </div>
    );
  } else if (loaded.error) {
    list = <QueryError title="Could not load the requests" error={loaded.error} onRetry={loaded.retry} />;
  } else if (view === "bids") {
    list = (
      <BidInspector
        scan={detail}
        auctions={pageAuctions}
        auctionId={search.auction}
        onAuctionChange={(auction) => onSearchChange({ auction })}
        rows={loaded.byId}
        selectedId={selected?.row.id}
        onOpen={(requestId) => openRequest(requestId)}
      />
    );
  } else if (pageRows.length === 0 && page !== undefined) {
    const reachedInPage = detail.pages.find((candidate) => candidate.index === page)?.reachedBy === "routeChange";
    list = (
      <Empty
        icon={InboxIcon}
        title="No requests on this page view"
        description={
          reachedInPage
            ? "It was reached by an in-page navigation and loaded no document. What the page requested afterwards can be listed under the page view that loaded it."
            : "Nothing was requested while this page was shown."
        }
      >
        <Button variant="outline" size="sm" onClick={() => setPage(undefined)}>
          Show all pages
        </Button>
      </Empty>
    );
  } else if (items.length === 0) {
    list = (
      <Empty
        icon={SearchXIcon}
        title="No request matches"
        description={
          timeWindow ? "Nothing matches this filter in the selected time window." : "Nothing matches this filter."
        }
      >
        {draft ? (
          <Button variant="outline" size="sm" onClick={() => setQuery("")}>
            Clear the filter
          </Button>
        ) : null}
        {timeWindow ? (
          <Button variant="outline" size="sm" onClick={() => setWindow(null)}>
            Show the whole timeline
          </Button>
        ) : null}
      </Empty>
    );
  } else {
    list = (
      <RequestGrid
        label="Requests"
        items={items}
        columns={columns}
        sort={sort}
        onSort={(key) => onSearchChange({ sort: formatSort(nextSort(sort, key)) })}
        activeKey={activeKey}
        onActiveChange={moveActive}
        selectedId={selected?.row.id}
        onOpen={(requestId) => openRequest(requestId)}
        onToggleGroup={toggleGroup}
        onCopyUrl={(url) => void copy(url)}
        scale={scale}
        rowHeight={comfortable ? ROW_HEIGHT.comfortable : ROW_HEIGHT.compact}
      />
    );
  }

  const panel = selected ? (
    <DetailPanel
      scan={detail}
      row={selected.row}
      tab={search.dt ?? "summary"}
      onTabChange={(dt) => onSearchChange({ dt: dt === "summary" ? undefined : dt })}
      dock={dock}
      onDockChange={setDockChoice}
      maximized={maximized}
      onMaximizedChange={setMaximized}
      onClose={() => openRequest(undefined)}
      onOpenRequest={(requestId) => openRequest(requestId)}
    />
  ) : null;

  return (
    <div ref={measure} className="flex h-full min-w-0 flex-col">
      <div className="flex shrink-0 flex-col gap-1.5 border-b px-3 py-2">
        <div className="flex items-center gap-2">
          <ToggleGroup<NetworkView>
            size="sm"
            aria-label="View"
            value={view}
            onValueChange={(next) => onSearchChange({ view: next === "requests" ? undefined : next })}
          >
            <ToggleGroupItem value="requests">Requests</ToggleGroupItem>
            <ToggleGroupItem value="bids">Bid exchanges</ToggleGroupItem>
          </ToggleGroup>
          {multiPage ? (
            <ToggleGroup
              size="sm"
              aria-label="Page view"
              value={page === undefined ? "all" : String(page)}
              onValueChange={(value) => setPage(value === "all" ? undefined : Number(value))}
              className="min-w-0 overflow-x-auto"
            >
              <ToggleGroupItem value="all">All pages</ToggleGroupItem>
              {detail.pages.map((candidate) => (
                <ToggleGroupItem
                  key={candidate.index}
                  value={String(candidate.index)}
                  title={candidate.url}
                  className="max-w-40"
                >
                  <span className="tabular-nums">{candidate.index + 1}</span>
                  {compact ? null : <span className="truncate font-mono text-[11px]">{pagePath(candidate.url)}</span>}
                </ToggleGroupItem>
              ))}
            </ToggleGroup>
          ) : null}
          <span className="flex-1" />
          {view === "requests" ? (
            <>
              <GroupMenu
                value={grouping}
                multiPage={multiPage && page === undefined}
                onChange={(group) => onSearchChange({ group })}
              />
              <ColumnsMenu
                shown={columns}
                choices={choices}
                onChoicesChange={setChoices}
                disabled={detailBeside}
                comfortable={comfortable}
                onComfortableChange={setComfortable}
              />
            </>
          ) : null}
        </div>
        {view === "requests" ? (
          <>
            <div className="flex items-center gap-2">
              <FilterBar
                inputRef={filterInput}
                value={draft}
                onChange={setQuery}
                notes={parsed.notes}
                facets={facets}
              />
              <StateChips query={draft} onQueryChange={setQuery} counts={states} />
              <LifecycleMenu scan={detail} auctions={pageAuctions} query={draft} onQueryChange={setQuery} />
              <RecipesMenu recents={recents} onPick={setQuery} />
            </div>
            <CategoryChips query={draft} onQueryChange={setQuery} counts={counts} compact={compact} />
          </>
        ) : null}
      </div>
      {state === "listOnly" ? (
        <p
          role="note"
          className="flex shrink-0 items-center gap-2 border-b bg-muted/50 px-3 py-1.5 text-xs text-muted-foreground"
        >
          <InfoIcon aria-hidden="true" className="size-3.5 shrink-0" />
          The network data of this scan was removed to stay under the storage limit. The list remains, without query
          strings, headers and bodies.
        </p>
      ) : null}
      {loaded.cut ? (
        <p
          role="note"
          className="flex shrink-0 items-center gap-2 border-b bg-amber-soft px-3 py-1.5 text-xs text-amber-fg"
        >
          <InfoIcon aria-hidden="true" className="size-3.5 shrink-0" />
          This visit made {formatInteger(loaded.total)} requests. The first {formatInteger(loaded.rows.length)} are
          shown.
        </p>
      ) : null}
      {view === "requests" && !loaded.isPending ? (
        <OverviewStrip
          rows={matching}
          axis={axis}
          marks={marks}
          window={timeWindow}
          onWindowChange={setWindow}
          selection={selection}
          compact={compact}
        />
      ) : null}
      <div ref={gridHost} className="min-h-0 flex-1">
        <SplitPane
          orientation={dock === "right" ? "horizontal" : "vertical"}
          size={dock === "right" ? sideWidth : bottomHeight}
          onSizeChange={dock === "right" ? setSideWidth : setBottomHeight}
          min={dock === "right" ? 360 : 160}
          max={dock === "right" ? 900 : 640}
          minOther={dock === "right" ? 380 : 120}
          label="Resize the request details"
          className="h-full"
          start={list}
          end={panel}
          startHidden={maximized}
        />
      </div>
      {view === "requests" ? <Footer summary={summary} loading={loaded.isLoadingMore} /> : null}
    </div>
  );
}
