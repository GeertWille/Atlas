import { ArrowDownIcon } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import type { LogEntry } from "@/bindings";
import { LogList } from "@/shared/components/log-list";
import { Button } from "@/shared/ui/button";
import { Skeleton } from "@/shared/ui/skeleton";

const PINNED_THRESHOLD_PX = 48;

/** The audit trail as it streams in. Follows the newest line unless the user scrolls up. */
export function ActivityFeed({ logs }: { logs: LogEntry[] | undefined }) {
  const scroller = useRef<HTMLDivElement>(null);
  const [pinned, setPinned] = useState(true);
  const count = logs?.length ?? 0;

  // biome-ignore lint/correctness/useExhaustiveDependencies: re-run when a line arrives
  useEffect(() => {
    const element = scroller.current;
    if (element && pinned) element.scrollTop = element.scrollHeight;
  }, [count, pinned]);

  const scrollToLatest = () => {
    const element = scroller.current;
    if (element) element.scrollTop = element.scrollHeight;
    setPinned(true);
  };

  return (
    <div className="relative flex min-h-0 flex-1 flex-col">
      {logs === undefined ? (
        <div className="flex flex-col gap-3 p-4">
          {[72, 56, 64, 40].map((width) => (
            <Skeleton key={width} className="h-4" style={{ width: `${width}%` }} />
          ))}
        </div>
      ) : logs.length === 0 ? (
        <p className="shimmer p-4 text-[13px]">Waiting for the first event…</p>
      ) : (
        <LogList
          ref={scroller}
          role="log"
          aria-label="Activity"
          aria-live="off"
          entries={logs}
          showNode
          liveTail
          onScroll={(event) => {
            const element = event.currentTarget;
            const distance = element.scrollHeight - element.scrollTop - element.clientHeight;
            setPinned(distance < PINNED_THRESHOLD_PX);
          }}
          className="min-h-0 flex-1"
        />
      )}
      {pinned ? null : (
        <Button
          variant="outline"
          size="sm"
          onClick={scrollToLatest}
          className="absolute bottom-3 left-1/2 -translate-x-1/2 rounded-full shadow-pop"
        >
          <ArrowDownIcon />
          Jump to latest
        </Button>
      )}
    </div>
  );
}
