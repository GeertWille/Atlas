import { CloudIcon, LaptopIcon } from "lucide-react";
import { type ReactNode, useId } from "react";
import type { BrowserbaseRegion, BrowserKind, Settings } from "@/bindings";
import { BrowserbaseLocationFields } from "@/shared/components/browserbase-location-fields";
import {
  BROWSERBASE_REGIONS,
  BROWSERBASE_UNREACHABLE,
  type BrowserbaseEstimate,
  type LocationError,
  regionName,
  regionOptionLabel,
} from "@/shared/domain/browserbase";
import { type ProxyChoiceKey, proxyChoiceLabel } from "@/shared/domain/proxy";
import { cn } from "@/shared/lib/cn";
import { formatUsd } from "@/shared/lib/format";
import { Label } from "@/shared/ui/label";
import { Select, SelectContent, SelectGroup, SelectItem, SelectTrigger, SelectValue } from "@/shared/ui/select";
import { ToggleGroup, ToggleGroupItem } from "@/shared/ui/toggle-group";
import { OptionHelp, OptionsSection } from "./option-layout";
import { SettingsLink } from "./SettingsLink";
import { type ProfileField, type ProxyField, proxyFieldFor, type ScanForm } from "./scan-form";

type Update = (patch: Partial<ScanForm>) => void;

/**
 * Two lines of help under every control, so the section keeps its height wherever the browser runs. The cell
 * beside "Where the browser runs" is the region's: on this Mac there is no region and it stays empty.
 */
const HELP = "min-h-8";

function BrowserRunsOn({
  settings,
  remote,
  unreachable,
  browserMissing,
  onBrowserChange,
}: {
  settings: Settings;
  remote: boolean;
  unreachable: boolean;
  browserMissing: boolean;
  onBrowserChange: (browser: BrowserKind) => void;
}) {
  const labelId = useId();
  const helpId = useId();
  const { connected } = settings.browserbase;
  // Credentials that are gone leave the choice standing, so that it can be seen and changed.
  const value: BrowserKind = remote || browserMissing ? "browserbase" : "local";
  let help: ReactNode = "Chrome on this Mac drives the visit.";
  if (browserMissing) {
    help = (
      <>
        Browserbase is not set up any more. <SettingsLink>Set it up in Settings</SettingsLink>, or choose This Mac.
      </>
    );
  } else if (unreachable) {
    help = BROWSERBASE_UNREACHABLE;
  } else if (!connected) {
    help = (
      <>
        Browserbase, a cloud browser, is not set up. <SettingsLink>Set it up in Settings</SettingsLink>
      </>
    );
  } else if (remote) {
    help = "The pages pass through Browserbase, which keeps a video and a log of the session.";
  }
  return (
    <div className="flex min-w-0 flex-col gap-1.5">
      <Label id={labelId}>Where the browser runs</Label>
      <ToggleGroup aria-labelledby={labelId} value={value} onValueChange={onBrowserChange} className="self-start">
        <ToggleGroupItem value="local">
          <LaptopIcon />
          This Mac
        </ToggleGroupItem>
        <ToggleGroupItem
          value="browserbase"
          disabled={value === "local" && (unreachable || !connected)}
          aria-describedby={helpId}
          className="disabled:pointer-events-none disabled:opacity-50"
        >
          <CloudIcon />
          Browserbase
        </ToggleGroupItem>
      </ToggleGroup>
      <OptionHelp
        id={helpId}
        role={browserMissing ? "alert" : undefined}
        className={cn(HELP, browserMissing && "text-red-fg")}
      >
        {help}
      </OptionHelp>
    </div>
  );
}

