import { EyeIcon, EyeOffIcon, KeyRoundIcon } from "lucide-react";
import { type FormEvent, useId, useState } from "react";
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
  const errorId = useId();
  const [key, setKey] = useState("");
  const [revealed, setRevealed] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const revealLabel = `${revealed ? "Hide" : "Show"} ${keyNoun}`;

  const save = (event: FormEvent) => {
    event.preventDefault();
    const parsed = apiKeySchema.safeParse(key);
    if (!parsed.success) {
      setError(parsed.error.issues[0]?.message ?? "Enter a valid API key.");
      return;
    }
    setError(null);
    setApiKey.mutate(
      { key: parsed.data, provider },
      {
        onSuccess: (status) => {
          if (status.valid) {
            setKey("");
            toast.success("API key validated and saved to your keychain");
          } else {
            setError(status.message ?? `${name} did not accept this key.`);
          }
        },
        onError: (reason) => notifyError("Could not save the API key", reason),
      },
    );
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
        <Button type="submit" variant={submitVariant} disabled={setApiKey.isPending}>
          {setApiKey.isPending ? "Validating…" : submitLabel}
        </Button>
      </div>
      {error ? (
        <p id={errorId} role="alert" className="text-[13px] text-red-fg">
          {error}
        </p>
      ) : null}
    </form>
  );
}
