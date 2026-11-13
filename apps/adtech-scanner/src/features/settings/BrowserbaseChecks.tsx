import { type ReactNode, useId } from "react";
import type { BrowserbaseProbe, BrowserbaseRegion, BrowserbaseStatus } from "@/bindings";
import { toApiError } from "@/lib/api";
import { useProbeBrowserbase, useTestBrowserbase } from "@/lib/query/hooks";
import {
  BROWSERBASE_RATES,
  BROWSERBASE_REGION_META,
  formatTraffic,
  regionName,
  regionOptionLabel,
  webdriverReading,
} from "@/shared/domain/browserbase";
import { formatInteger, pluralize } from "@/shared/lib/format";
import { Button } from "@/shared/ui/button";
import { Field } from "@/shared/ui/field";

function Failure({ reason }: { reason: unknown }) {
  return (
    <p role="alert" className="text-[13px] text-pretty text-red-fg">
      {toApiError(reason).message}
    </p>
  );
}

function StatusLine({ status }: { status: BrowserbaseStatus }) {
  const { projectName, concurrency, runningSessions, browserMinutes, proxyBytes } = status;
  return (
    <p className="text-[13px] text-pretty text-muted-foreground">
      <span className="font-medium text-foreground">Project “{projectName}”</span>: up to{" "}
      {pluralize(concurrency, "browser")} at once, {runningSessions} running now
      {runningSessions >= concurrency ? ", so a scan has to wait for one to finish" : ""}. Used so far by everything in
      the project: {formatInteger(browserMinutes)} browser minutes and {formatTraffic(proxyBytes)} of proxy traffic.
    </p>
  );
}

/** The free check of the account: its project, its limit of browsers and what is running. */
export function TestConnection() {
  const test = useTestBrowserbase();
  return (
    <Field
      stacked
      label="Test connection"
      description="Free: asks Browserbase for the project, how many browsers it may run at once and how many are running now. Another application that uses the same project shares that limit."
    >
      <div className="flex w-full flex-col gap-2" aria-live="polite">
        <div>
          <Button variant="outline" size="sm" disabled={test.isPending} onClick={() => test.mutate()}>
            {test.isPending ? "Testing…" : "Test connection"}
          </Button>
        </div>
        {test.isError ? <Failure reason={test.error} /> : null}
        {test.data ? <StatusLine status={test.data} /> : null}
      </div>
    </Field>
  );
}

const UNREAD = "could not be read";

function ProbeResult({ probe }: { probe: BrowserbaseProbe }) {
  const { timezone } = BROWSERBASE_REGION_META[probe.region];
  const facts: [string, ReactNode][] = [
    ["Region", regionOptionLabel(probe.region)],
    ["Browser", probe.browserVersion ?? UNREAD],
    ["User agent", probe.userAgent ?? UNREAD],
    [
      "navigator.webdriver",
      probe.webdriver === null ? UNREAD : webdriverReading(probe.webdriver, "the page is"),
    ],
    [
      "Timezone",
      probe.timezone === null
        ? UNREAD
        : probe.timezone === timezone
          ? timezone
          : `${probe.timezone}; a scan in this region sets ${timezone} unless it uses a Browserbase location`,
    ],
    ["Language", probe.language ?? UNREAD],
    ["Round trip", probe.roundTripMs === null ? UNREAD : `${Math.round(probe.roundTripMs)} ms per command`],
    [
      "Browser time",
      probe.sessionSeconds === null ? UNREAD : `${Math.round(probe.sessionSeconds)} s, billed as one minute`,
    ],
  ];
  return (
    <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-4 gap-y-1 rounded-lg border px-3.5 py-2.5 text-[13px]">
      {facts.map(([name, value]) => (
        <div key={name} className="col-span-2 grid grid-cols-subgrid">
          <dt className="text-muted-foreground">{name}</dt>
          <dd className="selectable min-w-0 break-words">{value}</dd>
        </div>
      ))}
    </dl>
  );
}

/** The check that costs a browser minute: what a blank page reads in a Browserbase browser. */
export function PageProbe({ region }: { region: BrowserbaseRegion }) {
  const probe = useProbeBrowserbase();
  const costId = useId();
  return (
    <Field
      stacked
      label="Check what a page sees"
      description={
        <span id={costId}>
          Starts a Browserbase browser for under a minute in {regionName(region)}, the default region, and reads what a
          blank page can tell there. Browserbase bills it as one browser minute, about $
          {(BROWSERBASE_RATES.browserUsdPerHour / 60).toFixed(3)} at its list price.
        </span>
      }
    >
      <div className="flex w-full flex-col gap-2" aria-live="polite">
        <div>
          <Button
            variant="outline"
            size="sm"
            aria-describedby={costId}
            disabled={probe.isPending}
            onClick={() => probe.mutate(region)}
          >
            {probe.isPending ? "Starting a browser…" : "Check what a page sees"}
          </Button>
        </div>
        {probe.isError ? <Failure reason={probe.error} /> : null}
        {probe.data ? (
          // biome-ignore lint/a11y/useSemanticElements: a fieldset is for form controls; this groups read-only facts
          <div role="group" aria-label="What a page sees">
            <ProbeResult probe={probe.data} />
          </div>
        ) : null}
      </div>
    </Field>
  );
}

