import { CheckCircle2Icon } from "lucide-react";
import { toast } from "sonner";
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

function SavedKey() {
  const clearApiKey = useClearApiKey();
  return (
    <div className="flex items-center justify-between gap-4 rounded-lg border border-green-line bg-green-soft/60 px-3.5 py-2.5">
      <p className="flex items-center gap-2 text-[13px] text-green-fg">
        <CheckCircle2Icon className="size-4" />A validated key is stored in your system keychain.
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

export function ApiKeyField({ hasApiKey }: { hasApiKey: boolean }) {
  return hasApiKey ? <SavedKey /> : <ApiKeyForm submitLabel="Validate and save" />;
}
