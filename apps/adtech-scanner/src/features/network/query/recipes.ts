/** Filters worth a click: the questions an ad-tech review asks of a page's traffic. */
export const RECIPES: readonly { label: string; query: string }[] = [
  { label: "Bid calls that failed or came late", query: "cat:bid is:failed,late" },
  { label: "Ad requests without Prebid keys", query: "cat:adserver is:no-hb-targeting" },
  { label: "Everything before the first auction", query: "is:before-auction" },
  { label: "Slow third parties", query: "is:third-party slower-than:1s" },
  { label: "Large responses", query: "larger-than:500k" },
];

const MAX_RECENTS = 8;

/** The recent filters with `query` in front: no duplicates, no empty ones, the last eight. */
export function pushRecent(recents: readonly string[], query: string): string[] {
  const trimmed = query.trim();
  if (trimmed === "") return [...recents];
  return [trimmed, ...recents.filter((recent) => recent !== trimmed)].slice(0, MAX_RECENTS);
}

export function isStringList(value: unknown): value is string[] {
  return Array.isArray(value) && value.every((item) => typeof item === "string");
}
