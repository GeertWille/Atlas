import { type ComponentProps, useId, useState } from "react";
import { Input } from "@/shared/ui/input";
import type { Parsed } from "./settings-schema";

/**
 * Text field that saves on blur or Enter. The draft is validated first; an
 * invalid draft stays in the field with its error, and Escape restores the
 * saved value.
 */
export function CommitInput<T extends string | number>({
  value,
  parse,
  onCommit,
  ...props
}: Omit<ComponentProps<typeof Input>, "value" | "onChange" | "onBlur"> & {
  value: T;
  parse: (input: string) => Parsed<T>;
  onCommit: (value: T) => void;
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
      <Input
        {...props}
        value={draft ?? String(value)}
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
      {error ? (
        <p id={errorId} role="alert" className="max-w-56 text-right text-xs text-red-fg">
          {error}
        </p>
      ) : null}
    </div>
  );
}
