import { CircleStopIcon, ExternalLinkIcon, MousePointerClickIcon } from "lucide-react";
import type { ReactNode } from "react";
import { toast } from "sonner";
import { useFinishCapture, useOpenBrowserbaseLiveView } from "@/lib/query/hooks";
import { MANUAL_SESSION_LIMIT_MS } from "@/shared/domain/scan-options";
import { cn } from "@/shared/lib/cn";
import { pluralize } from "@/shared/lib/format";
import { notifyError } from "@/shared/lib/notify-error";
import { Button } from "@/shared/ui/button";
import { formatTimer } from "./visit";

/** A manual visit while it is being recorded: how long it has run, what it has seen, and how to end it. */
export function ManualSessionBar({
  scanId,
  hostname,
  startedAt,
  now,
  pageCount,
  recording,
  browserbase,
  cancel,
}: {
  scanId: string;
  hostname: string;
  /** When the recording began. */
  startedAt: number;
  now: number;
  pageCount: number;
  /**
   * Chrome has opened the page. Before that the backend does not listen for "finish" yet, so the
   * button would be spent on a request that ends nothing.
   */
  recording: boolean;
  /** The visit runs in a Browserbase browser, which is used through its live view. */
  browserbase: boolean;
  /** The control that discards the scan instead. */
  cancel: ReactNode;
}) {
  const finishCapture = useFinishCapture();
  const openLiveView = useOpenBrowserbaseLiveView();
  const copy = browserbase
    ? {
        opening: `Starting a Browserbase browser for ${hostname}`,
        browsing:
          "Use Browserbase's live view, which opened in your own browser. Pages of this site are recorded as you open them; other sites are not. Browserbase bills the session until you finish.",
      }
    : {
        opening: `Opening Chrome for ${hostname}`,
        browsing:
          "Use the Chrome window that opened. Pages of this site are recorded as you open them; other sites are not.",
      };
  const elapsedMs = Math.max(0, now - startedAt);
  // The request went through: the bar stays until the capture step ends, with its button spent.
  const finishing = finishCapture.isPending || finishCapture.isSuccess;
  return (
    <section
      aria-label="Manual session"
      className="flex flex-wrap items-center gap-x-5 gap-y-2 rounded-xl border border-violet-line bg-primary-soft/60 px-4 py-3"
    >
      {/* With the live-view button beside the others, the text keeps a readable width and the controls wrap below it. */}
      <div className={cn("flex flex-1 items-start gap-2.5", browserbase ? "min-w-[26rem]" : "min-w-72")}>
        <MousePointerClickIcon aria-hidden="true" className="mt-0.5 size-4 shrink-0 text-primary-text" />
        <div className="flex min-w-0 flex-col gap-0.5">
          <h2 className="text-sm font-semibold">{recording ? `You are browsing ${hostname}` : copy.opening}</h2>
          <p className="text-[13px] text-pretty text-muted-foreground">
            {recording
              ? copy.browsing
              : "Recording starts when the page has opened. The session can last up to 15 minutes from now."}
          </p>
        </div>
      </div>
      <dl className="flex shrink-0 items-center gap-5 text-[13px] tabular-nums">
        <div className="flex flex-col">
          <dt className="text-xs text-muted-foreground">Elapsed</dt>
          <dd className="font-medium">{formatTimer(elapsedMs)}</dd>
        </div>
        <div className="flex flex-col">
          <dt className="text-xs text-muted-foreground">Time left</dt>
          <dd className="font-medium">{formatTimer(MANUAL_SESSION_LIMIT_MS - elapsedMs)}</dd>
        </div>
        <div className="flex flex-col">
          <dt className="text-xs text-muted-foreground">Recorded</dt>
          <dd className="font-medium">{pluralize(pageCount, "page")}</dd>
        </div>
      </dl>
      <div className="flex shrink-0 items-center gap-2">
        {browserbase ? (
          <Button
            variant="outline"
            disabled={!recording || finishing || openLiveView.isPending}
            onClick={() =>
              openLiveView.mutate(scanId, {
                onSuccess: () => toast.success("Live view opened in your browser"),
                onError: (reason) => notifyError("Could not open the live view", reason),
              })
            }
          >
            <ExternalLinkIcon />
            Open live view
          </Button>
        ) : null}
        <Button
          disabled={finishing || !recording}
          onClick={() =>
            finishCapture.mutate(scanId, { onError: (reason) => notifyError("Could not finish the capture", reason) })
          }
        >
          <CircleStopIcon />
          {finishing ? "Finishing…" : "Finish capture"}
        </Button>
        {cancel}
      </div>
    </section>
  );
}