function RegionChoice({ form, settings, update }: { form: ScanForm; settings: Settings; update: Update }) {
  return (
    <div className="flex min-w-0 flex-col gap-1.5">
      <Label>Region</Label>
      <Select
        value={form.region}
        onValueChange={(region) => update({ region: region as "settings" | BrowserbaseRegion })}
      >
        <SelectTrigger aria-label="Region" className="w-full">
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectGroup>
            <SelectItem value="settings">Settings default: {regionName(settings.browserbase.region)}</SelectItem>
            {BROWSERBASE_REGIONS.map((region) => (
              <SelectItem key={region} value={region}>
                {regionOptionLabel(region)}
              </SelectItem>
            ))}
          </SelectGroup>
        </SelectContent>
      </Select>
      <OptionHelp className={HELP}>
        A region near you is faster. Without a proxy, it is also where the visit comes from.
      </OptionHelp>
    </div>
  );
}

function ProxyChoice({
  form,
  settings,
  remote,
  proxyMissing,
  locationError,
  usage,
  update,
}: {
  form: ScanForm;
  settings: Settings;
  remote: boolean;
  proxyMissing: boolean;
  locationError: LocationError | undefined;
  usage: BrowserbaseEstimate | null;
  update: Update;
}) {
  const errorId = useId();
  const region = regionName(form.region === "settings" ? settings.browserbase.region : form.region);
  const settingsDefault = remote ? settings.browserbase.defaultProxy : settings.defaultProxy;
  const proxy = proxyFieldFor(form, remote);
  const location = proxy === "browserbaseLocation";
  const error = proxyMissing
    ? "This proxy was deleted. Choose another one."
    : location
      ? locationError?.message
      : undefined;
  const name = remote ? "Location" : "Proxy";

  let help: ReactNode = (
    <>
      The site sees the scan arrive from there. <SettingsLink>Manage proxies</SettingsLink>
    </>
  );
  if (location) {
    help = `A residential connection; the country is best effort.${usage ? ` About ${usage.proxyMb} MB${usage.proxyPerPage ? " per page" : ""} go through it, an estimated ${formatUsd(usage.proxyUsd)} at list price.` : ""}`;
  } else if (remote && proxy.startsWith("profile:")) {
    help = "Browserbase connects through your proxy. Its address and password are sent to Browserbase for that.";
  } else if (remote && (proxy === "none" || (proxy === "settings" && settingsDefault.kind === "none"))) {
    help = `The visit leaves from Browserbase's ${region} region, a data centre address.`;
  } else if (remote) {
    help = "Set in Settings, under Browserbase.";
  }

  return (
    <div className="flex min-w-0 flex-col gap-1.5">
      <Label>{name}</Label>
      <Select value={proxy} onValueChange={(chosen) => update({ proxy: chosen as ProxyField })}>
        <SelectTrigger
          aria-label={name}
          aria-invalid={proxyMissing ? true : undefined}
          aria-describedby={proxyMissing ? errorId : undefined}
          className={cn("w-full", proxyMissing && "border-destructive")}
        >
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectGroup>
            <SelectItem value="settings">
              Settings default ({proxyChoiceLabel(settingsDefault, settings.proxies)})
            </SelectItem>
            <SelectItem value="none">{remote ? `No proxy: leaves from ${region}` : "No proxy"}</SelectItem>
            {remote ? null : <SelectItem value="system">System proxy</SelectItem>}
            {settings.proxies.map((proxy) => (
              <SelectItem key={proxy.id} value={`profile:${proxy.id}` satisfies ProxyChoiceKey}>
                {proxy.name}
              </SelectItem>
            ))}
            {remote ? <SelectItem value="browserbaseLocation">Browserbase location…</SelectItem> : null}
            {proxyMissing ? (
              <SelectItem value={proxy} disabled>
                Deleted proxy
              </SelectItem>
            ) : null}
          </SelectGroup>
        </SelectContent>
      </Select>
      {location ? (
        <div className="flex gap-1.5">
          <BrowserbaseLocationFields
            value={{ country: form.locationCountry, state: form.locationState, city: form.locationCity }}
            onChange={({ country, state, city }) =>
              update({
                ...(country === undefined ? {} : { locationCountry: country }),
                ...(state === undefined ? {} : { locationState: state }),
                ...(city === undefined ? {} : { locationCity: city }),
              })
            }
            error={locationError}
            errorId={errorId}
          />
        </div>
      ) : null}
      {error ? (
        <OptionHelp id={errorId} role="alert" className={cn(HELP, "text-red-fg")}>
          {error}
        </OptionHelp>
      ) : (
        <OptionHelp className={HELP}>{help}</OptionHelp>
      )}
    </div>
  );
}

