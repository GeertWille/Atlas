import { ChevronDownIcon, CloudIcon } from "lucide-react";
import { Fragment, useState } from "react";
import type { ScanDetail } from "@/bindings";
import { useNow } from "@/shared/hooks/use-now";
import { cn } from "@/shared/lib/cn";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/shared/ui/collapsible";
import { browserbaseVisitFacts } from "./browserbase-visit";

/**
 * A visit that ran in a Browserbase browser: one line with where, how long and how much, and behind it
 * everything recorded about that browser and what the pages could read in it.
 */
export function BrowserbaseVisitDetails({ detail }: { detail: ScanDetail }) {
  const [open, setOpen] = useState(false);
  // Time only matters here while the proxy traffic is still being counted: the clock runs until it is not.
  const stillCounting = browserbaseVisitFacts(detail, Date.now())?.pending ?? false;
  const facts = browserbaseVisitFacts(detail, useNow(stillCounting, 1000));
  if (!facts) return null;
  return (
    <Collapsible
      role="group"
      aria-label="Browserbase visit"
      open={open}
      onOpenChange={setOpen}
      className="rounded-lg border bg-muted/30"
    >
      <div className="flex items-center justify-between gap-3 py-1 pr-1 pl-3">
        <p className="selectable flex min-w-0 items-center gap-2 text-[13px]">
          <CloudIcon aria-hidden="true" className="size-4 shrink-0 text-muted-foreground" />
          <span className="min-w-0 text-pretty text-muted-foreground" aria-live="polite">
            {facts.summary.map((part, index) => (
              <Fragment key={part}>
                {index > 0 ? " · " : null}
                <span className={cn(index === 0 && "font-medium text-foreground")}>{part}</span>
              </Fragment>
            ))}
          </span>
        </p>
        <CollapsibleTrigger className="flex h-7 shrink-0 items-center gap-1 rounded-md px-2 text-[13px] font-medium text-muted-foreground hover:bg-accent hover:text-foreground">
          Browser details
          <ChevronDownIcon className={cn("size-4 transition-transform", open && "rotate-180")} />
        </CollapsibleTrigger>
      </div>
      <CollapsibleContent>
        <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-5 gap-y-1.5 border-t px-3 py-2.5 text-[13px]">
          {facts.rows.map((row) => (
            <div key={row.name} className="col-span-2 grid grid-cols-subgrid">
              <dt className="text-muted-foreground">{row.name}</dt>
              <dd className={cn("selectable min-w-0 text-pretty break-words", row.mono && "font-mono text-xs")}>
                {row.value}
              </dd>
            </div>
          ))}
        </dl>
      </CollapsibleContent>
    </Collapsible>
  );
}
