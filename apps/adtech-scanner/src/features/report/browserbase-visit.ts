import type { BrowserUse, LogEntry, RemoteUsage, ScanDetail } from "@/bindings";
import {
  BROWSERBASE_LIST_PRICE,
  BROWSERBASE_PLAN_CAVEAT,
  browserbaseUsd,
  countryName,
  formatTraffic,
  regionName,
  regionOptionLabel,
  webdriverReading,
} from "@/shared/domain/browserbase";
import { visitorActivity } from "@/shared/domain/visitor-log";
import { formatDuration, formatUsd, pluralize } from "@/shared/lib/format";

type BrowserbaseVisit = Extract<BrowserUse, { kind: "browserbase" }>;

/** Browserbase reads a session's proxy traffic again half a minute after its end; until then a zero says nothing. */
const USAGE_SETTLES_AFTER_MS = 60_000;

const UNREAD = "Not read";
const UNREPORTED = "Not reported by Browserbase";

/**
 * Until when the proxy traffic of a Browserbase visit is still being counted: nothing was read yet, or the
 * first read came back empty and the second is due. `null` when there is nothing to wait for. The wait ends
 * a minute after the scan whatever was read: a count that never comes is "not reported", not "being counted".
 */
export function proxyUsageSettlesAt(
  visit: Pick<BrowserbaseVisit, "proxyKind">,
  usage: Pick<RemoteUsage, "proxyBytes"> | null,
  finishedAt: number | null,
): number | null {
  if (visit.proxyKind !== "browserbase") return null;
  if (usage?.proxyBytes) return null;
  return finishedAt === null ? Number.POSITIVE_INFINITY : finishedAt + USAGE_SETTLES_AFTER_MS;
}

/** Whether the proxy traffic of a finished Browserbase visit is still being counted at `now`. */
export function proxyUsagePending(
  visit: Pick<BrowserbaseVisit, "proxyKind">,
  usage: Pick<RemoteUsage, "proxyBytes"> | null,
  finishedAt: number | null,
  now: number,
): boolean {
  const settlesAt = proxyUsageSettlesAt(visit, usage, finishedAt);
  return settlesAt !== null && now < settlesAt;
}

/** The usage of a scan's Browserbase browser as it stands: the summary's, which the late second read updates. */
function usageOf(detail: Pick<ScanDetail, "summary">, visit: BrowserbaseVisit): RemoteUsage {
  return detail.summary.remote ?? visit;
}

/**
 * Until when a report has to keep looking at the clock for its Browserbase usage; `null` for a visit on this
 * computer and once the usage is final.
 */
export function browserbaseUsageSettlesAt(detail: Pick<ScanDetail, "environment" | "summary">): number | null {
  const browser = detail.environment?.browser;
  if (browser?.kind !== "browserbase") return null;
  return proxyUsageSettlesAt(browser, usageOf(detail, browser), detail.summary.finishedAt);
}

/** What only the audit trail knows about a scan's Browserbase browser. */
export type BrowserbaseLogFacts = {
  /** How many scripts of its own Browserbase ran in the pages; `null` when the capture did not say. */
  serviceScripts: number | null;
  /** Browserbase did not confirm that the browser ended. */
  releaseUnconfirmed: boolean;
};

const NO_LOG_FACTS: BrowserbaseLogFacts = { serviceScripts: null, releaseUnconfirmed: false };

export function browserbaseLogFacts(logs: LogEntry[] | undefined): BrowserbaseLogFacts {
  const facts = { ...NO_LOG_FACTS };
  for (const entry of logs ?? []) {
    const activity = visitorActivity(entry);
    if (activity?.activity !== "browser") continue;
    if (activity.event === "serviceScripts") facts.serviceScripts = activity.count;
    if (activity.event === "releaseUnconfirmed") facts.releaseUnconfirmed = true;
    // A later line that the browser has ended settles it.
    if (activity.event === "released") facts.releaseUnconfirmed = false;
  }
  return facts;
}

export type BrowserbaseVisitFacts = {
  /** The line that is always shown: time, traffic and what that costs. */
  summary: string[];
  /** Everything recorded about the browser, as name and value. */
  rows: { name: string; value: string; mono?: boolean }[];
  /** The proxy traffic is not final yet. */
  pending: boolean;
  /** What may still be costing money: the browser's end was never confirmed. */
  warning: string | null;
};

function leftFrom(visit: BrowserbaseVisit, proxyName: string | null): string {
  if (visit.proxyKind === "external") return `Your proxy${proxyName ? ` “${proxyName}”` : ""}, through Browserbase`;
  if (visit.proxyKind === "browserbase") {
    const country = visit.proxyCountry ? countryName(visit.proxyCountry) : "a country";
    return `Browserbase proxy: a residential connection, ${country} requested (best effort)`;
  }
  return `Browserbase's ${regionName(visit.region)} region, a data centre address (no proxy)`;
}

