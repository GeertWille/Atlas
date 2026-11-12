import { type FormEvent, useId, useState } from "react";
import { toast } from "sonner";
import type { ScanConfiguration, ScanOptions } from "@/bindings";
import { toApiError } from "@/lib/api";
import { useSaveScanConfiguration } from "@/lib/query/hooks";
import { hostnameOf } from "@/shared/lib/url";
import { Button } from "@/shared/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "@/shared/ui/dialog";
import { Input } from "@/shared/ui/input";
import { Label } from "@/shared/ui/label";

const NAME_MAX = 80;

/** Names the composer's current options and saves them as a new configuration. Mounted while it is open. */
export function SaveConfigurationDialog({
  options,
  onClose,
  onSaved,
}: {
  options: ScanOptions;
  onClose: () => void;
  onSaved: (configuration: ScanConfiguration) => void;
}) {
  const save = useSaveScanConfiguration();
  const nameId = useId();
  const errorId = useId();
  const [name, setName] = useState("");
  const [error, setError] = useState<string | null>(null);

  const submit = (event: FormEvent) => {
    event.preventDefault();
    // The dialog is a portal, but React still bubbles its submit to the composer's form.
    event.stopPropagation();
    const trimmed = name.trim();
    if (trimmed === "") {
      setError("Give the configuration a name.");
      return;
    }
    save.mutate(
      { name: trimmed, options },
      {
        onSuccess: (configuration) => {
          toast.success(`Saved “${configuration.name}”`);
          onClose();
          onSaved(configuration);
        },
        onError: (reason) => setError(toApiError(reason).message),
      },
    );
  };

  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent>
        <DialogTitle>Save as configuration</DialogTitle>
        <DialogDescription>
          Keeps the page, device, visitor, proxy and every other option under one name, so the same scan can be run
          again with one click.
        </DialogDescription>
        <form onSubmit={submit} noValidate className="flex flex-col gap-4">
          <div className="flex flex-col gap-1.5">
            <Label htmlFor={nameId}>Name</Label>
            <Input
              id={nameId}
              autoComplete="off"
              maxLength={NAME_MAX}
              placeholder={`${hostnameOf(options.url).replace(/^www\./, "")} from Belgium`}
              value={name}
              aria-invalid={error ? true : undefined}
              aria-describedby={error ? errorId : undefined}
              onChange={(event) => {
                setName(event.target.value);
                setError(null);
              }}
            />
            {error ? (
              <p id={errorId} role="alert" className="text-[13px] text-red-fg">
                {error}
              </p>
            ) : null}
          </div>
          <div className="flex justify-end gap-2">
            <Button variant="outline" onClick={onClose}>
              Cancel
            </Button>
            <Button type="submit" disabled={save.isPending}>
              Save configuration
            </Button>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  );
}
