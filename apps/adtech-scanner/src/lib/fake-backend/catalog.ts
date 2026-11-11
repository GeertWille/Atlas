import type { ScriptCategory } from "@/bindings";

export type BidderProfile = {
  code: string;
  /** Typical round-trip latency range in ms. */
  latency: [number, number];
  bidRate: number;
  cpm: [number, number];
};

export const BIDDERS: Record<string, BidderProfile> = {
  appnexus: { code: "appnexus", latency: [110, 340], bidRate: 0.74, cpm: [0.42, 3.1] },
  rubicon: { code: "rubicon", latency: [140, 420], bidRate: 0.7, cpm: [0.38, 3.6] },
  pubmatic: { code: "pubmatic", latency: [160, 480], bidRate: 0.66, cpm: [0.3, 2.9] },
  ix: { code: "ix", latency: [180, 620], bidRate: 0.58, cpm: [0.45, 4.2] },
  criteo: { code: "criteo", latency: [90, 260], bidRate: 0.62, cpm: [0.25, 2.4] },
  openx: { code: "openx", latency: [150, 520], bidRate: 0.52, cpm: [0.28, 2.2] },
  triplelift: { code: "triplelift", latency: [210, 760], bidRate: 0.44, cpm: [0.5, 3.9] },
  sovrn: { code: "sovrn", latency: [170, 560], bidRate: 0.4, cpm: [0.18, 1.6] },
  sharethrough: { code: "sharethrough", latency: [200, 680], bidRate: 0.38, cpm: [0.35, 2.8] },
};

export type ScriptTemplate = {
  /** `{host}` is replaced with the scanned hostname. */
  url: string;
  category: ScriptCategory;
  vendor: string | null;
  sizeKb: number;
  /** A known-vendor rule classifies it without the model. */
  byRule: boolean;
  reason: string;
  inline?: boolean;
  /** Handed to Phase 2 when AI runs. */
  analyzed?: boolean;
};

