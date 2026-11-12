import type { PageReach, PageView } from "@/bindings";
import { pathOf } from "@/shared/lib/url";

export const PAGE_REACH_LABEL: Record<PageReach, string> = {
  entry: "Entry page",
  linkClick: "Followed link",
  routeChange: "In-page navigation",
  userNavigation: "You navigated",
  redirect: "Redirect",
  reload: "Reload",
};

/** Path of a page view for compact labels; the bare origin reads "/". */
export function pagePath(url: string): string {
  return pathOf(url) || "/";
}

/** "2 /climate/kelp-carbon": page views are numbered from 1 for people. */
export function pageLabel(page: Pick<PageView, "index" | "url">): string {
  return `${page.index + 1} ${pagePath(page.url)}`;
}
