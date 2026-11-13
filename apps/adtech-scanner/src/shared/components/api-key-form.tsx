import { EyeIcon, EyeOffIcon, KeyRoundIcon } from "lucide-react";
import { type FormEvent, type ReactNode, useId, useState } from "react";
import { toast } from "sonner";
import { z } from "zod";
import type { ApiProvider } from "@/bindings";
import { useSetApiKey } from "@/lib/query/hooks";
import { notifyError } from "@/shared/lib/notify-error";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { Hint } from "@/shared/ui/tooltip";

const apiKeySchema = z
  .string()
  .trim()
  .min(1, "Paste your API key.")
  .refine((value) => !/\s/.test(value), "An API key cannot contain spaces.");

/** `reason`: why the service refused the key. Without one the failure has been reported elsewhere. */
export type KeySaveResult = { saved: true } | { saved: false; reason?: string };

/**
 * The field every API key is entered in: hidden until revealed, checked for shape, and handed to `onSave`.
 * The field empties once a key is saved, and shows the reason when the service refuses it.
 */
export function KeyForm({
  name,
  placeholder,
  keyNoun = "key",
  submitLabel,
  submitVariant = "default",
  pending,
  onSave,
  children,
}: {
  /** Whose key this is: "Anthropic" makes the field "Anthropic API key". */
  name: string;
  placeholder: string;
  /** What the reveal button calls it, where several keys share a section. */
  keyNoun?: string;
  submitLabel: string;
  submitVariant?: "default" | "outline";
  pending: boolean;
  onSave: (key: string) => Promise<KeySaveResult>;
  /** More to fill in before saving, shown between the field and its error. */
  children?: ReactNode;
}) {
  const errorId = useId();
  const [key, setKey] = useState("");
  const [revealed, setRevealed] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const revealLabel = `${revealed ? "Hide" : "Show"} ${keyNoun}`;

  const save = async (event: FormEvent) => {
    event.preventDefault();
    const parsed = apiKeySchema.safeParse(key);
    if (!parsed.success) {
      setError(parsed.error.issues[0]?.message ?? "Enter a valid API key.");
      return;
    }
    setError(null);
    const result = await onSave(parsed.data);
    if (result.saved) setKey("");
    else if (result.reason) setError(result.reason);
  };

  return (
    <form onSubmit={save} noValidate className="flex flex-col gap-1.5">
      <div className="flex items-center gap-2">
        <div className="relative min-w-0 flex-1">
          <KeyRoundIcon className="pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-subtle-foreground" />
          <Input
            type={revealed ? "text" : "password"}
            autoComplete="off"
            spellCheck={false}
            placeholder={placeholder}
            value={key}
            aria-label={`${name} API key`}
            aria-invalid={error ? true : undefined}
            aria-describedby={error ? errorId : undefined}
            onChange={(event) => {
              setKey(event.target.value);
              setError(null);
            }}
            className="px-8 font-mono"
          />
          <Hint label={revealLabel}>
            <button
              type="button"
              aria-label={revealLabel}
              aria-pressed={revealed}
              onClick={() => setRevealed((current) => !current)}
              className="absolute top-1/2 right-1.5 flex size-6 -translate-y-1/2 items-center justify-center rounded-sm text-subtle-foreground hover:bg-accent hover:text-foreground"
            >
              {revealed ? <EyeOffIcon className="size-4" /> : <EyeIcon className="size-4" />}
            </button>
          </Hint>
        </div>
        <Button type="submit" variant={submitVariant} disabled={pending}>
          {pending ? "Validating…" : submitLabel}
        </Button>
      </div>
      {children}
      {error ? (
        <p id={errorId} role="alert" className="text-[13px] text-red-fg">
          {error}
        </p>
      ) : null}
    </form>
  );
}

const PROVIDER_COPY: Record<ApiProvider, { name: string; placeholder: string; keyNoun: string }> = {
  anthropic: { name: "Anthropic", placeholder: "sk-ant-…", keyNoun: "key" },
  // TypeSafe does not document a key format, so there is no prefix to hint at.
  typesafe: { name: "TypeSafe", placeholder: "Paste your key", keyNoun: "TypeSafe key" },
};

/** Validates an API key against its provider and stores it in the keychain when it is accepted. */
export function ApiKeyForm({
  provider = "anthropic",
  submitLabel,
  submitVariant = "default",
}: {
  provider?: ApiProvider;
  submitLabel: string;
  submitVariant?: "default" | "outline";
}) {
  const { name, placeholder, keyNoun } = PROVIDER_COPY[provider];
  const setApiKey = useSetApiKey();

  const save = async (key: string): Promise<KeySaveResult> => {
    try {
      const status = await setApiKey.mutateAsync({ key, provider });
      if (!status.valid) return { saved: false, reason: status.message ?? `${name} did not accept this key.` };
      toast.success("API key validated and saved to your keychain");
      return { saved: true };
    } catch (reason) {
      notifyError("Could not save the API key", reason);
      return { saved: false };
    }
  };

  return (
    <KeyForm
      name={name}
      placeholder={placeholder}
      keyNoun={keyNoun}
      submitLabel={submitLabel}
      submitVariant={submitVariant}
      pending={setApiKey.isPending}
      onSave={save}
    />
  );
}
