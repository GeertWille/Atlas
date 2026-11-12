import { Link } from "@tanstack/react-router";
import { CornerDownRightIcon } from "lucide-react";
import { type ReactNode, useState } from "react";
import type { Header, NetworkRequestDetail, NetworkRow, ScanDetail } from "@/bindings";
import { SearchInput } from "@/shared/components/search-input";
import { EXCHANGE_VERDICT_LABEL, verdictSentence } from "@/shared/domain/exchange-verdict";
import { pageLabel } from "@/shared/domain/page-views";
import { BID_STATUS_META } from "@/shared/domain/vocabulary";
import { cn } from "@/shared/lib/cn";
import { EMPTY, formatBytes, formatCpm, formatDuration } from "@/shared/lib/format";
import { fileNameOf } from "@/shared/lib/url";
import { Badge } from "@/shared/ui/badge";
import { ToggleGroup, ToggleGroupItem } from "@/shared/ui/toggle-group";
import { rowOutcome } from "../ad-outcome";
import { CATEGORY_META, OUTCOME_LABEL, SERVED_FROM_LABEL } from "../labels";
import { AdOutcome, StatusMark, vendorLabel } from "../table/cells";
import { BodyView } from "./BodyView";
import { DecodedRequest, DecodedResponse } from "./Decoded";
import { parseQueryParams } from "./query-params";

function Heading({ children }: { children: ReactNode }) {
  return (
    <h3 className="px-3 pt-3 pb-1 text-[11px] font-semibold tracking-wide text-subtle-foreground uppercase">
      {children}
    </h3>
  );
}

function Fact({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex gap-3 px-3 py-[3px] text-xs">
      <dt className="w-28 shrink-0 text-muted-foreground">{label}</dt>
      <dd className="selectable min-w-0 flex-1 break-words">{children}</dd>
    </div>
  );
}

const linkClass = "rounded-sm text-primary-text underline-offset-4 hover:underline";

const REDACTION_REASON = {
  cookie: "cookie values are not stored",
  credential: "credentials are not stored",
} as const;

