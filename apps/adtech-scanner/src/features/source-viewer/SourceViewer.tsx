import { openSearchPanel } from "@codemirror/search";
import type { EditorView, ViewUpdate } from "@codemirror/view";
import { Link } from "@tanstack/react-router";
import {
  ClipboardCopyIcon,
  ExternalLinkIcon,
  InfoIcon,
  ListIcon,
  MoreHorizontalIcon,
  NetworkIcon,
  PanelRightOpenIcon,
  WrapTextIcon,
} from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import type { ScanDetail, ScriptRecord, SourceView } from "@/bindings";
import { useScriptSource } from "@/lib/query/hooks";
import { QueryError } from "@/shared/components/query-error";
import { isJevModel, modelLabel } from "@/shared/domain/model-pricing";
import { FINDING_SOURCE_LABEL, SCRIPT_CATEGORY_META, scriptCategoryLabel } from "@/shared/domain/vocabulary";
import { describeMatches, jumpToLine, type SearchSummary, searchSummaryField } from "@/shared/editor";
import { useCopy } from "@/shared/hooks/use-copy";
import { useElementSize } from "@/shared/hooks/use-element-size";
import { isBoolean, useStoredState } from "@/shared/hooks/use-stored-state";
import { cn } from "@/shared/lib/cn";
import { formatBytes, formatPercent } from "@/shared/lib/format";
import { hasModifier, MOD_KEY } from "@/shared/lib/platform";
import { openInBrowser } from "@/shared/lib/system";
import { fileNameOf } from "@/shared/lib/url";
import { Badge } from "@/shared/ui/badge";
import { Button } from "@/shared/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import { Popover, PopoverContent, PopoverTrigger } from "@/shared/ui/popover";
import { Skeleton } from "@/shared/ui/skeleton";
import { ToggleGroup, ToggleGroupItem } from "@/shared/ui/toggle-group";
import { Hint } from "@/shared/ui/tooltip";
import { adjacentMark, findingMarks, findingStep, readRanges } from "./anchors";
import { CodePane } from "./CodePane";
import { FileList, FileRail } from "./FileList";
import { type FindingActions, findingExtension, lensKey } from "./finding-extension";
import type { RulerViewport } from "./OverviewRuler";
import { QuickOpen } from "./QuickOpen";
import { SidePanel } from "./SidePanel";
import { SourceUnavailable } from "./SourceUnavailable";
import { type CaretInfo, StatusBar } from "./StatusBar";
import { rememberPlace, restorePlace } from "./scroll-memory";
import { type LineSpan, sameSpan, searchToSpan, selectionSpan, spanToSearch } from "./selection-url";

/** What the route's URL says about the viewer. */
export type SourceSearch = {
  line?: number;
  to?: number;
  view?: SourceView;
  /** The finding whose evidence the link came from. */
  finding?: string;
  /** A position in the script as captured (a request's initiator); shown at the formatted line it maps to. */
  oline?: number;
  ocol?: number;
};

const FILES_WIDTH = 240;
const FILES_WIDTH_COMPACT = 200;
const PANEL_WIDTH = 260;
const RULER_WIDTH = 14;
/** Below this the side panel floats over the code instead of taking room from it. */
const COMPACT_BELOW = 900;
/** The code never gets less than this: the file list folds to a rail first. */
const MIN_CODE_WIDTH = 520;
const URL_WRITE_DELAY_MS = 200;

/**
 * What a developer needs to place the file at a glance: what it is, whose it is and how large. How it was
 * classified, what loaded it and its fingerprint are a click away.
 */
