import type { BrowserUse, RemoteUsage, ScanDetail } from "@/bindings";
import {
  BROWSERBASE_RATES,
  browserbaseUsd,
  countryName,
  formatTraffic,
  regionOptionLabel,
  regionPlace,
} from "@/shared/domain/browserbase";
import { formatDuration, formatUsd } from "@/shared/lib/format";

type BrowserbaseVisit = Extract<BrowserUse, { kind: "browserbase" }>;

/** Browserbase reads a session's proxy traffic again half a minute after its end; until then a zero says nothing. */
const USAGE_SETTLES_AFTER_MS = 60_000;

const UNREAD = "Not read";

/**
 * Whether the proxy traffic of a finished Browserbase visit is still being counted: nothing was read yet, or
 * the first read came back empty and the second is still due.
 */
export function proxyUsagePending(
  visit: Pick<BrowserbaseVisit, "proxyKind">,
  usage: Pick<RemoteUsage, "proxyBytes"> | null,
  finishedAt: number | null,
  now: number,
): boolean {
  if (visit.proxyKind !== "browserbase") return false;
  if (usage === null || usage.proxyBytes === null) return true;
  return usage.proxyBytes === 0 && finishedAt !== null && now - finishedAt < USAGE_SETTLES_AFTER_MS;
}

export type BrowserbaseVisitFacts = {
  /** The line that is always shown: region, time, traffic and what that costs. */
  summary: string[];
  /** Everything recorded about the browser, as name and value. */
  rows: { name: string; value: string; mono?: boolean }[];
  /** The proxy traffic is not final yet. */
  pending: boolean;
};

function leftFrom(visit: BrowserbaseVisit, proxyName: string | null): string {
  if (visit.proxyKind === "external") return `Your proxy${proxyName ? ` “${proxyName}”` : ""}, through Browserbase`;
  if (visit.proxyKind === "browserbase") {
    const country = visit.proxyCountry ? countryName(visit.proxyCountry) : "a country";
    return `Browserbase proxy: a residential connection, ${country} requested (best effort)`;
  }
  return `Browserbase's ${regionPlace(visit.region)} region, a data centre address (no proxy)`;
}

function timezone(visit: BrowserbaseVisit): string {
  const { originalTimezone: original, appliedTimezone: applied } = visit;
  if (applied) {
    return `${applied}, set by the scanner to match the region${original ? ` (the browser reported ${original})` : ""}`;
  }
  if (!original) return UNREAD;
  return visit.proxyKind === "browserbase" ? `${original}, set by Browserbase for its proxy` : original;
}

function webdriver(value: boolean | null): string {
  if (value === null) return UNREAD;
  return value
    ? "true: the pages were told the visit is automated"
    : "false: the pages were not told the visit is automated";
}

/**
 * What a report says about a visit that ran in a Browserbase browser: where it ran and left from, what it
 * used, and what the pages could read. `null` for a visit on this computer and for scans without a recorded
 * environment. Usage comes from the summary, which the late second read of the proxy traffic updates.
 */
export function browserbaseVisitFacts(
  detail: Pick<ScanDetail, "environment" | "summary">,
  now: number,
): BrowserbaseVisitFacts | null {
  const { environment, summary } = detail;
  if (environment?.browser.kind !== "browserbase") return null;
  const visit = environment.browser;
  const usage: RemoteUsage = summary.remote ?? visit;
  const pending = proxyUsagePending(visit, usage, summary.finishedAt, now);
  const proxied = visit.proxyKind === "browserbase";
  const kept = [visit.settings.recordSession ? "a video" : null, visit.settings.logSession ? "a log" : null]
    .filter((part) => part !== null)
    .join(" and ");

  const time = usage.sessionSeconds === null ? null : formatDuration(usage.sessionSeconds * 1000);
  const minutes = usage.sessionSeconds === null ? null : Math.max(1, Math.ceil(usage.sessionSeconds / 60));
  let traffic = "None: the visit used no Browserbase proxy";
  if (proxied) traffic = pending ? "Still being counted by Browserbase" : formatTraffic(usage.proxyBytes ?? 0);
  const cost =
    usage.sessionSeconds === null || pending
      ? null
      : formatUsd(browserbaseUsd(usage.sessionSeconds, usage.proxyBytes ?? 0));

  return {
    pending,
    summary: [
      `Browserbase ${regionPlace(visit.region)}`,
      ...(time ? [`${time} of browser time`] : []),
      ...(proxied
        ? [pending ? "proxy traffic still being counted" : `${formatTraffic(usage.proxyBytes ?? 0)} through the proxy`]
        : []),
      ...(cost ? [`about ${cost} at Browserbase`] : []),
    ],
    rows: [
      { name: "Browser ran in", value: `Browserbase, ${regionOptionLabel(visit.region)}` },
      { name: "Left from", value: leftFrom(visit, environment.proxy.name) },
      {
        name: "Browser time",
        value: time ? `${time}, billed as ${minutes} ${minutes === 1 ? "minute" : "minutes"}` : "Not reported",
      },
      { name: "Proxy traffic", value: traffic },
      {
        name: "Estimated cost",
        value: cost
          ? `${cost} at Browserbase's ${BROWSERBASE_RATES.plan} plan overage prices of ${BROWSERBASE_RATES.asOf}; not part of the LLM cost`
          : "Known once the proxy traffic is counted",
      },
      { name: "Timezone the pages saw", value: timezone(visit) },
      { name: "Language", value: visit.language ? `${visit.language}, as the browser reported it` : UNREAD },
      { name: "navigator.webdriver", value: webdriver(environment.webdriver) },
      { name: "User agent", value: visit.userAgent ?? "Not recorded", mono: true },
      {
        name: "Browser",
        value: `${visit.browserVersion ?? "Version not reported"}, headless, which the user agent does not show`,
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
