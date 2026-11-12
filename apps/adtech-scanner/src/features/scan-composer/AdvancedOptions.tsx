import { useId } from "react";
import type { Settings } from "@/bindings";
import { type ProxyChoiceKey, proxyChoiceLabel } from "@/shared/domain/proxy";
import { cn } from "@/shared/lib/cn";
import { Input } from "@/shared/ui/input";
import { Label } from "@/shared/ui/label";
import { Select, SelectContent, SelectGroup, SelectItem, SelectTrigger, SelectValue } from "@/shared/ui/select";
import { SettingsLink } from "./SettingsLink";
import type { ProfileField, ProxyField, ScanForm } from "./scan-form";

const PROFILE_LABEL: Record<Exclude<ProfileField, "settings">, string> = {
  fresh: "Fresh profile",
  kept: "Kept scan profile",
};

/** What a scan can set apart from the settings: its proxy, its browser profile and its own LLM cap. */
export function AdvancedOptions({
  form,
  settings,
  budgetError,
  proxyMissing,
  update,
}: {
  form: ScanForm;
  settings: Settings;
  budgetError: string | undefined;
  /** The form names a proxy profile that has been deleted. */
  proxyMissing: boolean;
  update: (patch: Partial<ScanForm>) => void;
}) {
  const ids = { budget: useId(), budgetHelp: useId(), proxyError: useId() };
  const settingsProfile = settings.persistentProfile ? "kept" : "fresh";
  return (
    <div className="grid grid-cols-3 gap-x-6 gap-y-2 pt-3">
      <div className="flex min-w-0 flex-col gap-1.5">
        <Label>Proxy</Label>
        <Select value={form.proxy} onValueChange={(proxy) => update({ proxy: proxy as ProxyField })}>
          <SelectTrigger
            aria-label="Proxy"
            aria-invalid={proxyMissing ? true : undefined}
            aria-describedby={proxyMissing ? ids.proxyError : undefined}
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
          <p id={ids.proxyError} role="alert" className="text-xs text-red-fg">
            This proxy was deleted. Choose another one.
          </p>
        ) : (
          <p className="text-xs text-muted-foreground">
            <SettingsLink>Manage proxies</SettingsLink>
          </p>
        )}
      </div>
      <div className="flex min-w-0 flex-col gap-1.5">
        <Label>Browser profile</Label>
        <Select value={form.profile} onValueChange={(profile) => update({ profile: profile as ProfileField })}>
          <SelectTrigger aria-label="Browser profile" className="w-full">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectGroup>
              <SelectItem value="settings">
                Settings default ({PROFILE_LABEL[settingsProfile].toLowerCase()})
              </SelectItem>
              <SelectItem value="fresh">{PROFILE_LABEL.fresh}</SelectItem>
              <SelectItem value="kept">{PROFILE_LABEL.kept}</SelectItem>
            </SelectGroup>
          </SelectContent>
        </Select>
        <p className="text-xs text-pretty text-muted-foreground">
          {(form.profile === "settings" ? settingsProfile : form.profile) === "kept"
            ? "Cookies and consent carry over from earlier scans."
            : "Starts without cookies, like a first-time visitor."}
        </p>
      </div>
      <div className="flex min-w-0 flex-col gap-1.5">
        <Label htmlFor={ids.budget}>LLM budget</Label>
        <div className="relative">
          <span className="pointer-events-none absolute top-1/2 left-2.5 -translate-y-1/2 text-sm text-subtle-foreground">
            $
          </span>
          <Input
            id={ids.budget}
            inputMode="decimal"
            autoComplete="off"
            placeholder={settings.budgetUsd.toFixed(2)}
            value={form.budget}
            aria-invalid={budgetError ? true : undefined}
            aria-describedby={ids.budgetHelp}
            onChange={(event) => update({ budget: event.target.value })}
            className="pl-6 tabular-nums"
          />
        </div>
        <p
          id={ids.budgetHelp}
          role={budgetError ? "alert" : undefined}
          className={cn("text-xs text-pretty", budgetError ? "text-red-fg" : "text-muted-foreground")}
        >
          {budgetError ?? "Hard cap for this scan. Empty uses the settings."}
        </p>
      </div>
    </div>
  );
}