function AdContextFacts({
  scan,
  detail,
  row,
}: {
  scan: ScanDetail;
  detail: NetworkRequestDetail | undefined;
  row: NetworkRow;
}) {
  const scanId = scan.summary.id;
  const { ad } = row;
  const context = detail?.adContext;
  if (!ad && row.category !== "adServer" && row.category !== "bid") return null;
  const auctionNumber = ad ? scan.auctions.findIndex((auction) => auction.id === ad.auctionId) + 1 : 0;
  const isBid = row.category === "bid";
  const findings = scan.findings.filter((finding) => context?.findingIds.includes(finding.id));
  const checks = (scan.bidderDiagnostics?.bidders ?? [])
    .flatMap((bidder) => bidder.checks.map((check) => ({ bidder: bidder.bidder, check })))
    .filter(
      ({ check }) =>
        context?.checkCodes.includes(check.code) &&
        (check.requestIds.includes(row.id) || check.requestIds.length === 0),
    );
  return (
    <>
      <Heading>Ad context</Heading>
      <dl>
        {ad ? (
          <Fact label="Auction">
            <Link
              to="/scans/$scanId"
              params={{ scanId }}
              search={{ tab: "auctions", page: row.page }}
              className={linkClass}
            >
              Auction {auctionNumber > 0 ? auctionNumber : ad.auctionId}
            </Link>
            <span className="text-muted-foreground">
              {" "}
              ·{" "}
              {isBid
                ? ad.exact
                  ? "matched by an id in the request"
                  : "matched by bidder and timing"
                : "the auction of this slot's bids"}
            </span>
          </Fact>
        ) : (
          <Fact label="Auction">
            <span className="text-muted-foreground">
              {isBid
                ? "Not tied to a Prebid auction: this call was made outside Prebid"
                : "No auction ran for this slot"}
            </span>
          </Fact>
        )}
        {isBid ? (
          <Fact label="Bidder">{row.bidder ?? (ad?.viaServer ? "Prebid Server, for several bidders" : EMPTY)}</Fact>
        ) : null}
        {context?.slotElementId ? (
          <Fact label="Slot">
            <Link
              to="/scans/$scanId"
              params={{ scanId }}
              search={{ tab: "slots", page: row.page }}
              className={cn(linkClass, "font-mono")}
            >
              {context.slotElementId}
            </Link>
          </Fact>
        ) : ad ? (
          <Fact label="Ad units">
            <span className="font-mono">{ad.adUnits.join(", ")}</span>
          </Fact>
        ) : null}
        {row.category === "adServer" ? (
          <Fact label="Prebid keys">
            {row.noHbTargeting
              ? "None: this ad request carried no hb_ targeting, so the ad server could not see a header-bidding price"
              : "The request carried hb_ targeting"}
          </Fact>
        ) : null}
        {isBid && rowOutcome(row) ? (
          <Fact label="Outcome">
            <AdOutcome row={row} />
          </Fact>
        ) : null}
        {isBid && context && context.units.length > 0 ? (
          <Fact label="Per ad unit">
            <ul className="flex flex-col gap-0.5">
              {context.units.map((unit, index) => (
                // biome-ignore lint/suspicious/noArrayIndexKey: Prebid Server lists one ad unit once per bidder
                <li key={`${unit.adUnit}-${index}`}>
                  <span className="font-mono">{unit.adUnit}</span>: {BID_STATUS_META[unit.status].label}
                  {unit.cpm === null ? "" : `, ${formatCpm(unit.cpm)}`}
                </li>
              ))}
            </ul>
          </Fact>
        ) : null}
        {isBid && ad ? (
          <Fact label="Verdict">
            <span data-testid="verdict" className="font-medium">
              {verdictSentence({
                verdict: ad.verdict,
                lateByMs: ad.lateByMs,
                networkMs: row.durationMs,
                error: row.error,
              })}
            </span>
            <span className="text-muted-foreground"> · {EXCHANGE_VERDICT_LABEL[ad.verdict]}</span>
          </Fact>
        ) : null}
        {findings.length > 0 || checks.length > 0 ? (
          <Fact label="Related">
            <ul className="flex flex-col gap-0.5">
              {findings.map((finding) => (
                <li key={finding.id}>
                  <Link
                    to="/scans/$scanId"
                    params={{ scanId }}
                    search={{ tab: "findings", finding: finding.id }}
                    className={linkClass}
                  >
                    Finding: {finding.title}
                  </Link>
                </li>
              ))}
              {checks.map(({ bidder, check }) => (
                <li key={`${bidder}-${check.code}`}>
                  <Link
                    to="/scans/$scanId"
                    params={{ scanId }}
                    search={{ tab: "auctions", page: row.page }}
                    className={linkClass}
                  >
                    Bidder check ({bidder}): {check.summary}
                  </Link>
                </li>
              ))}
            </ul>
          </Fact>
        ) : null}
      </dl>
    </>
  );
}

