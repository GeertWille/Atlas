import { CheckCircle2Icon, EyeIcon, EyeOffIcon, KeyRoundIcon } from "lucide-react";
import { type FormEvent, useId, useState } from "react";
import { toast } from "sonner";
import { useClearApiKey, useSetApiKey } from "@/lib/query/hooks";
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
import { Input } from "@/shared/ui/input";
import { Label } from "@/shared/ui/label";
import { Hint } from "@/shared/ui/tooltip";
import { parseApiKey } from "./settings-schema";

function SavedKey() {
  const clearApiKey = useClearApiKey();
  return (
    <div className="flex items-center justify-between gap-4 rounded-lg border border-green-line bg-green-soft/60 px-3.5 py-2.5">
      <p className="flex items-center gap-2 text-[13px] text-green-fg">
        <CheckCircle2Icon className="size-4" />
        A validated key is stored in your system keychain.
      </p>
      <AlertDialog>
        <AlertDialogTrigger asChild>
          <Button variant="outline" size="sm" disabled={clearApiKey.isPending}>
            Remove key
          </Button>
        </AlertDialogTrigger>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Remove the API key?</AlertDialogTitle>
            <AlertDialogDescription>
              The key is deleted from your keychain. Scans keep working with the deterministic checks only.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              destructive
              onClick={() =>
                clearApiKey.mutate(undefined, {
                  onSuccess: () => toast.success("API key removed"),
                  onError: (reason) => notifyError("Could not remove the API key", reason),
                })
              }
            >
              Remove key
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  );
}

function KeyForm() {
  const setApiKey = useSetApiKey();
  const inputId = useId();
  const errorId = useId();
  const [key, setKey] = useState("");
  const [revealed, setRevealed] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const save = (event: FormEvent) => {
    event.preventDefault();
    const parsed = parseApiKey(key);
    if (!parsed.ok) {
      setError(parsed.error);
      return;
    }
    setError(null);
    setApiKey.mutate(parsed.value, {
      onSuccess: (status) => {
        if (status.valid) {
          setKey("");
          toast.success("API key validated and saved to your keychain");
        } else {
          setError(status.message ?? "Anthropic did not accept this key.");
        }
      },
      onError: (reason) => notifyError("Could not save the API key", reason),
    });
  };

  return (
    <form onSubmit={save} noValidate className="flex flex-col gap-1.5">
      <Label htmlFor={inputId} className="sr-only">
        Anthropic API key
      </Label>
      <div className="flex items-center gap-2">
        <div className="relative min-w-0 flex-1">
          <KeyRoundIcon className="pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-subtle-foreground" />
          <Input
            id={inputId}
            type={revealed ? "text" : "password"}
            autoComplete="off"
            spellCheck={false}
            placeholder="sk-ant-…"
            value={key}
            aria-invalid={error ? true : undefined}
            aria-describedby={error ? errorId : undefined}
            onChange={(event) => {
              setKey(event.target.value);
              setError(null);
            }}
            className="px-8 font-mono"
          />
          <Hint label={revealed ? "Hide key" : "Show key"}>
            <button
              type="button"
              aria-label={revealed ? "Hide key" : "Show key"}
              aria-pressed={revealed}
              onClick={() => setRevealed((current) => !current)}
              className="absolute top-1/2 right-1.5 flex size-6 -translate-y-1/2 items-center justify-center rounded-sm text-subtle-foreground hover:bg-accent hover:text-foreground"
            >
              {revealed ? <EyeOffIcon className="size-4" /> : <EyeIcon className="size-4" />}
            </button>
          </Hint>
        </div>
        <Button type="submit" disabled={setApiKey.isPending}>
          {setApiKey.isPending ? "Validating…" : "Validate and save"}
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

export function ApiKeyField({ hasApiKey }: { hasApiKey: boolean }) {
  return hasApiKey ? <SavedKey /> : <KeyForm />;
}
