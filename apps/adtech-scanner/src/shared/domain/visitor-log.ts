import type { LogEntry, PageReach } from "@/bindings";

export type LinkCandidate = { text: string; href: string; score: number | null };

export type VisitorActivity =
  | {
      activity: "pageView";
      page: number | null;
      index: number;
      url: string;
      title: string | null;
      reachedBy: PageReach;
      linkText: string | null;
    }
  | { activity: "action"; page: number | null; action: string | null; scrollDepth: number | null }
  | {
      activity: "linkChoice";
      page: number | null;
      /** `null`: no link on the page was worth following. */
      chosen: (LinkCandidate & { reasons: string[] }) | null;
      alternatives: LinkCandidate[];
      /** Skip reason and how many links it ruled out, most frequent first. */
      skipped: { reason: string; count: number }[];
    }
  | { activity: "blocked"; page: number | null; reason: string | null; host: string | null }
  | { activity: "environment"; page: number | null; webdriver: boolean | null; headless: boolean | null };

const PAGE_REACHES: readonly PageReach[] = [
  "entry",
  "linkClick",
  "routeChange",
  "userNavigation",
  "redirect",
  "reload",
];

/** What the scanner calls each reason it would not follow a link, in words a report reader knows. */
export const SKIP_REASON_LABEL: Record<string, string> = {
  hidden: "hidden",
  "inside-ad": "inside ads",
  scheme: "not a web link",
  anchor: "same page",
  "off-site": "off-site",
  file: "file downloads",
  rel: "nofollow or sponsored",
  sensitive: "sign-in, checkout and the like",
  control: "form controls",
  visited: "already visited",
};

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

const text = (value: unknown): string | null => (typeof value === "string" ? value : null);
const number = (value: unknown): number | null => (typeof value === "number" && Number.isFinite(value) ? value : null);
const flag = (value: unknown): boolean | null => (typeof value === "boolean" ? value : null);

function candidate(value: unknown): LinkCandidate | null {
  if (!isRecord(value) || typeof value.href !== "string") return null;
  return { text: text(value.text) ?? "", href: value.href, score: number(value.score) };
}

/**
 * The structured part of a `visitor` entry. The audit data is not part of the typed contract, so every
 * field is read defensively; an entry that does not have the expected shape is just a message.
 */
export function visitorActivity(entry: LogEntry): VisitorActivity | null {
  if (entry.kind !== "visitor" || !isRecord(entry.data)) return null;
  const page = number(entry.data.page);
  const detail = isRecord(entry.data.detail) ? entry.data.detail : {};
  switch (entry.data.activity) {
    case "pageView": {
      const index = number(detail.index) ?? page;
      const reachedBy = PAGE_REACHES.find((reach) => reach === detail.reachedBy);
      if (index === null || typeof detail.url !== "string" || !reachedBy) return null;
      const link = isRecord(detail.link) ? detail.link : {};
      return {
        activity: "pageView",
        page,
        index,
        url: detail.url,
        title: text(detail.title),
        reachedBy,
        linkText: text(link.text),
      };
    }
    case "action":
      return { activity: "action", page, action: text(detail.action), scrollDepth: number(detail.scrollDepth) };
    case "linkChoice": {
      const chosen = candidate(detail.chosen);
      const reasons = isRecord(detail.chosen) && Array.isArray(detail.chosen.reasons) ? detail.chosen.reasons : [];
      return {
        activity: "linkChoice",
        page,
        chosen: chosen && { ...chosen, reasons: reasons.filter((reason) => typeof reason === "string") },
        alternatives: Array.isArray(detail.alternatives)
          ? detail.alternatives.flatMap((alternative) => candidate(alternative) ?? [])
          : [],
        skipped: Object.entries(isRecord(detail.skipped) ? detail.skipped : {})
          .flatMap(([reason, count]) => (typeof count === "number" && count > 0 ? [{ reason, count }] : []))
          .sort((a, b) => b.count - a.count),
      };
    }
    case "blocked":
      return { activity: "blocked", page, reason: text(detail.reason), host: text(detail.host) };
    case "environment":
      return { activity: "environment", page, webdriver: flag(detail.webdriver), headless: flag(detail.headless) };
    default:
      return null;
  }
}

/** "9 off-site, 3 inside ads, 2 sign-in, checkout and the like". */
export function describeSkipped(skipped: { reason: string; count: number }[]): string {
  return skipped.map(({ reason, count }) => `${count} ${SKIP_REASON_LABEL[reason] ?? reason}`).join(", ");
}
