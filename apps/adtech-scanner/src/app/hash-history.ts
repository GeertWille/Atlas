import { createBrowserHistory, parseHref, type RouterHistory } from "@tanstack/history";

/**
 * Hash history whose route (path and search params) lives entirely in the URL
 * fragment. TanStack's `createHashHistory` takes the search params from the
 * document's real query string instead, so with `/?ref=x#/scans/a?tab=audit`
 * the route's own params are mangled and lost.
 */
export function createFragmentHistory(): RouterHistory {
  return createBrowserHistory({
    parseLocation: () => parseHref(window.location.hash.slice(1) || "/", window.history.state),
    createHref: (href) => `${window.location.pathname}${window.location.search}#${href}`,
  });
}