export const SCRIPT_TEMPLATES: ScriptTemplate[] = [
  { url: "https://{host}/static/js/runtime.1c9e2b.js", category: "application", vendor: null, sizeKb: 3.1, byRule: false, reason: "Webpack runtime bootstrap; no ad APIs referenced." },
  { url: "https://{host}/static/js/main.4f2a9c.js", category: "application", vendor: null, sizeKb: 412.6, byRule: false, reason: "Site application bundle (routing, article rendering). Mentions ad containers only as DOM placeholders." },
  { url: "https://{host}/static/js/vendors.react.8d1f03.js", category: "library", vendor: null, sizeKb: 138.4, byRule: false, reason: "React and react-dom vendor chunk." },
  { url: "https://{host}/static/js/chunk-article.88127a.js", category: "application", vendor: null, sizeKb: 64.9, byRule: false, reason: "Article page chunk: gallery, related stories, share bar." },
  { url: "https://{host}/static/js/chunk-comments.03be71.js", category: "application", vendor: null, sizeKb: 41.2, byRule: false, reason: "Comment widget loader." },
  { url: "https://{host}/static/js/paywall.5aa0c4.js", category: "application", vendor: null, sizeKb: 28.7, byRule: false, reason: "Metered paywall logic; toggles an `adsDisabled` flag for subscribers but does not request ads." },
  { url: "https://{host}/assets/ads/hb-wrapper.min.js", category: "wrapper", vendor: null, sizeKb: 48.2, byRule: false, analyzed: true, reason: "Publisher header-bidding wrapper: builds ad units, calls pbjs.requestBids and googletag.pubads().refresh, contains device and lazy-load gating." },
  { url: "https://{host}/assets/ads/ad-config.js", category: "wrapper", vendor: null, sizeKb: 11.8, byRule: false, analyzed: true, reason: "Ad unit and bidder parameter map consumed by the wrapper." },
  { url: "inline://{host}/#script-4", category: "wrapper", vendor: null, sizeKb: 1.9, byRule: false, inline: true, analyzed: true, reason: "Inline bootstrap: defines googletag.cmd / pbjs.que and sets page-level targeting." },
  { url: "inline://{host}/#script-7", category: "analytics", vendor: null, sizeKb: 0.8, byRule: false, inline: true, reason: "Inline dataLayer push for page metadata." },
  { url: "https://cdn.{host}/prebid/prebid.js", category: "prebid", vendor: "Prebid.org", sizeKb: 318.5, byRule: true, analyzed: true, reason: "Matches Prebid.js build signature (pbjs.version)." },
  { url: "https://securepubads.g.doubleclick.net/tag/js/gpt.js", category: "adServer", vendor: "Google", sizeKb: 31.4, byRule: true, reason: "Google Publisher Tag loader." },
  { url: "https://securepubads.g.doubleclick.net/pagead/managed/js/gpt/m202611050101/pubads_impl.js", category: "adServer", vendor: "Google", sizeKb: 486.2, byRule: true, reason: "Google Publisher Tag implementation." },
  { url: "https://c.amazon-adsystem.com/aax2/apstag.js", category: "bidder", vendor: "Amazon", sizeKb: 62.3, byRule: true, reason: "Amazon Publisher Services (TAM) library." },
  { url: "https://static.criteo.net/js/ld/publishertag.prebid.144.js", category: "bidder", vendor: "Criteo", sizeKb: 89.1, byRule: true, reason: "Criteo Prebid fast-bid adapter." },
  { url: "https://acdn.adnxs.com/dmp/async_usersync.js", category: "bidder", vendor: "Xandr", sizeKb: 6.4, byRule: true, reason: "Xandr (AppNexus) user sync." },
  { url: "https://eus.rubiconproject.com/usync.js", category: "bidder", vendor: "Magnite", sizeKb: 9.7, byRule: true, reason: "Magnite (Rubicon) user sync." },
  { url: "https://ads.pubmatic.com/AdServer/js/pwtSync/load-cookie.js", category: "bidder", vendor: "PubMatic", sizeKb: 7.2, byRule: true, reason: "PubMatic cookie sync." },
  { url: "https://js-sec.indexww.com/ht/p/184932-58217594.js", category: "bidder", vendor: "Index Exchange", sizeKb: 74.8, byRule: true, reason: "Index Exchange library." },
  { url: "https://cdn.taboola.com/libtrc/publisher-network/loader.js", category: "bidder", vendor: "Taboola", sizeKb: 97.3, byRule: true, reason: "Taboola content recommendation loader." },
  { url: "https://cdn.cookielaw.org/scripttemplates/otSDKStub.js", category: "cmp", vendor: "OneTrust", sizeKb: 22.6, byRule: true, reason: "OneTrust CMP stub." },
  { url: "https://cdn.cookielaw.org/scripttemplates/202410.1.0/otBannerSdk.js", category: "cmp", vendor: "OneTrust", sizeKb: 441.9, byRule: true, reason: "OneTrust banner SDK." },
  { url: "https://cdn.doubleverify.com/dvtp_src.js", category: "adQuality", vendor: "DoubleVerify", sizeKb: 58.6, byRule: true, reason: "DoubleVerify verification tag." },
  { url: "https://static.adsafeprotected.com/iasPET.1.js", category: "adQuality", vendor: "IAS", sizeKb: 35.2, byRule: true, reason: "Integral Ad Science publisher optimization tag." },
  { url: "https://cdn.id5-sync.com/api/1.0/id5-api.js", category: "adQuality", vendor: "ID5", sizeKb: 44.1, byRule: true, reason: "ID5 identity module." },
  { url: "https://www.googletagmanager.com/gtm.js?id=GTM-K4T8XQ", category: "analytics", vendor: "Google", sizeKb: 112.3, byRule: true, reason: "Google Tag Manager container." },
  { url: "https://www.googletagmanager.com/gtag/js?id=G-7H2M4XW1PQ", category: "analytics", vendor: "Google", sizeKb: 96.8, byRule: true, reason: "Google Analytics 4." },
  { url: "https://sb.scorecardresearch.com/beacon.js", category: "analytics", vendor: "Comscore", sizeKb: 4.9, byRule: true, reason: "Comscore audience beacon." },
  { url: "https://cdn.segment.com/analytics.js/v1/kT2x9aQe/analytics.min.js", category: "analytics", vendor: "Segment", sizeKb: 71.5, byRule: true, reason: "Segment analytics.js." },
  { url: "https://static.chartbeat.com/js/chartbeat_mab.js", category: "analytics", vendor: "Chartbeat", sizeKb: 38.4, byRule: true, reason: "Chartbeat headline testing." },
  { url: "https://cdn.jsdelivr.net/npm/swiper@11/swiper-bundle.min.js", category: "library", vendor: null, sizeKb: 148.7, byRule: false, reason: "Swiper carousel library." },
  { url: "https://cdn.jsdelivr.net/npm/lazysizes@5.3.2/lazysizes.min.js", category: "library", vendor: null, sizeKb: 7.9, byRule: false, reason: "Image lazy-loading library; unrelated to ad slots." },
  { url: "https://connect.facebook.net/en_US/sdk.js", category: "library", vendor: "Meta", sizeKb: 3.2, byRule: true, reason: "Facebook SDK for embeds." },
  { url: "https://platform.twitter.com/widgets.js", category: "library", vendor: "X", sizeKb: 93.6, byRule: true, reason: "X/Twitter embed widgets." },
  { url: "https://cdn.brid.tv/player/build/brid.min.js", category: "library", vendor: "Brid.TV", sizeKb: 356.1, byRule: false, reason: "Video player. Contains an IMA wrapper but video ads are out of scope for display wrapper analysis." },
  { url: "https://d2x3bkdslnxkuj.cloudfront.net/t.js", category: "unknown", vendor: null, sizeKb: 2.3, byRule: false, reason: "Minified beacon with no recognizable vendor signature." },
];

export const PREBID_VERSIONS = {
  outdated: "8.52.0",
  recent: "9.27.0",
  current: "9.41.0",
} as const;