function Facts({ script, multiPage }: { script: ScriptRecord; multiPage: boolean }) {
  const { copy } = useCopy();
  const category = SCRIPT_CATEGORY_META[script.category];
  const byJev = script.classifierModel !== null && isJevModel(script.classifierModel);
  const classifier = script.classifierModel
    ? modelLabel(script.classifierModel)
    : FINDING_SOURCE_LABEL[script.classifiedBy];
  let score: string | null = null;
  if (script.adProbability !== null) score = `${formatPercent(script.adProbability)} ad`;
  else if (script.classificationConfidence !== null) score = formatPercent(script.classificationConfidence);
  const origin = script.inline ? "inline script" : script.firstParty ? "first party" : "third party";
  const pages = multiPage && script.pages.length > 0 ? script.pages.map((page) => page + 1).join(", ") : null;
  const details: [label: string, value: React.ReactNode][] = [
    [
      "Address",
      <span key="url" className="font-mono break-all">
        {script.url}
      </span>,
    ],
    ["Origin", script.vendor ? `${script.vendor}, ${origin}` : origin],
    ["Size", `${formatBytes(script.sizeBytes)} as transferred`],
    ...(pages ? [["Page views", pages] as [string, string]] : []),
    ["Classified by", `${byJev ? "Jev" : classifier}${score ? `, ${score}` : ""}`],
    ...(script.classificationReason ? [["Reason", script.classificationReason] as [string, string]] : []),
    ...(script.initiator
      ? [
          [
            "Loaded by",
            <span key="initiator" className="font-mono break-all">
              {script.initiator}
            </span>,
          ] as [string, React.ReactNode],
        ]
      : []),
  ];
  return (
    <div className="flex min-w-0 flex-1 items-center gap-2 text-xs text-muted-foreground">
      <Badge tone={category.tone}>{scriptCategoryLabel(script.category, script.adRelated)}</Badge>
      {script.analyzed ? <Badge tone="violet">Analysed by the AI</Badge> : null}
      <span className="selectable min-w-0 truncate tabular-nums">
        {[script.vendor ?? origin, formatBytes(script.sizeBytes)].join(" · ")}
      </span>
      <Popover>
        <Hint label="About this file">
          <PopoverTrigger asChild>
            <Button variant="ghost" size="icon-sm" aria-label="About this file" className="shrink-0">
              <InfoIcon />
            </Button>
          </PopoverTrigger>
        </Hint>
        <PopoverContent aria-label="About this file" className="w-96 max-w-[calc(100vw-2rem)]">
          <dl className="flex flex-col gap-1.5 text-xs">
            {details.map(([label, value]) => (
              <div key={label} className="flex gap-3">
                <dt className="w-24 shrink-0 text-muted-foreground">{label}</dt>
                <dd className="selectable min-w-0 flex-1 break-words">{value}</dd>
              </div>
            ))}
            {script.sha256 ? (
              <div className="flex items-start gap-3">
                <dt className="w-24 shrink-0 text-muted-foreground">SHA-256</dt>
                <dd className="flex min-w-0 flex-1 items-start gap-1">
                  <span className="selectable min-w-0 flex-1 font-mono break-all">{script.sha256}</span>
                  <Button
                    variant="ghost"
                    size="icon-sm"
                    aria-label="Copy the SHA-256"
                    className="-my-1 shrink-0"
                    onClick={() => void copy(script.sha256 ?? "", "SHA-256 copied")}
                  >
                    <ClipboardCopyIcon />
                  </Button>
                </dd>
              </div>
            ) : null}
          </dl>
        </PopoverContent>
      </Popover>
    </div>
  );
}

function CodeSkeleton() {
  const widths = [38, 62, 55, 24, 70, 48, 66, 30, 58, 44, 72, 36, 52, 64, 28, 46];
  return (
    <div
      role="status"
      aria-label="Loading the source"
      className="flex min-h-0 flex-1 flex-col gap-[7px] bg-code px-4 py-3.5"
    >
      {widths.map((width, index) => (
        // biome-ignore lint/suspicious/noArrayIndexKey: a fixed decorative list
        <Skeleton key={index} className="h-3 rounded-sm" style={{ width: `${width}%` }} />
      ))}
    </div>
  );
}