export function SummaryTab({
  scan,
  row,
  detail,
  onOpenRequest,
}: {
  scan: ScanDetail;
  row: NetworkRow;
  detail: NetworkRequestDetail | undefined;
  onOpenRequest: (requestId: string) => void;
}) {
  const page = scan.pages.find((candidate) => candidate.index === row.page);
  return (
    <div className="pb-3">
      {detail && !detail.available ? (
        <p role="note" className="border-b bg-muted/50 px-3 py-2 text-xs text-pretty text-muted-foreground">
          The network data of this scan was removed to stay under the storage limit. What the list keeps is shown: the
          address without its query, status, sizes and timing. Headers and bodies are gone.
        </p>
      ) : null}
      <Heading>General</Heading>
      <dl>
        <Fact label="URL">
          <span className="font-mono break-all">{row.url}</span>
        </Fact>
        <Fact label="Method">{row.method}</Fact>
        <Fact label="Status">
          <span className="flex flex-wrap items-center gap-2">
            <StatusMark row={row} />
            <span className="text-muted-foreground">{row.error ?? OUTCOME_LABEL[row.outcome]}</span>
          </span>
        </Fact>
        <Fact label="Vendor">
          {vendorLabel(row)} <span className="text-muted-foreground">· {CATEGORY_META[row.category].label}</span>
        </Fact>
        {page ? <Fact label="Page view">{pageLabel(page)}</Fact> : null}
        <Fact label="Served from">
          {SERVED_FROM_LABEL[row.servedFrom]}
          {row.servedFrom === "network" ? "" : " (nothing was transferred)"}
        </Fact>
        <Fact label="Size">
          {formatBytes(row.transferredBytes)} transferred · {formatBytes(row.decodedBytes)} decoded
          {row.mime ? ` · ${row.mime}` : ""}
        </Fact>
        <Fact label="Timing">
          started {formatDuration(row.startedAtMs)}
          {row.durationMs === null
            ? ", no response before the visit ended"
            : `, took ${formatDuration(row.durationMs)}`}
        </Fact>
        {row.context !== "page" ? (
          <Fact label="Made in">
            {
              { iframe: "A frame inside the page", worker: "A worker", serviceWorker: "A service worker", page: "" }[
                row.context
              ]
            }
          </Fact>
        ) : null}
        {row.kind === "webSocket" || row.kind === "eventSource" ? (
          <Fact label="Messages">Frames are not recorded: one entry stands for the whole connection.</Fact>
        ) : null}
        {row.outcome === "cancelled" ? (
          <Fact label="Note">Cancelled, usually because the page navigated away.</Fact>
        ) : null}
        {detail?.protocol ? <Fact label="Protocol">{detail.protocol}</Fact> : null}
        {detail?.remoteAddress ? <Fact label="Remote address">{detail.remoteAddress}</Fact> : null}
        {detail?.priority ? <Fact label="Priority">{detail.priority}</Fact> : null}
      </dl>
      {detail && detail.redirectChain.length > 0 ? (
        <>
          <Heading>Redirect chain</Heading>
          <ol className="px-3 text-xs">
            {detail.redirectChain.map((hop, index) => (
              <li key={hop.requestId} className="flex items-center gap-1.5 py-[3px]">
                {index > 0 ? (
                  <CornerDownRightIcon aria-hidden="true" className="size-3.5 shrink-0 text-subtle-foreground" />
                ) : null}
                <span className="w-8 shrink-0 text-muted-foreground tabular-nums">{hop.status ?? EMPTY}</span>
                {hop.requestId === row.id ? (
                  <span className="min-w-0 truncate font-mono font-medium" title={hop.url}>
                    {hop.url}
                  </span>
                ) : (
                  <button
                    type="button"
                    onClick={() => onOpenRequest(hop.requestId)}
                    className={cn(linkClass, "min-w-0 truncate text-left font-mono")}
                    title={hop.url}
                  >
                    {hop.url}
                  </button>
                )}
              </li>
            ))}
          </ol>
        </>
      ) : null}
      <AdContextFacts scan={scan} detail={detail} row={row} />
    </div>
  );
}

function HeaderTable({ title, headers }: { title: string; headers: Header[] }) {
  return (
    <>
      <Heading>
        {title} ({headers.length})
      </Heading>
      {headers.length === 0 ? (
        <p className="px-3 text-xs text-muted-foreground">None match.</p>
      ) : (
        <dl>
          {headers.map((header, index) => (
            // biome-ignore lint/suspicious/noArrayIndexKey: a header name can repeat (Set-Cookie)
            <div key={`${header.name}-${index}`} className="flex gap-3 px-3 py-[3px] text-xs">
              <dt className="selectable w-40 shrink-0 font-medium break-words">{header.name}</dt>
              <dd className="selectable min-w-0 flex-1 font-mono break-all">
                {header.value}
                {header.redaction ? (
                  <Badge tone="amber" className="ml-2 align-middle font-sans">
                    redacted: {REDACTION_REASON[header.redaction]}
                  </Badge>
                ) : null}
              </dd>
            </div>
          ))}
        </dl>
      )}
    </>
  );
}