function timezone(visit: BrowserbaseVisit): string {
  const { originalTimezone: original, appliedTimezone: applied } = visit;
  if (applied) {
    return `${applied}, set by the scanner to match the region${original ? ` (the browser reported ${original})` : ""}`;
  }
  if (!original) return UNREAD;
  return visit.proxyKind === "browserbase" ? `${original}, set by Browserbase for its proxy` : original;
}

/**
 * What a report says about a visit that ran in a Browserbase browser: where it ran and left from, what it
 * used, and what the pages could read. `null` for a visit on this computer and for scans without a recorded
 * environment. Usage comes from the summary, which the late second read of the proxy traffic updates.
 */
export function browserbaseVisitFacts(
  detail: Pick<ScanDetail, "environment" | "summary">,
  now: number,
  log: BrowserbaseLogFacts = NO_LOG_FACTS,
): BrowserbaseVisitFacts | null {
  const { environment, summary } = detail;
  if (environment?.browser.kind !== "browserbase") return null;
  const visit = environment.browser;
  const usage = usageOf(detail, visit);
  const pending = proxyUsagePending(visit, usage, summary.finishedAt, now);
  const proxied = visit.proxyKind === "browserbase";
  const kept = [visit.settings.recordSession ? "a video" : null, visit.settings.logSession ? "a log" : null]
    .filter((part) => part !== null)
    .join(" and ");

  const time = usage.sessionSeconds === null ? null : formatDuration(usage.sessionSeconds * 1000);
  const minutes = usage.sessionSeconds === null ? null : Math.max(1, Math.ceil(usage.sessionSeconds / 60));
  // Final and known: `null` while it is being counted, and when Browserbase never said.
  const proxyBytes = proxied ? (pending ? null : usage.proxyBytes) : 0;
  let traffic = "None: the visit used no Browserbase proxy";
  if (proxied && pending) traffic = "Still being counted by Browserbase";
  else if (proxied) traffic = proxyBytes === null ? UNREPORTED : formatTraffic(proxyBytes);

  const cost =
    usage.sessionSeconds === null || proxyBytes === null
      ? null
      : formatUsd(browserbaseUsd(usage.sessionSeconds, proxyBytes));
  let costRow = `${cost}, an estimate at ${BROWSERBASE_LIST_PRICE}; ${BROWSERBASE_PLAN_CAVEAT}. Not part of the LLM cost`;
  if (pending) costRow = "Known once the proxy traffic is counted";
  else if (cost === null) {
    costRow = `Not known: Browserbase did not report the ${usage.sessionSeconds === null ? "browser time" : "proxy traffic"}`;
  }

  const ownScripts =
    log.serviceScripts === null ? "Ran in every page" : `${pluralize(log.serviceScripts, "script")} ran in the pages`;

  let proxySummary: string[] = [];
  if (proxied && pending) proxySummary = ["proxy traffic still being counted"];
  else if (proxied) {
    proxySummary = [
      proxyBytes === null ? "proxy traffic not reported" : `${formatTraffic(proxyBytes)} through the proxy`,
    ];
  }

  return {
    pending,
    warning: log.releaseUnconfirmed
      ? "Browserbase did not confirm that this browser ended. It may have kept running, and been billed, until its time limit."
      : null,
    summary: [
      "Browserbase",
      ...(time ? [`${time} of browser time`] : []),
      ...proxySummary,
      ...(cost ? [`about ${cost} at list price`] : []),
    ],
    rows: [
      { name: "Browser ran in", value: `Browserbase, ${regionOptionLabel(visit.region)}` },
      { name: "Left from", value: leftFrom(visit, environment.proxy.name) },
      {
        name: "Browser time",
        value: time ? `${time}, billed as ${minutes} ${minutes === 1 ? "minute" : "minutes"}` : UNREPORTED,
      },
      { name: "Proxy traffic", value: traffic },
      { name: "Estimated cost", value: costRow },
      { name: "Timezone the pages saw", value: timezone(visit) },
      { name: "Language", value: visit.language ? `${visit.language}, as the browser reported it` : UNREAD },
      {
        name: "navigator.webdriver",
        value: environment.webdriver === null ? UNREAD : webdriverReading(environment.webdriver, "the pages were"),
      },
      { name: "User agent", value: visit.userAgent ?? "Not recorded", mono: true },
      {
        name: "Browser",
        value: `${visit.browserVersion ?? "Version not reported"}, headless, which the user agent does not show`,
      },
      {
        name: "Browserbase's own scripts",
        value: `${ownScripts}; left out of this scan's scripts and results`,
      },
      {
        name: "CAPTCHAs",
        value: visit.settings.solveCaptchas ? "Solved by Browserbase automatically" : "Left unsolved",
      },
      ...(visit.roundTripMs === null
        ? []
        : [
            {
              name: "Round trip",
              value: `${Math.round(visit.roundTripMs)} ms per command between this Mac and the browser; frames and requests the scanner held for a moment waited that long`,
            },
          ]),
      { name: "Kept at Browserbase", value: kept ? `${kept.charAt(0).toUpperCase()}${kept.slice(1)}` : "Nothing" },
      { name: "Session", value: visit.sessionId, mono: true },
    ],
  };
}