const PROFILE_LABEL: Record<Exclude<ProfileField, "settings">, string> = {
  fresh: "Fresh each scan",
  kept: "Kept between scans",
};

function BrowserProfileChoice({
  form,
  settings,
  remote,
  update,
}: {
  form: ScanForm;
  settings: Settings;
  remote: boolean;
  update: Update;
}) {
  // A Browserbase browser keeps a profile only while the switch in Settings allows it.
  const allowed = remote ? settings.browserbase.persistentProfile : true;
  const settingsProfile = (remote ? settings.browserbase.persistentProfile : settings.persistentProfile)
    ? "kept"
    : "fresh";
  // A choice to keep the profile that cannot hold there reads as what happens instead; the form remembers it.
  const profile: ProfileField = !allowed && form.profile === "kept" ? "fresh" : form.profile;
  const kept = (profile === "settings" ? settingsProfile : profile) === "kept";
  let help = kept
    ? "Cookies and consent carry over from earlier scans, like a returning visitor's."
    : "Starts without cookies, like a first-time visitor.";
  if (remote && kept) help = "Cookies and consent carry over, in a profile kept at Browserbase.";
  if (remote && !allowed) help = "Starts fresh: keeping a profile at Browserbase is off.";
  return (
    <div className="flex min-w-0 flex-col gap-1.5">
      <Label>Browser profile</Label>
      <Select value={profile} onValueChange={(chosen) => update({ profile: chosen as ProfileField })}>
        <SelectTrigger aria-label="Browser profile" className="w-full">
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectGroup>
            <SelectItem value="settings">Settings default ({PROFILE_LABEL[settingsProfile].toLowerCase()})</SelectItem>
            <SelectItem value="fresh">{PROFILE_LABEL.fresh}</SelectItem>
            <SelectItem value="kept" disabled={!allowed}>
              {PROFILE_LABEL.kept}
            </SelectItem>
          </SelectGroup>
        </SelectContent>
      </Select>
      <OptionHelp className={HELP}>
        {help} {profile === "settings" ? <SettingsLink>Change the default</SettingsLink> : null}
      </OptionHelp>
    </div>
  );
}

/**
 * Where the visit comes from: the computer or cloud region the browser runs in, the proxy or location it
 * leaves through, and the browser profile it carries.
 */
export function OriginOptions({
  form,
  settings,
  remote,
  unreachable,
  browserMissing,
  proxyMissing,
  locationError,
  usage,
  update,
  onBrowserChange,
}: {
  form: ScanForm;
  settings: Settings;
  /** The scan runs in a Browserbase browser. */
  remote: boolean;
  /** The address is one a cloud browser cannot reach. */
  unreachable: boolean;
  /** Browserbase is chosen but its credentials are gone. */
  browserMissing: boolean;
  /** The form names a saved proxy that has been deleted. */
  proxyMissing: boolean;
  locationError: LocationError | undefined;
  /** What the scan is likely to use at Browserbase; `null` on this Mac. */
  usage: BrowserbaseEstimate | null;
  update: Update;
  onBrowserChange: (browser: BrowserKind) => void;
}) {
  return (
    <OptionsSection title="Origin" question="Where does the visit come from?">
      <div className="grid grid-cols-2 gap-x-8 gap-y-3">
        <BrowserRunsOn
          settings={settings}
          remote={remote}
          unreachable={unreachable}
          browserMissing={browserMissing}
          onBrowserChange={onBrowserChange}
        />
        {remote ? <RegionChoice form={form} settings={settings} update={update} /> : <div aria-hidden="true" />}
        <ProxyChoice
          form={form}
          settings={settings}
          remote={remote}
          proxyMissing={proxyMissing}
          locationError={locationError}
          usage={usage}
          update={update}
        />
        <BrowserProfileChoice form={form} settings={settings} remote={remote} update={update} />
      </div>
    </OptionsSection>
  );
}