export function HeadersTab({ detail }: { detail: NetworkRequestDetail }) {
  const [filter, setFilter] = useState("");
  const needle = filter.trim().toLowerCase();
  const matching = (headers: Header[]) => headers.filter((header) => header.name.toLowerCase().includes(needle));
  const redacted = [...detail.requestHeaders, ...detail.responseHeaders].some((header) => header.redaction !== null);
  if (detail.responseHeaders.length === 0 && detail.requestHeaders.length === 0) {
    return <p className="p-3 text-xs text-muted-foreground">No headers are stored for this request.</p>;
  }
  return (
    <div className="pb-3">
      <div className="sticky top-0 z-10 border-b bg-background p-2">
        <SearchInput
          aria-label="Filter headers by name"
          placeholder="Filter by name"
          value={filter}
          onValueChange={setFilter}
          inputClassName="h-7 text-xs"
        />
      </div>
      {redacted ? (
        <p role="note" className="border-b bg-muted/50 px-3 py-2 text-xs text-pretty text-muted-foreground">
          Cookie and authorization values were replaced by their length when this scan was stored. What stands in their
          place below is not what was sent.
        </p>
      ) : null}
      <HeaderTable title="Response headers" headers={matching(detail.responseHeaders)} />
      <HeaderTable title="Request headers" headers={matching(detail.requestHeaders)} />
    </div>
  );
}

function ViewToggle({ value, onChange }: { value: "decoded" | "raw"; onChange: (value: "decoded" | "raw") => void }) {
  return (
    <ToggleGroup size="sm" aria-label="Body view" value={value} onValueChange={onChange}>
      <ToggleGroupItem value="decoded">Decoded</ToggleGroupItem>
      <ToggleGroupItem value="raw">Raw</ToggleGroupItem>
    </ToggleGroup>
  );
}

/** Bid endpoints with no OpenRTB to read say so, instead of showing an empty decoded view. */
function NoDecoder() {
  return (
    <p className="border-b px-3 py-2 text-xs text-muted-foreground">
      No decoder for this adapter yet: its bodies are shown as they were sent.
    </p>
  );
}

export function PayloadTab({ scanId, detail }: { scanId: string; detail: NetworkRequestDetail }) {
  const { row } = detail;
  const params = parseQueryParams(row.url);
  const decoded = detail.decoded?.request ?? null;
  const [view, setView] = useState<"decoded" | "raw">(decoded ? "decoded" : "raw");
  const hasBody = detail.requestBody.state !== "empty";
  return (
    <div className="flex min-h-full flex-col">
      {params.length > 0 ? (
        <>
          <Heading>Query string ({params.length})</Heading>
          <dl className="pb-2">
            {params.map((param, index) => (
              // biome-ignore lint/suspicious/noArrayIndexKey: a key can repeat in a query string
              <div key={`${param.key}-${index}`} className="px-3 py-[3px] text-xs">
                <div className="flex gap-3">
                  <dt className="selectable w-36 shrink-0 font-medium break-words">{param.key}</dt>
                  <dd className="selectable min-w-0 flex-1 font-mono break-all">{param.value}</dd>
                </div>
                {param.nested.length > 0 ? (
                  <dl className="mt-0.5 ml-4 border-l pl-3">
                    {param.nested.map((nested, nestedIndex) => (
                      // biome-ignore lint/suspicious/noArrayIndexKey: nested keys can repeat too
                      <div key={`${nested.key}-${nestedIndex}`} className="flex gap-3 py-px">
                        <dt className="selectable w-32 shrink-0 text-muted-foreground break-words">{nested.key}</dt>
                        <dd className="selectable min-w-0 flex-1 font-mono break-all">{nested.value}</dd>
                      </div>
                    ))}
                  </dl>
                ) : null}
              </div>
            ))}
          </dl>
        </>
      ) : null}
      {hasBody ? (
        <>
          <div className="flex items-center justify-between border-t pr-2">
            <Heading>Request body</Heading>
            {decoded ? <ViewToggle value={view} onChange={setView} /> : null}
          </div>
          {row.category === "bid" && !detail.decoded ? <NoDecoder /> : null}
          {decoded && view === "decoded" ? (
            <DecodedRequest request={decoded} />
          ) : (
            <div className="flex min-h-64 flex-1 flex-col border-t">
              <BodyView
                scanId={scanId}
                requestId={row.id}
                side="request"
                body={detail.requestBody}
                // Adapters post JSON as text/plain to avoid a preflight; it is still JSON.
                mime={row.category === "bid" ? "application/json" : detail.requestBody.mime}
              />
            </div>
          )}
        </>
      ) : params.length === 0 ? (
        <p className="p-3 text-xs text-muted-foreground">This request has no query string and no body.</p>
      ) : null}
    </div>
  );
}

