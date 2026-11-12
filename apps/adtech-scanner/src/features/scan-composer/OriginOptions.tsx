import { useId } from "react";
import type { Settings } from "@/bindings";
import { type ProxyChoiceKey, proxyChoiceLabel } from "@/shared/domain/proxy";
import { cn } from "@/shared/lib/cn";
import { Label } from "@/shared/ui/label";
import { Select, SelectContent, SelectGroup, SelectItem, SelectTrigger, SelectValue } from "@/shared/ui/select";
import { OptionHelp, OptionsSection } from "./option-layout";
import { SettingsLink } from "./SettingsLink";
import type { ProfileField, ProxyField, ScanForm } from "./scan-form";

const PROFILE_LABEL: Record<Exclude<ProfileField, "settings">, string> = {
  fresh: "Fresh each scan",
  kept: "Kept between scans",
};

function ProxyChoice({
  form,
  settings,
  proxyMissing,
  update,
}: {
  form: ScanForm;
  settings: Settings;
  proxyMissing: boolean;
  update: (patch: Partial<ScanForm>) => void;
}) {
  const errorId = useId();
  return (
    <div className="flex min-w-0 flex-col gap-1.5">
      <Label>Proxy</Label>
      <Select value={form.proxy} onValueChange={(proxy) => update({ proxy: proxy as ProxyField })}>
        <SelectTrigger
          aria-label="Proxy"
          aria-invalid={proxyMissing ? true : undefined}
          aria-describedby={proxyMissing ? errorId : undefined}
          className={cn("w-full", proxyMissing && "border-destructive")}
        >
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectGroup>
            <SelectItem value="settings">
              Settings default ({proxyChoiceLabel(settings.defaultProxy, settings.proxies)})
            </SelectItem>
            <SelectItem value="none">No proxy</SelectItem>
            <SelectItem value="system">System proxy</SelectItem>
            {settings.proxies.map((proxy) => (
              <SelectItem key={proxy.id} value={`profile:${proxy.id}` satisfies ProxyChoiceKey}>
                {proxy.name}
              </SelectItem>
            ))}
            {proxyMissing ? (
              <SelectItem value={form.proxy} disabled>
                Deleted proxy
              </SelectItem>
            ) : null}
          </SelectGroup>
        </SelectContent>
      </Select>
      {proxyMissing ? (
        <OptionHelp id={errorId} role="alert" className="text-red-fg">
          This proxy was deleted. Choose another one.
        </OptionHelp>
      ) : (
        <OptionHelp>
          The site sees the scan arrive from there. <SettingsLink>Manage proxies</SettingsLink>
        </OptionHelp>
      )}
    </div>
  );
}

function BrowserProfileChoice({
  form,
  settings,
  update,
}: {
  form: ScanForm;
  settings: Settings;
  update: (patch: Partial<ScanForm>) => void;
}) {
  const settingsProfile = settings.persistentProfile ? "kept" : "fresh";
  const kept = (form.profile === "settings" ? settingsProfile : form.profile) === "kept";
  return (
    <div className="flex min-w-0 flex-col gap-1.5">
      <Label>Browser profile</Label>
      <Select value={form.profile} onValueChange={(profile) => update({ profile: profile as ProfileField })}>
        <SelectTrigger aria-label="Browser profile" className="w-full">
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectGroup>
            <SelectItem value="settings">Settings default ({PROFILE_LABEL[settingsProfile].toLowerCase()})</SelectItem>
            <SelectItem value="fresh">{PROFILE_LABEL.fresh}</SelectItem>
            <SelectItem value="kept">{PROFILE_LABEL.kept}</SelectItem>
          </SelectGroup>
        </SelectContent>
      </Select>
      <OptionHelp>
        {kept
          ? "Cookies and consent carry over from earlier scans, like a returning visitor's."
          : "Starts without cookies, like a first-time visitor."}{" "}
        {form.profile === "settings" ? <SettingsLink>Change the default</SettingsLink> : null}
      </OptionHelp>
    </div>
  );
}

/**
 * Where the visit comes from: the proxy it goes through and the browser profile it carries. Where the
 * browser itself runs belongs here too, once there is more than this computer to choose from.
 */
export function OriginOptions({
  form,
  settings,
  proxyMissing,
  update,
}: {
  form: ScanForm;
  settings: Settings;
  /** The form names a saved proxy that has been deleted. */
  proxyMissing: boolean;
  update: (patch: Partial<ScanForm>) => void;
}) {
  return (
    <OptionsSection title="Origin" question="Where does the visit come from?">
      <div className="grid grid-cols-2 gap-x-8 gap-y-3">
        <ProxyChoice form={form} settings={settings} proxyMissing={proxyMissing} update={update} />
        <BrowserProfileChoice form={form} settings={settings} update={update} />
      </div>
    </OptionsSection>
  );
}
