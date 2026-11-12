import {
  CornerDownRightIcon,
  LogInIcon,
  type LucideIcon,
  MousePointerClickIcon,
  RefreshCwIcon,
  RouteIcon,
  UserIcon,
} from "lucide-react";
import type { PageReach } from "@/bindings";
import { Panel } from "@/shared/components/panel";
import { PAGE_REACH_LABEL, pagePath } from "@/shared/domain/page-views";
import { cn } from "@/shared/lib/cn";
import type { VisitState } from "./visit";

const REACH_ICON: Record<PageReach, LucideIcon> = {
  entry: LogInIcon,
  linkClick: MousePointerClickIcon,
  routeChange: RouteIcon,
  userNavigation: UserIcon,
  redirect: CornerDownRightIcon,
  reload: RefreshCwIcon,
};

/** Where the visit is and what the visitor just did, while the capture step runs. */
export function VisitPanel({ visit }: { visit: VisitState }) {
  const { pages, current, actions, skipped } = visit;
  if (!current) return null;
  return (
    <Panel aria-label="Visit" className="flex shrink-0 flex-col gap-2.5 px-4 py-3">
      <div className="flex items-baseline justify-between gap-4">
        <h2 className="text-sm font-semibold">Visit</h2>
        <p className="min-w-0 truncate text-[13px] text-muted-foreground" title={current.url}>
          {/* The audit trail announces a page before its title is known; its path always is. */}
          Now on page {current.index + 1}: {current.title ?? pagePath(current.url)}
        </p>
      </div>
      <ol aria-label="Pages visited" className="flex flex-wrap items-center gap-x-1.5 gap-y-1">
        {pages.map((page) => {
          const Icon = REACH_ICON[page.reachedBy];
          return (
            <li
              key={page.index}
              title={page.linkText ? `${PAGE_REACH_LABEL[page.reachedBy]}: “${page.linkText}”` : page.url}
              className={cn(
                "flex h-6 max-w-56 items-center gap-1.5 rounded-md border px-2 text-xs",
                page === current ? "border-violet-line bg-primary-soft text-foreground" : "text-muted-foreground",
              )}
            >
              <span className="font-medium tabular-nums">{page.index + 1}</span>
              <Icon role="img" aria-label={PAGE_REACH_LABEL[page.reachedBy]} className="size-3.5 shrink-0" />
              <span className="truncate font-mono">{pagePath(page.url)}</span>
            </li>
          );
        })}
      </ol>
      {actions.length > 0 ? (
        <ul aria-label="Latest actions" className="flex flex-col gap-0.5 text-[13px]">
          {actions.map((action, index) => (
            <li
              key={action.id}
              className={cn(
                "truncate",
                action.warning ? "text-amber-fg" : "text-muted-foreground",
                index === actions.length - 1 && !action.warning && "text-foreground",
              )}
            >
              {action.message}
            </li>
          ))}
        </ul>
      ) : null}
      {skipped ? <p className="truncate text-xs text-subtle-foreground">{skipped}</p> : null}
    </Panel>
  );
}
