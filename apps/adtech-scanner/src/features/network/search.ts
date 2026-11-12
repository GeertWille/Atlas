/** How the request list is grouped. `none` is one flat list. */
export const NETWORK_GROUPINGS = ["page", "vendor", "type", "none"] as const;
export type NetworkGrouping = (typeof NETWORK_GROUPINGS)[number];

export const NETWORK_VIEWS = ["requests", "bids"] as const;
export type NetworkView = (typeof NETWORK_VIEWS)[number];

export const DETAIL_TABS = ["summary", "headers", "payload", "response", "timing", "initiator"] as const;
export type DetailTab = (typeof DETAIL_TABS)[number];

/**
 * Everything about the workbench that a link can carry. Filter, selection and window are all here,
 * so a reload, Back and a pasted link show the same thing.
 */
export type NetworkSearch = {
  /** The filter, in the query language. */
  q?: string;
  /** The page view shown; shared with the report. */
  page?: number;
  group?: NetworkGrouping;
  /** Column key, with a leading `-` for descending. */
  sort?: string;
  /** The selected request. */
  req?: string;
  dt?: DetailTab;
  view?: NetworkView;
  /** The auction the bid exchanges view shows. */
  auction?: string;
  /** The time window in ms on the strip's axis. */
  from?: number;
  to?: number;
};
