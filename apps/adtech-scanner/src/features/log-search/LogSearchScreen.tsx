import { Link } from "@tanstack/react-router";
import { ArrowRightIcon, SearchIcon, SearchXIcon } from "lucide-react";
import { useEffect, useState } from "react";
import type { LogSearchHit } from "@/bindings";
import { useLogSearch } from "@/lib/query/hooks";
import { LOG_KIND_ICON } from "@/shared/components/log-entry-row";
import { Panel } from "@/shared/components/panel";
import { QueryError } from "@/shared/components/query-error";
import { SearchInput } from "@/shared/components/search-input";
import { PIPELINE_NODE_META } from "@/shared/domain/pipeline";
import { LOG_KIND_LABEL } from "@/shared/domain/vocabulary";
import { useDebouncedValue } from "@/shared/hooks/use-debounced-value";
import { formatDateTime, pluralize } from "@/shared/lib/format";
import { pathOf } from "@/shared/lib/url";
import { Button } from "@/shared/ui/button";
import { Empty } from "@/shared/ui/empty";
import { Skeleton } from "@/shared/ui/skeleton";
import { groupHitsByScan, type HitGroup } from "./group-hits";
import { parseSnippet } from "./snippet";

const MIN_QUERY_LENGTH = 2;
const EXAMPLES = ["timeout", "report_finding", "IS_MOBILE", "auctionInit", "budget"];

function Snippet({ snippet }: { snippet: string }) {
  return (
    <span className="selectable line-clamp-2 font-mono text-xs leading-relaxed break-words text-muted-foreground">
      {parseSnippet(snippet).map((part, index) =>
        part.marked ? (
          // biome-ignore lint/suspicious/noArrayIndexKey: parts of a static snippet never reorder
          <mark key={index} className="font-medium text-foreground">
            {part.text}
          </mark>
        ) : (
          // biome-ignore lint/suspicious/noArrayIndexKey: parts of a static snippet never reorder
          <span key={index}>{part.text}</span>
        ),
      )}
    </span>
  );
}

function HitRow({ hit }: { hit: LogSearchHit }) {
  const { entry } = hit;
  const Icon = LOG_KIND_ICON[entry.kind];
  return (
    <li className="border-t">
      <Link
        to="/scans/$scanId"
        params={{ scanId: entry.scanId }}
        search={{ tab: "audit", log: entry.id }}
        className="group flex items-start gap-3 px-4 py-2.5 transition-colors hover:bg-accent/60"
      >
        <Icon aria-hidden="true" className="mt-0.5 size-3.5 shrink-0 text-subtle-foreground" />
        <span className="flex min-w-0 flex-1 flex-col gap-0.5">
          <span className="text-xs text-subtle-foreground">
            {LOG_KIND_LABEL[entry.kind]} · {PIPELINE_NODE_META[entry.node].label} · {formatDateTime(entry.ts)}
          </span>
          <Snippet snippet={hit.snippet} />
        </span>
        <ArrowRightIcon className="mt-1 size-3.5 shrink-0 text-subtle-foreground opacity-0 transition-opacity group-hover:opacity-100 group-focus-visible:opacity-100" />
      </Link>
    </li>
  );
}

function GroupPanel({ group }: { group: HitGroup }) {
  const hostname = new URL(group.scanUrl).hostname;
  return (
    <Panel aria-label={`Matches in ${hostname}`} className="overflow-hidden">
      <header className="flex items-center justify-between gap-4 px-4 py-2.5">
        <h2 className="min-w-0 truncate text-sm">
          <span className="font-semibold">{hostname}</span>
          <span className="text-subtle-foreground">{pathOf(group.scanUrl)}</span>
        </h2>
        <div className="flex shrink-0 items-center gap-3">
          <span className="text-xs text-muted-foreground tabular-nums">{pluralize(group.hits.length, "match", "matches")}</span>
          <Button asChild variant="ghost" size="sm">
            <Link to="/scans/$scanId" params={{ scanId: group.scanId }} search={{ tab: "audit" }}>
              Open audit trail
            </Link>
          </Button>
        </div>
      </header>
      <ul>
        {group.hits.map((hit) => (
          <HitRow key={hit.entry.id} hit={hit} />
        ))}
      </ul>
    </Panel>
  );
}

function ResultsSkeleton() {
  return (
    <Panel className="flex flex-col gap-4 p-4">
      {[80, 64, 72].map((width) => (
        <div key={width} className="flex flex-col gap-2">
          <Skeleton className="h-3 w-48" />
          <Skeleton className="h-3.5" style={{ width: `${width}%` }} />
        </div>
      ))}
    </Panel>
  );
}

export function LogSearchScreen({
  query,
  onQueryChange,
}: {
  /** The committed query, owned by the route so searches are linkable. */
  query: string;
  onQueryChange: (query: string) => void;
}) {
  const [draft, setDraft] = useState(query);
  const debounced = useDebouncedValue(draft.trim(), 250);
  const search = useLogSearch(query);

  useEffect(() => {
    if (debounced !== query) onQueryChange(debounced);
  }, [debounced, query, onQueryChange]);

  const tooShort = query.length < MIN_QUERY_LENGTH;
  const groups = groupHitsByScan(search.data ?? []);

  return (
    <>
      <div className="flex flex-col gap-2">
        <SearchInput
          autoFocus
          aria-label="Search all audit trails"
          placeholder="Search every scan's audit trail: messages, prompts, tool calls, findings"
          value={draft}
          onValueChange={setDraft}
          inputClassName="h-10 text-[15px]"
        />
        <p className="h-5 text-[13px] text-muted-foreground tabular-nums" aria-live="polite">
          {tooShort || search.isPending
            ? ""
            : `${pluralize(search.data?.length ?? 0, "match", "matches")} in ${pluralize(groups.length, "scan")}`}
        </p>
      </div>

      {tooShort ? (
        <Empty
          icon={SearchIcon}
          title="Search how every conclusion was reached"
          description="Full-text search across the audit trail of all scans: progress messages, model prompts and responses, tool calls and reported findings."
        >
          {EXAMPLES.map((example) => (
            <Button key={example} variant="outline" size="sm" className="font-mono text-xs" onClick={() => setDraft(example)}>
              {example}
            </Button>
          ))}
        </Empty>
      ) : search.isPending ? (
        <ResultsSkeleton />
      ) : search.isError ? (
        <QueryError title="Search failed" error={search.error} onRetry={() => void search.refetch()} />
      ) : groups.length === 0 ? (
        <Empty icon={SearchXIcon} title={`No matches for “${query}”`} description="Try fewer or different words. Search matches whole audit-trail entries, including their structured data." />
      ) : (
        groups.map((group) => <GroupPanel key={group.scanId} group={group} />)
      )}
    </>
  );
}
