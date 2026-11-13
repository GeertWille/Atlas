import { CheckCircle2Icon } from "lucide-react";
import { type FormEvent, type ReactNode, useId, useState } from "react";
import { toast } from "sonner";
import type {
  BrowserbaseProbe,
  BrowserbaseProject,
  BrowserbaseRegion,
  BrowserbaseStatus,
  Settings,
  SettingsPatch,
} from "@/bindings";
import { toApiError } from "@/lib/api";
import {
  useBrowserbaseProfile,
  useClearBrowserbaseCredentials,
  useClearBrowserbaseProfile,
  useProbeBrowserbase,
  useSetBrowserbaseCredentials,
  useTestBrowserbase,
} from "@/lib/query/hooks";
import { KeyForm, type KeySaveResult } from "@/shared/components/api-key-form";
import {
  BROWSERBASE_DISCLOSURE,
  BROWSERBASE_RATES,
  BROWSERBASE_REGION_META,
  BROWSERBASE_REGIONS,
  cityName,
  formatTraffic,
  locationLabel,
  parseLocation,
  regionOptionLabel,
  regionPlace,
} from "@/shared/domain/browserbase";
import { proxyChoiceFromKey, proxyChoiceKey, type SavedProxyKey } from "@/shared/domain/proxy";
import { formatInteger, formatRelativeTime, pluralize } from "@/shared/lib/format";
import { notifyError } from "@/shared/lib/notify-error";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogTrigger,
} from "@/shared/ui/alert-dialog";
import { Button } from "@/shared/ui/button";
import { Field } from "@/shared/ui/field";
import { Input } from "@/shared/ui/input";
import { Select, SelectContent, SelectGroup, SelectItem, SelectTrigger, SelectValue } from "@/shared/ui/select";
import { Switch } from "@/shared/ui/switch";

type Save = (patch: SettingsPatch) => void;

function Failure({ reason }: { reason: unknown }) {
  return (
    <p role="alert" className="text-[13px] text-pretty text-red-fg">
      {toApiError(reason).message}
    </p>
  );
}

/** The key field, and the choice between projects for a key that has more than one. */
function Connect() {
  const setCredentials = useSetBrowserbaseCredentials();
  const projectLabelId = useId();
  const [projects, setProjects] = useState<BrowserbaseProject[]>([]);
  const [projectId, setProjectId] = useState<string | undefined>(undefined);

  const save = async (apiKey: string): Promise<KeySaveResult> => {
    try {
      const status = await setCredentials.mutateAsync({ apiKey, projectId });
      if (status.valid) {
        toast.success("Browserbase key validated and saved to your keychain");
        return { saved: true };
      }
      setProjects(status.projects);
      return { saved: false, reason: status.message ?? "Browserbase did not accept the API key." };
    } catch (reason) {
      notifyError("Could not save the Browserbase key", reason);
      return { saved: false };
    }
  };

  return (
    <KeyForm
      name="Browserbase"
      placeholder="bb_live_…"
      keyNoun="Browserbase key"
      submitLabel="Save Browserbase key"
      pending={setCredentials.isPending}
      onSave={save}
    >
      {projects.length > 1 ? (
        <div className="flex items-center gap-3">
          <span id={projectLabelId} className="text-[13px] font-medium">
            Project
          </span>
          <Select value={projectId ?? ""} onValueChange={setProjectId}>
            <SelectTrigger aria-labelledby={projectLabelId} className="w-56">
              <SelectValue placeholder="Choose a project" />
            </SelectTrigger>
            <SelectContent>
              <SelectGroup>
                {projects.map((project) => (
                  <SelectItem key={project.id} value={project.id}>
                    {project.name}
                  </SelectItem>
                ))}
              </SelectGroup>
            </SelectContent>
          </Select>
        </div>
      ) : null}
    </KeyForm>
  );
}

