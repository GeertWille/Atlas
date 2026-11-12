import { type ComponentProps, useId, useState } from "react";
import { Input } from "@/shared/ui/input";
import type { Parsed } from "./settings-schema";

const ADORNMENT = "pointer-events-none absolute top-1/2 -translate-y-1/2 text-sm text-subtle-foreground";

/**
 * Text field that saves on blur or Enter. The draft is validated first; an
 * invalid draft stays in the field with its error, and Escape restores the
 * saved value.
 */
export function CommitInput<T extends string | number>({
  value,
  parse,
  format = String,
  onCommit,
  leading,
  trailing,
  ...props
}: Omit<ComponentProps<typeof Input>, "value" | "onChange" | "onBlur"> & {
  value: T;
  parse: (input: string) => Parsed<T>;
  /** How the saved value is shown while the field is not being edited. */
  format?: (value: T) => string;
  onCommit: (value: T) => void;
  /** Unit shown inside the field, before or after the value. */
  leading?: string;
  trailing?: string;
}) {
  const [draft, setDraft] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const errorId = useId();

  const commit = () => {
    if (draft === null) return;
    const result = parse(draft);
    if (!result.ok) {
      setError(result.error);
      return;
    }
    setError(null);
    setDraft(null);
    if (result.value !== value) onCommit(result.value);
  };

  return (
    <div className="flex flex-col items-end gap-1.5">
      <div className="relative">
        {leading ? <span className={`${ADORNMENT} left-2.5`}>{leading}</span> : null}
        {trailing ? <span className={`${ADORNMENT} right-2.5`}>{trailing}</span> : null}
        <Input
          {...props}
          value={draft ?? format(value)}
          aria-invalid={error ? true : undefined}
          aria-describedby={error ? errorId : undefined}
          onChange={(event) => setDraft(event.target.value)}
          onBlur={commit}
          onKeyDown={(event) => {
            if (event.key === "Enter") commit();
            if (event.key === "Escape" && draft !== null) {
              event.stopPropagation();
              setDraft(null);
              setError(null);
            }
          }}
        />
      </div>
      {error ? (
        <p id={errorId} role="alert" className="max-w-56 text-right text-xs text-red-fg">
          {error}
        </p>
      ) : null}
    </div>
  );
}
