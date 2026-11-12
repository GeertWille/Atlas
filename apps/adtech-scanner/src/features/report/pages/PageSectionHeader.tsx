import type { PageView } from "@/bindings";
import { PAGE_REACH_LABEL, pagePath } from "@/shared/domain/page-views";
import { formatDuration } from "@/shared/lib/format";

/** Heads the auctions or slots of one page view when a multi-page visit is shown whole. */
export function PageSectionHeader({ page, summary }: { page: PageView; summary: string }) {
  return (
    <header className="flex flex-wrap items-baseline gap-x-3 gap-y-0.5 pt-1">
      <h3 className="flex min-w-0 items-baseline gap-2 text-sm font-semibold">
        <span className="shrink-0">Page {page.index + 1}</span>
        <span className="selectable truncate font-mono text-[13px] font-normal text-muted-foreground" title={page.url}>
          {pagePath(page.url)}
        </span>
      </h3>
      <p className="text-xs text-muted-foreground tabular-nums">
        {PAGE_REACH_LABEL[page.reachedBy]}
        {page.index === 0 ? "" : ` · opened ${formatDuration(page.startedAtMs)} into the visit`} · {summary}
      </p>
    </header>
  );
}
