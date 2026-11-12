import { useId } from "react";
import { toast } from "sonner";
import type { Settings, SettingsPatch } from "@/bindings";
import { useClearScanProfile, useScanProfile } from "@/lib/query/hooks";
import { formatBytes, formatRelativeTime } from "@/shared/lib/format";
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
import { Field } from "@/shared/ui/field";
import { Switch } from "@/shared/ui/switch";

function ProfileState() {
  const profile = useScanProfile();
  const clear = useClearScanProfile();
  if (!profile.data) return null;
  const { exists, sizeBytes, lastUsedAt, inUse } = profile.data;
  if (!exists) {
    return <p className="text-[13px] text-muted-foreground">No browser profile is stored yet.</p>;
  }
  return (
    <div className="flex w-full items-center justify-between gap-4">
      <p className="text-[13px] text-muted-foreground" aria-live="polite">
        <span className="font-medium text-foreground tabular-nums">{formatBytes(sizeBytes)}</span> on this Mac
        {lastUsedAt === null ? "" : `, last used ${formatRelativeTime(lastUsedAt)}`}
        {inUse ? ". A running scan is using it." : ""}
      </p>
      <AlertDialog>
        <AlertDialogTrigger asChild>
          <Button variant="outline" size="sm" disabled={inUse || clear.isPending}>
            Clear browser profile…
          </Button>
        </AlertDialogTrigger>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Clear the browser profile?</AlertDialogTitle>
            <AlertDialogDescription>
              Its cookies, consent choices and cache are deleted. The next scan starts like a first-time visitor. Your
              own Chrome is not touched.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              destructive
              onClick={() =>
                clear.mutate(undefined, {
                  onSuccess: () => toast.success("Browser profile cleared"),
                  onError: (reason) => notifyError("Could not clear the browser profile", reason),
                })
              }
            >
              Clear profile
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  );
}

/** The switch for a browser profile that survives between scans, and what is stored in it right now. */
export function ScanProfileField({ settings, save }: { settings: Settings; save: (patch: SettingsPatch) => void }) {
  const id = useId();
  const descriptionId = useId();
  return (
    <>
      <Field
        label="Keep the browser profile between scans"
        htmlFor={id}
        description={
          <span id={descriptionId}>
            Cookies and consent persist like a returning visitor's, so a banner accepted once does not come back. Off:
            every scan starts with a fresh profile. Stored only on this Mac, and separate from your own Chrome.
          </span>
        }
      >
        <Switch
          id={id}
          aria-describedby={descriptionId}
          checked={settings.persistentProfile}
          onCheckedChange={(persistentProfile) => save({ persistentProfile })}
        />
      </Field>
      <Field stacked label="Stored browser profile">
        <ProfileState />
      </Field>
    </>
  );
}
