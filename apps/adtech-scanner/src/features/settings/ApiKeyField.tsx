import { CheckCircle2Icon } from "lucide-react";
import { toast } from "sonner";
import type { ApiProvider } from "@/bindings";
import { useClearApiKey } from "@/lib/query/hooks";
import { ApiKeyForm } from "@/shared/components/api-key-form";
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

// The two fields sit in one section, so every control of the TypeSafe one names its provider.
const COPY: Record<
  ApiProvider,
  { save: string; stored: string; remove: string; confirmTitle: string; consequence: string }
> = {
  anthropic: {
    save: "Validate and save",
    stored: "A validated key is stored in your system keychain.",
    remove: "Remove key",
    confirmTitle: "Remove the API key?",
    consequence: "Scans keep working with the deterministic checks only.",
  },
  typesafe: {
    save: "Save TypeSafe key",
    stored: "A validated TypeSafe key is stored in your system keychain.",
    remove: "Remove TypeSafe key",
    confirmTitle: "Remove the TypeSafe key?",
    consequence: "Jev is turned off and scans stop sending script excerpts to api.typesafe.ai.",
  },
};

function SavedKey({ provider }: { provider: ApiProvider }) {
  const clearApiKey = useClearApiKey();
  const copy = COPY[provider];
  return (
    <div className="flex items-center justify-between gap-4 rounded-lg border border-green-line bg-green-soft/60 px-3.5 py-2.5">
      <p className="flex items-center gap-2 text-[13px] text-green-fg">
        <CheckCircle2Icon className="size-4 shrink-0" />
        {copy.stored}
      </p>
      <AlertDialog>
        <AlertDialogTrigger asChild>
          <Button variant="outline" size="sm" disabled={clearApiKey.isPending}>
            {copy.remove}
          </Button>
        </AlertDialogTrigger>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{copy.confirmTitle}</AlertDialogTitle>
            <AlertDialogDescription>The key is deleted from your keychain. {copy.consequence}</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              destructive
              onClick={() =>
                clearApiKey.mutate(provider, {
                  onSuccess: () => toast.success("API key removed"),
                  onError: (reason) => notifyError("Could not remove the API key", reason),
                })
              }
            >
              {copy.remove}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  );
}

export function ApiKeyField({ provider, stored }: { provider: ApiProvider; stored: boolean }) {
  return stored ? (
    <SavedKey provider={provider} />
  ) : (
    <ApiKeyForm provider={provider} submitLabel={COPY[provider].save} />
  );
}
