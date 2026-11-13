import { type FormEvent, useId, useState } from "react";
import type { Settings, SettingsPatch } from "@/bindings";
import { BrowserbaseLocationFields } from "@/shared/components/browserbase-location-fields";
import {
  BROWSERBASE_RATES,
  cityName,
  type LocationDraft,
  type LocationError,
  locationLabel,
  parseLocation,
  regionName,
} from "@/shared/domain/browserbase";
import { proxyChoiceFromKey, proxyChoiceKey, type SavedProxyKey } from "@/shared/domain/proxy";
import { Button } from "@/shared/ui/button";
import { Field } from "@/shared/ui/field";
import { Select, SelectContent, SelectGroup, SelectItem, SelectTrigger, SelectValue } from "@/shared/ui/select";

/**
 * Where Browserbase scans leave from unless a scan says otherwise. A region or a saved proxy is stored when
 * it is chosen; a location only with "Save location", and until then the form says that it is not saved.
 */
export function BrowserbaseDefaultLocation({
  settings,
  save,
}: {
  settings: Settings;
  save: (patch: SettingsPatch) => void;
}) {
  const stored = settings.browserbase.defaultProxy;
  const storedLocation: LocationDraft =
    stored.kind === "browserbaseLocation"
      ? { country: stored.country, state: stored.state ?? "", city: stored.city ? cityName(stored.city) : "" }
      : { country: "", state: "", city: "" };
  // The location as typed, from the moment it is chosen or edited.
  const [draft, setDraft] = useState<LocationDraft | null>(null);
  const [error, setError] = useState<LocationError | null>(null);
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
  const region = regionName(settings.browserbase.region);

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
      description={`Where a Browserbase scan leaves from unless its Options say otherwise: the region itself (a data centre address), one of your saved proxies (Browserbase is then given its address and password), or a residential connection that Browserbase provides in a country you name. That last one is best effort and billed per megabyte: $${BROWSERBASE_RATES.proxyUsdPerGb} per GB at Browserbase's list price.`}
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
              <BrowserbaseLocationFields
                value={fields}
                onChange={edit}
                error={error}
                errorId={errorId}
                scope="Default"
                cityClassName="w-48"
              />
              <Button type="submit" variant="outline" size="sm" disabled={saved}>
                Save location
              </Button>
              {saved ? null : (
                <span role="status" className="text-[13px] text-amber-fg">
                  Not saved yet
                </span>
              )}
            </div>
            {error ? (
              <p id={errorId} role="alert" className="text-[13px] text-red-fg">
                {error.message}
              </p>
            ) : null}
          </form>
        ) : null}
      </div>
    </Field>
  );
}
