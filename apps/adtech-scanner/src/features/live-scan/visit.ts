import type { LogEntry, PageReach } from "@/bindings";
import { describeSkipped, visitorActivity } from "@/shared/domain/visitor-log";

type VisitPage = {
  index: number;
  url: string;
  title: string | null;
  reachedBy: PageReach;
  /** Text of the link that led here, when the scanner followed one. */
  linkText: string | null;
};

export type VisitState = {
  /** Page views so far, in the order they opened. */
  pages: VisitPage[];
  /** The page the browser is on now. */
  current: VisitPage | null;
  /** The newest things the visitor did, oldest first. */
  actions: { id: string; message: string; warning: boolean }[];
  /** What the last link choice would not follow, e.g. "Skipped 14 links: 9 off-site, 3 inside ads". */
  skipped: string | null;
};

const ACTIONS_SHOWN = 3;

/** The state of a visit as far as the audit trail has told it: nothing here is known from anywhere else. */
export function deriveVisit(logs: LogEntry[]): VisitState {
  const pages = new Map<number, VisitPage>();
  const actions: VisitState["actions"] = [];
  let skipped: string | null = null;

  for (const entry of logs) {
    const activity = visitorActivity(entry);
    if (!activity) continue;
    switch (activity.activity) {
      case "pageView":
        // A page view that is announced again (a reload replacing it) keeps its place.
        pages.set(activity.index, {
          index: activity.index,
          url: activity.url,
          title: activity.title,
          reachedBy: activity.reachedBy,
          linkText: activity.linkText,
        });
        break;
      case "environment":
        // Said once in the activity feed; the panel follows where the visit is, not what it is.
        break;
      case "linkChoice": {
        const count = activity.skipped.reduce((sum, { count }) => sum + count, 0);
        skipped =
          count > 0 ? `Skipped ${count} link${count === 1 ? "" : "s"}: ${describeSkipped(activity.skipped)}` : null;
        actions.push({ id: entry.id, message: entry.message, warning: false });
        break;
      }
      case "action":
      case "blocked":
        actions.push({ id: entry.id, message: entry.message, warning: activity.activity === "blocked" });
        break;
    }
  }

  const ordered = [...pages.values()].sort((a, b) => a.index - b.index);
  return {
    pages: ordered,
    current: ordered[ordered.length - 1] ?? null,
    actions: actions.slice(-ACTIONS_SHOWN),
    skipped,
  };
}

/** A countdown or stopwatch reading: "02:14". */
export function formatTimer(ms: number): string {
  const totalSeconds = Math.max(0, Math.floor(ms / 1000));
  const minutes = Math.floor(totalSeconds / 60);
  return `${String(minutes).padStart(2, "0")}:${String(totalSeconds % 60).padStart(2, "0")}`;
}