export function SourceViewer({
  detail,
  script,
  search,
  onSearchChange,
  onOpenScript,
  onOpenFinding,
}: {
  detail: ScanDetail;
  script: ScriptRecord;
  search: SourceSearch;
  /**
   * Changes the named parts of the URL's state and leaves the rest as it is at that moment; `undefined`
   * removes a part. Selection and view changes replace the URL; they never add history entries.
   */
  onSearchChange: (patch: SourceSearch) => void;
  onOpenScript: (scriptId: string) => void;
  onOpenFinding: (findingId: string) => void;
}) {
  const scanId = detail.summary.id;
  const viewKind: SourceView = search.view ?? "formatted";
  const original = search.oline === undefined ? undefined : { line: search.oline, column: search.ocol };
  const source = useScriptSource(scanId, script.id, viewKind, viewKind === "formatted" ? original : undefined);
  const data = source.data;
  const text = data?.availability === "available" ? data.text : null;
  const lineCount = data?.lineCount ?? 0;

  const [measure, size] = useElementSize();
  const width = size?.width ?? 1040;
  const compact = width < COMPACT_BELOW;
  const [filesOpen, setFilesOpen] = useStoredState("adscan.viewer.files", true, isBoolean);
  const [panelPinned, setPanelPinned] = useStoredState("adscan.viewer.panel", true, isBoolean);
  const [drawerOpen, setDrawerOpen] = useState(false);
  const [wrapChoice, setWrapChoice] = useStoredState("adscan.viewer.wrap", false, isBoolean);
  const [dimUnread, setDimUnread] = useState(false);
  const [openLenses, setOpenLenses] = useState<ReadonlySet<string>>(() => new Set());
  const [quickOpen, setQuickOpen] = useState(false);
  const [view, setView] = useState<EditorView | null>(null);
  const [guarded, setGuarded] = useState(false);
  const [caret, setCaret] = useState<CaretInfo | null>(null);
  const [summary, setSummary] = useState<SearchSummary | null>(null);
  const [viewport, setViewport] = useState<RulerViewport | null>(null);
  const { copy } = useCopy();

  const panelOpen = compact ? drawerOpen : panelPinned;
  const filesWidth = compact ? FILES_WIDTH_COMPACT : FILES_WIDTH;
  const codeWidthWithFiles = width - filesWidth - RULER_WIDTH - (panelOpen && !compact ? PANEL_WIDTH : 0);
  const filesShown = filesOpen && codeWidthWithFiles >= MIN_CODE_WIDTH;
  // The original is one long line more often than not; it always wraps.
  const wrap = viewKind === "original" || wrapChoice;

  const marks = useMemo(
    () => (data?.view === "formatted" ? findingMarks(data.anchors, detail.findings, data.lineCount) : []),
    [data, detail.findings],
  );
  const regions = data?.regions ?? [];
  const read = useMemo(() => readRanges(data?.regions ?? [], lineCount), [data, lineCount]);

  const actions: FindingActions = useMemo(
    () => ({
      toggleLens: (key) =>
        setOpenLenses((current) => {
          const next = new Set(current);
          if (!next.delete(key)) next.add(key);
          return next;
        }),
      showLens: (key) => setOpenLenses((current) => new Set(current).add(key)),
      openFinding: onOpenFinding,
    }),
    [onOpenFinding],
  );
  const togglePanel = () => (compact ? setDrawerOpen(!drawerOpen) : setPanelPinned(!panelPinned));

  /** The span this screen last put in the URL: seeing it come back is an echo, not a request to move. */
  const written = useRef<LineSpan | null>(null);
  /** The line a jump left the caret on. A caret that is still there is not a selection the user made. */
  const settled = useRef<number | null>(null);
  const positioned = useRef<EditorView | null>(null);
  const urlTimer = useRef<ReturnType<typeof setTimeout>>(undefined);
  const placeKey = `${scanId}|${script.id}|${viewKind}`;

  // The URL's span as it is when a delayed write runs, not as it was when the caret moved.
  const latestSpan = useRef<LineSpan | null>(null);
  useEffect(() => {
    latestSpan.current = searchToSpan(search, lineCount);
  }, [search, lineCount]);

  const writeSpan = (span: LineSpan) => {
    clearTimeout(urlTimer.current);
    urlTimer.current = setTimeout(() => {
      if (sameSpan(span, latestSpan.current)) return;
      written.current = span;
      onSearchChange({ ...spanToSearch(span), finding: undefined, oline: undefined, ocol: undefined });
    }, URL_WRITE_DELAY_MS);
  };

  /** Goes to a place in the file and names it in the URL, so a reload comes back to it. */
  const goTo = (line: number, endLine = line, lens?: string) => {
    if (!view) return;
    settled.current = line;
    jumpToLine(view, line, endLine);
    writeSpan({ from: line, to: endLine });
    if (lens) actions.showLens(lens);
  };

  const goToAdjacentFinding = (direction: 1 | -1) => {
    if (!view) return;
    const line = view.state.doc.lineAt(view.state.selection.main.head).number;
    const mark = adjacentMark(marks, line, direction);
    if (mark) goTo(mark.startLine, mark.endLine, lensKey(mark));
  };

  const extensions = useMemo(
    () => findingExtension({ marks, openLenses, read, dimUnread, actions }),
    [marks, openLenses, read, dimUnread, actions],
  );

  // Where the URL wants the caret: a line range, else the finding the link came from, else the
  // formatted line of the captured position it named.
  const linkedMark = search.finding ? marks.find((mark) => mark.findingId === search.finding) : undefined;
  const searchSpan = searchToSpan(search, lineCount);
  let wanted: LineSpan | null = searchSpan;
  if (!wanted && linkedMark) wanted = { from: linkedMark.startLine, to: linkedMark.endLine };
  if (!wanted && data?.focusLine) wanted = { from: data.focusLine, to: data.focusLine };
  const wantedFrom = wanted?.from;
  const wantedTo = wanted?.to;
  const linkedLens = linkedMark ? lensKey(linkedMark) : undefined;

  useEffect(() => {
    if (!view) return;
    const fresh = positioned.current !== view;
    if (fresh) {
      positioned.current = view;
      written.current = null;
    }
    if (wantedFrom === undefined || wantedTo === undefined) {
      if (fresh) restorePlace(placeKey, view);
      return;
    }
    const span = { from: wantedFrom, to: wantedTo };
    if (sameSpan(span, written.current)) return;
    written.current = span;
    settled.current = span.from;
    jumpToLine(view, span.from, span.to);
    if (linkedLens) actions.showLens(linkedLens);
  }, [view, wantedFrom, wantedTo, linkedLens, placeKey, actions]);

  // The ruler's viewport box follows the scroll position, and the place is remembered for a return.
  useEffect(() => {
    if (!view) return;
    const scroller = view.scrollDOM;
    let frame = 0;
    const measureViewport = () => {
      frame = 0;
      const { scrollTop, scrollHeight, clientHeight } = scroller;
      setViewport(scrollHeight > 0 ? { top: scrollTop / scrollHeight, height: clientHeight / scrollHeight } : null);
      rememberPlace(placeKey, view);
    };
    const onScroll = () => {
      if (frame === 0) frame = requestAnimationFrame(measureViewport);
    };
    measureViewport();
    scroller.addEventListener("scroll", onScroll, { passive: true });
    return () => {
      scroller.removeEventListener("scroll", onScroll);
      cancelAnimationFrame(frame);
    };
  }, [view, placeKey]);

  useEffect(() => () => clearTimeout(urlTimer.current), []);

  const onUpdate = (update: ViewUpdate) => {
    const { state } = update;
    const nextSummary = state.field(searchSummaryField);
    if (nextSummary !== update.startState.field(searchSummaryField) || update.selectionSet) setSummary(nextSummary);
    if (!update.selectionSet) return;
    const { main } = state.selection;
    const fromLine = state.doc.lineAt(main.from);
    const toLine = state.doc.lineAt(main.to);
    const headLine = state.doc.lineAt(main.head);
    const span = selectionSpan(fromLine.number, toLine.number, main.to - toLine.from + 1);
    setCaret({
      line: headLine.number,
      column: main.head - headLine.from + 1,
      selectedLines: main.empty ? 0 : span.to - span.from + 1,
    });
    rememberPlace(placeKey, update.view);
    if (main.empty && headLine.number === settled.current) return;
    settled.current = null;
    writeSpan(span);
  };

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      const step = findingStep(event);
      // In a text field (find, go to line) Alt with an arrow belongs to the field.
      if (step !== null && (event.key === "F8" || !(event.target instanceof HTMLInputElement))) {
        event.preventDefault();
        goToAdjacentFinding(step);
        return;
      }
      if (!hasModifier(event) || event.shiftKey) return;
      // `code`, not `key`: with Option held, macOS reports the character the key types.
      if (event.code === "KeyB") {
        event.preventDefault();
        if (event.altKey) togglePanel();
        else setFilesOpen(!filesShown);
      } else if (event.code === "KeyP" && !event.altKey) {
        event.preventDefault();
        setQuickOpen(true);
      } else if (event.code === "KeyF" && !event.altKey && view && !view.dom.contains(event.target as Node)) {
        // Find works from anywhere on the screen, not only with the caret in the code.
        event.preventDefault();
        openSearchPanel(view);
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  });

  const hitLines = useMemo(() => {
    if (!view || !summary?.active) return [];
    const lines: number[] = [];
    for (const start of summary.starts) {
      if (start > view.state.doc.length) break;
      const line = view.state.doc.lineAt(start).number;
      if (lines[lines.length - 1] !== line) lines.push(line);
    }
    return lines;
  }, [view, summary]);
  // `active` means the find bar is open with a query that can run.
  const searchText = view && summary?.active ? describeMatches(summary, view.state.selection.main.from) : null;

  const setViewKind = (next: SourceView) =>
    // Line numbers of one view mean nothing in the other.
    onSearchChange({
      view: next === "formatted" ? undefined : next,
      line: undefined,
      to: undefined,
      finding: undefined,
      oline: undefined,
      ocol: undefined,
    });

  const multiPage = detail.pages.length > 1;
  const networkAvailable = detail.network?.availability === "available" || detail.network?.availability === "evicted";

  return (
    <div ref={measure} className="flex h-full min-w-0 flex-col">
      <div className="flex h-10 shrink-0 items-center gap-3 border-b px-3">
        <Facts script={script} multiPage={multiPage} />
        <div className="flex shrink-0 items-center gap-1.5">
          <ToggleGroup<SourceView> size="sm" aria-label="View" value={viewKind} onValueChange={setViewKind}>
            <ToggleGroupItem value="formatted">Formatted</ToggleGroupItem>
            <ToggleGroupItem value="original">Original</ToggleGroupItem>
          </ToggleGroup>
          <Hint label={viewKind === "original" ? "The original view always wraps" : "Wrap long lines"}>
            <Button
              variant="ghost"
              size="icon-sm"
              aria-label="Wrap long lines"
              aria-pressed={wrap}
              disabled={viewKind === "original"}
              className={cn(wrap && "bg-accent text-foreground")}
              onClick={() => setWrapChoice(!wrapChoice)}
            >
              <WrapTextIcon />
            </Button>
          </Hint>
          {panelOpen ? null : (
            <Hint label={`Show findings and calls in this file (${MOD_KEY}⌥B)`}>
              <Button
                variant="ghost"
                size="sm"
                aria-label="Show the file panel"
                className="gap-1 px-1.5"
                onClick={togglePanel}
              >
                <PanelRightOpenIcon />
                {marks.length > 0 ? (
                  <span className="rounded-full bg-primary-soft px-1.5 text-[11px] leading-4 font-semibold text-primary-text tabular-nums">
                    {marks.length}
                  </span>
                ) : null}
              </Button>
            </Hint>
          )}
          <DropdownMenu>
            <Hint label="More">
              <DropdownMenuTrigger asChild>
                <Button variant="ghost" size="icon-sm" aria-label="More actions for this script">
                  <MoreHorizontalIcon />
                </Button>
              </DropdownMenuTrigger>
            </Hint>
            <DropdownMenuContent align="end">
              <DropdownMenuGroup>
                <DropdownMenuItem onSelect={() => void copy(script.url, "URL copied")}>
                  <ClipboardCopyIcon />
                  Copy URL
                </DropdownMenuItem>
                <DropdownMenuItem disabled={!text} onSelect={() => text && void copy(text, "Source copied")}>
                  <ClipboardCopyIcon />
                  Copy all
                </DropdownMenuItem>
                {script.inline ? null : (
                  <DropdownMenuItem onSelect={() => openInBrowser(script.url)}>
                    <ExternalLinkIcon />
                    Open URL in browser
                  </DropdownMenuItem>
                )}
              </DropdownMenuGroup>
              <DropdownMenuSeparator />
              <DropdownMenuGroup>
                <DropdownMenuItem asChild>
                  <Link to="/scans/$scanId" params={{ scanId }} search={{ tab: "scripts", script: script.url }}>
                    <ListIcon />
                    Show in Scripts tab
                  </Link>
                </DropdownMenuItem>
                {script.requestId && networkAvailable ? (
                  <DropdownMenuItem asChild>
                    <Link to="/scans/$scanId/network" params={{ scanId }} search={{ req: script.requestId }}>
                      <NetworkIcon />
                      View its request
                    </Link>
                  </DropdownMenuItem>
                ) : null}
              </DropdownMenuGroup>
            </DropdownMenuContent>
          </DropdownMenu>
        </div>
      </div>

      <div className="relative flex min-h-0 flex-1">
        {filesShown ? (
          <FileList
            scanId={scanId}
            scripts={detail.scripts}
            findings={detail.findings}
            currentScriptId={script.id}
            width={filesWidth}
            onCollapse={() => setFilesOpen(false)}
          />
        ) : (
          <FileRail onExpand={() => setFilesOpen(true)} onQuickOpen={() => setQuickOpen(true)} />
        )}

        <div className="flex min-h-0 min-w-0 flex-1 flex-col">
          {viewKind === "original" && text !== null ? (
            <p className="flex shrink-0 items-center gap-2 border-b bg-muted/50 px-3 py-1.5 text-xs text-muted-foreground">
              <InfoIcon aria-hidden="true" className="size-3.5 shrink-0" />
              The script as it was captured. Findings cite lines of the formatted view and are not marked here.
            </p>
          ) : null}
          {source.isPending ? (
            <CodeSkeleton />
          ) : source.isError ? (
            <QueryError title="Could not load this script" error={source.error} onRetry={() => void source.refetch()} />
          ) : text === null || text === undefined ? (
            <SourceUnavailable
              detail={detail}
              script={script}
              availability={data && data.availability !== "available" ? data.availability : "notRetained"}
            />
          ) : (
            <CodePane
              text={text}
              label={`Source of ${fileNameOf(script.url)}`}
              wrap={wrap}
              extensions={extensions}
              marks={marks}
              regions={regions}
              hitLines={hitLines}
              lineCount={lineCount}
              viewport={viewport}
              onView={setView}
              onUpdate={onUpdate}
              onGuarded={setGuarded}
              onJump={goTo}
            />
          )}
        </div>

        {panelOpen && data && text != null ? (
          <SidePanel
            script={script}
            source={data}
            marks={marks}
            aiUsed={detail.summary.aiUsed}
            dimUnread={dimUnread}
            onDimUnreadChange={setDimUnread}
            onJump={goTo}
            onClose={togglePanel}
            className={cn(compact && "absolute inset-y-0 right-0 z-20 shadow-pop")}
          />
        ) : null}
      </div>

      <StatusBar
        source={data}
        guarded={guarded && text != null}
        caret={text != null ? caret : null}
        findingCount={marks.length}
        searchText={searchText}
      />
      <QuickOpen
        open={quickOpen}
        onOpenChange={setQuickOpen}
        scripts={detail.scripts}
        findings={detail.findings}
        onSelect={onOpenScript}
      />
    </div>
  );
}