export function ResponseTab({ scanId, detail }: { scanId: string; detail: NetworkRequestDetail }) {
  const { row } = detail;
  const decoded = detail.decoded?.response ?? null;
  const [view, setView] = useState<"decoded" | "raw">(decoded ? "decoded" : "raw");
  const scriptId = detail.responseBody.scriptId;
  return (
    <div className="flex min-h-full flex-col">
      {decoded || scriptId ? (
        <div className="flex h-9 shrink-0 items-center justify-between gap-2 border-b px-2">
          {scriptId ? (
            <Link
              to="/scans/$scanId/source/$scriptId"
              params={{ scanId, scriptId }}
              className={cn(linkClass, "px-1 text-xs")}
            >
              Open in script viewer
            </Link>
          ) : (
            <span />
          )}
          {decoded ? <ViewToggle value={view} onChange={setView} /> : null}
        </div>
      ) : null}
      {row.category === "bid" && row.ad && !detail.decoded && detail.responseBody.state === "kept" ? (
        <NoDecoder />
      ) : null}
      {decoded && view === "decoded" ? (
        <DecodedResponse response={decoded} />
      ) : (
        <div className="flex min-h-64 flex-1 flex-col">
          <BodyView
            scanId={scanId}
            requestId={row.id}
            side="response"
            body={detail.responseBody}
            mime={detail.responseBody.mime}
          />
        </div>
      )}
    </div>
  );
}

export function TimingTab({ detail }: { detail: NetworkRequestDetail }) {
  const { row, timing, adContext } = detail;
  if (!timing) {
    return (
      <p className="p-3 text-xs text-pretty text-muted-foreground">
        {row.durationMs === null
          ? "No response before the visit ended: there is no timing to break down."
          : "The browser reported no phases for this request. That is the case for answers from a cache or a service worker, for failed requests, and once the network data of a scan has been removed."}
      </p>
    );
  }
  const phases: [label: string, ms: number | null, color: string][] = [
    ["Queued", timing.queuedMs, "var(--net-queue)"],
    ["DNS lookup", timing.dnsMs, "var(--net-connect)"],
    ["Connecting", timing.connectMs, "var(--net-connect)"],
    ["TLS", timing.tlsMs, "var(--net-connect)"],
    ["Sending", timing.sendMs, "var(--net-wait)"],
    ["Waiting for the server", timing.waitMs, "var(--net-wait)"],
    ["Receiving", timing.receiveMs, "var(--net-receive)"],
  ];
  const present = phases.filter((phase): phase is [string, number, string] => phase[1] !== null);
  const total = present.reduce((sum, [, ms]) => sum + ms, 0) || 1;
  let offset = 0;
  const close = adContext?.auctionEndedAtMs ?? null;
  const end = timing.finishedAtMs;
  return (
    <div className="pb-3">
      <Heading>Phases</Heading>
      <ul className="px-3">
        {present.map(([label, ms, color]) => {
          const left = (offset / total) * 100;
          offset += ms;
          return (
            <li key={label} className="grid grid-cols-[150px_minmax(0,1fr)_64px] items-center gap-3 py-1 text-xs">
              <span className="text-muted-foreground">{label}</span>
              <span className="relative h-2.5 rounded-sm bg-muted">
                <span
                  className="absolute inset-y-0 rounded-sm"
                  style={{ left: `${left}%`, width: `max(2px, ${(ms / total) * 100}%)`, backgroundColor: color }}
                />
              </span>
              <span className="text-right tabular-nums">{formatDuration(ms)}</span>
            </li>
          );
        })}
      </ul>
      {present.length < phases.length ? (
        <p className="px-3 pt-1 text-[11px] text-muted-foreground">
          No DNS, connection or TLS time: the request used a connection that was already open.
        </p>
      ) : null}
      <dl className="pt-2">
        <Fact label="Started">{formatDuration(row.startedAtMs)} into the page view</Fact>
        {end !== null ? <Fact label="Finished">{formatDuration(end)} into the page view</Fact> : null}
        {detail.preflightMs !== null ? (
          <Fact label="Preflight">
            The browser asked permission first (CORS preflight): +{formatDuration(detail.preflightMs)} before this
            request could start
          </Fact>
        ) : null}
        {close !== null && row.category === "bid" && row.ad ? (
          <Fact label="Its auction">
            closed at {formatDuration(close)}.{" "}
            {verdictSentence({
              verdict: row.ad.verdict,
              lateByMs: row.ad.lateByMs,
              networkMs: row.durationMs,
              error: row.error,
            })}
            .
          </Fact>
        ) : null}
      </dl>
    </div>
  );
}

