import type { LocationDraft, LocationError } from "@/shared/domain/browserbase";
import { cn } from "@/shared/lib/cn";
import { Input } from "@/shared/ui/input";

/**
 * The three fields of a Browserbase location: a country code, and optionally a state code and a city. They
 * come without a wrapper, so that each place lays them out in its own row. The field that is wrong is the
 * one marked invalid and described by the error.
 */
export function BrowserbaseLocationFields({
  value,
  onChange,
  error,
  errorId,
  scope,
  cityClassName,
}: {
  value: LocationDraft;
  onChange: (patch: Partial<LocationDraft>) => void;
  error?: LocationError | null;
  /** The element that shows `error.message`. */
  errorId: string;
  /** What the location is of, where the fields need telling apart: "Default" makes "Default country code". */
  scope?: string;
  cityClassName?: string;
}) {
  const label = (name: string) => (scope ? `${scope} ${name.charAt(0).toLowerCase()}${name.slice(1)}` : name);
  const invalid = (field: keyof LocationDraft) =>
    error?.field === field ? ({ "aria-invalid": true, "aria-describedby": errorId } as const) : {};
  return (
    <>
      <Input
        aria-label={label("Country code")}
        {...invalid("country")}
        placeholder="BE"
        maxLength={2}
        autoCapitalize="characters"
        autoComplete="off"
        spellCheck={false}
        value={value.country}
        onChange={(event) => onChange({ country: event.target.value })}
        className="w-14 shrink-0 uppercase placeholder:normal-case"
      />
      <Input
        aria-label={label("State code, optional")}
        {...invalid("state")}
        placeholder="State"
        maxLength={2}
        autoCapitalize="characters"
        autoComplete="off"
        spellCheck={false}
        value={value.state}
        onChange={(event) => onChange({ state: event.target.value })}
        className="w-16 shrink-0 uppercase placeholder:normal-case"
      />
      <Input
        aria-label={label("City, optional")}
        {...invalid("city")}
        placeholder="City"
        maxLength={60}
        autoComplete="off"
        spellCheck={false}
        value={value.city}
        onChange={(event) => onChange({ city: event.target.value })}
        className={cn(cityClassName)}
      />
    </>
  );
}