function Connected({ projectName }: { projectName: string | null }) {
  const clear = useClearBrowserbaseCredentials();
  return (
    <div className="flex items-center justify-between gap-4 rounded-lg border border-green-line bg-green-soft/60 px-3.5 py-2.5">
      <p className="flex items-center gap-2 text-[13px] text-green-fg">
        <CheckCircle2Icon className="size-4 shrink-0" />
        <span>
          Connected to the project {projectName ? `“${projectName}”` : "of this key"}. The key is in your system
          keychain.
        </span>
      </p>
      <AlertDialog>
        <AlertDialogTrigger asChild>
          <Button variant="outline" size="sm" disabled={clear.isPending}>
            Remove Browserbase key
          </Button>
        </AlertDialogTrigger>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Remove the Browserbase key?</AlertDialogTitle>
            <AlertDialogDescription>
              The key and its project are deleted from your keychain. Scans run on this Mac again, and saved
              configurations that use Browserbase cannot run until a key is added. A browser profile kept at Browserbase
              is deleted there too, if Browserbase can be reached.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              destructive
              onClick={() =>
                clear.mutate(undefined, {
                  onSuccess: () => toast.success("Browserbase key removed"),
                  onError: (reason) => notifyError("Could not remove the Browserbase key", reason),
                })
              }
            >
              Remove Browserbase key
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
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

function TestConnection() {
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
      probe.webdriver === null
        ? UNREAD
        : probe.webdriver
          ? "true: the page is told the visit is automated"
          : "false: the page is not told the visit is automated",
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

function PageProbe({ region }: { region: BrowserbaseRegion }) {
  const probe = useProbeBrowserbase();
  const costId = useId();
  return (
    <Field
      stacked
      label="Check what a page sees"
      description={
        <span id={costId}>
          Starts a Browserbase browser for under a minute in {regionPlace(region)}, the default region, and reads what a
          blank page can tell there. Browserbase bills it as one browser minute, about $
          {(BROWSERBASE_RATES.browserUsdPerHour / 60).toFixed(3)}.
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

type LocationDraft = { country: string; state: string; city: string };

/** Where Browserbase scans leave from unless a scan says otherwise. A location is saved once its country is filled in. */
function DefaultLocation({ settings, save }: { settings: Settings; save: Save }) {
  const stored = settings.browserbase.defaultProxy;
  const storedLocation: LocationDraft =
    stored.kind === "browserbaseLocation"
      ? { country: stored.country, state: stored.state ?? "", city: stored.city ? cityName(stored.city) : "" }
      : { country: "", state: "", city: "" };
  // The location as typed, from the moment it is chosen or edited.
  const [draft, setDraft] = useState<LocationDraft | null>(null);
  const [error, setError] = useState<string | null>(null);
  const errorId = useId();
  const choosing = draft !== null || stored.kind === "browserbaseLocation";
  const fields = draft ?? storedLocation;
  const parsed = parseLocation(fields);
  // What is typed is what is stored: there is nothing to save.
  const saved =
    stored.kind === "browserbaseLocation" &&
    parsed.ok &&
    parsed.location.country === stored.country &&
    parsed.location.state === (stored.state ?? undefined) &&
    parsed.location.city === (stored.city ?? undefined);
  const region = regionPlace(settings.browserbase.region);

  const choose = (key: string) => {
    setError(null);
    if (key === "browserbaseLocation") {
      setDraft(storedLocation);
      return;
    }
    setDraft(null);
    save({ browserbase: { defaultProxy: proxyChoiceFromKey(key as SavedProxyKey) } });
  };

  const saveLocation = (event: FormEvent) => {
    event.preventDefault();
    if (!parsed.ok) {
      setError(parsed.error);
      return;
    }
    setError(null);
    save({ browserbase: { defaultProxy: parsed.location } });
  };

  const edit = (patch: Partial<LocationDraft>) => {
    setDraft({ ...fields, ...patch });
    setError(null);
  };

  return (
    <Field
      stacked
      label="Default location"
      description={`Where a Browserbase scan leaves from unless its Options say otherwise: the region itself (a data centre address), one of your saved proxies (Browserbase is then given its address and password), or a residential connection that Browserbase provides in a country you name. That last one is best effort and billed per megabyte, about $${BROWSERBASE_RATES.proxyUsdPerGb} per GB.`}
    >
      <div className="flex w-full flex-col gap-2">
        <Select value={choosing ? "browserbaseLocation" : proxyChoiceKey(stored)} onValueChange={choose}>
          <SelectTrigger aria-label="Default location" className="w-72">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectGroup>
              <SelectItem value="none">Region only: leaves from {region}</SelectItem>
              {settings.proxies.map((proxy) => (
                <SelectItem key={proxy.id} value={proxyChoiceKey({ kind: "profile", id: proxy.id })}>
                  {proxy.name}
                </SelectItem>
              ))}
              <SelectItem value="browserbaseLocation">
                {stored.kind === "browserbaseLocation" && saved
                  ? `Browserbase location: ${locationLabel(stored)}`
                  : "Browserbase location…"}
              </SelectItem>
            </SelectGroup>
          </SelectContent>
        </Select>
        {choosing ? (
          <form onSubmit={saveLocation} noValidate className="flex flex-col gap-1.5">
            <div className="flex items-center gap-2">
              <Input
                aria-label="Default country code"
                aria-invalid={error ? true : undefined}
                aria-describedby={error ? errorId : undefined}
                placeholder="BE"
                maxLength={2}
                autoComplete="off"
                spellCheck={false}
                value={fields.country}
                onChange={(event) => edit({ country: event.target.value })}
                className="w-14 uppercase placeholder:normal-case"
              />
              <Input
                aria-label="Default state code, optional"
                placeholder="State"
                maxLength={2}
                autoComplete="off"
                spellCheck={false}
                value={fields.state}
                onChange={(event) => edit({ state: event.target.value })}
                className="w-16 uppercase placeholder:normal-case"
              />
              <Input
                aria-label="Default city, optional"
                placeholder="City (optional)"
                maxLength={60}
                autoComplete="off"
                spellCheck={false}
                value={fields.city}
                onChange={(event) => edit({ city: event.target.value })}
                className="w-48"
              />
              <Button type="submit" variant="outline" size="sm" disabled={saved}>
                Save location
              </Button>
            </div>
            {error ? (
              <p id={errorId} role="alert" className="text-[13px] text-red-fg">
                {error}
              </p>
            ) : null}
          </form>
        ) : null}
      </div>
    </Field>
  );
}

function ProfileState() {
  const profile = useBrowserbaseProfile();
  const clear = useClearBrowserbaseProfile();
  if (!profile.data) return null;
  const { exists, lastUsedAt, inUse } = profile.data;
  if (!exists) {
    return <p className="text-[13px] text-muted-foreground">No browser profile is kept at Browserbase.</p>;
  }
  return (
    <div className="flex w-full items-center justify-between gap-4">
      <p className="text-[13px] text-muted-foreground" aria-live="polite">
        A browser profile is kept at Browserbase
        {lastUsedAt === null ? "" : `, last used ${formatRelativeTime(lastUsedAt)}`}
        {inUse ? ". A running scan is using it." : "."}
      </p>
      <AlertDialog>
        <AlertDialogTrigger asChild>
          <Button variant="outline" size="sm" disabled={inUse || clear.isPending}>
            Clear Browserbase profile…
          </Button>
        </AlertDialogTrigger>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Clear the browser profile at Browserbase?</AlertDialogTitle>
            <AlertDialogDescription>
              Its cookies, consent choices and site storage are deleted at Browserbase. The next Browserbase scan starts
              like a first-time visitor. The profile on this Mac is not touched.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              destructive
              onClick={() =>
                clear.mutate(undefined, {
                  onSuccess: () => toast.success("Browserbase profile cleared"),
                  onError: (reason) => notifyError("Could not clear the Browserbase profile", reason),
                })
              }
            >
              Clear profile
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  );
}

/**
 * Browserbase as a second place for a scan's browser to run: what it means for the scanned pages, the
 * credentials, two checks of the account, and what a Browserbase scan starts from.
 */
export function BrowserbaseSection({ settings, save }: { settings: Settings; save: Save }) {
  const ids = { disclosure: useId(), keep: useId(), keepNote: useId() };
  const { browserbase } = settings;
  return (
    <>
      <Field stacked label="Browserbase account">
        <div className="flex w-full flex-col gap-3">
          <p
            id={ids.disclosure}
            role="note"
            aria-label="What using Browserbase means"
            className="selectable rounded-lg border bg-muted/40 px-3.5 py-3 text-[13px] text-pretty text-muted-foreground"
          >
            {BROWSERBASE_DISCLOSURE}
          </p>
          {browserbase.connected ? <Connected projectName={browserbase.projectName} /> : <Connect />}
        </div>
      </Field>
      {browserbase.connected ? (
        <>
          <TestConnection />
          <PageProbe region={browserbase.region} />
          <Field
            label="Default region"
            description="Where Browserbase starts the browser. The region decides both speed (one near you answers faster) and, without a proxy, where the visit comes from."
          >
            <Select
              value={browserbase.region}
              onValueChange={(region) => save({ browserbase: { region: region as BrowserbaseRegion } })}
            >
              <SelectTrigger aria-label="Default region" className="w-64">
                <SelectValue />
              </SelectTrigger>
              <SelectContent align="end">
                <SelectGroup>
                  {BROWSERBASE_REGIONS.map((region) => (
                    <SelectItem key={region} value={region}>
                      {regionOptionLabel(region)}
                    </SelectItem>
                  ))}
                </SelectGroup>
              </SelectContent>
            </Select>
          </Field>
          <DefaultLocation settings={settings} save={save} />
          <Field
            label="Keep a browser profile on Browserbase"
            htmlFor={ids.keep}
            description={
              <span id={ids.keepNote}>
                On: Browserbase scans share one profile per device type that Browserbase stores: cookies, consent
                choices and site storage, until you clear it here. Off: every Browserbase scan starts fresh, whatever
                the setting for this Mac's browser says.
              </span>
            }
          >
            <Switch
              id={ids.keep}
              aria-describedby={ids.keepNote}
              checked={browserbase.persistentProfile}
              onCheckedChange={(persistentProfile) => save({ browserbase: { persistentProfile } })}
            />
          </Field>
          <Field stacked label="Browser profile at Browserbase">
            <ProfileState />
          </Field>
        </>
      ) : null}
    </>
  );
}
