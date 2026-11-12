import { Link } from "@tanstack/react-router";
import { WrapTextIcon } from "lucide-react";
import { lazy, Suspense, useState } from "react";
import type { BodyRef, BodySide } from "@/bindings";
import { MAX_BODY_VIEW_BYTES, useNetworkBody } from "@/lib/query/hooks";
import { QueryError } from "@/shared/components/query-error";
import { languageForMime } from "@/shared/editor/mime";
import { cn } from "@/shared/lib/cn";
import { formatBytes } from "@/shared/lib/format";
import { Button } from "@/shared/ui/button";
import { Skeleton } from "@/shared/ui/skeleton";
import { Hint } from "@/shared/ui/tooltip";
import { bodyNotice } from "./body-state";

const BodyEditor = lazy(() => import("./BodyEditor"));

/** JSON larger than this is shown as it was sent: re-printing it would cost more than it helps. */
const PRETTY_PRINT_LIMIT = 2 * 1024 * 1024;

/** Indents JSON for reading. Anything that does not parse (a truncated body, JSONP) is left as it is. */
export function prettyPrint(text: string, mime: string | null, complete: boolean): string {
  if (!complete || text.length > PRETTY_PRINT_LIMIT || languageForMime(mime) !== "json") return text;
  try {
    return JSON.stringify(JSON.parse(text), null, 2);
  } catch {
    return text;
  }
}

function Notice({ body, side }: { body: BodyRef; side: BodySide }) {
  const notice = bodyNotice(body, side);
  if (!notice) return null;
  return (
    <div role="note" className="flex flex-col gap-0.5 border-b bg-muted/50 px-3 py-2 text-xs">
      <p className="font-medium">{notice.title}</p>
      {notice.detail ? <p className="text-pretty text-muted-foreground">{notice.detail}</p> : null}
      {notice.storage ? (
        <Link to="/settings" className="w-fit text-primary-text underline-offset-4 hover:underline">
          Storage settings
        </Link>
      ) : null}
    </div>
  );
}

export function BodyView({
  scanId,
  requestId,
  side,
  body,
  mime,
}: {
  scanId: string;
  requestId: string;
  side: BodySide;
  body: BodyRef;
  /** The type to highlight as; a bid request is JSON whatever its Content-Type says. */
  mime: string | null;
}) {
  const readable = body.state === "kept" || body.state === "truncated";
  const query = useNetworkBody(scanId, requestId, side, readable);
  const [wrap, setWrap] = useState(true);
  const chunks = query.data?.pages ?? [];
  const loaded = chunks.reduce((sum, chunk) => sum + (chunk.text?.length ?? 0), 0);
  const total = chunks[0]?.totalBytes ?? body.storedBytes;
  const complete = readable && !query.hasNextPage && body.state === "kept";
  const canLoadMore = query.hasNextPage && loaded < MAX_BODY_VIEW_BYTES;
  const language = languageForMime(mime);

  if (!readable) return <Notice body={body} side={side} />;
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <Notice body={body} side={side} />
      <div className="flex h-7 shrink-0 items-center gap-2 border-b px-2 text-[11px] text-muted-foreground tabular-nums">
        <span className="truncate">
          {mime ?? "unknown type"} · {formatBytes(body.bytes)}
          {query.hasNextPage ? ` · showing the first ${formatBytes(loaded)} of ${formatBytes(total)}` : ""}
        </span>
        <span className="ml-auto flex items-center gap-1">
          {canLoadMore ? (
            <Button
              variant="outline"
              size="sm"
              className="h-5 px-1.5 text-[11px]"
              disabled={query.isFetchingNextPage}
              onClick={() => void query.fetchNextPage()}
            >
              Load more
            </Button>
          ) : null}
          <Hint label="Wrap long lines">
            <Button
              variant="ghost"
              size="icon-sm"
              aria-label="Wrap long lines"
              aria-pressed={wrap}
              className={cn("size-5 [&_svg]:size-3.5!", wrap && "bg-accent text-foreground")}
              onClick={() => setWrap(!wrap)}
            >
              <WrapTextIcon />
            </Button>
          </Hint>
        </span>
      </div>
      {query.isError ? (
        <QueryError title="Could not read this body" error={query.error} onRetry={() => void query.refetch()} />
      ) : query.isPending ? (
        <div className="flex flex-col gap-2 p-3">
          <Skeleton className="h-3 w-2/3" />
          <Skeleton className="h-3 w-1/2" />
          <Skeleton className="h-3 w-3/5" />
        </div>
      ) : (
        <Suspense fallback={<Skeleton className="m-3 h-24" />}>
          <BodyEditor
            text={prettyPrint(chunks.map((chunk) => chunk.text ?? "").join(""), mime, complete)}
            language={language}
            wrap={wrap}
            label={side === "request" ? "Request body" : "Response body"}
          />
        </Suspense>
      )}
    </div>
  );
}
