import type { NetworkCategory, RequestOutcome, ResourceKind, ServedFrom } from "@/bindings";

/** Categories in the order the chips show them. `token` is the value of `cat:` in a filter. */
export const CATEGORIES: readonly NetworkCategory[] = [
  "bid",
  "adServer",
  "wrapper",
  "consent",
  "identity",
  "measurement",
  "creative",
  "site",
  "otherThirdParty",
];

/** `short` is the chip's wording, where nine of them share one row. */
export const CATEGORY_META: Record<
  NetworkCategory,
  { label: string; short: string; token: string; description: string }
> = {
  bid: {
    label: "Bid requests",
    short: "Bids",
    token: "bid",
    description: "Calls to bidders' and Prebid Server's auction endpoints",
  },
  adServer: { label: "Ad server", short: "Ad server", token: "adserver", description: "The ad server's ad calls" },
  wrapper: {
    label: "Wrapper and Prebid",
    short: "Wrapper",
    token: "wrapper",
    description: "Prebid, the publisher's wrapper and the ad libraries themselves",
  },
  consent: { label: "Consent", short: "Consent", token: "consent", description: "The consent platform" },
  identity: {
    label: "Identity and sync",
    short: "Identity",
    token: "identity",
    description: "User id modules and cookie syncs",
  },
  measurement: {
    label: "Measurement",
    short: "Measurement",
    token: "measurement",
    description: "Analytics, viewability, verification, beacons",
  },
  creative: {
    label: "Creative",
    short: "Creative",
    token: "creative",
    description: "What an ad loads once it renders",
  },
  site: { label: "First-party", short: "First-party", token: "site", description: "The scanned site itself" },
  otherThirdParty: {
    label: "Other",
    short: "Other",
    token: "other",
    description: "Third parties with no ad-tech role known to the app",
  },
};

/** The short type names of the Type column. */
export const KIND_LABEL: Record<ResourceKind, string> = {
  document: "doc",
  script: "script",
  stylesheet: "css",
  image: "img",
  media: "media",
  font: "font",
  xhr: "xhr",
  fetch: "fetch",
  ping: "ping",
  webSocket: "ws",
  eventSource: "sse",
  preflight: "preflight",
  other: "other",
};

export const OUTCOME_LABEL: Record<RequestOutcome, string> = {
  ok: "Answered",
  httpError: "Error status",
  failed: "Failed",
  blocked: "Blocked",
  cancelled: "Cancelled",
  pending: "No response",
};

/** What stands in the Size column when nothing crossed the network; `null` for a real transfer. */
export const SERVED_FROM_NOTE: Record<ServedFrom, string | null> = {
  network: null,
  diskCache: "(disk cache)",
  memoryCache: "(memory cache)",
  serviceWorker: "(service worker)",
  prefetch: "(prefetch)",
};

export const SERVED_FROM_LABEL: Record<ServedFrom, string> = {
  network: "The network",
  diskCache: "The disk cache",
  memoryCache: "The memory cache",
  serviceWorker: "A service worker",
  prefetch: "The prefetch cache",
};