export function InitiatorTab({ scanId, detail }: { scanId: string; detail: NetworkRequestDetail }) {
  const { row, stack } = detail;
  const kind = row.initiator?.kind;
  if (stack.length === 0) {
    const words: Record<string, string> = {
      parser: "The HTML parser made this request: it is in the page's markup.",
      preload: "A preload hint in the page made this request.",
      preflight: "The browser sent this preflight by itself, to ask whether the request that follows is allowed.",
      other: "The browser made this request by itself.",
      script: "A script made this request; its call stack was not recorded.",
    };
    return (
      <div className="p-3 text-xs">
        <p className="text-pretty text-muted-foreground">
          {kind
            ? words[kind]
            : row.kind === "document"
              ? "A navigation: nothing in a page made this request."
              : "No initiator was recorded for this request."}
        </p>
        {row.initiator?.url ? <p className="selectable mt-2 font-mono break-all">{row.initiator.url}</p> : null}
      </div>
    );
  }
  return (
    <div className="pb-3">
      <Heading>Call stack ({stack.length})</Heading>
      <ol className="px-3 text-xs">
        {stack.map((frame, index) => (
          // biome-ignore lint/suspicious/noArrayIndexKey: a stack can pass through one place twice
          <li key={index} className="flex items-baseline gap-3 py-[3px]">
            <span className="selectable w-40 shrink-0 truncate font-mono" title={frame.function ?? "(anonymous)"}>
              {frame.function ?? "(anonymous)"}
            </span>
            {frame.scriptId ? (
              <Link
                to="/scans/$scanId/source/$scriptId"
                params={{ scanId, scriptId: frame.scriptId }}
                search={{ oline: frame.line, ocol: frame.column }}
                className={cn(linkClass, "min-w-0 truncate font-mono")}
                title={`${frame.url}, line ${frame.line}, column ${frame.column} as captured`}
              >
                {fileNameOf(frame.url)}:{frame.line}:{frame.column}
              </Link>
            ) : (
              <span className="selectable min-w-0 truncate font-mono text-muted-foreground" title={frame.url}>
                {fileNameOf(frame.url)}:{frame.line}:{frame.column}
              </span>
            )}
          </li>
        ))}
      </ol>
      <p className="px-3 pt-2 text-[11px] text-pretty text-muted-foreground">
        Innermost call first. Positions are in the scripts as captured; a link opens the formatted line they map to.
      </p>
    </div>
  );
}
