import type { PageView } from "@/bindings";
import { PAGE_REACH_LABEL, pagePath } from "@/shared/domain/page-views";
import { formatDuration, formatPercent, pluralize } from "@/shared/lib/format";
import { ToggleGroup, ToggleGroupItem } from "@/shared/ui/toggle-group";

const ALL = "all";

function PageFacts({ page }: { page: PageView }) {
  const { metrics } = page;
  const facts = [
    page.link ? `${PAGE_REACH_LABEL[page.reachedBy]} “${page.link.text}”` : PAGE_REACH_LABEL[page.reachedBy],
    `opened ${formatDuration(page.startedAtMs)} into the visit`,
    `watched for ${formatDuration(page.dwellMs)}`,
    pluralize(metrics.auctionCount, "auction"),
    metrics.slotCount > 0
      ? `${pluralize(metrics.slotCount, "slot")}, ${formatPercent(metrics.fillRate)} of requested filled`
      : "no ad slots",
  ];
  return (
    <p className="text-[13px] text-pretty text-muted-foreground">
      <span className="selectable font-medium text-foreground">{page.title ?? pagePath(page.url)}</span>
      {" · "}
      {facts.join(" · ")}
    </p>
  );
}

/** Narrows the report to one page view of a multi-page visit. Not rendered for a single page. */
export function PageSwitcher({
  pages,
  selected,
  onSelect,
}: {
  pages: PageView[];
  selected: PageView | null;
  /** `null` shows the whole visit again. */
  onSelect: (pageIndex: number | null) => void;
}) {
  if (pages.length <= 1) return null;
  return (
    <section aria-label="Page views" className="flex flex-col gap-2">
      {/* A manual session can record two dozen pages: the row scrolls instead of wrapping the report away. */}
      <div className="-mx-1 overflow-x-auto px-1 py-0.5">
        <ToggleGroup
          aria-label="Page view"
          value={selected ? String(selected.index) : ALL}
          onValueChange={(value) => onSelect(value === ALL ? null : Number(value))}
        >
          <ToggleGroupItem value={ALL}>All pages</ToggleGroupItem>
          {pages.map((page) => (
            <ToggleGroupItem key={page.index} value={String(page.index)} title={page.url} className="max-w-56">
              <span className="tabular-nums">{page.index + 1}</span>
              <span className="truncate font-mono text-xs">{pagePath(page.url)}</span>
            </ToggleGroupItem>
          ))}
        </ToggleGroup>
      </div>
      {selected ? (
        <PageFacts page={selected} />
      ) : (
        <p className="text-[13px] text-muted-foreground">
          The visit covered {pluralize(pages.length, "page view")}. Times on the tabs below count from the start of each
          page view.
        </p>
      )}
    </section>
  );
}
